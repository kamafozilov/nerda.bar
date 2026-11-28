//! The background thread: asks each provider for its limits (or DeepSeek for its balance) on a
//! schedule, keeps their history, reads the session logs, and hands all of it to the UI.
//! Nothing here touches the main thread.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::burn::History;
use crate::codex::{self, Redeemed};
use crate::codex_logs::CodexLogs;
use crate::deepseek::{self, Ledger};
use crate::model::{Problem, Provider, Snapshot};
use crate::prices::Prices;
use crate::stats::{Scanner, StatsView};
use crate::{claude, time};

/// Opening the panel refreshes limits older than this many seconds.
const OPEN_REFRESH_AFTER: i64 = 60;
/// A refresh asked for by hand is ignored this soon after the last one.
const MANUAL_REFRESH_AFTER: i64 = 10;
/// The logs are read again when the panel opens and this many seconds have passed.
const OPEN_SCAN_AFTER: i64 = 10;
/// Seconds before trying again when the network is down, and when there is no usable login.
const OFFLINE_RETRY: i64 = 60;
const LOGIN_RETRY: i64 = 120;
const SIGNED_OUT_RETRY: i64 = 1800;
/// Seconds between looks at an expired Claude login. The look is local (the Keychain, no
/// request), so the limits come back within a minute of Claude Code renewing the login.
const EXPIRED_RETRY: i64 = 60;
/// The longest sleep. Waking this often keeps the schedule on the wall clock, so a Mac that
/// slept refreshes within a minute of waking.
const TICK: Duration = Duration::from_secs(60);

pub enum Command {
    /// The panel opened.
    Opened,
    /// Refresh this provider now.
    Refresh(Provider),
    /// Spend a Codex limit reset; the panel asked the person first.
    Redeem { credit: String, request_id: String },
    /// Seconds between refreshes, as the settings now have them.
    Every(i64),
    /// Whether a provider is read at all: one with no menu bar item and no tab is left alone.
    Enabled(Provider, bool),
    /// DeepSeek's key was removed: its balance and history go with it.
    ForgetDeepSeek,
}

pub enum Event {
    /// A request to the provider started.
    Refreshing(Provider),
    Limits(Provider, Result<Snapshot, Problem>),
    Stats(Provider, Arc<StatsView>),
    /// Every limit's readings, after one was added.
    History(Provider, Arc<History>),
    /// DeepSeek's balance and its history.
    Balance(Result<Arc<Ledger>, Problem>),
    /// What spending a reset came to.
    Redeemed(Redeemed),
}

/// What survives a relaunch: the last limits, and a pause the provider asked for.
#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Cache {
    pub snapshot: Option<Snapshot>,
    pub paused_until: i64,
}

impl Cache {
    fn file(provider: Provider) -> String {
        format!("{}.json", provider.id())
    }

    pub fn load(dir: &Path, provider: Provider) -> Cache {
        std::fs::read_to_string(dir.join(Cache::file(provider))).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
    }

    fn save(&self, dir: &Path, provider: Provider) {
        if std::fs::create_dir_all(dir).is_ok()
            && let Ok(text) = serde_json::to_string(self)
        {
            let _ = crate::config::write_file(dir, &Cache::file(provider), &text);
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

/// A provider's session logs.
enum Logs {
    Claude(Scanner),
    Codex(CodexLogs),
}

impl Logs {
    fn scan(&mut self, now: i64) -> bool {
        match self {
            Logs::Claude(scanner) => scanner.scan(now),
            Logs::Codex(logs) => logs.scan(now),
        }
    }

    fn view(&self, prices: &Prices, now: i64) -> StatsView {
        match self {
            Logs::Claude(scanner) => scanner.view(prices, now),
            Logs::Codex(logs) => logs.view(prices, now),
        }
    }
}

/// One provider's schedule and what it keeps.
struct Job {
    provider: Provider,
    cache: Cache,
    history: History,
    logs: Option<Logs>,
    ledger: Ledger,
    next_refresh: i64,
    last_refresh: i64,
    last_scan: i64,
    scan_due: bool,
    /// The local day the last counts handed over were for: a new day changes what "today" is
    /// with no new message.
    stats_day: i32,
    /// Off while the settings show the provider nowhere.
    enabled: bool,
}

impl Job {
    fn new(provider: Provider, dir: &Path) -> Job {
        Job {
            provider,
            cache: Cache::load(dir, provider),
            history: history_file(provider).map(|file| History::load(dir, file)).unwrap_or_default(),
            logs: logs(provider),
            ledger: if provider == Provider::DeepSeek { Ledger::load(dir) } else { Ledger::default() },
            next_refresh: 0,
            last_refresh: 0,
            last_scan: 0,
            scan_due: true,
            stats_day: i32::MIN,
            enabled: true,
        }
    }

    /// When the provider is next asked: never while it is off.
    fn due(&self) -> i64 {
        if self.enabled { self.next_refresh.max(self.cache.paused_until) } else { i64::MAX }
    }
}

/// The file a provider's limit readings are kept in.
fn history_file(provider: Provider) -> Option<&'static str> {
    match provider {
        Provider::Claude => Some(crate::burn::FILE),
        Provider::Codex => Some("codex-history.json"),
        Provider::DeepSeek => None,
    }
}

fn logs(provider: Provider) -> Option<Logs> {
    match provider {
        Provider::Claude => Some(Logs::Claude(Scanner::new(&claude::config_dirs()))),
        Provider::Codex => Some(Logs::Codex(CodexLogs::new(&codex::home()))),
        Provider::DeepSeek => None,
    }
}

pub fn spawn(data_dir: PathBuf, refresh_every: i64, enabled: [bool; 3], on_event: impl Fn(Event) + Send + 'static) -> Worker {
    let (commands, inbox) = mpsc::channel::<Command>();
    let run = move || {
        let agent = claude::agent();
        let mut refresh_every = refresh_every;
        let mut jobs: Vec<Job> = Provider::ALL.iter().map(|&provider| Job::new(provider, &data_dir)).collect();
        for (job, enabled) in jobs.iter_mut().zip(enabled) {
            job.enabled = enabled;
        }
        for job in &jobs {
            if job.provider != Provider::DeepSeek {
                on_event(Event::History(job.provider, Arc::new(job.history.clone())));
            } else if job.ledger.last.is_some() {
                on_event(Event::Balance(Ok(Arc::new(job.ledger.clone()))));
            }
        }
        let mut prices = Prices::load(&data_dir);
        loop {
            let now = time::unix_now();
            for job in &mut jobs {
                if !job.enabled {
                    continue;
                }
                if now >= job.due() {
                    refresh(job, &agent, &data_dir, refresh_every, now, &on_event);
                }
                // A new local day changes today's counts with nothing new in the logs.
                if job.scan_due || (job.logs.is_some() && time::local(now).day != job.stats_day) {
                    scan(job, &prices, now, &on_event);
                }
            }
            // After the first counts are on screen, so a slow catalog never delays them.
            if prices.stale(now) {
                match Prices::fetch(&agent, &data_dir, now) {
                    Ok(fresh) => {
                        crate::debug(|| format!("prices for {} models", fresh.models.len()));
                        prices = fresh;
                        for job in jobs.iter().filter(|job| job.enabled) {
                            if let Some(logs) = &job.logs {
                                on_event(Event::Stats(job.provider, Arc::new(logs.view(&prices, now))));
                            }
                        }
                    }
                    // Offline or the catalog is down: don't ask again on every pass.
                    Err(_) => prices.fetched_at = now - 86_400 + 1800,
                }
            }

            let now = time::unix_now();
            let next = jobs.iter().map(Job::due).min().unwrap_or(i64::MAX);
            let wait = next.saturating_sub(now).clamp(1, TICK.as_secs() as i64);
            match inbox.recv_timeout(Duration::from_secs(wait as u64)) {
                Ok(Command::Redeem { credit, request_id }) => {
                    let redeemed = codex::redeem(&agent, &credit, &request_id);
                    crate::debug(|| format!("redeem: {redeemed:?}"));
                    on_event(Event::Redeemed(redeemed));
                    // Whatever came of it, the limits and the resets left show the truth.
                    if let Some(job) = jobs.iter_mut().find(|job| job.provider == Provider::Codex) {
                        job.next_refresh = time::unix_now();
                    }
                }
                Ok(Command::Every(seconds)) => {
                    // Sooner than planned when the interval shrank; never later than it now is.
                    let now = time::unix_now();
                    refresh_every = seconds;
                    for job in &mut jobs {
                        job.next_refresh = job.next_refresh.min(now + seconds);
                    }
                }
                Ok(Command::Enabled(provider, enabled)) => {
                    if let Some(job) = jobs.iter_mut().find(|job| job.provider == provider && job.enabled != enabled) {
                        job.enabled = enabled;
                        // Back on: what it missed is read at once.
                        job.next_refresh = 0;
                        job.scan_due = true;
                    }
                }
                Ok(Command::ForgetDeepSeek) => {
                    if let Some(job) = jobs.iter_mut().find(|job| job.provider == Provider::DeepSeek) {
                        job.ledger = Ledger::default();
                        job.ledger.save(&data_dir);
                        job.cache.paused_until = 0;
                        job.next_refresh = 0;
                    }
                }
                Ok(command) => {
                    let now = time::unix_now();
                    for job in &mut jobs {
                        // A clock set back leaves the last refresh in the future: that is due too.
                        let since = now - job.last_refresh;
                        let wanted = match command {
                            Command::Opened => !(0..OPEN_REFRESH_AFTER).contains(&since),
                            Command::Refresh(provider) => provider == job.provider && !(0..MANUAL_REFRESH_AFTER).contains(&since),
                            _ => false,
                        };
                        if wanted {
                            job.next_refresh = now;
                        }
                        let scanned = now - job.last_scan;
                        job.scan_due |= matches!(command, Command::Refresh(p) if p == job.provider) || !(0..OPEN_SCAN_AFTER).contains(&scanned);
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    };
    std::thread::Builder::new().name("nerda-bar-worker".to_owned()).spawn(run).expect("the worker thread starts");
    Worker { commands }
}

fn refresh(job: &mut Job, agent: &ureq::Agent, dir: &Path, refresh_every: i64, now: i64, on_event: &impl Fn(Event)) {
    let provider = job.provider;
    on_event(Event::Refreshing(provider));
    job.last_refresh = now;
    job.scan_due = true;
    let retry = |problem: &Problem| match problem {
        Problem::LoginExpired if provider == Provider::Claude => EXPIRED_RETRY,
        Problem::Failed(_) | Problem::LoginExpired => refresh_every,
        Problem::RateLimited { until } => until - now,
        Problem::Offline(_) => OFFLINE_RETRY,
        // Claude's login is the one most people have; the others are looked for again when
        // the panel opens, and otherwise rarely.
        Problem::SignedOut if provider == Provider::Claude => LOGIN_RETRY,
        Problem::SignedOut => SIGNED_OUT_RETRY,
    };
    if provider == Provider::DeepSeek {
        let result = deepseek::fetch(agent);
        crate::debug(|| format!("deepseek: {result:?}"));
        job.next_refresh = now + result.as_ref().err().map_or(refresh_every, retry);
        // A pause DeepSeek asked for holds when the panel opens, as Claude's and Codex's do. It
        // is kept in memory only: `deepseek.json` is the balance's history.
        job.cache.paused_until = match &result {
            Err(Problem::RateLimited { until }) => *until,
            _ => 0,
        };
        match result {
            Ok(balance) => {
                job.ledger.record(balance);
                job.ledger.save(dir);
                on_event(Event::Balance(Ok(Arc::new(job.ledger.clone()))));
            }
            Err(problem) => on_event(Event::Balance(Err(problem))),
        }
        return;
    }
    let result = match provider {
        Provider::Codex => codex::fetch(agent),
        _ => claude::fetch(agent),
    };
    crate::debug(|| match &result {
        Ok(snapshot) => format!("{}: {} ({} limits)", provider.id(), snapshot.plan, snapshot.limits.len()),
        Err(problem) => format!("{}: {problem:?}", provider.id()),
    });
    job.next_refresh = now + result.as_ref().err().map_or(refresh_every, retry);
    match &result {
        Ok(snapshot) => {
            job.cache = Cache { snapshot: Some(snapshot.clone()), paused_until: 0 };
            job.cache.save(dir, provider);
            job.history.record(snapshot);
            if let Some(file) = history_file(provider) {
                job.history.save(dir, file);
            }
            on_event(Event::History(provider, Arc::new(job.history.clone())));
        }
        Err(Problem::RateLimited { until }) => {
            job.cache.paused_until = *until;
            job.cache.save(dir, provider);
        }
        Err(_) => {}
    }
    on_event(Event::Limits(provider, result));
}

fn scan(job: &mut Job, prices: &Prices, now: i64, on_event: &impl Fn(Event)) {
    job.scan_due = false;
    let Some(logs) = &mut job.logs else { return };
    let today = time::local(now).day;
    let new_day = std::mem::replace(&mut job.stats_day, today) != today;
    job.last_scan = now;
    let started = std::time::Instant::now();
    let changed = logs.scan(now);
    crate::debug(|| format!("{} logs read in {:.0} ms, changed: {changed}", job.provider.id(), started.elapsed().as_secs_f64() * 1e3));
    if changed || new_day {
        on_event(Event::Stats(job.provider, Arc::new(logs.view(prices, now))));
    }
}
