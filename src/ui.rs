//! The panel's state and behaviour: what it shows, when it opens under its menu bar item and
//! when it goes away.

use std::path::PathBuf;
use std::sync::Arc;

use rune::{AppHandle, DisplayTarget, Key, KeyEvent, OverlayPlacement, SystemMaterial, Tx};

use crate::config::{self, Config};
use crate::model::{self, Problem, Severity, Snapshot};
use crate::stats::StatsView;
use crate::status_item;
use crate::time;
use crate::worker::{Command, Event, Worker};

pub const PANEL_W: f32 = 344.0;
/// Room beside and under the panel for its shadow.
pub const SHADOW_ROOM: f32 = 28.0;
/// Between the menu bar and the panel. The window starts at the menu bar's bottom edge, which
/// macOS keeps windows below, so this is all the room there is above the panel.
pub const GAP: f32 = 5.0;
pub const WINDOW_W: f32 = PANEL_W + 2.0 * SHADOW_ROOM;
/// Taller than the panel ever is; the panel takes the height of its content.
pub const WINDOW_H: f32 = 820.0;

/// A click on the menu bar item first takes focus from the open panel, which closes it, and
/// then arrives as a click. Within this long of that close, the click is the same gesture.
const SAME_CLICK_SECONDS: f64 = 0.35;
/// How often an open panel re-reads the clock for its countdowns.
const TICK_SECONDS: f64 = 15.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Phase {
    Hidden,
    Shown,
}

/// What the panel's body shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Limits,
    Settings,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Range {
    Today,
    Week,
    Month,
}

impl Range {
    pub const ALL: [Range; 3] = [Range::Today, Range::Week, Range::Month];

    pub fn label(self) -> &'static str {
        match self {
            Range::Today => "Today",
            Range::Week => "7d",
            Range::Month => "30d",
        }
    }

    pub fn index(self) -> usize {
        self as usize
    }
}

/// Where a menu bar item is, in points from the primary display's top-left corner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Anchor {
    /// The item's horizontal centre.
    pub x: f32,
    /// The item's bottom edge.
    pub y: f32,
    /// The left and right edges of the display the item is on.
    pub screen: [f32; 2],
}

pub struct Ui {
    pub me: AppHandle<Ui>,
    pub worker: Option<Worker>,
    pub config: Config,
    pub data_dir: PathBuf,
    pub material: SystemMaterial,
    /// A headless render: sample data, no menu bar item, no requests.
    pub demo: bool,
    pub snapshot: Option<Snapshot>,
    pub problem: Option<Problem>,
    pub refreshing: bool,
    pub stats: Option<Arc<StatsView>>,
    pub range: Range,
    /// The chart bar under the pointer: the totals above the chart read for it.
    pub hovered: Option<usize>,
    pub page: Page,
    /// Launch at Login: whether it can be switched here, and whether it is on.
    pub login: (bool, bool),
    pub phase: Phase,
    /// Unix seconds, as of the last tick: what countdowns count from.
    pub now: i64,
    /// The app clock when losing focus last closed the panel.
    blur_closed_at: f64,
    tick_due: f64,
}

impl Ui {
    pub fn new(me: AppHandle<Ui>, worker: Option<Worker>, config: Config, data_dir: PathBuf, cached: Option<Snapshot>, demo: bool) -> Ui {
        let mut ui = Ui {
            me,
            worker,
            material: if config.material.eq_ignore_ascii_case("glass") { SystemMaterial::Glass } else { SystemMaterial::Popover },
            config,
            data_dir,
            demo,
            snapshot: cached,
            problem: None,
            refreshing: false,
            stats: None,
            range: Range::Week,
            hovered: None,
            page: Page::Limits,
            login: (false, false),
            phase: if demo { Phase::Shown } else { Phase::Hidden },
            now: time::unix_now(),
            blur_closed_at: f64::NEG_INFINITY,
            tick_due: 0.0,
        };
        if demo {
            crate::demo::fill(&mut ui);
        }
        ui
    }

    /// The limits as of now: windows that ended since the refresh read as fresh.
    pub fn limits(&self) -> Option<Snapshot> {
        self.snapshot.as_ref().map(|snapshot| snapshot.rolled(self.now))
    }

    /// Puts the current limits on the menu bar item.
    pub fn sync_menu_bar(&self) {
        if self.demo {
            return;
        }
        let limits = self.limits();
        let shown = limits.as_ref().and_then(|snapshot| model::menu_bar_limit(snapshot, self.config.menu_bar));
        let (text, severity) = match shown {
            Some(limit) => (model::percent(limit, self.config.show_used), limit.severity()),
            None => ("–".to_owned(), Severity::Normal),
        };
        let mut tooltip: Vec<String> = Vec::new();
        if let Some(problem) = &self.problem {
            tooltip.push(format!("Claude: {}", problem.title()));
        }
        for limit in limits.iter().flat_map(|snapshot| &snapshot.limits) {
            let reset = limit.resets_in(self.now).map_or_else(String::new, |left| format!(", resets in {}", time::span(left)));
            tooltip.push(format!("{}: {} left{reset}", limit.label, model::percent(limit, false)));
        }
        status_item::set("claude", &text, severity, self.problem.is_some(), &tooltip.join("\n"));
    }

    fn send(&self, command: Command) {
        if let Some(worker) = &self.worker {
            worker.send(command);
        }
    }
}

/// A click on the menu bar item: opens the panel under it, or closes it.
pub fn toggle(s: &mut Ui, anchor: Anchor, tx: &mut Tx) {
    match s.phase {
        Phase::Shown => close(s, tx),
        _ if tx.clock() - s.blur_closed_at < SAME_CLICK_SECONDS => {}
        _ => open(s, anchor, tx),
    }
}

pub fn open(s: &mut Ui, anchor: Anchor, tx: &mut Tx) {
    // The panel is there at once, as a menu is: nothing springs into place.
    tx.instant();
    s.now = time::unix_now();
    s.page = Page::Limits;
    s.hovered = None;
    let [left, right] = anchor.screen;
    // Centred under the item, kept on its display.
    let x = (anchor.x - WINDOW_W / 2.0).min(right - WINDOW_W + SHADOW_ROOM - 8.0).max(left - SHADOW_ROOM + 8.0);
    crate::debug(|| format!("opening under {anchor:?} at x {x}"));
    s.me.place(DisplayTarget::Primary, OverlayPlacement::Rect { x, y: anchor.y, width: WINDOW_W, height: WINDOW_H });
    s.me.open();
    s.phase = Phase::Shown;
    s.tick_due = tx.clock() + TICK_SECONDS;
    tx.wake_at(s.tick_due);
    s.send(Command::Opened);
}

pub fn close(s: &mut Ui, tx: &mut Tx) {
    if s.phase != Phase::Shown {
        return;
    }
    tx.instant();
    s.phase = Phase::Hidden;
    s.hovered = None;
    s.me.hide();
}

pub fn poll(s: &mut Ui, clock: f64, tx: &mut Tx) -> bool {
    match s.phase {
        // Countdowns move only while the panel shows, so a hidden panel never wakes.
        Phase::Shown if !s.demo && clock >= s.tick_due => {
            s.now = time::unix_now();
            s.tick_due = clock + TICK_SECONDS;
            tx.wake_at(s.tick_due);
            true
        }
        _ => false,
    }
}

/// A click elsewhere, or another app coming forward, puts the panel away.
pub fn on_focus(s: &mut Ui, focused: bool, tx: &mut Tx) {
    if !focused && !s.demo && s.phase == Phase::Shown {
        s.blur_closed_at = tx.clock();
        close(s, tx);
    }
}

/// An event from the worker thread.
pub fn apply(s: &mut Ui, event: Event, tx: &mut Tx) {
    if s.phase != Phase::Shown {
        tx.instant();
    }
    s.now = time::unix_now();
    match event {
        Event::Refreshing => s.refreshing = true,
        Event::Limits(Ok(snapshot)) => {
            s.snapshot = Some(snapshot);
            s.problem = None;
            s.refreshing = false;
        }
        Event::Limits(Err(problem)) => {
            s.problem = Some(problem);
            s.refreshing = false;
        }
        Event::Stats(stats) => s.stats = Some(stats),
    }
    s.sync_menu_bar();
}

pub fn refresh(s: &mut Ui) {
    s.send(Command::Refresh);
}

pub fn set_range(s: &mut Ui, range: Range) {
    s.range = range;
    s.hovered = None;
}

/// The pointer entered or left the chart bar at `at`.
pub fn hover_bar(s: &mut Ui, at: usize, inside: bool) {
    if inside {
        s.hovered = Some(at);
    } else if s.hovered == Some(at) {
        s.hovered = None;
    }
}

/// Switches the panel between the limits and the settings.
pub fn toggle_settings(s: &mut Ui, tx: &mut Tx) {
    // A different page, not a change to this one: nothing slides or fades across.
    tx.instant();
    s.page = if s.page == Page::Settings { Page::Limits } else { Page::Settings };
    if s.page == Page::Settings && !s.demo {
        // Launch at Login can also be switched in System Settings: read it each time.
        s.login = status_item::launch_at_login();
    }
}

pub fn set_menu_bar(s: &mut Ui, metric: model::MenuBarMetric) {
    s.config.menu_bar = metric;
    if !s.demo {
        config::save(&s.data_dir, &s.config);
    }
    s.sync_menu_bar();
}

pub fn set_launch_at_login(s: &mut Ui, on: bool) {
    if s.demo {
        s.login.1 = on;
        return;
    }
    if let Err(reason) = status_item::set_launch_at_login(on) {
        eprintln!("nerda-bar: launch at login: {reason}");
    }
    s.login = status_item::launch_at_login();
}

pub fn open_settings_file(s: &mut Ui, tx: &mut Tx) {
    if !s.demo {
        status_item::open(&s.data_dir.join("config.json"));
    }
    close(s, tx);
}

/// Switches every percentage between what is used and what is left, and remembers it.
pub fn set_show_used(s: &mut Ui, show_used: bool) {
    s.config.show_used = show_used;
    if !s.demo {
        config::save(&s.data_dir, &s.config);
        status_item::set_show_used(show_used);
    }
    s.sync_menu_bar();
}

pub fn on_key(s: &mut Ui, ev: &KeyEvent, tx: &mut Tx) -> bool {
    match &ev.key {
        Key::Escape if s.page == Page::Settings => toggle_settings(s, tx),
        Key::Escape => close(s, tx),
        Key::Command(',') => toggle_settings(s, tx),
        Key::Command('r') => refresh(s),
        Key::Command('q') => tx.quit(),
        Key::Command('w') => close(s, tx),
        _ => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use rune::{Env, Mods, Runtime};

    fn runtime() -> Runtime<Ui> {
        let app = rune::App::new(rune::AppSpec::new("test", (WINDOW_W, WINDOW_H), || unreachable!(), crate::view::view));
        let ui = Ui::new(app.handle(), None, Config::default(), PathBuf::new(), None, true);
        let mut rt = Runtime::new(ui, crate::view::view, (WINDOW_W, WINDOW_H), Env { system_dark: true, scale: 2.0 });
        rt.on_key = Some(on_key);
        rt.frame(0.0);
        rt
    }

    #[test]
    fn the_panel_draws_its_limits_and_usage() {
        let rt = runtime();
        for key in ["panel", "limit-session", "limit-weekly", "usage", "chart", "footer"] {
            assert!(rt.presented(key).is_some(), "{key} is on screen");
        }
        // The panel fits the window with its shadow room to spare.
        let [_, y, _, h] = rt.presented("panel").unwrap().0;
        assert!(y + h + SHADOW_ROOM <= WINDOW_H, "the panel is {h} tall");
    }

    #[test]
    fn escape_closes_the_panel() {
        let mut rt = runtime();
        rt.key(KeyEvent { key: Key::Escape, mods: Mods::default() });
        rt.frame(1.0 / 60.0);
        assert_eq!(rt.state.phase, Phase::Hidden);
    }

    /// The centre of the element with `key`.
    fn centre(rt: &Runtime<Ui>, key: &str) -> (f32, f32) {
        let [x, y, w, h] = rt.presented(key).unwrap_or_else(|| panic!("{key} is on screen")).0;
        (x + w / 2.0, y + h / 2.0)
    }

    fn click(rt: &mut Runtime<Ui>, key: &str) {
        let (x, y) = centre(rt, key);
        rt.pointer_move(x, y);
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(1.0 / 60.0);
    }

    #[test]
    fn a_click_on_a_range_switches_the_usage() {
        let mut rt = runtime();
        click(&mut rt, "range-Today");
        assert_eq!(rt.state.range, Range::Today);
        assert!(rt.presented("bar-23").is_some(), "today has a bar per hour");
        click(&mut rt, "range-30d");
        assert_eq!(rt.state.range, Range::Month);
        assert_eq!(rt.state.phase, Phase::Shown, "a click inside the panel doesn't close it");
    }

    #[test]
    fn the_pointer_over_a_bar_reads_that_bar() {
        let mut rt = runtime();
        let (x, y) = centre(&rt, "bar-2");
        rt.pointer_move(x, y);
        rt.frame(1.0 / 60.0);
        assert_eq!(rt.state.hovered, Some(2));
        let (x, y) = centre(&rt, "bar-5");
        rt.pointer_move(x, y);
        rt.frame(1.0 / 60.0);
        assert_eq!(rt.state.hovered, Some(5));
        let (x, y) = centre(&rt, "footer");
        rt.pointer_move(x, y);
        rt.frame(1.0 / 60.0);
        assert_eq!(rt.state.hovered, None);
    }

    #[test]
    fn the_settings_button_swaps_the_body_and_settings_apply() {
        let mut rt = runtime();
        click(&mut rt, "settings");
        assert_eq!(rt.state.page, Page::Settings);
        assert!(rt.presented("limit-session").is_none() && rt.presented("settings-page").is_some());
        click(&mut rt, "menu-bar-Weekly");
        assert_eq!(rt.state.config.menu_bar, model::MenuBarMetric::Weekly);
        click(&mut rt, "percent-Used");
        assert!(rt.state.config.show_used);
        click(&mut rt, "settings");
        assert_eq!(rt.state.page, Page::Limits);
    }

    #[test]
    fn a_click_that_closed_the_panel_does_not_reopen_it() {
        let mut rt = runtime();
        let anchor = Anchor { x: 900.0, y: 30.0, screen: [0.0, 1512.0] };
        rt.state.demo = false;
        rt.update(|s, tx| on_focus(s, false, tx));
        rt.frame(1.0 / 60.0);
        assert_eq!(rt.state.phase, Phase::Hidden);
        rt.update(move |s, tx| toggle(s, anchor, tx));
        rt.frame(1.0 / 60.0);
        assert_eq!(rt.state.phase, Phase::Hidden);
    }

    #[test]
    fn a_failed_refresh_keeps_the_last_limits() {
        let mut rt = runtime();
        let before = rt.state.snapshot.clone();
        rt.update(|s, tx| apply(s, Event::Limits(Err(Problem::Offline("No network.".to_owned()))), tx));
        rt.frame(1.0 / 60.0);
        assert_eq!(rt.state.snapshot, before);
        assert!(rt.state.problem.is_some());
        assert!(rt.presented("problem").is_some());
    }
}
