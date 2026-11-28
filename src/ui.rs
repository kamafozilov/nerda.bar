//! The panel's state and behaviour: what it shows, when it opens under its menu bar item and
//! when it goes away.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use rune::{AppHandle, DisplayTarget, Image, ImageCache, Key, KeyEvent, OverlayPlacement, SystemMaterial, TextEdit, Tx};

use crate::agents::{Agent, DeepSeek, ResetFlow};
use crate::config::{self, Config, MenuStyle, Module};
use crate::look::{PANEL_H, PANEL_W};
use crate::model::{self, Problem, Provider, Severity, Snapshot};
use crate::status_item;
use crate::system::{Pressure, Rate, Sampler, SystemView};
use crate::time;
use crate::worker::{Command, Event, Worker};

/// Room beside and under the panel for its shadow.
pub const SHADOW_ROOM: f32 = 28.0;
/// Between the menu bar and the panel. The window starts at the menu bar's bottom edge, which
/// macOS keeps windows below, so this is all the room there is above the panel.
pub const GAP: f32 = 5.0;
pub const WINDOW_W: f32 = PANEL_W + 2.0 * SHADOW_ROOM;
/// The panel has one size on every tab, so the window is the panel and its shadow's room.
pub const WINDOW_H: f32 = PANEL_H + GAP + SHADOW_ROOM;

/// Decoded app icons kept before the cache starts over, and how many of each tab's top apps
/// are decoded ahead of being shown.
const ICONS_KEPT: usize = 256;
const ICONS_AHEAD: usize = 16;
/// A program like any other: its icon stands for every program that isn't an app.
const PROGRAM_ICON: &str = "/bin/ls";

/// The path whose icon shows for `path`: its app bundle's, or for a program outside an app, the
/// one every program has.
fn icon_source(path: &std::path::Path) -> &std::path::Path {
    let in_app = path.components().any(|part| part.as_os_str().to_str().is_some_and(|name| name.ends_with(".app")));
    if in_app { path } else { std::path::Path::new(PROGRAM_ICON) }
}

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
    /// Claude's limits and usage.
    Claude,
    Codex,
    /// DeepSeek's balance.
    DeepSeek,
    Cpu,
    Memory,
    Disk,
    Network,
    Battery,
    Gpu,
    /// Keeping this Mac awake, its idle timers and its lid.
    Sleep,
}

impl Tab {
    pub const SYSTEM: [Tab; 6] = [Tab::Cpu, Tab::Memory, Tab::Disk, Tab::Network, Tab::Battery, Tab::Gpu];

    /// The menu bar item's id.
    pub fn id(self) -> &'static str {
        match self {
            Tab::Claude => "claude",
            Tab::Codex => "codex",
            Tab::DeepSeek => "deepseek",
            Tab::Cpu => "cpu",
            Tab::Memory => "memory",
            Tab::Disk => "disk",
            Tab::Network => "network",
            Tab::Battery => "battery",
            Tab::Gpu => "gpu",
            Tab::Sleep => "sleep",
        }
    }

    pub fn from_id(id: &str) -> Tab {
        match id {
            "codex" => Tab::Codex,
            "deepseek" => Tab::DeepSeek,
            "cpu" => Tab::Cpu,
            "memory" => Tab::Memory,
            "disk" => Tab::Disk,
            "network" => Tab::Network,
            "battery" => Tab::Battery,
            "gpu" => Tab::Gpu,
            "sleep" => Tab::Sleep,
            _ => Tab::Claude,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Tab::Claude => "Claude",
            Tab::Codex => "Codex",
            Tab::DeepSeek => "DeepSeek",
            Tab::Cpu => "CPU",
            Tab::Memory => "Memory",
            Tab::Disk => "Disk",
            Tab::Network => "Network",
            Tab::Battery => "Battery",
            Tab::Gpu => "GPU",
            Tab::Sleep => "Sleep",
        }
    }

    /// One of this Mac's measures, which the sampler reads.
    pub fn is_system(self) -> bool {
        Tab::SYSTEM.contains(&self)
    }

    /// The provider a tab shows, for a provider's tab.
    pub fn provider(self) -> Option<Provider> {
        match self {
            Tab::Claude => Some(Provider::Claude),
            Tab::Codex => Some(Provider::Codex),
            Tab::DeepSeek => Some(Provider::DeepSeek),
            _ => None,
        }
    }

    /// The top bar's icon for the tab: the agents share one.
    pub fn icon(self) -> &'static rune::Icon {
        match self {
            Tab::Claude | Tab::Codex | Tab::DeepSeek => &crate::icons::SPARKLES,
            Tab::Sleep => &crate::icons::MOON,
            tab => crate::system_view::glyph(tab),
        }
    }

    /// The short name a menu bar item's text tag uses.
    pub fn short(self) -> &'static str {
        match self {
            Tab::Claude => "Claude",
            Tab::Codex => "Codex",
            Tab::DeepSeek => "DeepSeek",
            Tab::Cpu => "CPU",
            Tab::Memory => "RAM",
            Tab::Disk => "Disk",
            Tab::Network => "Net",
            Tab::Battery => "Batt",
            Tab::Gpu => "GPU",
            Tab::Sleep => "Sleep",
        }
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
    pub claude: Agent,
    pub codex: Agent,
    pub deepseek: DeepSeek,
    /// Spending a Codex limit reset, from the question to its answer.
    pub reset: ResetFlow,
    /// Where the pointer is along the limit chart, from 0 to 1.
    pub trace: Option<f32>,
    pub range: Range,
    /// The usage chart's bar under the pointer: the line under the chart reads it.
    pub hovered: Option<usize>,
    pub page: Page,
    pub tab: Tab,
    /// The agent the Agents tab last showed: where the top bar's Agents tile goes back to.
    pub agent_tab: Tab,
    /// The Add an agent popover is open.
    pub adding: bool,
    /// The DeepSeek API key being typed in the Add an agent popover.
    pub deepseek_key: TextEdit,
    /// Why the last key couldn't be saved to the Keychain.
    pub key_error: Option<String>,
    pub sampler: Option<Sampler>,
    /// The last reading of this Mac's measures, and their history.
    pub system: Option<Arc<SystemView>>,
    /// The resolution the system charts show, an index into `history::TIERS`.
    pub tier: usize,
    /// The battery chart shows the week rather than the day.
    pub battery_week: bool,
    /// The system chart's point under the pointer.
    pub point: Option<usize>,
    /// The rate the sampler was last asked for.
    rate: Cell<Rate>,
    /// Decodes app icons off the main thread; set once the app starts.
    pub images: Option<ImageCache>,
    /// The icons the panel has asked for, held so they decode once.
    icons: RefCell<HashMap<PathBuf, Image>>,
    /// Keeping this Mac awake, and sleep's state as the Sleep tab last read it.
    pub awake: crate::awake::Awake,
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
    /// Where the panel last opened, and the id of the item it opened under.
    last_anchor: Option<Anchor>,
    last_item: Option<&'static str>,
    /// The agent whose options the settings show.
    pub settings_agent: Provider,
    /// The Add an agent popover takes a new DeepSeek key in place of the one there.
    pub replacing_key: bool,
    /// Remove was pressed on DeepSeek's key: the next press removes it.
    pub confirm_remove_key: bool,
    /// When `config.json` was last read or written here: later than this, it was edited by hand.
    config_stamp: Option<std::time::SystemTime>,
}

impl Ui {
    pub fn new(me: AppHandle<Ui>, worker: Option<Worker>, sampler: Option<Sampler>, config: Config, data_dir: PathBuf, cached: (Option<Snapshot>, Option<Snapshot>), demo: bool) -> Ui {
        let awake = crate::awake::Awake::new(&data_dir, demo);
        let config_stamp = if demo { None } else { config::modified(&data_dir) };
        let mut ui = Ui {
            me,
            worker,
            material: if config.material.eq_ignore_ascii_case("glass") { SystemMaterial::Glass } else { SystemMaterial::Popover },
            config,
            data_dir,
            demo,
            claude: Agent { snapshot: cached.0, ..Agent::default() },
            codex: Agent { snapshot: cached.1, ..Agent::default() },
            deepseek: DeepSeek::default(),
            reset: ResetFlow::Idle,
            trace: None,
            range: Range::Week,
            hovered: None,
            page: Page::Tab,
            tab: Tab::Claude,
            agent_tab: Tab::Claude,
            adding: false,
            deepseek_key: TextEdit::new("").secure(),
            key_error: None,
            sampler,
            system: None,
            tier: 1,
            battery_week: false,
            point: None,
            rate: Cell::new(Rate::Off),
            images: None,
            icons: RefCell::new(HashMap::new()),
            awake,
            login: (false, false),
            phase: if demo { Phase::Shown } else { Phase::Hidden },
            now: time::unix_now(),
            blur_closed_at: f64::NEG_INFINITY,
            tick_due: 0.0,
            drawn_scale: Cell::new(0.0),
            last_anchor: None,
            last_item: None,
            settings_agent: Provider::Claude,
            replacing_key: false,
            confirm_remove_key: false,
            config_stamp,
        };
        if demo {
            crate::demo::fill(&mut ui);
        }
        ui.sync_rate();
        ui
    }

    /// The Finder icon of an app bundle, `size` points square, once it has decoded. A program
    /// outside any app (a daemon, a command-line tool, a script) shows the one icon macOS gives
    /// every program, so they all share a single decoded image, and so does an icon that
    /// couldn't be read.
    pub fn icon(&self, path: &std::path::Path, size: f32) -> Option<Image> {
        let image = self.request_icon(icon_source(path), size)?;
        if image.error().is_some() {
            return self.request_icon(std::path::Path::new(PROGRAM_ICON), size).filter(|image| image.error().is_none());
        }
        Some(image)
    }

    /// `path`'s icon, decoded off the main thread the first time it is asked for, and ready
    /// or failed now.
    fn request_icon(&self, path: &std::path::Path, size: f32) -> Option<Image> {
        let cache = self.images.as_ref()?;
        let mut icons = self.icons.borrow_mut();
        // Apps come and go: forget the lot now and then rather than track which still show. An
        // icon is a few kilobytes, so this many is still small.
        if icons.len() > ICONS_KEPT {
            icons.clear();
        }
        let image = icons.entry(path.to_owned()).or_insert_with(|| cache.app_icon_scaled(path, size, 2.0)).clone();
        if self.demo {
            // A headless render is one frame: it can't wait for the icon to arrive later.
            let _ = image.wait();
        }
        image.is_ready().then_some(image)
    }

    /// Starts decoding the icons of the apps this Mac's tabs would list, so they are there when
    /// the panel shows them rather than arriving a moment after.
    fn prefetch_icons(&self, view: &SystemView) {
        let Some(apps) = view.apps.as_deref() else { return };
        let mut order: Vec<&crate::system::apps::AppUsage> = apps.iter().collect();
        let measures: [fn(&crate::system::apps::AppUsage) -> f64; 3] = [|app| app.cpu as f64, |app| app.memory as f64, |app| app.disk as f64];
        for measure in measures {
            order.sort_unstable_by(|a, b| measure(b).total_cmp(&measure(a)));
            for app in order.iter().take(ICONS_AHEAD) {
                if let Some(path) = app.path.as_deref() {
                    self.request_icon(icon_source(path), crate::system_view::ICON);
                }
            }
        }
    }

    pub fn module(&self, tab: Tab) -> Option<Module> {
        match tab {
            Tab::Claude | Tab::Codex | Tab::DeepSeek | Tab::Sleep => None,
            Tab::Cpu => Some(self.config.cpu),
            Tab::Memory => Some(self.config.memory),
            Tab::Disk => Some(self.config.disk),
            Tab::Network => Some(self.config.network),
            Tab::Battery => Some(self.config.battery),
            Tab::Gpu => Some(self.config.gpu),
        }
    }

    fn module_mut(&mut self, tab: Tab) -> Option<&mut Module> {
        match tab {
            Tab::Claude | Tab::Codex | Tab::DeepSeek | Tab::Sleep => None,
            Tab::Cpu => Some(&mut self.config.cpu),
            Tab::Memory => Some(&mut self.config.memory),
            Tab::Disk => Some(&mut self.config.disk),
            Tab::Network => Some(&mut self.config.network),
            Tab::Battery => Some(&mut self.config.battery),
            Tab::Gpu => Some(&mut self.config.gpu),
        }
    }

    /// The agents the Agents tab has a tab for: each provider that is signed in, or whose
    /// login is still being read. Claude counts until it is known to be signed out; Codex and
    /// DeepSeek once they answered, so a provider that was never set up never flashes in.
    pub fn agents(&self) -> Vec<Tab> {
        let mut agents = Vec::new();
        for (tab, provider) in [(Tab::Claude, Provider::Claude), (Tab::Codex, Provider::Codex), (Tab::DeepSeek, Provider::DeepSeek)] {
            if self.config.has_tab(provider) && self.connected(provider) {
                agents.push(tab);
            }
        }
        agents
    }

    /// Whether `provider` is signed in, or its login is still being read. Claude counts until
    /// it is known to be signed out; Codex and DeepSeek once they answered.
    pub fn connected(&self, provider: Provider) -> bool {
        let known = |there: bool, problem: Option<&Problem>| there || problem.is_some_and(|p| !matches!(p, Problem::SignedOut));
        match provider {
            Provider::Claude => !matches!(self.claude.problem, Some(Problem::SignedOut)) || self.claude.snapshot.is_some(),
            Provider::Codex => known(self.codex.snapshot.is_some(), self.codex.problem.as_ref()),
            Provider::DeepSeek => known(self.deepseek.ledger.is_some(), self.deepseek.problem.as_ref()),
        }
    }

    /// Whether the top bar has its Agents tab: the settings show at least one agent's.
    pub fn agents_enabled(&self) -> bool {
        Provider::ALL.into_iter().any(|provider| self.config.has_tab(provider))
    }

    /// The agent the Agents tile opens: the one last shown, while it is still there.
    pub fn current_agent(&self) -> Tab {
        let agents = self.agents();
        if agents.contains(&self.agent_tab) { self.agent_tab } else { agents.first().copied().unwrap_or(Tab::Claude) }
    }

    /// The tab the panel falls back to when the one asked for isn't shown: the agents while
    /// they have their tab, else the first of the others. `None` when every tab is hidden.
    pub fn home_tab(&self) -> Option<Tab> {
        if self.agents_enabled() { Some(self.current_agent()) } else { self.bar_tabs().first().copied() }
    }

    /// This Mac's tabs the top bar shows, in its order. A Mac without a battery has no battery
    /// tab.
    pub fn mac_tabs(&self) -> Vec<Tab> {
        let no_battery = self.system.as_ref().is_some_and(|view| view.battery.is_none());
        Tab::SYSTEM.into_iter().filter(|&tab| self.module(tab).is_some_and(|m| m.tab) && !(tab == Tab::Battery && no_battery)).collect()
    }

    /// The tabs after Agents in the top bar, in its order: this Mac's measures, then Sleep.
    pub fn bar_tabs(&self) -> Vec<Tab> {
        let mut tabs = self.mac_tabs();
        if self.config.sleep.tab {
            tabs.push(Tab::Sleep);
        }
        tabs
    }

    /// The shortcut that shows `tab`: ⌘1 for the agents, ⌘2 on for the top bar's tabs after
    /// it in order.
    pub fn shortcut(&self, tab: Tab) -> Option<String> {
        if tab.provider().is_some() {
            return Some("⌘1".to_owned());
        }
        let at = self.bar_tabs().iter().position(|&t| t == tab)?;
        (at < 8).then(|| format!("⌘{}", at + 2))
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

    /// The measures beyond the CPU and memory that a tab or a menu bar item shows, and those
    /// of them a menu bar item shows, which are read as often as the item changes.
    fn wanted(&self) -> (u8, u8) {
        use crate::system::{WANT_BATTERY, WANT_DISK, WANT_GPU, WANT_NETWORK};
        [(Tab::Disk, WANT_DISK), (Tab::Network, WANT_NETWORK), (Tab::Battery, WANT_BATTERY), (Tab::Gpu, WANT_GPU)].into_iter().fold((0, 0), |(wanted, often), (tab, bit)| {
            let module = self.module(tab).unwrap_or_default();
            let in_menu_bar = module.menu_bar != MenuStyle::Off;
            (wanted | if module.tab || in_menu_bar { bit } else { 0 }, often | if in_menu_bar { bit } else { 0 })
        })
    }

    /// Tells the sampler the rate what is on screen needs, when it changed.
    pub fn sync_rate(&self) {
        if let Some(sampler) = &self.sampler {
            let (wanted, often) = self.wanted();
            sampler.set_wanted(wanted, often);
        }
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
            status_item::install_measure(&self.me, &self.data_dir, tab.id(), tab.label());
            // Not read yet: its name as it will read, and a dash for the number.
            let Some(measure) = self.system.as_ref().and_then(|view| measure(view, tab)) else {
                status_item::set(tab.id(), None, &[status_item::Part { tag: Some(tab.short().to_uppercase()), text: "–".to_owned(), severity: Severity::Normal }], true, tab.label());
                continue;
            };
            let percent = measure.share.map(|share| (share * 100.0).round().clamp(0.0, 100.0) as u8);
            let glyph = match (style, percent) {
                (MenuStyle::Ring, Some(percent)) => Some(status_item::Glyph::Ring(percent)),
                (MenuStyle::Graph, _) => Some(status_item::Glyph::Bars(measure.recent.iter().map(|v| (v.clamp(0.0, 1.0) * 100.0).round() as u8).collect())),
                _ => None,
            };
            let tag = (style == MenuStyle::Text || glyph.is_none()).then(|| tab.short().to_uppercase());
            status_item::set(tab.id(), glyph, &[status_item::Part { tag, text: measure.text, severity: measure.severity }], false, &measure.tooltip);
        }
        self.sync_app_item();
    }

    /// Keeps a way to the panel: when the settings leave the menu bar no item at all, Nerda
    /// Bar's own stands in, and goes when another is back.
    pub fn sync_app_item(&self) {
        if self.demo {
            return;
        }
        if status_item::others(status_item::APP) > 0 {
            status_item::remove(status_item::APP);
        } else if !status_item::installed(status_item::APP) {
            status_item::install(&self.me, &self.data_dir, status_item::APP, "Nerda Bar", &status_item::APP_MARK);
            status_item::set(status_item::APP, None, &[], false, "Nerda Bar: every menu bar item is switched off in its settings");
        }
    }
}

/// A click on the menu bar item `item` at `anchor`: opens the panel under it on its tab, or
/// closes it. With the panel open under the item on another display's menu bar, it moves there
/// instead.
pub fn toggle(s: &mut Ui, anchor: Anchor, item: &'static str, tx: &mut Tx) {
    // The same item on the same display, wherever it now sits: an item moves along the menu
    // bar whenever a neighbour's number changes width.
    let same_item = s.last_item == Some(item) && s.last_anchor.is_some_and(|last| last.screen == anchor.screen);
    let tab = Tab::from_id(item);
    match s.phase {
        Phase::Shown if same_item => close(s, tx),
        // A second click before the panel is up takes the first one back.
        Phase::Opening { .. } if same_item => s.phase = Phase::Hidden,
        // The click that took focus from the panel, on the item it was open under.
        Phase::Hidden if same_item && tx.clock() - s.blur_closed_at < SAME_CLICK_SECONDS => {}
        _ => {
            s.last_item = Some(item);
            open(s, anchor, tab, tx);
        }
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
    reload_config(s);
    s.page = Page::Tab;
    match shown_tab(s, tab) {
        Some(tab) => s.tab = tab,
        // Every tab is hidden: the settings are where one comes back.
        None => s.page = Page::Settings,
    }
    if s.tab.provider().is_some() {
        s.agent_tab = s.tab;
    }
    s.adding = false;
    s.replacing_key = false;
    s.confirm_remove_key = false;
    s.hovered = None;
    s.point = None;
    s.trace = None;
    if s.tab == Tab::Sleep {
        crate::awake::read(s);
    }
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
    // The icons of the apps this Mac's tabs list start decoding now, ahead of the first frame.
    if let Some(view) = s.system.clone() {
        s.prefetch_icons(&view);
    }
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
    // A question about spending a reset never waits behind a closed panel.
    if !matches!(s.reset, ResetFlow::Resetting { .. }) {
        s.reset = ResetFlow::Idle;
    }
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
            if s.page == Page::Tab && s.tab == Tab::Sleep {
                crate::awake::refresh_state(s);
            }
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
    crate::agents::apply(s, event, tx);
}

/// A reading of this Mac's measures. It redraws the panel only while the panel shows them.
pub fn apply_system(s: &mut Ui, view: Arc<SystemView>, tx: &mut Tx) -> bool {
    let shown = s.phase == Phase::Shown && s.page == Page::Tab && s.tab.is_system();
    if !shown {
        tx.instant();
    }
    // Icons are fetched for an open panel only, and only when the list of apps is a new one:
    // a panel that is never opened decodes none, and holds none in memory.
    let same_apps = match (s.system.as_ref().and_then(|last| last.apps.as_ref()), view.apps.as_ref()) {
        (Some(last), Some(now)) => Arc::ptr_eq(last, now),
        (None, None) => true,
        _ => false,
    };
    if s.phase != Phase::Hidden && !same_apps {
        s.prefetch_icons(&view);
    }
    s.system = Some(view);
    s.sync_system_items();
    shown
}

/// The tab to show for `tab`: itself while the panel has it, else the one it falls back to.
fn shown_tab(s: &Ui, tab: Tab) -> Option<Tab> {
    let there = if tab.provider().is_some() { s.agents().contains(&tab) } else { s.bar_tabs().contains(&tab) };
    if there { Some(tab) } else { s.home_tab() }
}

/// Moves off a tab the settings just hid, to the one the panel falls back to.
fn leave_hidden_tab(s: &mut Ui) {
    let shown = match s.tab.provider() {
        Some(provider) => s.config.has_tab(provider),
        None => s.bar_tabs().contains(&s.tab),
    };
    if !shown && let Some(tab) = s.home_tab() {
        s.tab = tab;
    }
}

/// Reads `config.json` again when it was edited by hand since it was last read or written
/// here, so a change made in the file shows the next time the panel opens, and a change made
/// in the panel never writes over it.
fn reload_config(s: &mut Ui) {
    if s.demo {
        return;
    }
    let stamp = config::modified(&s.data_dir);
    if stamp == s.config_stamp {
        return;
    }
    s.config_stamp = stamp;
    match config::read(&s.data_dir) {
        Some(Ok(edited)) if edited != s.config => {
            s.config = edited;
            apply_config(s);
        }
        Some(Err(reason)) => eprintln!("nerda-bar: config.json is not valid, keeping the settings as they were (kept as config.invalid.json): {reason}"),
        _ => {}
    }
}

/// Puts the settings as they now are into effect everywhere: the menu bar, the sampler, the
/// worker and the panel's material.
fn apply_config(s: &mut Ui) {
    s.material = if s.config.material.eq_ignore_ascii_case("glass") { SystemMaterial::Glass } else { SystemMaterial::Popover };
    s.send(Command::Every(s.config.refresh_seconds()));
    for provider in Provider::ALL {
        s.send(Command::Enabled(provider, s.config.shows(provider)));
    }
    leave_hidden_tab(s);
    s.sync_rate();
    s.sync_menu_bar();
    s.sync_system_items();
    crate::awake::sync_item(s);
}

/// Shows `tab` in the panel.
pub fn set_tab(s: &mut Ui, tab: Tab, tx: &mut Tx) {
    // A different tab, not a change to this one: nothing slides or fades across.
    tx.instant();
    s.tab = tab;
    if tab.provider().is_some() {
        s.agent_tab = tab;
    }
    s.page = Page::Tab;
    s.adding = false;
    s.hovered = None;
    s.point = None;
    s.trace = None;
    s.sync_rate();
    if tab == Tab::Sleep {
        crate::awake::read(s);
    }
}

/// Shows the Agents tab, on the agent it last showed.
pub fn show_agents(s: &mut Ui, tx: &mut Tx) {
    if s.agents_enabled() {
        let agent = s.current_agent();
        set_tab(s, agent, tx);
    }
}

/// Moves the Agents tab `step` agents along, wrapping round.
fn cycle_agent(s: &mut Ui, step: isize, tx: &mut Tx) -> bool {
    let agents = s.agents();
    if s.page != Page::Tab || s.tab.provider().is_none() || agents.len() < 2 {
        return false;
    }
    let at = agents.iter().position(|&t| t == s.tab).unwrap_or(0) as isize;
    let next = agents[(at + step).rem_euclid(agents.len() as isize) as usize];
    set_tab(s, next, tx);
    true
}

/// Opens or closes the Add an agent popover.
pub fn toggle_adding(s: &mut Ui, tx: &mut Tx) {
    tx.instant();
    s.adding = !s.adding;
    s.replacing_key = false;
    s.key_error = None;
    if !s.adding {
        s.deepseek_key = TextEdit::new("").secure();
    }
}

/// Shows the Agents tab with the Add an agent popover open: Settings' Add buttons lead here.
/// An agent being added gets its tab back if the settings had hidden it.
pub fn start_adding(s: &mut Ui, tx: &mut Tx) {
    if !s.config.deepseek_tab {
        set_agent_tab(s, Provider::DeepSeek, true, tx);
    }
    show_agents(s, tx);
    s.page = Page::Tab;
    s.adding = true;
    s.replacing_key = false;
    s.key_error = None;
}

/// The same popover, asking for a new DeepSeek key in place of the one there.
pub fn replace_deepseek_key(s: &mut Ui, tx: &mut Tx) {
    start_adding(s, tx);
    s.replacing_key = true;
    s.deepseek_key = TextEdit::new("").secure();
}

/// Removes DeepSeek's key from the Keychain, and with it the balance and its history. The
/// first press asks; the second, on the same button, does it.
pub fn remove_deepseek_key(s: &mut Ui, tx: &mut Tx) {
    tx.instant();
    if !s.confirm_remove_key {
        s.confirm_remove_key = true;
        return;
    }
    s.confirm_remove_key = false;
    if !s.demo
        && let Err(reason) = crate::deepseek::remove_key()
    {
        s.key_error = Some(reason);
        return;
    }
    s.key_error = None;
    s.deepseek = DeepSeek { problem: Some(Problem::SignedOut), ..DeepSeek::default() };
    s.send(Command::ForgetDeepSeek);
    leave_hidden_tab(s);
    if s.tab == Tab::DeepSeek {
        s.tab = s.current_agent();
    }
    s.sync_menu_bar();
}

pub fn cancel_remove_key(s: &mut Ui) {
    s.confirm_remove_key = false;
}

/// Saves the DeepSeek key typed in the Add an agent popover to the Keychain, and reads the
/// balance with it. The DeepSeek tab appears once the balance arrives.
pub fn add_deepseek_key(s: &mut Ui, tx: &mut Tx) {
    let key = s.deepseek_key.text.trim().to_owned();
    if key.is_empty() {
        s.key_error = Some("Paste the key first.".to_owned());
        return;
    }
    if !s.demo
        && let Err(reason) = crate::deepseek::save_key(&key)
    {
        s.key_error = Some(reason);
        return;
    }
    s.deepseek_key = TextEdit::new("").secure();
    s.key_error = None;
    s.adding = false;
    s.agent_tab = Tab::DeepSeek;
    // Another key is another account: its balance starts its own history.
    let replaced = std::mem::take(&mut s.replacing_key);
    if replaced {
        s.deepseek = DeepSeek::default();
    }
    s.send(Command::Enabled(Provider::DeepSeek, true));
    s.send(if replaced { Command::ForgetDeepSeek } else { Command::Refresh(Provider::DeepSeek) });
    tx.instant();
}

/// Reads `provider`'s login again, after the person signed in.
pub fn check_again(s: &mut Ui, provider: Provider) {
    s.send(Command::Refresh(provider));
}

pub fn set_battery_week(s: &mut Ui, week: bool) {
    s.battery_week = week;
}

/// Ejects the volume mounted at `mount`, off the main thread: it can take a few seconds while
/// the disk finishes writing. The next reading drops it from the list.
pub fn eject(s: &mut Ui, mount: &std::path::Path) {
    if s.demo {
        return;
    }
    let mount = mount.to_owned();
    std::thread::spawn(move || {
        if let Err(reason) = crate::system::disk::eject(&mount) {
            eprintln!("nerda-bar: can't eject {}: {reason}", mount.display());
        }
    });
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
    leave_hidden_tab(s);
    save(s);
    s.sync_rate();
}

/// Shows or hides the Sleep tab.
pub fn set_sleep_tab(s: &mut Ui, on: bool, tx: &mut Tx) {
    tx.instant();
    s.config.sleep.tab = on;
    leave_hidden_tab(s);
    save(s);
}

/// Whether a keep-awake puts a cup in the menu bar while it runs.
pub fn set_sleep_menu_bar(s: &mut Ui, on: bool) {
    s.config.sleep.menu_bar = on;
    save(s);
    crate::awake::sync_item(s);
}

fn save(s: &mut Ui) {
    if !s.demo {
        config::save(&s.data_dir, &s.config);
        s.config_stamp = config::modified(&s.data_dir);
    }
}

/// Hides a measure's tab, for a headless scene.
pub fn hide_tab_for_scene(s: &mut Ui, tab: Tab) {
    if let Some(module) = s.module_mut(tab) {
        module.tab = false;
    }
}

/// Writes the settings after a change made outside this file's own setters.
pub fn save_config(s: &mut Ui) {
    save(s);
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

/// A measure as its menu bar item shows it.
struct Measure {
    /// The share a ring fills, when the measure is one.
    share: Option<f32>,
    text: String,
    severity: Severity,
    /// The last half minute, each from 0 to 1, for a graph.
    recent: Vec<f32>,
    tooltip: String,
}

fn measure(view: &SystemView, tab: Tab) -> Option<Measure> {
    let share_text = |share: f32| format!("{:.0}%", (share * 100.0).clamp(0.0, 100.0));
    let scaled = |series: &[Vec<Option<crate::system::history::Point>>; 3]| {
        let recent = SystemView::recent(series, GRAPH_BARS);
        let top = recent.iter().copied().fold(0.0f32, f32::max).max(1.0);
        recent.iter().map(|v| v / top).collect::<Vec<f32>>()
    };
    Some(match tab {
        Tab::Memory => {
            let memory = view.memory.as_ref()?;
            let severity = match memory.pressure {
                Pressure::Critical => Severity::Critical,
                Pressure::Warning => Severity::Warning,
                Pressure::Normal => Severity::Normal,
            };
            let tooltip = format!("Memory: {} of {} used", system_bytes(memory.used()), system_bytes(memory.total));
            Measure { share: Some(memory.used_share()), text: share_text(memory.used_share()), severity, recent: SystemView::recent(&view.memory_used, GRAPH_BARS), tooltip }
        }
        Tab::Disk => {
            let boot = view.disk.as_ref()?.volumes.first()?;
            let share = boot.used() as f32 / boot.total.max(1) as f32;
            let tooltip = format!("Disk: {} used of {}, {} available", crate::mac_view::disk_bytes(boot.used()), crate::mac_view::disk_bytes(boot.total), crate::mac_view::disk_bytes(boot.available));
            Measure { share: Some(share), text: share_text(share), severity: Severity::Normal, recent: scaled(&view.disk_read), tooltip }
        }
        Tab::Network => {
            let net = view.net.as_ref()?;
            let (down, up) = (crate::mac_view::rate_string(net.down), crate::mac_view::rate_string(net.up));
            Measure { share: None, text: format!("↓{down} ↑{up}"), severity: Severity::Normal, recent: scaled(&view.net_down), tooltip: format!("Network: {down} down, {up} up") }
        }
        Tab::Battery => {
            let battery = view.battery.as_ref()?;
            let severity = match battery.charge {
                _ if battery.plugged => Severity::Normal,
                charge if charge <= 0.1 => Severity::Critical,
                charge if charge <= 0.2 => Severity::Warning,
                _ => Severity::Normal,
            };
            let tooltip = format!("Battery: {}{}", share_text(battery.charge), if battery.charging { ", charging" } else if battery.plugged { ", plugged in" } else { "" });
            let recent = view.charge.iter().rev().take(GRAPH_BARS).rev().map(|p| p.charge).collect();
            Measure { share: Some(battery.charge), text: share_text(battery.charge), severity, recent, tooltip }
        }
        Tab::Gpu => {
            let gpu = view.gpu.as_ref()?;
            Measure { share: Some(gpu.busy), text: share_text(gpu.busy), severity: Severity::Normal, recent: SystemView::recent(&view.gpu_busy, GRAPH_BARS), tooltip: format!("GPU: {} busy", share_text(gpu.busy)) }
        }
        _ => {
            let cpu = view.cpu.as_ref()?;
            let tooltip = format!("CPU: {:.0}% in use, {:.0}% apps, {:.0}% system", cpu.total() * 100.0, cpu.user * 100.0, cpu.system * 100.0);
            Measure { share: Some(cpu.total()), text: share_text(cpu.total()), severity: Severity::Normal, recent: SystemView::recent(&view.cpu_total, GRAPH_BARS), tooltip }
        }
    })
}

/// Refreshes the provider whose tab shows, or Claude's from elsewhere; on Sleep, reads sleep's
/// state again.
pub fn refresh(s: &mut Ui) {
    if s.page == Page::Tab && s.tab == Tab::Sleep {
        crate::awake::read(s);
        return;
    }
    s.send(Command::Refresh(s.tab.provider().unwrap_or(Provider::Claude)));
}

/// The provider whose limits the panel shows now.
pub fn shown_provider(s: &Ui) -> Provider {
    s.tab.provider().unwrap_or(Provider::Claude)
}

pub fn set_range(s: &mut Ui, range: Range) {
    s.range = range;
    s.hovered = None;
}

/// Shows `id`'s history in the limit chart.
pub fn select_limit(s: &mut Ui, id: &str) {
    let provider = shown_provider(s);
    s.agent_mut(provider).chart = Some(id.to_owned());
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
    s.adding = false;
    s.confirm_remove_key = false;
    // The settings open on the options of the agent the panel was showing.
    if s.page == Page::Settings && let Some(provider) = s.tab.provider() {
        s.settings_agent = provider;
    }
    s.point = None;
    s.sync_rate();
    if s.page == Page::Settings && !s.demo {
        // Launch at Login can also be switched in System Settings: read it each time.
        s.login = status_item::launch_at_login();
    }
}

/// Which limit `provider`'s menu bar item shows.
pub fn set_menu_bar(s: &mut Ui, provider: Provider, metric: model::MenuBarMetric) {
    match provider {
        Provider::Codex => s.config.codex.menu_bar = metric,
        _ => s.config.menu_bar = metric,
    }
    save(s);
    s.sync_menu_bar();
}

/// Whether `provider`'s menu bar item also shows each model's own limit.
pub fn set_menu_bar_models(s: &mut Ui, provider: Provider, on: bool) {
    match provider {
        Provider::Codex => s.config.codex.models = on,
        _ => s.config.menu_bar_models = on,
    }
    save(s);
    s.sync_menu_bar();
}

/// Whether `provider` has an item in the menu bar. An agent that shows nowhere is not read.
pub fn set_agent_menu_bar(s: &mut Ui, provider: Provider, on: bool) {
    s.config.set_in_menu_bar(provider, on);
    save(s);
    s.send(Command::Enabled(provider, s.config.shows(provider)));
    s.sync_menu_bar();
}

/// Whether `provider` has a tab in the panel.
pub fn set_agent_tab(s: &mut Ui, provider: Provider, on: bool, tx: &mut Tx) {
    tx.instant();
    s.config.set_tab(provider, on);
    leave_hidden_tab(s);
    save(s);
    s.send(Command::Enabled(provider, s.config.shows(provider)));
}

/// The agent whose options the settings show.
pub fn set_settings_agent(s: &mut Ui, provider: Provider, tx: &mut Tx) {
    tx.instant();
    s.settings_agent = provider;
    s.confirm_remove_key = false;
}

/// Minutes between refreshes of every agent.
pub fn set_refresh_minutes(s: &mut Ui, minutes: u32) {
    s.config.refresh_minutes = minutes;
    save(s);
    s.send(Command::Every(s.config.refresh_seconds()));
}

/// The amount under which DeepSeek's balance reads as a warning; 0 for never.
pub fn set_deepseek_warn_below(s: &mut Ui, amount: f64) {
    s.config.deepseek_warn_below = amount;
    save(s);
    s.sync_menu_bar();
}

/// The panel's material: Liquid Glass, or frosted vibrancy.
pub fn set_material(s: &mut Ui, glass: bool, tx: &mut Tx) {
    tx.instant();
    s.config.material = if glass { "glass" } else { "popover" }.to_owned();
    s.material = if glass { SystemMaterial::Glass } else { SystemMaterial::Popover };
    save(s);
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

/// Switches `provider`'s percentages between what is used and what is left, in the panel and
/// in its menu bar item, and remembers it.
pub fn set_provider_show_used(s: &mut Ui, provider: Provider, show_used: bool) {
    match provider {
        Provider::Codex => s.config.codex.show_used = show_used,
        _ => s.config.show_used = show_used,
    }
    save(s);
    s.sync_menu_bar();
}

pub fn on_key(s: &mut Ui, ev: &KeyEvent, tx: &mut Tx) -> bool {
    match &ev.key {
        Key::Escape if s.adding => toggle_adding(s, tx),
        Key::Escape if s.confirm_remove_key => cancel_remove_key(s),
        Key::Escape if matches!(s.reset, ResetFlow::Confirm(_) | ResetFlow::Failed { .. }) => crate::agents::cancel_reset(s),
        Key::Escape if s.page == Page::Settings => toggle_settings(s, tx),
        Key::Escape => close(s, tx),
        Key::Command(',') => toggle_settings(s, tx),
        Key::Command('r') => refresh(s),
        Key::Command('q') => tx.quit(),
        // ⌘1: the agents. ⌘2, ⌘3…: the top bar's tabs after it, in its order.
        Key::Command('1') if s.agents_enabled() => show_agents(s, tx),
        Key::Command(c @ '2'..='9') => match s.bar_tabs().get(*c as usize - '2' as usize) {
            Some(&tab) => set_tab(s, tab, tx),
            None => return false,
        },
        // ⌘⇧[ and ⌘⇧]: the previous and next agent, as tabs switch across macOS.
        Key::Command('[' | '{') if ev.mods.shift => return cycle_agent(s, -1, tx),
        Key::Command(']' | '}') if ev.mods.shift => return cycle_agent(s, 1, tx),
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
        let ui = Ui::new(app.handle(), None, None, Config::default(), PathBuf::new(), (None, None), true);
        let mut rt = Runtime::new(ui, crate::view::view, (WINDOW_W, WINDOW_H), Env { system_dark: true, scale: 2.0 });
        rt.on_key = Some(on_key);
        rt.frame(0.0);
        rt
    }

    #[test]
    fn the_panel_draws_its_limits_and_usage() {
        let rt = runtime();
        for key in ["panel", "top-bar", "context", "agent-claude", "add-agent", "limit-weekly", "limit-weekly-fable", "limit-chart", "limit-pill", "usage", "chart", "totals", "projects", "project-0", "refresh"] {
            assert!(rt.presented(key).is_some(), "{key} is on screen");
        }
        // The panel is its one size, and fits the window with its shadow room to spare.
        let [_, y, _, h] = rt.presented("panel").unwrap().0;
        assert_eq!(h, PANEL_H);
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
        let default = rt.state.claude.limits(rt.state.now).and_then(|snapshot| rt.state.claude.chart_limit(&snapshot).map(|l| l.id.clone()));
        assert_eq!(default.as_deref(), Some("weekly-fable"), "the weekly limit with the least left");
        click(&mut rt, "limit-weekly");
        assert_eq!(rt.state.claude.chart.as_deref(), Some("weekly"));
        assert!(!rt.state.config.show_used, "a click on a limit no longer flips the percentages");
        click(&mut rt, "limit-weekly-fable");
        assert_eq!(rt.state.claude.chart.as_deref(), Some("weekly-fable"));
        rt.update(|s, tx| {
            tx.instant();
            crate::demo::plan(s, "Pro", 76.0, 41.0, 58.0);
        });
        rt.frame(1.0 / 60.0);
        click(&mut rt, "limit-session");
        assert_eq!(rt.state.claude.chart.as_deref(), Some("session"));
        // One chart, under the limits that pick it; every row keeps its ring.
        let [_, chart_y, _, _] = rt.presented("limit-chart").unwrap().0;
        let [_, rows_y, _, rows_h] = rt.presented("limits").unwrap().0;
        assert!(rows_y + rows_h < chart_y, "the chart is under the limits");
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
        let (fx, fy) = centre(&rt, "context");
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
        let (x, y) = centre(&rt, "context");
        rt.pointer_move(x, y);
        rt.frame(1.0 / 60.0);
        assert_eq!(rt.state.hovered, None);
    }

    #[test]
    fn the_settings_button_swaps_the_body_and_settings_apply() {
        let mut rt = runtime();
        click(&mut rt, "tab-settings");
        assert_eq!(rt.state.page, Page::Settings);
        assert!(rt.presented("limit-weekly").is_none() && rt.presented("settings-page").is_some());
        click(&mut rt, "menu-bar-Weekly");
        assert_eq!(rt.state.config.menu_bar, model::MenuBarMetric::Weekly);
        click(&mut rt, "percent-Used");
        assert!(rt.state.config.show_used);
        click(&mut rt, "tab-settings");
        assert_eq!(rt.state.page, Page::Tab);
    }

    #[test]
    fn a_click_that_closed_the_panel_does_not_reopen_it() {
        let mut rt = runtime();
        let anchor = Anchor { x: 900.0, y: 30.0, screen: [0.0, 1512.0], scale: 2.0 };
        (rt.state.last_anchor, rt.state.last_item) = (Some(anchor), Some("claude"));
        rt.state.demo = false;
        rt.update(|s, tx| on_focus(s, false, tx));
        rt.frame(1.0 / 60.0);
        assert_eq!(rt.state.phase, Phase::Hidden);
        rt.update(move |s, tx| toggle(s, anchor, "claude", tx));
        rt.frame(1.0 / 60.0);
        assert_eq!(rt.state.phase, Phase::Hidden);
    }

    #[test]
    fn the_panel_shows_at_once_on_a_display_of_the_scale_it_was_drawn_at() {
        let mut rt = runtime();
        rt.update(close);
        rt.frame(1.0 / 60.0);
        let anchor = Anchor { x: 900.0, y: 30.0, screen: [0.0, 1512.0], scale: 2.0 };
        rt.update(move |s, tx| toggle(s, anchor, "claude", tx));
        rt.frame(1.0 / 60.0);
        assert_eq!(rt.state.phase, Phase::Shown);
    }

    #[test]
    fn on_a_display_of_another_scale_the_panel_waits_to_be_drawn_at_it() {
        let mut rt = runtime();
        rt.update(close);
        rt.frame(1.0 / 60.0);
        let anchor = Anchor { x: -400.0, y: 30.0, screen: [-1080.0, 0.0], scale: 1.0 };
        rt.update(move |s, tx| toggle(s, anchor, "claude", tx));
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
        rt.update(move |s, tx| toggle(s, here, "claude", tx));
        rt.frame(1.0 / 60.0);
        assert_eq!(rt.state.phase, Phase::Shown);
        // The click on the other item first takes focus from the panel, then arrives.
        rt.state.demo = false;
        rt.update(|s, tx| on_focus(s, false, tx));
        rt.update(move |s, tx| toggle(s, there, "claude", tx));
        rt.frame(1.0 / 60.0);
        assert_eq!(rt.state.phase, Phase::Shown);
        assert_eq!(rt.state.last_anchor, Some(there));
        // And with the panel still open, the same.
        rt.update(move |s, tx| toggle(s, here, "claude", tx));
        rt.frame(1.0 / 60.0);
        assert_eq!((rt.state.phase, rt.state.last_anchor), (Phase::Shown, Some(here)));
        // A click on the item it is open under closes it.
        rt.update(move |s, tx| toggle(s, here, "claude", tx));
        assert_eq!(rt.state.phase, Phase::Hidden);
    }

    #[test]
    fn a_display_that_never_redraws_still_gets_the_panel() {
        let mut rt = runtime();
        rt.update(close);
        rt.frame(1.0 / 60.0);
        let anchor = Anchor { x: -400.0, y: 30.0, screen: [-1080.0, 0.0], scale: 1.0 };
        rt.update(move |s, tx| toggle(s, anchor, "claude", tx));
        rt.update(|s, tx| { poll(s, 10.0, tx); });
        rt.frame(1.0 / 60.0);
        assert_eq!(rt.state.phase, Phase::Shown);
    }

    #[test]
    fn the_top_bar_switches_tabs_and_each_draws_its_own() {
        let mut rt = runtime();
        for key in ["top-bar", "tab-agents", "tab-cpu", "tab-memory", "tab-settings"] {
            assert!(rt.presented(key).is_some(), "{key} is on screen");
        }
        click(&mut rt, "tab-cpu");
        assert_eq!(rt.state.tab, Tab::Cpu);
        assert!(rt.presented("cpu-chart").is_some() && rt.presented("chart-readout").is_some());
        assert!(rt.presented("limit-weekly").is_none());
        click(&mut rt, "tier-24h");
        assert_eq!(rt.state.tier, 2);
        rt.key(KeyEvent { key: Key::Command('3'), mods: Mods::default() });
        rt.frame(1.0 / 60.0);
        assert_eq!(rt.state.tab, Tab::Memory, "⌘3 is this Mac's second tab");
        rt.key(KeyEvent { key: Key::Command('1'), mods: Mods::default() });
        rt.frame(1.0 / 60.0);
        assert_eq!(rt.state.tab, Tab::Claude);
        let [_, _, _, h] = rt.presented("panel").unwrap().0;
        assert_eq!(h, PANEL_H, "every tab keeps the panel's size");
    }

    #[test]
    fn the_agents_tab_switches_agents_and_remembers_the_last() {
        let mut rt = runtime();
        click(&mut rt, "agent-codex");
        assert_eq!(rt.state.tab, Tab::Codex);
        click(&mut rt, "tab-cpu");
        click(&mut rt, "tab-agents");
        assert_eq!(rt.state.tab, Tab::Codex, "Agents goes back to the agent it last showed");
        let next = KeyEvent { key: Key::Command(']'), mods: Mods { shift: true, cmd: true, ..Mods::default() } };
        rt.key(next.clone());
        rt.frame(1.0 / 60.0);
        assert_eq!(rt.state.tab, Tab::DeepSeek);
        rt.key(next);
        rt.frame(1.0 / 60.0);
        assert_eq!(rt.state.tab, Tab::Claude, "the next agent wraps round");
    }

    #[test]
    fn add_opens_its_popover_and_escape_closes_it() {
        let mut rt = runtime();
        rt.update(|s, tx| {
            tx.instant();
            s.deepseek.ledger = None;
            s.deepseek.problem = Some(Problem::SignedOut);
        });
        rt.frame(1.0 / 60.0);
        assert!(rt.presented("agent-deepseek").is_none(), "an agent never set up has no tab");
        click(&mut rt, "add-agent");
        assert!(rt.state.adding && rt.presented("add-popover").is_some() && rt.presented("deepseek-key").is_some());
        click(&mut rt, "add-deepseek");
        assert_eq!(rt.state.key_error.as_deref(), Some("Paste the key first."));
        rt.key(KeyEvent { key: Key::Escape, mods: Mods::default() });
        rt.frame(1.0 / 60.0);
        assert!(!rt.state.adding && rt.state.phase == Phase::Shown, "esc closes the popover before the panel");
    }

    #[test]
    fn with_no_agent_the_tab_says_how_to_connect_one() {
        let mut rt = runtime();
        rt.update(|s, tx| {
            tx.instant();
            for agent in [&mut s.claude, &mut s.codex] {
                agent.snapshot = None;
                agent.problem = Some(Problem::SignedOut);
            }
            s.deepseek.ledger = None;
            s.deepseek.problem = Some(Problem::SignedOut);
        });
        rt.frame(1.0 / 60.0);
        assert!(rt.presented("no-agents").is_some() && rt.presented("check-claude").is_some() && rt.presented("add-key").is_some());
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
        let (fx, fy) = centre(&rt, "context");
        rt.pointer_move(fx, fy);
        rt.frame(1.0 / 60.0);
        assert_eq!(rt.state.point, None);
    }

    #[test]
    fn a_hidden_tab_leaves_the_top_bar_and_falls_back() {
        let mut rt = runtime();
        click(&mut rt, "tab-cpu");
        click(&mut rt, "tab-settings");
        click(&mut rt, "cpu-tab");
        assert_eq!(rt.state.page, Page::Settings, "a switch keeps the settings open");
        assert!(!rt.state.config.cpu.tab);
        assert_eq!(rt.state.tab, Tab::Claude, "the hidden tab gives way to the agents");
        click(&mut rt, "memory-tab");
        assert!(!rt.state.config.memory.tab);
        for tab in [Tab::Disk, Tab::Network, Tab::Battery, Tab::Gpu] {
            click(&mut rt, &format!("{}-tab", tab.id()));
        }
        click(&mut rt, "tab-settings");
        assert!(rt.state.mac_tabs().is_empty());
        // The top bar keeps Agents and Settings, with no measures and no hairline between.
        assert!(rt.presented("tab-agents").is_some() && rt.presented("tab-cpu").is_none() && rt.presented("bar-separator").is_none());
        assert!(rt.presented("limit-weekly").is_some());
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
        for tab in Tab::SYSTEM {
            *rt.state.module_mut(tab).unwrap() = Module { menu_bar: MenuStyle::Off, tab: false };
        }
        assert_eq!(rt.state.wanted_rate(), Rate::Off);
    }

    #[test]
    fn each_provider_has_its_tab_and_a_reset_is_asked_before_it_is_spent() {
        let mut rt = runtime();
        click(&mut rt, "agent-codex");
        for key in ["limit-session", "limit-weekly", "resets", "use-0", "limit-chart", "usage"] {
            assert!(rt.presented(key).is_some(), "{key} is on Codex's tab");
        }
        click(&mut rt, "use-0");
        assert_eq!(rt.state.reset, ResetFlow::Confirm("demo-1".to_owned()));
        assert!(rt.presented("reset-confirm-card").is_some());
        click(&mut rt, "reset-cancel");
        assert_eq!(rt.state.reset, ResetFlow::Idle, "Cancel spends nothing");
        click(&mut rt, "use-0");
        click(&mut rt, "reset-confirm");
        let ResetFlow::Resetting { request, .. } = rt.state.reset.clone() else { panic!("the reset runs") };
        rt.update(|s, tx| apply(s, Event::Redeemed(crate::codex::Redeemed::Unknown("The server answered 503.".to_owned())), tx));
        rt.frame(1.0 / 60.0);
        assert!(matches!(&rt.state.reset, ResetFlow::Failed { certain: false, .. }), "an unanswered reset keeps the question and says so");
        click(&mut rt, "reset-confirm");
        assert!(matches!(&rt.state.reset, ResetFlow::Resetting { request: again, .. } if *again == request), "a retry reuses the request's id");
        rt.update(|s, tx| apply(s, Event::Redeemed(crate::codex::Redeemed::Failed("No.".to_owned())), tx));
        rt.key(KeyEvent { key: Key::Escape, mods: Mods::default() });
        rt.frame(1.0 / 60.0);
        assert_eq!((rt.state.reset.clone(), rt.state.phase), (ResetFlow::Idle, Phase::Shown), "esc takes the question back before it closes the panel");
        click(&mut rt, "agent-deepseek");
        for key in ["balance", "balance-total", "spend", "spend-6", "top-up"] {
            assert!(rt.presented(key).is_some(), "{key} is on DeepSeek's tab");
        }
    }

    #[test]
    fn the_panel_keeps_its_size_in_its_tallest_states() {
        let mut rt = runtime();
        let fits = |rt: &Runtime<Ui>, what: &str| {
            let [_, y, _, h] = rt.presented("panel").unwrap().0;
            assert_eq!(h, PANEL_H, "{what}: the panel is {h} tall");
            assert!(y + h + SHADOW_ROOM <= WINDOW_H, "{what}: the panel is {h} tall");
        };
        rt.update(|s, tx| {
            tx.instant();
            crate::demo::plan(s, "Pro", 94.0, 73.0, 92.0);
            s.claude.problem = Some(model::Problem::Offline("Couldn't reach Claude.".to_owned()));
        });
        rt.frame(1.0);
        fits(&rt, "Claude running low and offline");
        let [_, py, _, ph] = rt.presented("panel").unwrap().0;
        let [_, ly, _, lh] = rt.presented("limit-legend").unwrap().0;
        assert!(ly + lh <= py + ph, "the chart's legend stays inside the panel under the banner");
        click(&mut rt, "agent-codex");
        click(&mut rt, "use-0");
        rt.frame(2.0);
        fits(&rt, "Codex asking about a reset");
        click(&mut rt, "tab-settings");
        rt.frame(3.0);
        fits(&rt, "the settings");
    }

    #[test]
    fn the_sleep_tab_keeps_the_mac_awake_for_the_time_picked() {
        let mut rt = runtime();
        click(&mut rt, "tab-sleep");
        assert_eq!(rt.state.tab, Tab::Sleep);
        for key in ["keep-awake", "durations", "while-awake", "keep-display", "keep-lid", "holders", "holder-0", "when-idle", "display-after", "sleep-after", "power-lid", "now-sleep", "now-lock"] {
            assert!(rt.presented(key).is_some(), "{key} is on the Sleep tab");
        }
        assert!(rt.state.awake.session.is_none());
        click(&mut rt, "duration-1h");
        let session = rt.state.awake.session.as_ref().expect("a session runs");
        assert_eq!((session.minutes, session.until.map(|until| until - session.started)), (60, Some(3600)));
        click(&mut rt, "duration-No limit");
        assert_eq!(rt.state.awake.session.as_ref().map(|s| (s.minutes, s.until)), Some((0, None)), "no limit runs until it is turned off");
        click(&mut rt, "keep-lid");
        assert_eq!(rt.state.awake.session.as_ref().map(|s| s.lid), Some(crate::awake::LidHold::On));
        click(&mut rt, "duration-off");
        assert!(rt.state.awake.session.is_none(), "Off lets it sleep again");
        click(&mut rt, "display-after-10m");
        assert_eq!(rt.state.awake.view.as_ref().and_then(|v| v.battery.as_ref()).and_then(|t| t.display), Some(10));
        click(&mut rt, "idle-adapter");
        assert!(rt.presented("display-after-20m").is_some(), "the adapter's own timers show");
        let [_, _, _, h] = rt.presented("panel").unwrap().0;
        assert_eq!(h, PANEL_H);
    }

    #[test]
    fn sleep_follows_the_mac_tabs_in_the_shortcuts_and_can_be_hidden() {
        let mut rt = runtime();
        assert_eq!(rt.state.shortcut(Tab::Sleep).as_deref(), Some("⌘8"), "after this Mac's six tabs");
        rt.key(KeyEvent { key: Key::Command('8'), mods: Mods::default() });
        rt.frame(1.0 / 60.0);
        assert_eq!(rt.state.tab, Tab::Sleep);
        assert_eq!(rt.state.wanted_rate(), Rate::Background, "Sleep reads no measures");
        click(&mut rt, "tab-settings");
        click(&mut rt, "sleep-tab");
        assert!(!rt.state.config.sleep.tab);
        assert_eq!(rt.state.tab, Tab::Claude, "the hidden tab gives way to the agents");
        click(&mut rt, "tab-settings");
        assert!(rt.presented("tab-sleep").is_none() && rt.presented("sleep-separator").is_none());
    }

    #[test]
    fn the_sleep_tab_keeps_the_panels_size_with_its_banner() {
        let mut rt = runtime();
        rt.update(|s, tx| {
            tx.instant();
            set_tab(s, Tab::Sleep, tx);
            s.awake.left_off = true;
        });
        rt.frame(1.0);
        assert!(rt.presented("sleep-banner").is_some() && rt.presented("allow-sleep").is_some());
        let [_, py, _, ph] = rt.presented("panel").unwrap().0;
        assert_eq!(ph, PANEL_H);
        for key in ["now-display-off", "holders"] {
            let [_, y, _, h] = rt.presented(key).unwrap().0;
            assert!(y + h <= py + ph + 0.5, "{key} stays inside the panel under the banner");
        }
        click(&mut rt, "allow-sleep");
        assert!(!rt.state.awake.left_off);
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
        let before = rt.state.claude.snapshot.clone();
        rt.update(|s, tx| apply(s, Event::Limits(Provider::Claude, Err(model::Problem::Offline("No network.".to_owned()))), tx));
        rt.frame(1.0 / 60.0);
        assert_eq!(rt.state.claude.snapshot, before);
        assert!(rt.state.claude.problem.is_some());
        assert!(rt.presented("problem").is_some());
    }

    #[test]
    fn a_click_on_the_item_that_moved_along_the_menu_bar_still_closes_the_panel() {
        let mut rt = runtime();
        rt.update(close);
        rt.frame(1.0 / 60.0);
        let anchor = Anchor { x: 900.0, y: 30.0, screen: [0.0, 1512.0], scale: 2.0 };
        rt.update(move |s, tx| toggle(s, anchor, "memory", tx));
        rt.frame(1.0 / 60.0);
        assert_eq!((rt.state.phase, rt.state.tab), (Phase::Shown, Tab::Memory));
        // A neighbour's number grew a digit: the item now sits four points along.
        let moved = Anchor { x: 904.0, ..anchor };
        rt.update(move |s, tx| toggle(s, moved, "memory", tx));
        assert_eq!(rt.state.phase, Phase::Hidden, "the same item closes the panel, wherever it is now");
        // Another item at the very same place opens it on its own tab.
        rt.update(move |s, tx| toggle(s, moved, "cpu", tx));
        rt.frame(1.0 / 60.0);
        assert_eq!((rt.state.phase, rt.state.tab), (Phase::Shown, Tab::Cpu));
    }

    #[test]
    fn each_providers_percentages_switch_on_their_own() {
        let mut rt = runtime();
        // As an item's menu does it: by the provider, whatever tab the panel is on.
        rt.update(|s, _| set_provider_show_used(s, Provider::Codex, true));
        assert!(rt.state.config.codex.show_used && !rt.state.config.show_used, "Codex's menu leaves Claude's as it was");
        rt.update(|s, tx| set_tab(s, Tab::Codex, tx));
        rt.update(|s, _| set_provider_show_used(s, Provider::Claude, true));
        assert!(rt.state.config.show_used, "and Claude's is switched from Codex's tab");
    }

    #[test]
    fn every_agent_has_a_menu_bar_switch_a_tab_switch_and_its_options() {
        let mut rt = runtime();
        click(&mut rt, "tab-settings");
        for provider in Provider::ALL {
            for key in [format!("settings-{}", provider.id()), format!("{}-in-menu-bar", provider.id()), format!("{}-has-tab", provider.id()), format!("options-{}", provider.id())] {
                assert!(rt.presented(&key).is_some(), "{key} is in the settings");
            }
        }
        click(&mut rt, "claude-in-menu-bar");
        assert!(!rt.state.config.claude_in_menu_bar && rt.state.config.shows(Provider::Claude), "off the menu bar, Claude keeps its tab");
        click(&mut rt, "codex-in-menu-bar");
        click(&mut rt, "codex-has-tab");
        assert!(!rt.state.config.shows(Provider::Codex), "with neither, Codex is not read");
        assert!(!rt.state.agents().contains(&Tab::Codex));
        // Each agent's own options, picked at the group's end.
        click(&mut rt, "options-codex");
        assert_eq!(rt.state.settings_agent, Provider::Codex);
        click(&mut rt, "codex-menu-bar-Weekly");
        click(&mut rt, "codex-percent-Used");
        click(&mut rt, "codex-menu-bar-models");
        let codex = rt.state.config.codex;
        assert_eq!((codex.menu_bar, codex.show_used, codex.models), (model::MenuBarMetric::Weekly, true, true));
        assert_eq!(rt.state.config.menu_bar, model::MenuBarMetric::Session, "Claude's stay as they were");
        click(&mut rt, "options-deepseek");
        click(&mut rt, "deepseek-warn-5");
        assert_eq!(rt.state.config.deepseek_warn_below, 5.0);
        click(&mut rt, "refresh-every-15");
        assert_eq!(rt.state.config.refresh_seconds(), 900);
        click(&mut rt, "material-Glass");
        assert_eq!((rt.state.config.material.as_str(), rt.state.material), ("glass", SystemMaterial::Glass));
        let [_, _, _, h] = rt.presented("panel").unwrap().0;
        assert_eq!(h, PANEL_H);
    }

    #[test]
    fn deepseeks_key_is_removed_only_after_the_question() {
        let mut rt = runtime();
        click(&mut rt, "tab-settings");
        click(&mut rt, "options-deepseek");
        assert!(rt.presented("deepseek-key-replace").is_some());
        click(&mut rt, "deepseek-key-remove");
        assert!(rt.state.confirm_remove_key && rt.state.deepseek.ledger.is_some(), "the first press only asks");
        click(&mut rt, "deepseek-key-keep");
        assert!(!rt.state.confirm_remove_key && rt.state.deepseek.ledger.is_some());
        click(&mut rt, "deepseek-key-remove");
        click(&mut rt, "deepseek-key-remove");
        assert!(rt.state.deepseek.ledger.is_none() && !rt.state.connected(Provider::DeepSeek));
        assert!(rt.presented("settings-add-deepseek").is_some(), "with no key, the row offers to add one");
        // Replace opens the popover on a field, though a key is there.
        rt.update(|s, tx| {
            tx.instant();
            crate::demo::fill(s);
            replace_deepseek_key(s, tx);
        });
        rt.frame(1.0 / 60.0);
        assert!(rt.state.adding && rt.state.replacing_key && rt.presented("deepseek-key").is_some());
    }

    #[test]
    fn with_every_agent_tab_off_the_panel_is_this_macs() {
        let mut rt = runtime();
        click(&mut rt, "tab-settings");
        for provider in Provider::ALL {
            click(&mut rt, &format!("{}-has-tab", provider.id()));
        }
        assert!(!rt.state.agents_enabled());
        assert_eq!(rt.state.tab, Tab::Cpu, "the panel moves to the first tab left");
        assert!(rt.presented("tab-agents").is_none() && rt.presented("tab-cpu").is_some());
        rt.key(KeyEvent { key: Key::Command('1'), mods: Mods::default() });
        rt.frame(1.0 / 60.0);
        assert_eq!(rt.state.tab, Tab::Cpu, "⌘1 has no agents to show");
        // A click on an agent's menu bar item opens the panel on what it does show.
        rt.update(close);
        let anchor = Anchor { x: 900.0, y: 30.0, screen: [0.0, 1512.0], scale: 2.0 };
        rt.update(move |s, tx| toggle(s, anchor, "claude", tx));
        rt.frame(1.0 / 60.0);
        assert_eq!((rt.state.phase, rt.state.page, rt.state.tab), (Phase::Shown, Page::Tab, Tab::Cpu));
        // With this Mac's tabs and Sleep off too, the panel opens on the settings.
        rt.update(|s, tx| {
            tx.instant();
            for tab in Tab::SYSTEM {
                hide_tab_for_scene(s, tab);
            }
            s.config.sleep.tab = false;
            close(s, tx);
        });
        rt.update(move |s, tx| toggle(s, anchor, "claude", tx));
        rt.frame(1.0 / 60.0);
        assert_eq!((rt.state.phase, rt.state.page), (Phase::Shown, Page::Settings));
        click(&mut rt, "tab-settings");
        assert!(rt.presented("no-tabs").is_some() && rt.presented("open-settings").is_some(), "and behind them, the way back");
    }

    #[test]
    fn limits_from_before_a_sign_out_say_so() {
        let mut rt = runtime();
        rt.update(|s, tx| apply(s, Event::Limits(Provider::Claude, Err(Problem::SignedOut)), tx));
        rt.frame(1.0 / 60.0);
        assert!(rt.state.claude.snapshot.is_some(), "a login that can't be read for a moment keeps the limits");
        assert!(rt.presented("problem").is_some() && rt.presented("limit-weekly").is_some(), "with why they are not current");
    }

    #[test]
    fn sleep_that_is_off_for_no_session_offers_to_turn_it_on() {
        let mut rt = runtime();
        rt.update(|s, tx| {
            tx.instant();
            set_tab(s, Tab::Sleep, tx);
            // Sleep is off and no flag says an earlier run did it: the button is still there.
            if let Some(view) = &mut s.awake.view {
                Arc::make_mut(view).sleep_disabled = true;
            }
        });
        rt.frame(1.0);
        assert!(!rt.state.awake.left_off && rt.presented("allow-sleep").is_some());
        // While this app's own session holds the lid, sleep being off is as asked.
        click(&mut rt, "keep-lid");
        click(&mut rt, "duration-1h");
        // The banner leaves over a moment: look once it has.
        rt.frame(5.0);
        assert!(rt.presented("allow-sleep").is_none());
        click(&mut rt, "duration-off");
        rt.frame(10.0);
        assert!(rt.presented("allow-sleep").is_none(), "ending the session turned sleep back on");
    }

    #[test]
    fn the_keep_awake_counts_minutes_up_everywhere() {
        // One second short of five hours reads "5:00" in the ring, as the status reads "5h".
        assert_eq!(crate::awake::left_words(5 * 3600 - 10), "5h");
        let left = 5 * 3600 - 10;
        let minutes_left = (left + 59) / 60;
        assert_eq!(format!("{}:{:02}", minutes_left / 60, minutes_left % 60), "5:00");
    }
}
