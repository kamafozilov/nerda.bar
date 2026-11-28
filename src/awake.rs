//! The Sleep tab's behaviour: keeping this Mac awake for a while or until it is turned off,
//! with its display on or not and, on a notebook, with its lid closed; changing the idle
//! timers; and putting the display, the Mac or the screen to sleep now.
//!
//! A keep-awake is a power assertion that macOS itself lets go of when its time is up or the
//! app quits, so nothing here needs to wake to end it. While one with an end runs, a thread
//! wakes once a minute to move the menu bar item's countdown on. The tab reads sleep's state
//! when it opens and with the panel's tick while it shows; nothing is read in the background.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use rune::Tx;

use crate::status_item;
use crate::system::power::{self, Hold, PowerView, Setting, Source};
use crate::time;
use crate::ui::{self, Page, Phase, Tab, Ui};

/// The longest the lid's watcher takes to turn sleep back on once its flag is gone, with room.
const SETTLE_SECONDS: i64 = 5;

/// What the durations offer, in minutes: 0 keeps the Mac awake until it is turned off.
pub const DURATIONS: [(u32, &str); 6] = [(15, "15m"), (30, "30m"), (60, "1h"), (120, "2h"), (300, "5h"), (0, "No limit")];

/// Keeping the Mac awake, from when it started until when it ends.
#[derive(Debug)]
pub struct Session {
    pub started: i64,
    /// `None` until it is turned off.
    pub until: Option<i64>,
    /// The duration picked, from [`DURATIONS`].
    pub minutes: u32,
    /// The assertion; `None` in a headless render, or when macOS refused it.
    hold: Option<Hold>,
    pub lid: LidHold,
}

/// Staying awake with the lid closed, which needs a password.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LidHold {
    Off,
    /// macOS's password dialog is open.
    Asking,
    On,
}

/// A change that waits on macOS's password dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Asking {
    Timer(Source, Setting),
    AllowSleep,
}

#[derive(Default)]
pub struct Awake {
    pub session: Option<Session>,
    /// The last reading of sleep's state.
    pub view: Option<Arc<PowerView>>,
    /// The source whose idle timers show; the one the Mac runs on until another is picked.
    pub source: Option<Source>,
    pub asking: Option<Asking>,
    /// Why the last change didn't happen, until the next one.
    pub error: Option<String>,
    /// An earlier run turned sleep off for the lid and couldn't turn it back on (the Mac was
    /// shut down with it on): this Mac sleeps for nothing until it is.
    pub left_off: bool,
    /// When this app last let go of the lid, in Unix seconds: its watcher takes a moment to
    /// turn sleep back on, and until then sleep being off is no cause for alarm.
    lid_released: Option<i64>,
    /// Counts sessions, so a minute's tick for one that ended does nothing.
    generation: Arc<AtomicU64>,
}

impl std::fmt::Debug for Awake {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Awake").field("session", &self.session).field("asking", &self.asking).field("error", &self.error).finish()
    }
}

/// Where the lid's watcher looks: while this file is there, sleep stays off.
fn lid_flag(s: &Ui) -> PathBuf {
    s.data_dir.join("lid-awake")
}

impl Awake {
    /// The state at launch: an earlier run's lid flag still there means its watcher never
    /// turned sleep back on.
    pub fn new(data_dir: &std::path::Path, demo: bool) -> Awake {
        let mut awake = Awake::default();
        if demo {
            awake.view = Some(Arc::new(power::demo(time::unix_now())));
            return awake;
        }
        let flag = data_dir.join("lid-awake");
        if flag.exists() {
            if power::read_state().sleep_disabled {
                // Perhaps: the watcher of a run that ended a moment ago (an update, a crash)
                // may be about to turn sleep back on. `confirm_left_off` looks again.
                awake.left_off = true;
                awake.lid_released = Some(time::unix_now());
            } else {
                let _ = std::fs::remove_file(flag);
            }
        }
        awake
    }

    /// The seconds left of a session with an end.
    pub fn left(&self, now: i64) -> Option<i64> {
        self.session.as_ref().and_then(|session| session.until).map(|until| (until - now).max(0))
    }

    /// Whether sleep is off with nothing of this app's asking for that: an earlier run left it
    /// so, or it was turned off elsewhere. The Mac then sleeps for nothing, its lid included,
    /// until it is turned back on.
    pub fn sleep_stuck_off(&self, now: i64) -> bool {
        let ours = self.session.as_ref().is_some_and(|session| session.lid != LidHold::Off);
        let settling = self.lid_released.is_some_and(|at| (0..SETTLE_SECONDS).contains(&(now - at)));
        !ours && !settling && (self.left_off || self.view.as_ref().is_some_and(|view| view.sleep_disabled))
    }

    /// The source whose timers show.
    pub fn shown_source(&self) -> Source {
        let view = self.view.as_deref();
        match self.source {
            Some(Source::Battery) if view.is_some_and(|v| v.battery.is_some()) => Source::Battery,
            Some(Source::Adapter) => Source::Adapter,
            _ => view.map_or(Source::Adapter, PowerView::source),
        }
    }
}

/// A keep-awake as it is kept on disk while it runs, so that one cut short by an update or a
/// crash of this app picks up where it was when the app is back.
#[derive(serde::Serialize, serde::Deserialize)]
struct Kept {
    started: i64,
    until: Option<i64>,
    minutes: u32,
}

const KEPT_FILE: &str = "awake.json";

/// Writes the running session to disk, or takes the file away when none runs.
fn remember(s: &Ui) {
    if s.demo {
        return;
    }
    match &s.awake.session {
        Some(session) => {
            let kept = Kept { started: session.started, until: session.until, minutes: session.minutes };
            if let Ok(text) = serde_json::to_string(&kept) {
                let _ = crate::config::write_file(&s.data_dir, KEPT_FILE, &text);
            }
        }
        None => forget(&s.data_dir),
    }
}

/// Forgets the session kept on disk: the app is being quit, which ends a keep-awake for good.
pub fn forget(data_dir: &std::path::Path) {
    let _ = std::fs::remove_file(data_dir.join(KEPT_FILE));
}

/// At launch, when an earlier run's lid flag is still there with sleep off: looks again once
/// that run's watcher has had its time to turn sleep back on, and only then says in the menu
/// bar that sleep was left off.
pub fn confirm_left_off(s: &Ui) {
    if s.demo || !s.awake.left_off {
        return;
    }
    let me = s.me.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(SETTLE_SECONDS as u64 + 1));
        let disabled = power::read_state().sleep_disabled;
        me.update(move |s, tx| {
            tx.instant();
            s.now = time::unix_now();
            if !disabled && s.awake.left_off {
                s.awake.left_off = false;
                let _ = std::fs::remove_file(lid_flag(s));
            }
            sync_item(s);
        });
    });
}

/// At launch: takes up the keep-awake that ran when this app last stopped without being quit
/// (an update replaced it, or it crashed), with the end it had. Not one from before the Mac
/// last started, nor one whose time has passed. The lid is not held again: that would ask for
/// a password nobody is waiting to give.
pub fn resume(s: &mut Ui) {
    if s.demo || s.awake.session.is_some() {
        return;
    }
    let kept = std::fs::read_to_string(s.data_dir.join(KEPT_FILE)).ok().and_then(|text| serde_json::from_str::<Kept>(&text).ok());
    match kept.filter(|kept| kept.resumable(power::booted_at(), time::unix_now())) {
        Some(kept) => begin(s, kept.minutes, Some((kept.started, kept.until))),
        None => forget(&s.data_dir),
    }
}

impl Kept {
    /// Whether the session is still one to take up at `now`, on a Mac that started at `boot`.
    fn resumable(&self, boot: Option<i64>, now: i64) -> bool {
        boot.is_some_and(|boot| self.started >= boot) && self.started <= now && self.until.is_none_or(|until| until > now)
    }
}

/// Keeps the Mac awake for `minutes` from now, or until turned off with 0. A session already
/// running starts over with the new length.
pub fn start(s: &mut Ui, minutes: u32) {
    begin(s, minutes, None);
}

/// Starts a session: a new one of `minutes`, or `resumed`, one that ran before with the start
/// and the end it had.
fn begin(s: &mut Ui, minutes: u32, resumed: Option<(i64, Option<i64>)>) {
    let now = time::unix_now();
    let (started, until) = resumed.unwrap_or((now, (minutes > 0).then(|| now + minutes as i64 * 60)));
    let lid_was = s.awake.session.as_ref().map(|session| session.lid);
    // The old assertion goes before the new one is made, so the two never overlap in the list.
    s.awake.session = None;
    let generation = s.awake.generation.fetch_add(1, Ordering::SeqCst) + 1;
    let hold = if s.demo { None } else { power::hold(s.config.sleep.display, until.map(|until| (until - now).max(1))) };
    s.awake.session = Some(Session { started, until, minutes, hold, lid: LidHold::Off });
    s.awake.error = None;
    // The countdown reads the panel's clock: it starts from this moment, not the last tick.
    s.now = now;
    if resumed.is_some() {
        // Its lid hold, if it had one, ended with the app that made it.
    } else if s.config.sleep.lid && has_lid(s) {
        match lid_was {
            // The watcher of the session before keeps running: it ends with the flag or the
            // app, not with a time, so it carries over without another password.
            Some(lid @ (LidHold::On | LidHold::Asking)) => set_lid_hold(s, lid),
            _ => ask_lid(s),
        }
    } else if lid_was.is_some_and(|lid| lid != LidHold::Off) {
        release_lid(s);
    }
    if let Some(until) = until {
        tick_until(s, generation, until);
    }
    remember(s);
    read_soon(s);
    sync_item(s);
}

/// Lets the Mac sleep as it is set to again.
pub fn stop(s: &mut Ui, _tx: &mut Tx) {
    end(s);
}

fn end(s: &mut Ui) {
    s.now = time::unix_now();
    s.awake.generation.fetch_add(1, Ordering::SeqCst);
    if let Some(session) = s.awake.session.take()
        && session.lid != LidHold::Off
    {
        release_lid(s);
    }
    remember(s);
    read_soon(s);
    sync_item(s);
}

/// Picks a duration: Off ends the session, any other starts one.
pub fn pick(s: &mut Ui, minutes: Option<u32>) {
    match minutes {
        Some(minutes) => start(s, minutes),
        None => end(s),
    }
}

/// Whether a session keeps the display on too, or only the Mac. A running session swaps its
/// assertion and keeps its end.
pub fn set_display(s: &mut Ui, on: bool) {
    s.config.sleep.display = on;
    save(s);
    let now = time::unix_now();
    s.now = now;
    if let Some(session) = &mut s.awake.session {
        session.hold = None;
        if !s.demo {
            session.hold = power::hold(on, session.until.map(|until| (until - now).max(1)));
        }
    }
    read_soon(s);
}

/// Whether a session also keeps the Mac awake with its lid closed. It asks for the password
/// when a session runs, and otherwise when the next one starts.
pub fn set_lid(s: &mut Ui, on: bool) {
    s.config.sleep.lid = on;
    s.awake.error = None;
    save(s);
    let Some(lid) = s.awake.session.as_ref().map(|session| session.lid) else { return };
    match (on, lid) {
        (true, LidHold::Off) => ask_lid(s),
        (false, LidHold::On | LidHold::Asking) => {
            release_lid(s);
            set_lid_hold(s, LidHold::Off);
            read_soon(s);
        }
        _ => {}
    }
}

/// Lets go of the lid: the watcher turns sleep back on within a couple of seconds, and the
/// tab reads again once it has.
fn release_lid(s: &mut Ui) {
    if s.demo {
        if let Some(view) = &mut s.awake.view {
            Arc::make_mut(view).sleep_disabled = false;
        }
        return;
    }
    power::release_lid(&lid_flag(s));
    s.awake.lid_released = Some(time::unix_now());
    let me = s.me.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(SETTLE_SECONDS as u64));
        let view = Arc::new(power::read_state());
        me.update(move |s, tx| {
            if !(s.phase == Phase::Shown && s.tab == Tab::Sleep) {
                tx.instant();
            }
            s.now = time::unix_now();
            keep_timers(s, view);
            sync_item(s);
        });
    });
}

fn set_lid_hold(s: &mut Ui, lid: LidHold) {
    if let Some(session) = &mut s.awake.session {
        session.lid = lid;
    }
}

/// Asks for the password that turns sleep off for the lid, on a thread of its own: the dialog
/// waits for the person.
fn ask_lid(s: &mut Ui) {
    if s.demo {
        set_lid_hold(s, LidHold::On);
        return;
    }
    set_lid_hold(s, LidHold::Asking);
    let (me, flag) = (s.me.clone(), lid_flag(s));
    std::thread::spawn(move || {
        let result = power::hold_lid(&flag, std::process::id());
        me.update(move |s, tx| {
            tx.instant();
            let wanted = s.config.sleep.lid && s.awake.session.as_ref().is_some_and(|session| session.lid == LidHold::Asking);
            match result {
                Ok(()) if wanted => set_lid_hold(s, LidHold::On),
                // The session ended, or the switch went off, while the dialog was open.
                Ok(()) => release_lid(s),
                Err(reason) => {
                    set_lid_hold(s, LidHold::Off);
                    // The switch says what is so: without the password, the lid sleeps the Mac.
                    s.config.sleep.lid = false;
                    save(s);
                    if reason != "Cancelled" {
                        s.awake.error = Some(format!("Couldn't keep it awake with the lid closed: {reason}"));
                    }
                }
            }
            read_soon(s);
        });
    });
}

/// Wakes once a minute while a session with an end runs, for the menu bar's countdown, and at
/// its end, which macOS has already acted on by then.
fn tick_until(s: &Ui, generation: u64, until: i64) {
    if s.demo {
        return;
    }
    let (me, gens) = (s.me.clone(), s.awake.generation.clone());
    std::thread::spawn(move || {
        loop {
            let left = until - time::unix_now();
            // To the moment the minutes shown change, and no later than the end.
            let wait = if left <= 0 { 0 } else { (left % 60).max(1).min(left) };
            std::thread::sleep(std::time::Duration::from_secs(wait.max(1) as u64));
            if gens.load(Ordering::SeqCst) != generation {
                return;
            }
            let over = time::unix_now() >= until;
            me.update(move |s, tx| {
                if !(s.phase == Phase::Shown && s.page == Page::Tab && s.tab == Tab::Sleep) {
                    tx.instant();
                }
                if s.awake.generation.load(Ordering::SeqCst) != generation {
                    return;
                }
                s.now = time::unix_now();
                if over {
                    end(s);
                } else {
                    sync_item(s);
                }
            });
            if over {
                return;
            }
        }
    });
}

/// Reads sleep's state again, the timers too, off the main thread.
pub fn read(s: &Ui) {
    if s.demo {
        return;
    }
    let me = s.me.clone();
    std::thread::spawn(move || {
        let view = Arc::new(power::read());
        me.update(move |s, tx| apply(s, view, tx));
    });
}

/// Reads again once what just changed has settled: an assertion shows in the list a moment
/// after it is made.
fn read_soon(s: &Ui) {
    if s.demo || s.tab != Tab::Sleep {
        return;
    }
    let me = s.me.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(150));
        let view = Arc::new(power::read());
        me.update(move |s, tx| apply(s, view, tx));
    });
}

/// What changes moment to moment, read on the panel's tick while the tab shows; the timers
/// keep their last reading.
pub fn refresh_state(s: &mut Ui) {
    if s.demo {
        return;
    }
    keep_timers(s, Arc::new(power::read_state()));
}

/// Takes a reading made without the timers, which keep their last reading.
fn keep_timers(s: &mut Ui, mut state: Arc<PowerView>) {
    if let Some(view) = &s.awake.view {
        let state = Arc::make_mut(&mut state);
        (state.battery, state.adapter, state.timers_read) = (view.battery.clone(), view.adapter.clone(), view.timers_read);
    }
    apply_view(s, state);
}

fn apply(s: &mut Ui, view: Arc<PowerView>, tx: &mut Tx) {
    if !(s.phase == Phase::Shown && s.tab == Tab::Sleep) {
        tx.instant();
    }
    apply_view(s, view);
}

fn apply_view(s: &mut Ui, view: Arc<PowerView>) {
    // The earlier run's sleep-off is gone once sleep is back on, whoever turned it on.
    let was_left_off = s.awake.left_off;
    if s.awake.left_off && !view.sleep_disabled {
        s.awake.left_off = false;
        let _ = std::fs::remove_file(lid_flag(s));
    }
    s.awake.view = Some(view);
    if was_left_off != s.awake.left_off {
        sync_item(s);
    }
}

/// Shows `source`'s idle timers.
pub fn set_source(s: &mut Ui, source: Source) {
    s.awake.source = Some(source);
}

/// Changes an idle timer of `source`, after macOS's password dialog.
pub fn set_timer(s: &mut Ui, source: Source, setting: Setting) {
    let Some(now) = s.awake.view.as_ref().and_then(|view| view.timers(source).cloned()) else { return };
    if s.awake.asking.is_some() {
        return;
    }
    s.awake.error = None;
    if s.demo {
        let mut view = s.awake.view.as_deref().cloned().unwrap_or_default();
        let timers = match source {
            Source::Battery => view.battery.get_or_insert_with(Default::default),
            Source::Adapter => &mut view.adapter,
        };
        match setting {
            Setting::Display(m) => timers.display = Some(m),
            Setting::Sleep(m) => timers.sleep = Some(m),
            Setting::LowPower(on) => timers.low_power = Some(on),
            Setting::WakeNetwork(on) => timers.wake_network = Some(on),
            Setting::PowerNap(on) => timers.power_nap = Some(on),
        }
        s.awake.view = Some(Arc::new(view));
        return;
    }
    s.awake.asking = Some(Asking::Timer(source, setting));
    let me = s.me.clone();
    std::thread::spawn(move || {
        let result = power::set(source, setting, &now);
        let view = Arc::new(power::read());
        me.update(move |s, tx| {
            tx.instant();
            s.awake.asking = None;
            if let Err(reason) = result
                && reason != "Cancelled"
            {
                s.awake.error = Some(format!("Couldn't change it: {reason}"));
            }
            apply_view(s, view);
        });
    });
}

/// Turns sleep back on after an earlier run left it off.
pub fn allow_sleep(s: &mut Ui) {
    if s.demo {
        s.awake.left_off = false;
        if let Some(view) = &mut s.awake.view {
            Arc::make_mut(view).sleep_disabled = false;
        }
        return;
    }
    if s.awake.asking.is_some() {
        return;
    }
    s.awake.asking = Some(Asking::AllowSleep);
    let (me, flag) = (s.me.clone(), lid_flag(s));
    std::thread::spawn(move || {
        let result = power::allow_sleep(&flag);
        let view = Arc::new(power::read());
        me.update(move |s, tx| {
            tx.instant();
            s.awake.asking = None;
            if let Err(reason) = result
                && reason != "Cancelled"
            {
                s.awake.error = Some(format!("Couldn't turn sleep back on: {reason}"));
            }
            apply_view(s, view);
        });
    });
}

pub fn dismiss_error(s: &mut Ui) {
    s.awake.error = None;
}

/// Turns the display off now. The panel goes first, so it isn't what wakes the display.
pub fn display_off(s: &mut Ui, tx: &mut Tx) {
    ui::close(s, tx);
    if !s.demo {
        std::thread::spawn(power::display_off);
    }
}

pub fn sleep_now(s: &mut Ui, tx: &mut Tx) {
    ui::close(s, tx);
    if !s.demo {
        std::thread::spawn(power::sleep_now);
    }
}

pub fn lock_screen(s: &mut Ui, tx: &mut Tx) {
    if s.demo {
        return;
    }
    match power::lock_screen() {
        Ok(()) => ui::close(s, tx),
        Err(reason) => s.awake.error = Some(reason),
    }
}

/// Whether this Mac has a lid, as far as is known yet.
pub fn has_lid(s: &Ui) -> bool {
    s.awake.view.as_ref().is_some_and(|view| view.lid.is_some())
}

/// The minutes a countdown shows: rounded up, so the last minute reads "1m" until it ends.
pub fn left_words(seconds: i64) -> String {
    let words = time::span(((seconds + 59) / 60) * 60);
    words.strip_suffix(" 0m").map_or(words.clone(), str::to_owned)
}

/// Puts the keep-awake item in the menu bar while a session runs, if the settings want it.
pub fn sync_item(s: &Ui) {
    if s.demo {
        return;
    }
    let Some(session) = s.awake.session.as_ref().filter(|_| s.config.sleep.menu_bar) else {
        // Sleep left off by an earlier run is said in the menu bar, whatever the settings show
        // there: the Mac won't sleep with its lid closed, and nothing else would say so.
        if s.awake.session.is_none() && s.awake.sleep_stuck_off(time::unix_now()) && s.awake.left_off {
            status_item::install(&s.me, &s.data_dir, status_item::AWAKE, "Keep awake", &status_item::CUP_MARK);
            let part = status_item::Part { tag: None, text: "Sleep off".to_owned(), severity: crate::model::Severity::Warning };
            status_item::set(status_item::AWAKE, None, &[part], false, "Sleep is off: this Mac won't sleep, even with its lid closed. Open to turn it back on.");
        } else {
            status_item::remove(status_item::AWAKE);
        }
        s.sync_app_item();
        return;
    };
    status_item::install(&s.me, &s.data_dir, status_item::AWAKE, "Keep awake", &status_item::CUP_MARK);
    let now = time::unix_now();
    let (text, tooltip) = match session.until {
        Some(until) => (left_words(until - now), format!("Keeping this Mac awake until {}", time::clock(until, false))),
        None => (String::new(), "Keeping this Mac awake until you turn it off".to_owned()),
    };
    let parts = if text.is_empty() { Vec::new() } else { vec![status_item::Part { tag: None, text, severity: crate::model::Severity::Normal }] };
    status_item::set(status_item::AWAKE, None, &parts, false, &tooltip);
    s.sync_app_item();
}

fn save(s: &mut Ui) {
    ui::save_config(s);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_keep_awake_cut_short_is_taken_up_only_while_it_still_stands() {
        let (boot, now) = (1_000, 5_000);
        let kept = |started: i64, until: Option<i64>| Kept { started, until, minutes: 0 };
        assert!(kept(2_000, None).resumable(Some(boot), now), "one with no end, from this boot");
        assert!(kept(2_000, Some(6_000)).resumable(Some(boot), now), "one whose end is still ahead");
        assert!(!kept(2_000, Some(4_000)).resumable(Some(boot), now), "not one whose time has passed");
        assert!(!kept(500, None).resumable(Some(boot), now), "not one from before the Mac last started");
        assert!(!kept(2_000, None).resumable(None, now), "not when the start of this boot is unknown");
        assert!(!kept(9_000, None).resumable(Some(boot), now), "not one from a clock ahead of this one");
    }

    #[test]
    fn the_last_minute_reads_as_one_until_it_ends() {
        assert_eq!((left_words(59), left_words(61), left_words(3600), left_words(3601)), ("1m".to_owned(), "2m".to_owned(), "1h".to_owned(), "1h 1m".to_owned()));
    }
}
