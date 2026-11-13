//! The panel's state and behaviour: what it shows, when it opens under its menu bar item and
//! when it goes away.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use rune::{AppHandle, DisplayTarget, Image, ImageCache, Key, KeyEvent, OverlayPlacement, SystemMaterial, Tx};

use crate::burn::History;
use crate::config::{self, Config, MenuStyle, Module};
use crate::model::{self, Limit, Problem, Severity, Snapshot};
use crate::stats::StatsView;
use crate::status_item;
use crate::system::{Pressure, Rate, Sampler, SystemView};
use crate::time;
use crate::worker::{Command, Event, Worker};

/// The tab's own column, and the rail of tabs beside it when there is more than one.
pub const CONTENT_W: f32 = 344.0;
pub const RAIL_W: f32 = 68.0;
pub const PANEL_W: f32 = RAIL_W + CONTENT_W;
/// Room beside and under the panel for its shadow.
pub const SHADOW_ROOM: f32 = 28.0;
/// Between the menu bar and the panel. The window starts at the menu bar's bottom edge, which
/// macOS keeps windows below, so this is all the room there is above the panel.
pub const GAP: f32 = 5.0;
pub const WINDOW_W: f32 = PANEL_W + 2.0 * SHADOW_ROOM;
/// Taller than the panel ever is; the panel takes the height of its content.
pub const WINDOW_H: f32 = 880.0;

/// A click on the menu bar item first takes focus from the open panel, which closes it, and
/// then arrives as a click. Within this long of that close, the click is the same gesture.
const SAME_CLICK_SECONDS: f64 = 0.35;
/// The longest a panel waits, hidden, to be drawn at a new display's scale, and how often it
/// looks.
const RESCALE_SECONDS: f64 = 0.25;
const RESCALE_STEP: f64 = 0.008;
/// How often an open panel re-reads the clock for its countdowns.
const TICK_SECONDS: f64 = 15.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Phase {
    Hidden,
    /// Placed on a display of another scale and still hidden, until a frame is drawn at that
    /// scale or `deadline` on the app clock passes.
    Opening { scale: f32, deadline: f64, drawn: bool },
    Shown,
}

/// What the panel's body shows: the tab, or the settings in its place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Tab,
    Settings,
}

/// A tab of the panel, and the menu bar item that opens it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    /// The AI plans' limits and usage.
    Agents,
    Cpu,
    Memory,
}

impl Tab {
    pub const SYSTEM: [Tab; 2] = [Tab::Cpu, Tab::Memory];

    /// The menu bar item's id.
    pub fn id(self) -> &'static str {
        match self {
            Tab::Agents => "claude",
            Tab::Cpu => "cpu",
            Tab::Memory => "memory",
        }
    }

    pub fn from_id(id: &str) -> Tab {
        match id {
            "cpu" => Tab::Cpu,
            "memory" => Tab::Memory,
            _ => Tab::Agents,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Tab::Agents => "Agents",
            Tab::Cpu => "CPU",
            Tab::Memory => "Memory",
        }
    }

    pub fn is_system(self) -> bool {
        self != Tab::Agents
    }
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

    /// The range as the usage totals name it.
    pub fn span(self) -> &'static str {
        match self {
            Range::Today => "Today",
            Range::Week => "Last 7 days",
            Range::Month => "Last 30 days",
        }
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
    /// That display's pixels per point.
    pub scale: f32,
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
    /// Every limit's readings, for the limit chart.
    pub history: Arc<History>,
    /// The id of the limit the chart shows, once one was picked.
    pub chart: Option<String>,
    /// Where the pointer is along the limit chart, from 0 to 1.
    pub trace: Option<f32>,
    pub range: Range,
    /// The usage chart's bar under the pointer: the line under the chart reads it.
    pub hovered: Option<usize>,
    pub page: Page,
    pub tab: Tab,
    pub sampler: Option<Sampler>,
    /// The last reading of this Mac's measures, and their history.
    pub system: Option<Arc<SystemView>>,
    /// The resolution the system charts show, an index into `history::TIERS`.
    pub tier: usize,
    /// The system chart's point under the pointer.
    pub point: Option<usize>,
    /// The rate the sampler was last asked for.
    rate: Cell<Rate>,
    /// Decodes app icons off the main thread; set once the app starts.
    pub images: Option<ImageCache>,
    /// The icons the panel has asked for, held so they decode once.
    icons: RefCell<HashMap<PathBuf, Image>>,
    /// Launch at Login: whether it can be switched here, and whether it is on.
    pub login: (bool, bool),
    pub phase: Phase,
    /// Unix seconds, as of the last tick: what countdowns count from.
    pub now: i64,
    /// The app clock when losing focus last closed the panel.
    blur_closed_at: f64,
    tick_due: f64,
    /// The pixels per point the view last drew at.
    pub drawn_scale: Cell<f32>,
    /// The item the panel last opened under.
    last_anchor: Option<Anchor>,
}

impl Ui {
    pub fn new(me: AppHandle<Ui>, worker: Option<Worker>, sampler: Option<Sampler>, config: Config, data_dir: PathBuf, cached: Option<Snapshot>, demo: bool) -> Ui {
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
            history: Arc::new(History::default()),
            chart: None,
            trace: None,
            range: Range::Week,
            hovered: None,
            page: Page::Tab,
            tab: Tab::Agents,
            sampler,
            system: None,
            tier: 1,
            point: None,
            rate: Cell::new(Rate::Off),
            images: None,
            icons: RefCell::new(HashMap::new()),
            login: (false, false),
            phase: if demo { Phase::Shown } else { Phase::Hidden },
            now: time::unix_now(),
            blur_closed_at: f64::NEG_INFINITY,
            tick_due: 0.0,
            drawn_scale: Cell::new(0.0),
            last_anchor: None,
        };
        if demo {
            crate::demo::fill(&mut ui);
        }
        ui.sync_rate();
        ui
    }

    /// The Finder icon of an app bundle or a tool, `size` points square, once it has decoded.
    pub fn icon(&self, path: &std::path::Path, size: f32) -> Option<Image> {
        let cache = self.images.as_ref()?;
        let mut icons = self.icons.borrow_mut();
        // Apps come and go: forget the lot now and then rather than track which still show.
        if icons.len() > 96 {
            icons.clear();
        }
        let image = icons.entry(path.to_owned()).or_insert_with(|| cache.app_icon_scaled(path, size, 2.0)).clone();
        if self.demo {
            // A headless render is one frame: it can't wait for the icon to arrive later.
            let _ = image.wait();
        }
        (image.is_ready() && image.error().is_none()).then_some(image)
    }

    pub fn module(&self, tab: Tab) -> Option<Module> {
        match tab {
            Tab::Agents => None,
            Tab::Cpu => Some(self.config.cpu),
            Tab::Memory => Some(self.config.memory),
        }
    }

    fn module_mut(&mut self, tab: Tab) -> Option<&mut Module> {
        match tab {
            Tab::Agents => None,
            Tab::Cpu => Some(&mut self.config.cpu),
            Tab::Memory => Some(&mut self.config.memory),
        }
    }

    /// The panel's tabs, in the rail's order.
    pub fn tabs(&self) -> Vec<Tab> {
        std::iter::once(Tab::Agents).chain(Tab::SYSTEM.into_iter().filter(|&tab| self.module(tab).is_some_and(|m| m.tab))).collect()
    }

    /// How often this Mac's measures need reading for what shows them now.
    fn wanted_rate(&self) -> Rate {
        let modules = Tab::SYSTEM.map(|tab| self.module(tab).unwrap_or_default());
        if self.phase == Phase::Shown && self.page == Page::Tab && self.tab.is_system() {
            Rate::Live
        } else if modules.iter().any(|m| m.menu_bar != MenuStyle::Off) {
            Rate::MenuBar
        } else if modules.iter().any(|m| m.tab) {
            Rate::Background
        } else {
            Rate::Off
        }
    }

    /// Tells the sampler the rate what is on screen needs, when it changed.
    pub fn sync_rate(&self) {
        let rate = self.wanted_rate();
        if let Some(sampler) = &self.sampler
            && self.rate.replace(rate) != rate
        {
            crate::debug(|| format!("system rate {rate:?}"));
            sampler.set_rate(rate);
        }
    }

    /// Puts each measure the settings show in the menu bar there, and takes the rest away.
    pub fn sync_system_items(&self) {
        if self.demo {
            return;
        }
        // macOS puts a new item to the left of the others: memory goes first, so CPU lands left of it.
        for tab in Tab::SYSTEM.into_iter().rev() {
            let style = self.module(tab).map_or(MenuStyle::Off, |m| m.menu_bar);
            if style == MenuStyle::Off {
                status_item::remove(tab.id());
                continue;
            }
            status_item::install_measure(self.me.clone(), self.data_dir.clone(), tab.id(), tab.label());
            let Some(view) = &self.system else {
                status_item::set(tab.id(), None, &[status_item::Part { tag: Some(tab.label().to_owned()), text: "–".to_owned(), severity: Severity::Normal }], true, tab.label());
                continue;
            };
            let (share, severity, series, tooltip) = match tab {
                Tab::Memory => {
                    let memory = view.memory.as_ref();
                    let severity = match memory.map(|m| m.pressure) {
                        Some(Pressure::Critical) => Severity::Critical,
                        Some(Pressure::Warning) => Severity::Warning,
                        _ => Severity::Normal,
                    };
                    let tooltip = memory.map_or_else(String::new, |m| format!("Memory: {} of {} used", system_bytes(m.used()), system_bytes(m.total)));
                    (memory.map(|m| m.used_share()), severity, &view.memory_used, tooltip)
                }
                _ => {
                    let cpu = view.cpu.as_ref();
                    let tooltip = cpu.map_or_else(String::new, |c| format!("CPU: {:.0}% in use, {:.0}% apps, {:.0}% system", c.total() * 100.0, c.user * 100.0, c.system * 100.0));
                    (cpu.map(|c| c.total()), Severity::Normal, &view.cpu_total, tooltip)
                }
            };
            let percent = share.map(|share| (share * 100.0).round().clamp(0.0, 100.0) as u8);
            let text = percent.map_or_else(|| "–".to_owned(), |p| format!("{p}%"));
            let glyph = match style {
                MenuStyle::Ring => Some(status_item::Glyph::Ring(percent.unwrap_or(0))),
                MenuStyle::Graph => Some(status_item::Glyph::Bars(SystemView::recent(series, GRAPH_BARS).iter().map(|v| (v * 100.0).round() as u8).collect())),
                _ => None,
            };
            let tag = (style == MenuStyle::Text).then(|| if tab == Tab::Memory { "MEM".to_owned() } else { tab.label().to_owned() });
            status_item::set(tab.id(), glyph, &[status_item::Part { tag, text, severity }], percent.is_none(), &tooltip);
        }
    }

    /// The limits as of now: windows that ended since the refresh read as fresh.
    pub fn limits(&self) -> Option<Snapshot> {
        self.snapshot.as_ref().map(|snapshot| snapshot.rolled(self.now))
    }

    /// The limit the chart shows: the one picked, while the plan still has it, else the
    /// default.
    pub fn chart_limit<'a>(&self, snapshot: &'a Snapshot) -> Option<&'a Limit> {
        self.chart.as_deref().and_then(|id| snapshot.limit(id)).or_else(|| model::chart_default(snapshot))
    }

    /// Puts the current limits on the menu bar item.
    pub fn sync_menu_bar(&self) {
        if self.demo {
            return;
        }
        let limits = self.limits();
        let shown = limits.as_ref().map(|snapshot| model::menu_bar_limits(snapshot, self.config.menu_bar, self.config.menu_bar_models)).unwrap_or_default();
        // One limit reads alone; with several, each takes its letter.
        let tagged = shown.len() > 1;
        let mut parts: Vec<status_item::Part> = shown
            .iter()
            .map(|limit| status_item::Part { tag: tagged.then(|| model::tag(limit).to_string()), text: model::percent(limit, self.config.show_used), severity: limit.severity() })
            .collect();
        if parts.is_empty() {
            parts.push(status_item::Part { tag: None, text: "–".to_owned(), severity: Severity::Normal });
        }
        let mut tooltip: Vec<String> = Vec::new();
        if let Some(problem) = &self.problem {
            tooltip.push(format!("Claude: {}", problem.title()));
        }
        for limit in limits.iter().flat_map(|snapshot| &snapshot.limits) {
            let reset = limit.resets_in(self.now).map_or_else(String::new, |left| format!(", resets in {}", time::span(left)));
            tooltip.push(format!("{}: {} left{reset}", limit.label, model::percent(limit, false)));
        }
        status_item::set("claude", None, &parts, self.problem.is_some(), &tooltip.join("\n"));
    }

    fn send(&self, command: Command) {
        if let Some(worker) = &self.worker {
            worker.send(command);
        }
    }
}

/// A click on the menu bar item at `anchor`: opens the panel under it on `tab`, or closes it.
/// With the panel open under the item on another display's menu bar, it moves there instead.
pub fn toggle(s: &mut Ui, anchor: Anchor, tab: Tab, tx: &mut Tx) {
    let same_item = s.last_anchor.is_some_and(|last| (last.x - anchor.x).abs() < 2.0 && (last.y - anchor.y).abs() < 2.0);
    match s.phase {
        Phase::Shown if same_item => close(s, tx),
        // A second click before the panel is up takes the first one back.
        Phase::Opening { .. } if same_item => s.phase = Phase::Hidden,
        // The click that took focus from the panel, on the item it was open under.
        Phase::Hidden if same_item && tx.clock() - s.blur_closed_at < SAME_CLICK_SECONDS => {}
        _ => open(s, anchor, tab, tx),
    }
}

pub fn open(s: &mut Ui, anchor: Anchor, tab: Tab, tx: &mut Tx) {
    // The panel is there at once, as a menu is: nothing springs into place.
    tx.instant();
    if s.phase != Phase::Hidden {
        // Moving to another item: off screen first, so the move is never seen.
        s.me.hide();
        s.phase = Phase::Hidden;
    }
    s.last_anchor = Some(anchor);
    s.now = time::unix_now();
    s.page = Page::Tab;
    s.tab = if s.tabs().contains(&tab) { tab } else { Tab::Agents };
    s.hovered = None;
    s.point = None;
    s.trace = None;
    let [left, right] = anchor.screen;
    // Centred under the item, kept on its display.
    let x = (anchor.x - WINDOW_W / 2.0).min(right - WINDOW_W + SHADOW_ROOM - 8.0).max(left - SHADOW_ROOM + 8.0);
    crate::debug(|| format!("opening under {anchor:?} at x {x}, last drawn at scale {}", s.drawn_scale.get()));
    if !s.demo {
        // Before the move: a scale change set with actions on would animate.
        status_item::hold_layers_still();
    }
    s.me.place(DisplayTarget::Primary, OverlayPlacement::Rect { x, y: anchor.y, width: WINDOW_W, height: WINDOW_H });
    s.send(Command::Opened);
    if s.drawn_scale.get() == anchor.scale {
        reveal(s, tx);
    } else {
        // The hidden panel's last frame was drawn for a display of another scale, and shown
        // now it would be stretched to this one until the next frame. It moves there hidden,
        // is drawn again, and only then shows.
        s.phase = Phase::Opening { scale: anchor.scale, deadline: tx.clock() + RESCALE_SECONDS, drawn: false };
        tx.wake_at(tx.clock() + RESCALE_STEP);
    }
}

/// Brings the placed panel on screen.
fn reveal(s: &mut Ui, tx: &mut Tx) {
    tx.instant();
    s.me.open();
    s.phase = Phase::Shown;
    s.tick_due = tx.clock() + TICK_SECONDS;
    tx.wake_at(s.tick_due);
    s.sync_rate();
}

pub fn close(s: &mut Ui, tx: &mut Tx) {
    if s.phase != Phase::Shown {
        return;
    }
    tx.instant();
    s.phase = Phase::Hidden;
    s.hovered = None;
    s.point = None;
    s.trace = None;
    s.me.hide();
    s.sync_rate();
}

pub fn poll(s: &mut Ui, clock: f64, tx: &mut Tx) -> bool {
    match s.phase {
        Phase::Opening { scale, deadline, drawn } => {
            let at_scale = s.drawn_scale.get() == scale;
            if (at_scale && drawn) || clock >= deadline {
                crate::debug(|| format!("shown at scale {scale} after {:.0} ms{}", (clock - (deadline - RESCALE_SECONDS)) * 1e3, if at_scale { "" } else { ", not yet drawn at it" }));
                reveal(s, tx);
            } else {
                // One more step after the view ran at the new scale, for that frame to be drawn.
                s.phase = Phase::Opening { scale, deadline, drawn: at_scale };
                tx.wake_at(clock + RESCALE_STEP);
            }
            true
        }
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
        Event::History(history) => s.history = history,
    }
    s.sync_menu_bar();
}

/// A reading of this Mac's measures. It redraws the panel only while the panel shows them.
pub fn apply_system(s: &mut Ui, view: Arc<SystemView>, tx: &mut Tx) -> bool {
    let shown = s.phase == Phase::Shown && s.page == Page::Tab && s.tab.is_system();
    if !shown {
        tx.instant();
    }
    s.system = Some(view);
    s.sync_system_items();
    shown
}

/// Shows `tab` in the panel.
pub fn set_tab(s: &mut Ui, tab: Tab, tx: &mut Tx) {
    // A different tab, not a change to this one: nothing slides or fades across.
    tx.instant();
    s.tab = tab;
    s.page = Page::Tab;
    s.hovered = None;
    s.point = None;
    s.trace = None;
    s.sync_rate();
}

pub fn set_tier(s: &mut Ui, tier: usize) {
    s.tier = tier;
    s.point = None;
}

/// The pointer at `at` along the system chart, from 0 to 1.
pub fn hover_point(s: &mut Ui, at: f32, len: usize) {
    if len > 0 {
        s.point = Some(((at.clamp(0.0, 1.0) * (len - 1) as f32).round() as usize).min(len - 1));
    }
}

pub fn leave_point(s: &mut Ui) {
    s.point = None;
}

pub fn set_module_menu_bar(s: &mut Ui, tab: Tab, style: MenuStyle) {
    if let Some(module) = s.module_mut(tab) {
        module.menu_bar = style;
    }
    save(s);
    s.sync_rate();
    s.sync_system_items();
}

/// Shows or hides a measure's tab. The panel changes width with its rail at once, as a new
/// page does, rather than sliding.
pub fn set_module_tab(s: &mut Ui, tab: Tab, on: bool, tx: &mut Tx) {
    tx.instant();
    if let Some(module) = s.module_mut(tab) {
        module.tab = on;
    }
    if !on && s.tab == tab {
        s.tab = Tab::Agents;
    }
    save(s);
    s.sync_rate();
}

fn save(s: &Ui) {
    if !s.demo {
        config::save(&s.data_dir, &s.config);
    }
}

/// Bytes as the panel and the menu bar write them: "13.1 GB", "812 MB".
pub fn system_bytes(bytes: u64) -> String {
    let gb = bytes as f64 / (1u64 << 30) as f64;
    if gb >= 100.0 {
        format!("{gb:.0} GB")
    } else if gb >= 1.0 {
        // A whole number of gigabytes (the Mac's memory) reads without its ".0".
        let text = format!("{gb:.1}");
        format!("{} GB", text.strip_suffix(".0").unwrap_or(&text))
    } else {
        format!("{:.0} MB", bytes as f64 / (1u64 << 20) as f64)
    }
}

/// Bars in a measure's menu bar graph: the last half minute at its 2-second steps.
const GRAPH_BARS: usize = 14;

pub fn refresh(s: &mut Ui) {
    s.send(Command::Refresh);
}

pub fn set_range(s: &mut Ui, range: Range) {
    s.range = range;
    s.hovered = None;
}

/// Shows `id`'s history in the limit chart.
pub fn select_limit(s: &mut Ui, id: &str) {
    s.chart = Some(id.to_owned());
    s.trace = None;
}

/// The pointer at `at` along the limit chart, from 0 to 1.
pub fn trace_limit(s: &mut Ui, at: f32) {
    s.trace = Some(at.clamp(0.0, 1.0));
}

pub fn leave_trace(s: &mut Ui) {
    s.trace = None;
}

/// The pointer came onto the chart bar at `at`.
pub fn hover_bar(s: &mut Ui, at: usize) {
    s.hovered = Some(at);
}

/// The pointer left the chart.
pub fn leave_chart(s: &mut Ui) {
    s.hovered = None;
}

/// Switches the panel between the limits and the settings.
pub fn toggle_settings(s: &mut Ui, tx: &mut Tx) {
    // A different page, not a change to this one: nothing slides or fades across.
    tx.instant();
    s.page = if s.page == Page::Settings { Page::Tab } else { Page::Settings };
    s.point = None;
    s.sync_rate();
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

pub fn set_menu_bar_models(s: &mut Ui, on: bool) {
    s.config.menu_bar_models = on;
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
        // ⌘1, ⌘2…: the rail's tabs in order.
        Key::Command(c @ '1'..='9') => match s.tabs().get(*c as usize - '1' as usize) {
            Some(&tab) => set_tab(s, tab, tx),
            None => return false,
        },
        Key::Command('w') => close(s, tx),
        _ => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system;
    use rune::{Env, Mods, Runtime};

    fn runtime() -> Runtime<Ui> {
        let app = rune::App::new(rune::AppSpec::new("test", (WINDOW_W, WINDOW_H), || unreachable!(), crate::view::view));
        let ui = Ui::new(app.handle(), None, None, Config::default(), PathBuf::new(), None, true);
        let mut rt = Runtime::new(ui, crate::view::view, (WINDOW_W, WINDOW_H), Env { system_dark: true, scale: 2.0 });
        rt.on_key = Some(on_key);
        rt.frame(0.0);
        rt
    }

    #[test]
    fn the_panel_draws_its_limits_and_usage() {
        let rt = runtime();
        for key in ["panel", "limit-session", "limit-weekly", "limit-chart", "usage", "chart", "totals", "footer"] {
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
    fn a_click_on_a_limit_shows_it_in_the_chart() {
        let mut rt = runtime();
        let default = rt.state.limits().and_then(|snapshot| rt.state.chart_limit(&snapshot).map(|l| l.id.clone()));
        assert_eq!(default.as_deref(), Some("weekly-fable"), "the weekly limit with the least left");
        click(&mut rt, "limit-weekly");
        assert_eq!(rt.state.chart.as_deref(), Some("weekly"));
        assert!(!rt.state.config.show_used, "a click on a limit no longer flips the percentages");
        click(&mut rt, "limit-session");
        assert_eq!(rt.state.chart.as_deref(), Some("session"));
        // One chart, over the limits that pick it; every row keeps its meter.
        let [_, chart_y, _, chart_h] = rt.presented("limit-chart").unwrap().0;
        let [_, rows_y, _, _] = rt.presented("limits").unwrap().0;
        assert!(chart_y + chart_h < rows_y, "the chart is over the limits");
        for id in ["session", "weekly", "weekly-fable"] {
            assert!(rt.presented(&format!("limit-{id}")).is_some(), "{id} keeps its row");
        }
        let [_, y, _, h] = rt.presented("panel").unwrap().0;
        assert!(y + h + SHADOW_ROOM <= WINDOW_H, "the panel is {h} tall with the session picked");
    }

    #[test]
    fn the_pointer_over_the_limit_chart_reads_that_moment() {
        let mut rt = runtime();
        let [x, y, w, h] = rt.presented("limit-chart").unwrap().0;
        rt.pointer_move(x + w * 0.25, y + h / 2.0);
        rt.frame(1.0 / 60.0);
        assert!(rt.state.trace.is_some_and(|at| (at - 0.25).abs() < 0.01));
        let (fx, fy) = centre(&rt, "footer");
        rt.pointer_move(fx, fy);
        rt.frame(1.0 / 60.0);
        assert_eq!(rt.state.trace, None);
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
        assert_eq!(rt.state.page, Page::Tab);
    }

    #[test]
    fn a_click_that_closed_the_panel_does_not_reopen_it() {
        let mut rt = runtime();
        let anchor = Anchor { x: 900.0, y: 30.0, screen: [0.0, 1512.0], scale: 2.0 };
        rt.state.last_anchor = Some(anchor);
        rt.state.demo = false;
        rt.update(|s, tx| on_focus(s, false, tx));
        rt.frame(1.0 / 60.0);
        assert_eq!(rt.state.phase, Phase::Hidden);
        rt.update(move |s, tx| toggle(s, anchor, Tab::Agents, tx));
        rt.frame(1.0 / 60.0);
        assert_eq!(rt.state.phase, Phase::Hidden);
    }

    #[test]
    fn the_panel_shows_at_once_on_a_display_of_the_scale_it_was_drawn_at() {
        let mut rt = runtime();
        rt.update(close);
        rt.frame(1.0 / 60.0);
        let anchor = Anchor { x: 900.0, y: 30.0, screen: [0.0, 1512.0], scale: 2.0 };
        rt.update(move |s, tx| toggle(s, anchor, Tab::Agents, tx));
        rt.frame(1.0 / 60.0);
        assert_eq!(rt.state.phase, Phase::Shown);
    }

    #[test]
    fn on_a_display_of_another_scale_the_panel_waits_to_be_drawn_at_it() {
        let mut rt = runtime();
        rt.update(close);
        rt.frame(1.0 / 60.0);
        let anchor = Anchor { x: -400.0, y: 30.0, screen: [-1080.0, 0.0], scale: 1.0 };
        rt.update(move |s, tx| toggle(s, anchor, Tab::Agents, tx));
        rt.frame(1.0 / 60.0);
        assert!(matches!(rt.state.phase, Phase::Opening { .. }), "still hidden: its frame is for the other scale");
        // Not yet drawn at the new scale: it keeps waiting.
        rt.update(|s, tx| { poll(s, 0.01, tx); });
        assert!(matches!(rt.state.phase, Phase::Opening { drawn: false, .. }));
        // The view ran at the new scale, and one step later the panel shows.
        rt.state.drawn_scale.set(1.0);
        rt.update(|s, tx| { poll(s, 0.02, tx); });
        assert!(matches!(rt.state.phase, Phase::Opening { drawn: true, .. }));
        rt.update(|s, tx| { poll(s, 0.03, tx); });
        rt.frame(1.0 / 60.0);
        assert_eq!(rt.state.phase, Phase::Shown);
    }

    #[test]
    fn a_click_on_the_item_on_another_display_moves_the_panel_there() {
        let mut rt = runtime();
        rt.update(close);
        rt.frame(1.0 / 60.0);
        let here = Anchor { x: 900.0, y: 30.0, screen: [0.0, 1512.0], scale: 2.0 };
        let there = Anchor { x: 2900.0, y: 30.0, screen: [1512.0, 3432.0], scale: 2.0 };
        rt.update(move |s, tx| toggle(s, here, Tab::Agents, tx));
        rt.frame(1.0 / 60.0);
        assert_eq!(rt.state.phase, Phase::Shown);
        // The click on the other item first takes focus from the panel, then arrives.
        rt.state.demo = false;
        rt.update(|s, tx| on_focus(s, false, tx));
        rt.update(move |s, tx| toggle(s, there, Tab::Agents, tx));
        rt.frame(1.0 / 60.0);
        assert_eq!(rt.state.phase, Phase::Shown);
        assert_eq!(rt.state.last_anchor, Some(there));
        // And with the panel still open, the same.
        rt.update(move |s, tx| toggle(s, here, Tab::Agents, tx));
        rt.frame(1.0 / 60.0);
        assert_eq!((rt.state.phase, rt.state.last_anchor), (Phase::Shown, Some(here)));
        // A click on the item it is open under closes it.
        rt.update(move |s, tx| toggle(s, here, Tab::Agents, tx));
        assert_eq!(rt.state.phase, Phase::Hidden);
    }

    #[test]
    fn a_display_that_never_redraws_still_gets_the_panel() {
        let mut rt = runtime();
        rt.update(close);
        rt.frame(1.0 / 60.0);
        let anchor = Anchor { x: -400.0, y: 30.0, screen: [-1080.0, 0.0], scale: 1.0 };
        rt.update(move |s, tx| toggle(s, anchor, Tab::Agents, tx));
        rt.update(|s, tx| { poll(s, 10.0, tx); });
        rt.frame(1.0 / 60.0);
        assert_eq!(rt.state.phase, Phase::Shown);
    }

    #[test]
    fn the_rail_switches_tabs_and_each_draws_its_own() {
        let mut rt = runtime();
        for key in ["rail", "tab-claude", "tab-cpu", "tab-memory"] {
            assert!(rt.presented(key).is_some(), "{key} is on screen");
        }
        click(&mut rt, "tab-cpu");
        assert_eq!(rt.state.tab, Tab::Cpu);
        for key in ["cpu-hero", "cpu-chart", "cpu-apps-0", "cpu-apps-rest", "cores", "core-9"] {
            assert!(rt.presented(key).is_some(), "{key} is on screen");
        }
        assert!(rt.presented("limit-session").is_none());
        click(&mut rt, "tier-24h");
        assert_eq!(rt.state.tier, 2);
        click(&mut rt, "tab-memory");
        for key in ["memory-bar", "memory-apps-0", "memory-apps-4", "swap"] {
            assert!(rt.presented(key).is_some(), "{key} is on screen");
        }
        rt.key(KeyEvent { key: Key::Command('1'), mods: Mods::default() });
        rt.frame(1.0 / 60.0);
        assert_eq!(rt.state.tab, Tab::Agents);
        let [_, y, _, h] = rt.presented("panel").unwrap().0;
        assert!(y + h + SHADOW_ROOM <= WINDOW_H, "the panel is {h} tall");
    }

    #[test]
    fn the_pointer_over_the_system_chart_reads_a_point() {
        let mut rt = runtime();
        click(&mut rt, "tab-cpu");
        let [x, y, w, h] = rt.presented("cpu-chart").unwrap().0;
        rt.pointer_move(x + w - 0.5, y + h / 2.0);
        rt.frame(1.0 / 60.0);
        assert_eq!(rt.state.point, Some(system::history::TIERS[rt.state.tier].len - 1));
        rt.pointer_move(x + 0.2, y + h / 2.0);
        rt.frame(1.0 / 60.0);
        assert_eq!(rt.state.point, Some(0));
        let (fx, fy) = centre(&rt, "footer");
        rt.pointer_move(fx, fy);
        rt.frame(1.0 / 60.0);
        assert_eq!(rt.state.point, None);
    }

    #[test]
    fn without_system_tabs_there_is_no_rail_and_a_hidden_tab_falls_back() {
        let mut rt = runtime();
        click(&mut rt, "tab-cpu");
        click(&mut rt, "settings");
        click(&mut rt, "cpu-tab");
        assert!(!rt.state.config.cpu.tab);
        assert_eq!(rt.state.tab, Tab::Agents, "the hidden tab gives way to Agents");
        click(&mut rt, "memory-tab");
        assert!(!rt.state.config.memory.tab);
        click(&mut rt, "settings");
        assert_eq!(rt.state.tabs(), vec![Tab::Agents]);
        assert!(rt.presented("rail").is_none());
        assert!(rt.presented("limit-session").is_some());
    }

    #[test]
    fn the_sampler_reads_only_as_often_as_something_shows() {
        let mut rt = runtime();
        rt.state.demo = false;
        assert_eq!(rt.state.wanted_rate(), Rate::Background, "tabs keep a history");
        rt.update(|s, tx| set_tab(s, Tab::Cpu, tx));
        assert_eq!(rt.state.wanted_rate(), Rate::Live, "the panel shows the CPU");
        rt.update(close);
        rt.state.config.memory.menu_bar = MenuStyle::Ring;
        assert_eq!(rt.state.wanted_rate(), Rate::MenuBar);
        rt.state.config.memory = Module { menu_bar: MenuStyle::Off, tab: false };
        rt.state.config.cpu.tab = false;
        assert_eq!(rt.state.wanted_rate(), Rate::Off);
    }

    #[test]
    fn bytes_read_with_one_decimal_under_a_hundred_gigabytes() {
        assert_eq!(system_bytes(13 * (1 << 30) + (1 << 27)), "13.1 GB");
        assert_eq!(system_bytes(812 << 20), "812 MB");
        assert_eq!(system_bytes(128 << 30), "128 GB");
        assert_eq!(system_bytes(16 << 30), "16 GB");
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
