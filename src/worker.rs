//! The background thread: asks the provider for its limits on a schedule, keeps their history,
//! reads the session logs, and hands all of it to the UI. Nothing here touches the main thread.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::burn::History;
use crate::claude;
use crate::model::{Problem, Snapshot};
use crate::prices::Prices;
use crate::stats::{Scanner, StatsView};
use crate::time;

/// Opening the panel refreshes limits older than this many seconds.
const OPEN_REFRESH_AFTER: i64 = 60;
/// A refresh asked for by hand is ignored this soon after the last one.
const MANUAL_REFRESH_AFTER: i64 = 10;
/// The logs are read again when the panel opens and this many seconds have passed.
const OPEN_SCAN_AFTER: i64 = 10;
/// Seconds before trying again when the network is down, and when there is no usable login.
const OFFLINE_RETRY: i64 = 60;
const LOGIN_RETRY: i64 = 120;
/// The longest sleep. Waking this often keeps the schedule on the wall clock, so a Mac that
/// slept refreshes within a minute of waking.
const TICK: Duration = Duration::from_secs(60);
const CACHE_FILE: &str = "claude.json";

pub enum Command {
    /// The panel opened.
    Opened,
    /// Refresh now.
    Refresh,
}

pub enum Event {
    /// A request to the provider started.
    Refreshing,
    Limits(Result<Snapshot, Problem>),
    Stats(Arc<StatsView>),
    /// Every limit's readings, after one was added.
    History(Arc<History>),
}

/// What survives a relaunch: the last limits, and a pause the provider asked for.
#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Cache {
    pub snapshot: Option<Snapshot>,
    pub paused_until: i64,
}

impl Cache {
    pub fn load(dir: &Path) -> Cache {
        std::fs::read_to_string(dir.join(CACHE_FILE)).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
    }

    fn save(&self, dir: &Path) {
        if std::fs::create_dir_all(dir).is_ok()
            && let Ok(text) = serde_json::to_string(self)
        {
            let _ = std::fs::write(dir.join(CACHE_FILE), text);
        }
    }
}

pub struct Worker {
    commands: Sender<Command>,
}

impl Worker {
    pub fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }
}

pub fn spawn(data_dir: PathBuf, refresh_every: i64, on_event: impl Fn(Event) + Send + 'static) -> Worker {
    let (commands, inbox) = mpsc::channel::<Command>();
    let run = move || {
        let agent = claude::agent();
        let mut cache = Cache::load(&data_dir);
        let mut history = History::load(&data_dir);
        on_event(Event::History(Arc::new(history.clone())));
        let mut prices = Prices::load(&data_dir);
        let mut scanner = Scanner::new(&claude::config_dirs());
        let (mut next_refresh, mut last_refresh, mut last_scan) = (0i64, 0i64, 0i64);
        let mut scan_due = true;
        loop {
            let now = time::unix_now();
            if now >= next_refresh.max(cache.paused_until) {
                on_event(Event::Refreshing);
                let result = claude::fetch(&agent);
                crate::debug(|| match &result {
                    Ok(snapshot) => format!("limits: {} ({} limits)", snapshot.plan, snapshot.limits.len()),
                    Err(problem) => format!("limits: {problem:?}"),
                });
                last_refresh = now;
                next_refresh = now
                    + match &result {
                        Ok(_) | Err(Problem::Failed(_) | Problem::LoginExpired) => refresh_every,
                        Err(Problem::RateLimited { until }) => until - now,
                        Err(Problem::Offline(_)) => OFFLINE_RETRY,
                        Err(Problem::SignedOut) => LOGIN_RETRY,
                    };
                match &result {
                    Ok(snapshot) => {
                        cache = Cache { snapshot: Some(snapshot.clone()), paused_until: 0 };
                        cache.save(&data_dir);
                        history.record(snapshot);
                        history.save(&data_dir);
                        on_event(Event::History(Arc::new(history.clone())));
                    }
                    Err(Problem::RateLimited { until }) => {
                        cache.paused_until = *until;
                        cache.save(&data_dir);
                    }
                    Err(_) => {}
                }
                on_event(Event::Limits(result));
                scan_due = true;
            }
            if scan_due {
                scan_due = false;
                let first = last_scan == 0;
                last_scan = now;
                let started = std::time::Instant::now();
                let changed = scanner.scan(now);
                crate::debug(|| format!("logs read in {:.0} ms, changed: {changed}", started.elapsed().as_secs_f64() * 1e3));
                crate::debug(|| {
                    let view = scanner.view(&prices, now);
                    let line = |range: &crate::stats::RangeView| format!("{:?} = {} tokens, {}", range.tokens, range.total(), range.cost_label());
                    format!("today {}; 7d {}; 30d {}", line(&view.ranges[0]), line(&view.ranges[1]), line(&view.ranges[2]))
                });
                if changed || first {
                    on_event(Event::Stats(Arc::new(scanner.view(&prices, now))));
                }
                // After the first counts are on screen, so a slow catalog never delays them.
                if prices.stale(now)
                    && let Ok(fresh) = Prices::fetch(&agent, &data_dir, now)
                {
                    crate::debug(|| format!("prices for {} models", fresh.models.len()));
                    prices = fresh;
                    on_event(Event::Stats(Arc::new(scanner.view(&prices, now))));
                } else if prices.stale(now) {
                    // Offline or the catalog is down: don't ask again on every scan.
                    prices.fetched_at = now - 86_400 + 1800;
                }
            }

            let now = time::unix_now();
            let wait = (next_refresh.max(cache.paused_until) - now).clamp(1, TICK.as_secs() as i64);
            match inbox.recv_timeout(Duration::from_secs(wait as u64)) {
                Ok(command) => {
                    let now = time::unix_now();
                    let since = now - last_refresh;
                    let wanted = match command {
                        Command::Opened => since >= OPEN_REFRESH_AFTER,
                        Command::Refresh => since >= MANUAL_REFRESH_AFTER,
                    };
                    if wanted {
                        next_refresh = now;
                    }
                    scan_due = matches!(command, Command::Refresh) || now - last_scan >= OPEN_SCAN_AFTER;
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    };
    std::thread::Builder::new().name("nerda-bar-worker".to_owned()).spawn(run).expect("the worker thread starts");
    Worker { commands }
}
