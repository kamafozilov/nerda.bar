//! How the panel looks: a rail of tabs, and the tab beside it. The Agents tab has the provider
//! and its plan, a meter per limit, then usage from the session logs as totals, a chart and a
//! row per model; this Mac's tabs are in `system_view`.

use rune::{Color, Cx, El, Icon, TextStyle, col, el, icon, row, rule, spacer, text};

use crate::config::MenuStyle;
use crate::model::{self, ExtraUsage, Limit, MenuBarMetric, Severity, Snapshot};
use crate::stats::{CACHE_READ, CACHE_WRITE_1H, CACHE_WRITE_5M, INPUT, OUTPUT, RangeView};
use crate::system_view;
use crate::time;
use crate::ui::{self, CONTENT_W, PANEL_W, Page, RAIL_W, Range, SHADOW_ROOM, Tab, Ui};

const RADIUS: f32 = 18.0;
pub(crate) const PAD_X: f32 = 16.0;
pub(crate) const INNER_W: f32 = CONTENT_W - 2.0 * PAD_X;
const METER_H: f32 = 6.0;
const CHART_H: f32 = 56.0;
const MODEL_ROWS: usize = 5;

pub static CLAUDE: Icon = Icon {
    name: "nerda-claude",
    view: 100.0,
    body: concat!(r#"<path d=""#, include_str!("../assets/providers/claude.path"), r#"" fill="currentColor"/>"#),
};
static REFRESH: Icon = Icon {
    name: "nerda-refresh",
    view: 24.0,
    body: r#"<path d="M19.5 12 A7.5 7.5 0 1 1 17.3 6.7" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"/><path d="M19.6 3.6 V7.6 H15.6" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>"#,
};

static SETTINGS: Icon = Icon {
    name: "nerda-settings",
    view: 24.0,
    body: r#"<path d="M4 8 H12 M18 8 H20 M4 16 H6 M12 16 H20" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"/><circle cx="15" cy="8" r="2.5" fill="none" stroke="currentColor" stroke-width="2"/><circle cx="9" cy="16" r="2.5" fill="none" stroke="currentColor" stroke-width="2"/>"#,
};
static AGENTS: Icon = Icon {
    name: "nerda-agents",
    view: 24.0,
    body: r#"<path d="M12 3.5 C12.5 8.2 15.8 11.5 20.5 12 C15.8 12.5 12.5 15.8 12 20.5 C11.5 15.8 8.2 12.5 3.5 12 C8.2 11.5 11.5 8.2 12 3.5 Z" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linejoin="round"/>"#,
};
static CPU: Icon = Icon {
    name: "nerda-cpu",
    view: 24.0,
    body: r#"<path d="M8 6 H16 A2 2 0 0 1 18 8 V16 A2 2 0 0 1 16 18 H8 A2 2 0 0 1 6 16 V8 A2 2 0 0 1 8 6 Z" fill="none" stroke="currentColor" stroke-width="1.8"/><path d="M10 10 H14 V14 H10 Z" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linejoin="round"/><path d="M10 3 V6 M14 3 V6 M10 18 V21 M14 18 V21 M3 10 H6 M3 14 H6 M18 10 H21 M18 14 H21" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"/>"#,
};
static MEMORY: Icon = Icon {
    name: "nerda-memory",
    view: 24.0,
    body: r#"<path d="M4.5 6.5 H19.5 A1.5 1.5 0 0 1 21 8 V15 H3 V8 A1.5 1.5 0 0 1 4.5 6.5 Z" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linejoin="round"/><path d="M7 9.5 V12 M10.5 9.5 V12 M14 9.5 V12 M17.5 9.5 V12 M6 15 V18 M9 15 V18 M12 15 V18 M15 15 V18 M18 15 V18" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"/>"#,
};

static QUIT: Icon = Icon {
    name: "nerda-quit",
    view: 24.0,
    body: r#"<path d="M12 4 V11.5" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"/><path d="M7.4 7 A7 7 0 1 0 16.6 7" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"/>"#,
};

/// The colours of one appearance.
#[derive(Clone, Copy)]
pub(crate) struct Look {
    pub text: Color,
    pub dim: Color,
    pub faint: Color,
    pub hover: Color,
    pub stroke: Color,
    pub track: Color,
    pub tick: Color,
    pub chip: Color,
    /// Behind a group of rows.
    pub card: Color,
    pub panel: Color,
    pub panel_edge: Color,
    pub shadow: Color,
    pub bar: Color,
    pub bar_hover: Color,
    pub normal: Color,
    pub warning: Color,
    pub critical: Color,
    /// Warning and critical as text, darker in light mode where the fills are too pale to read.
    pub warning_text: Color,
    pub critical_text: Color,
    pub brand: Color,
    /// Status: all is well (memory pressure).
    pub good: Color,
    /// The chart series, in this fixed order: blue, teal, purple. Checked for colour-blind
    /// separation and against the panel in both appearances.
    pub series: [Color; 3],
}

impl Look {
    pub(crate) fn new(dark: bool) -> Look {
        let (white, black) = (Color::WHITE, Color::BLACK);
        let hex = |s: &str| rune::color::hex(s).unwrap_or(Color::WHITE);
        if dark {
            Look {
                text: white.alpha(0.92),
                dim: white.alpha(0.58),
                faint: white.alpha(0.36),
                hover: white.alpha(0.08),
                stroke: white.alpha(0.09),
                track: white.alpha(0.12),
                tick: white.alpha(0.7),
                chip: white.alpha(0.1),
                card: white.alpha(0.055),
                panel: hex("#1e1e21").alpha(0.9),
                panel_edge: white.alpha(0.14),
                shadow: black.alpha(0.45),
                bar: white.alpha(0.78),
                bar_hover: white,
                normal: hex("#0a84ff"),
                warning: hex("#ff9f0a"),
                critical: hex("#ff453a"),
                warning_text: hex("#ff9f0a"),
                critical_text: hex("#ff453a"),
                brand: hex("#d97757"),
                good: hex("#30d158"),
                series: [hex("#0a84ff"), hex("#1aa3b5"), hex("#bf5af2")],
            }
        } else {
            Look {
                text: black.alpha(0.88),
                dim: black.alpha(0.55),
                faint: black.alpha(0.36),
                hover: black.alpha(0.06),
                stroke: black.alpha(0.08),
                track: black.alpha(0.09),
                tick: black.alpha(0.6),
                chip: black.alpha(0.07),
                card: black.alpha(0.04),
                panel: hex("#f6f6f8").alpha(0.92),
                panel_edge: white.alpha(0.7),
                shadow: black.alpha(0.22),
                bar: black.alpha(0.7),
                bar_hover: black.alpha(0.9),
                normal: hex("#007aff"),
                warning: hex("#ff9500"),
                critical: hex("#ff3b30"),
                warning_text: hex("#b25e00"),
                critical_text: hex("#d6001a"),
                brand: hex("#d97757"),
                good: hex("#28a745"),
                series: [hex("#007aff"), hex("#1597ab"), hex("#a64fe0")],
            }
        }
    }

    fn fill(&self, severity: Severity) -> Color {
        match severity {
            Severity::Normal => self.normal,
            Severity::Warning => self.warning,
            Severity::Critical => self.critical,
        }
    }

    fn ink(&self, severity: Severity) -> Color {
        match severity {
            Severity::Normal => self.text,
            Severity::Warning => self.warning_text,
            Severity::Critical => self.critical_text,
        }
    }
}

pub(crate) const fn font(size: f32, weight: f32) -> TextStyle {
    TextStyle::new(size, weight)
}

/// Digits that keep their width, so a changing number doesn't shift what is beside it.
pub(crate) const fn digits(size: f32, weight: f32) -> TextStyle {
    TextStyle::new(size, weight).features("tnum")
}

pub fn view(s: &Ui, cx: &mut Cx<Ui>) -> El {
    let l = Look::new(cx.system_dark());
    // What a display of another scale waits for before it shows the panel.
    s.drawn_scale.set(cx.env().scale);
    let limits = s.limits();

    let mut panel = col().key("content").w(CONTENT_W).child(match (s.page, s.tab) {
        (Page::Settings, _) => settings_header(&l),
        (_, Tab::Agents) => header(&l, limits.as_ref()),
        (_, tab) => system_view::header(s, &l, tab),
    });

    if s.page == Page::Settings {
        panel = panel.child(settings(s, &l));
    } else if s.tab == Tab::Cpu {
        panel = panel.child(system_view::cpu(s, &l));
    } else if s.tab == Tab::Memory {
        panel = panel.child(system_view::memory(s, &l));
    } else {
        if let Some(problem) = &s.problem {
            panel = panel.child(
                col()
                    .key("problem")
                    .w_full()
                    .px(PAD_X)
                    .pb(12.0)
                    .gap(2.0)
                    .child(text(problem.title()).font(font(12.0, 600.0)).color(l.warning_text))
                    .child(text(problem.detail(s.now)).font(font(11.5, 400.0).lh(15.5)).color(l.dim).w_full().wrap()),
            );
        }
        match &limits {
            Some(snapshot) => {
                let mut rows = col().key("limits").w_full().px(PAD_X).pb(14.0).gap(14.0);
                for limit in &snapshot.limits {
                    rows = rows.child(limit_row(&l, limit, s.now, s.config.show_used));
                }
                if let Some(extra) = &snapshot.extra {
                    rows = rows.child(extra_row(&l, extra));
                }
                panel = panel.child(rows);
            }
            None if s.problem.is_none() => {
                panel = panel.child(row().key("loading").w_full().px(PAD_X).pb(14.0).child(text("Reading your limits…").font(font(12.5, 400.0)).color(l.dim)));
            }
            None => {}
        }
        panel = panel.child(rule(1.0, l.stroke)).child(usage(s, &l));
    }
    panel = panel.child(rule(1.0, l.stroke)).child(footer(s, &l));

    let tabs = s.tabs();
    // One tab needs no rail: the panel is the tab alone, as it was before there were tabs.
    let panel = if tabs.len() > 1 {
        row().w(PANEL_W).child(rail(s, &l, &tabs)).child(el().key("rail-edge").w(1.0).self_stretch().fill(l.stroke)).child(panel)
    } else {
        row().w(CONTENT_W).child(panel)
    };
    let panel = panel
        .key("panel")
        .items_start()
        .radius(RADIUS)
        .continuous_corners()
        .fill(l.panel)
        .border(1.0, l.panel_edge)
        .shadow(10.0, 30.0, l.shadow)
        .system_material(s.material)
        // Presses on the panel stay with it; only the empty window around it closes.
        .on_press(|_: &mut Ui| {});

    let root = col().key("root").w_full().h_full().px(SHADOW_ROOM).pt(ui::GAP).items_center().on_press_tx(|s: &mut Ui, tx| ui::close(s, tx)).child(panel);
    // A headless render has no desktop behind the window: stand a wallpaper in for it.
    if s.demo {
        let hex = |s: &str| rune::color::hex(s).unwrap_or(Color::BLACK);
        let (top, bottom) = if cx.system_dark() { (hex("#16213e"), hex("#5b2a6e")) } else { (hex("#8ec5fc"), hex("#e0c3fc")) };
        root.gradient(top, bottom)
    } else {
        root
    }
}

/// The tabs, one above the other: each its icon and its name, the open one on a chip.
fn rail(s: &Ui, l: &Look, tabs: &[Tab]) -> El {
    let mut rail = col().key("rail").w(RAIL_W - 1.0).self_stretch().px(6.0).py(8.0).gap(2.0);
    for (at, &tab) in tabs.iter().enumerate() {
        let on = s.page == Page::Tab && s.tab == tab;
        let glyph = match tab {
            Tab::Agents => &AGENTS,
            Tab::Cpu => &CPU,
            Tab::Memory => &MEMORY,
        };
        rail = rail.child(
            col()
                .key(format!("tab-{}", tab.id()))
                .role("tab")
                .label(tab.label())
                .help(format!("{} (⌘{})", tab.label(), at + 1))
                .w_full()
                .h(50.0)
                .radius(10.0)
                .continuous_corners()
                .center()
                .gap(4.0)
                .fill(if on { l.chip } else { Color::TRANSPARENT })
                .hover(l.hover)
                .press_scale(0.96)
                .on_press_tx(move |s: &mut Ui, tx| ui::set_tab(s, tab, tx))
                .child(icon(glyph, 18.0).color(if on { l.normal } else { l.dim }))
                .child(text(tab.label()).font(font(10.5, if on { 600.0 } else { 500.0 })).color(if on { l.text } else { l.dim })),
        );
    }
    rail
}

/// The title row of a tab: its name, and a chip saying what it is about.
pub(crate) fn title_bar(l: &Look, title: &str, chip: Option<&str>) -> El {
    let mut bar = row().key("header").w_full().h(50.0).px(PAD_X).gap(8.0).center_y().child(text(title).font(font(15.0, 600.0)).color(l.text));
    if let Some(chip) = chip.filter(|chip| !chip.is_empty()) {
        bar = bar.child(row().key("chip").h(18.0).px(6.0).radius(5.0).center().fill(l.chip).child(text(chip).font(font(10.5, 600.0)).color(l.dim)));
    }
    bar
}

fn settings_header(l: &Look) -> El {
    title_bar(l, "Settings", None).child(spacer()).child(text(concat!("Nerda Bar ", env!("CARGO_PKG_VERSION"))).font(digits(11.0, 400.0)).color(l.faint))
}

fn header(l: &Look, limits: Option<&Snapshot>) -> El {
    let mut bar = row()
        .key("header")
        .w_full()
        .h(50.0)
        .px(PAD_X)
        .gap(8.0)
        .center_y()
        .child(icon(&CLAUDE, 18.0).color(l.brand))
        .child(text("Claude").font(font(15.0, 600.0)).color(l.text));
    if let Some(plan) = limits.map(|snapshot| snapshot.plan.as_str()).filter(|plan| !plan.is_empty()) {
        bar = bar.child(row().key("plan").h(18.0).px(6.0).radius(5.0).center().fill(l.chip).child(text(plan).font(font(10.5, 600.0)).color(l.dim)));
    }
    bar
}

/// A row of choices where one is on, as AppKit's segmented control.
pub(crate) fn segments(l: &Look, key: &str) -> El {
    row().key(key).h(24.0).px(2.0).radius(8.0).fill(l.track).center_y()
}

pub(crate) fn segment(l: &Look, key: String, label: &str, on: bool, pick: impl Fn(&mut Ui) + 'static) -> El {
    row()
        .key(key)
        .role("button")
        .label(label)
        .h(20.0)
        .px(9.0)
        .radius(6.0)
        .center()
        .fill(if on { l.chip.mix(l.text, 0.12) } else { Color::TRANSPARENT })
        .on_press(pick)
        .child(text(label).font(font(11.5, if on { 600.0 } else { 500.0 })).color(if on { l.text } else { l.dim }))
}

/// A switch, as System Settings draws one.
pub(crate) fn switch(l: &Look, key: &str, label: &str, on: bool, enabled: bool, flip: impl Fn(&mut Ui) + 'static) -> El {
    let mut switch = row()
        .key(key)
        .role("switch")
        .label(label)
        .size(36.0, 22.0)
        .radius(11.0)
        .px(2.0)
        .center_y()
        .fill(if on { l.normal } else { l.track })
        .opacity(if enabled { 1.0 } else { 0.45 })
        .child(el().size(18.0, 18.0).radius(9.0).fill(Color::WHITE));
    if on {
        switch = switch.justify_end();
    }
    if enabled {
        switch = switch.on_press(flip);
    }
    switch
}

/// The settings, in place of the limits and usage: one line each, name on the left and its
/// control on the right, as System Settings lays a group out.
fn settings(s: &Ui, l: &Look) -> El {
    let setting = |name: &'static str, control: El| row().w_full().h(44.0).px(12.0).center_y().child(text(name).font(font(13.0, 400.0)).color(l.text)).child(spacer()).child(control);

    let mut menu_bar = segments(l, "menu-bar");
    for (metric, label) in [(MenuBarMetric::Session, "Session"), (MenuBarMetric::Weekly, "Weekly"), (MenuBarMetric::Lowest, "Lowest")] {
        menu_bar = menu_bar.child(segment(l, format!("menu-bar-{label}"), label, s.config.menu_bar == metric, move |s: &mut Ui| ui::set_menu_bar(s, metric)));
    }
    let mut percent = segments(l, "percent");
    for (used, label) in [(false, "Left"), (true, "Used")] {
        percent = percent.child(segment(l, format!("percent-{label}"), label, s.config.show_used == used, move |s: &mut Ui| ui::set_show_used(s, used)));
    }

    let (can_login, login) = (s.login.0 || s.demo, s.login.1);
    let mut login_switch = switch(l, "login", "Launch at Login", login, can_login, move |s: &mut Ui| ui::set_launch_at_login(s, !login));
    if !can_login {
        login_switch = login_switch.help("Available when run as Nerda Bar.app");
    }
    let models = s.config.menu_bar_models;
    let models_switch = switch(l, "menu-bar-models", "Model limits in the menu bar", models, true, move |s: &mut Ui| ui::set_menu_bar_models(s, !models));

    let open_file = row()
        .key("settings-file")
        .role("button")
        .label("Open settings file")
        .h(24.0)
        .px(12.0)
        .radius(8.0)
        .center()
        .fill(l.track)
        .hover(l.hover)
        .on_press_tx(ui::open_settings_file)
        .child(text("Open").font(font(11.5, 500.0)).color(l.text));

    // A group of settings under its caption, the rows split by hairlines.
    let group = |key: &'static str, caption: &'static str, rows: Vec<El>| {
        let mut card = col().key(key).w_full().radius(10.0).fill(l.card);
        for (at, setting) in rows.into_iter().enumerate() {
            if at > 0 {
                card = card.child(rule(1.0, l.stroke).mx(12.0));
            }
            card = card.child(setting);
        }
        col().w_full().gap(6.0).child(text(caption).font(font(11.0, 600.0)).color(l.dim).px(12.0)).child(card)
    };
    let module = |tab: Tab| {
        let module = s.module(tab).unwrap_or_default();
        let mut styles = segments(l, &format!("{}-menu-bar", tab.id()));
        for style in MenuStyle::ALL {
            styles = styles.child(segment(l, format!("{}-menu-bar-{}", tab.id(), style.label()), style.label(), module.menu_bar == style, move |s: &mut Ui| ui::set_module_menu_bar(s, tab, style)));
        }
        let shown = module.tab;
        let tab_switch = switch(l, &format!("{}-tab", tab.id()), &format!("{} tab", tab.label()), shown, true, |_: &mut Ui| {})
            .on_press_tx(move |s: &mut Ui, tx| ui::set_module_tab(s, tab, !shown, tx));
        vec![setting("Menu bar", styles), setting("Tab in the panel", tab_switch)]
    };

    col()
        .key("settings-page")
        .w_full()
        .px(PAD_X)
        .pb(14.0)
        .gap(14.0)
        .child(group("settings-group", "Claude", vec![setting("Menu bar shows", menu_bar), setting("Percentages", percent), setting("Model limits", models_switch)]))
        .child(group("settings-cpu", "CPU", module(Tab::Cpu)))
        .child(group("settings-memory", "Memory", module(Tab::Memory)))
        .child(group("settings-general", "General", vec![setting("Launch at Login", login_switch), setting("Settings file", open_file)]))
}

/// A track with its filled share, and a tick where an even pace would have the fill end.
fn meter(l: &Look, share: f64, fill: Color, tick: Option<f64>) -> El {
    let share = share.clamp(0.0, 1.0) as f32;
    let mut track = el().key("meter").w_full().h(METER_H).radius(METER_H / 2.0).fill(l.track);
    if share > 0.0 {
        // Never narrower than a dot: a sliver of a limit still shows.
        track = track.child(el().key("fill").absolute(0.0, 0.0).size((INNER_W * share).max(METER_H), METER_H).radius(METER_H / 2.0).fill(fill));
    }
    if let Some(tick) = tick {
        let x = (INNER_W * tick.clamp(0.0, 1.0) as f32 - 1.0).clamp(0.0, INNER_W - 2.0);
        track = track.child(el().key("pace").absolute(x, -2.0).size(2.0, METER_H + 4.0).radius(1.0).fill(l.tick));
    }
    track
}

fn limit_row(l: &Look, limit: &Limit, now: i64, show_used: bool) -> El {
    let severity = limit.severity();
    let share = if show_used { limit.used / 100.0 } else { limit.left() / 100.0 };
    // The tick marks an even pace: where the fill would be if use had kept step with time.
    let tick = limit.elapsed(now).filter(|_| !limit.used_up()).map(|elapsed| if show_used { elapsed } else { 1.0 - elapsed });
    let value = format!("{} {}", model::percent(limit, show_used), if show_used { "used" } else { "left" });
    let reset = match limit.resets_in(now) {
        Some(left) => format!("Resets in {}", time::span(left)),
        None if limit.used <= 0.0 => "Starts with your next message".to_owned(),
        None => "Resetting".to_owned(),
    };
    let mut notes = row().w_full().center_y().child(text(reset).font(digits(11.0, 400.0)).color(l.dim)).child(spacer());
    if limit.used_up() {
        notes = notes.child(text("Used up").font(font(11.0, 500.0)).color(l.critical_text));
    } else if let Some(runway) = limit.runs_out_in(now) {
        notes = notes.child(text(format!("Runs out in {}", time::span(runway))).font(digits(11.0, 500.0)).color(l.warning_text));
    }
    col()
        .key(format!("limit-{}", limit.id))
        .w_full()
        .gap(7.0)
        .child(
            row()
                .w_full()
                .center_y()
                .child(text(limit.label.as_str()).font(font(13.0, 500.0)).color(l.text))
                .child(spacer())
                .child(text(value).font(digits(13.0, 600.0)).color(l.ink(severity))),
        )
        .child(meter(l, share, l.fill(severity), tick))
        .child(notes)
        .role("button")
        .label(format!("{}: {}", limit.label, model::percent(limit, show_used)))
        .help(if show_used { "Click to show what is left" } else { "Click to show what is used" })
        .on_press(move |s: &mut Ui| ui::set_show_used(s, !show_used))
}

fn extra_row(l: &Look, extra: &ExtraUsage) -> El {
    let amount = |value: f64| if extra.currency == "USD" { model::money(value) } else { format!("{value:.2} {}", extra.currency) };
    let value = match extra.limit {
        Some(limit) => format!("{} of {}", amount(extra.used), amount(limit)),
        None => amount(extra.used),
    };
    let mut body = col().key("limit-extra").w_full().gap(7.0).child(
        row()
            .w_full()
            .center_y()
            .child(text("Extra usage").font(font(13.0, 500.0)).color(l.text))
            .child(spacer())
            .child(text(value).font(digits(13.0, 600.0)).color(l.text)),
    );
    if let Some(limit) = extra.limit {
        let spent = extra.used / limit;
        let fill = if spent >= 0.9 { l.critical } else if spent >= 0.7 { l.warning } else { l.normal };
        body = body.child(meter(l, spent, fill, None));
    }
    body
}

fn usage(s: &Ui, l: &Look) -> El {
    let mut ranges = segments(l, "ranges");
    for range in Range::ALL {
        ranges = ranges.child(segment(l, format!("range-{}", range.label()), range.label(), s.range == range, move |s: &mut Ui| ui::set_range(s, range)));
    }
    let mut body = col()
        .key("usage")
        .w_full()
        .px(PAD_X)
        .pt(12.0)
        .pb(14.0)
        .gap(12.0)
        .child(row().w_full().center_y().child(text("Usage").font(font(13.0, 600.0)).color(l.text)).child(spacer()).child(ranges));

    let Some(stats) = &s.stats else {
        return body.child(text("Reading Claude Code's session logs…").key("usage-loading").font(font(12.0, 400.0)).color(l.dim));
    };
    let view = &stats.ranges[s.range.index()];
    if view.total() == 0 {
        return body.child(text("No Claude Code activity in this period.").key("usage-empty").font(font(12.0, 400.0)).color(l.dim));
    }

    let figure = |key: &'static str, value: String, caption: &'static str, end: bool| {
        let cell = col().key(key).gap(1.0).child(text(value).font(digits(22.0, 700.0).tracking(-0.4)).color(l.text)).child(text(caption).font(font(11.0, 400.0)).color(l.dim));
        if end { cell.items_end() } else { cell }
    };
    body = body.child(
        row()
            .key("totals")
            .w_full()
            .child(figure("total-tokens", model::tokens(view.total()), "tokens", false))
            .child(spacer())
            .child(figure("total-cost", view.cost_label(), "at API prices", true)),
    );
    body = body.child(chart(l, view, s.hovered));

    let part = |name: &'static str, count: u64| col().grow(1.0).gap(1.0).child(text(name).font(font(10.5, 400.0)).color(l.faint)).child(text(model::tokens(count)).font(digits(12.0, 500.0)).color(l.text));
    body = body.child(
        row()
            .key("parts")
            .w_full()
            .child(part("Input", view.tokens[INPUT]))
            .child(part("Output", view.tokens[OUTPUT]))
            .child(part("Cache write", view.tokens[CACHE_WRITE_5M] + view.tokens[CACHE_WRITE_1H]))
            .child(part("Cache read", view.tokens[CACHE_READ])),
    );
    body.child(models(l, view))
}

fn chart(l: &Look, view: &RangeView, hovered: Option<usize>) -> El {
    let peak_at = view.bars.iter().enumerate().max_by_key(|(_, bar)| bar.tokens).map_or(0, |(at, _)| at);
    let peak = view.bars.get(peak_at).map_or(0, |bar| bar.tokens).max(1) as f32;
    let few = view.bars.len() <= 7;
    let gap = if few { 6.0 } else { 2.0 };
    let mut bars = row()
        .key("chart")
        .w_full()
        .h(CHART_H)
        .gap(gap)
        // Leaving the chart lets go of the bar; crossing the gap between two bars doesn't.
        .on_hover_change(|s: &mut Ui, inside, _| {
            if !inside {
                ui::leave_chart(s);
            }
        });
    for (at, bar) in view.bars.iter().enumerate() {
        let height = if bar.tokens == 0 { 2.0 } else { (bar.tokens as f32 / peak * CHART_H).max(3.0) };
        let on = hovered == Some(at);
        // With a bar picked out, the others step back.
        let fill = match (bar.tokens == 0, on, hovered.is_some()) {
            (true, true, _) => l.dim,
            (true, ..) => l.track,
            (false, true, _) => l.bar_hover,
            (false, false, true) => l.bar.alpha(0.45),
            (false, false, false) => l.bar,
        };
        // The whole column answers the pointer, so a short bar is as easy to reach as a tall one.
        bars = bars.child(
            col()
                .key(format!("bar-{at}"))
                .grow(1.0)
                .min_w(0.0)
                .h(CHART_H)
                .justify_end()
                .on_hover_change(move |s: &mut Ui, inside, _| {
                    if inside {
                        ui::hover_bar(s, at);
                    }
                })
                .child(el().w_full().h(height).radius(if few { 3.0 } else { 1.5 }).fill(fill)),
        );
    }
    let mut axis = row().key("axis").w_full();
    if view.axis_per_bar {
        axis = axis.gap(gap);
        for (at, name) in view.axis.iter().enumerate() {
            axis = axis.child(row().grow(1.0).min_w(0.0).center().child(text(name.as_str()).font(digits(10.0, 400.0)).color(if hovered == Some(at) { l.text } else { l.faint })));
        }
    } else {
        axis = axis.justify_between();
        for name in &view.axis {
            axis = axis.child(text(name.as_str()).font(digits(10.0, 400.0)).color(l.faint));
        }
    }

    // One line reads a single bar: the one under the pointer, or the busiest. It keeps its
    // place and its shape, so moving across the chart changes only the words in it.
    let (lead, bar) = match hovered.and_then(|at| view.bars.get(at)) {
        Some(bar) => ("", Some(bar)),
        None => ("Peak · ", view.bars.get(peak_at)),
    };
    let (name, tokens, cost) = bar.map_or_else(Default::default, |bar| (format!("{lead}{}", bar.label), model::tokens(bar.tokens), model::money(bar.cost)));
    let readout = row()
        .key("readout")
        .w_full()
        .h(26.0)
        .px(8.0)
        .radius(6.0)
        .fill(l.card)
        .center_y()
        .child(text(name).font(digits(11.5, 500.0)).color(if hovered.is_some() { l.text } else { l.dim }))
        .child(spacer())
        .child(text(format!("{tokens} tokens")).font(digits(11.5, 400.0)).color(l.dim))
        .child(row().w(58.0).justify_end().child(text(cost).font(digits(11.5, 500.0)).color(l.text)));
    col().w_full().gap(5.0).child(bars).child(axis).child(el().h(3.0)).child(readout)
}

fn models(l: &Look, view: &RangeView) -> El {
    let most = view.models.iter().map(|m| m.tokens).max().unwrap_or(0).max(1) as f32;
    let mut list = col().key("models").w_full().gap(2.0);
    for (at, model) in view.models.iter().take(MODEL_ROWS).enumerate() {
        // A wash behind the row as wide as the model's share of the busiest one.
        let share = (model.tokens as f32 / most).clamp(0.0, 1.0);
        list = list.child(
            el().key(format!("model-{at}"))
                .w_full()
                .h(24.0)
                .child(el().absolute(0.0, 0.0).size((INNER_W * share).max(3.0), 24.0).radius(if INNER_W * share < 12.0 { 1.5 } else { 6.0 }).fill(l.chip))
                .child(
                    row()
                        .absolute(0.0, 0.0)
                        .size(INNER_W, 24.0)
                        .px(8.0)
                        .gap(8.0)
                        .center_y()
                        .child(text(model.name.as_str()).font(font(12.0, 500.0)).color(l.text).truncate().grow(1.0).min_w(0.0))
                        .child(text(model::tokens(model.tokens)).font(digits(12.0, 400.0)).color(l.dim))
                        .child(row().w(58.0).justify_end().child(text(model.cost.map_or_else(|| "—".to_owned(), model::money)).font(digits(12.0, 500.0)).color(l.text))),
                ),
        );
    }
    if view.models.len() > MODEL_ROWS {
        let rest = &view.models[MODEL_ROWS..];
        let tokens: u64 = rest.iter().map(|m| m.tokens).sum();
        list = list.child(text(format!("{} more models, {} tokens", rest.len(), model::tokens(tokens))).font(font(11.0, 400.0)).color(l.faint).px(8.0).pt(3.0));
    }
    list
}

fn footer(s: &Ui, l: &Look) -> El {
    let booted = s.system.as_ref().and_then(|view| view.machine.booted_at);
    let status = match (&s.snapshot, s.refreshing) {
        _ if s.tab.is_system() && s.page == Page::Tab => booted.map_or_else(String::new, |at| format!("Up {}", time::span(s.now - at))),
        (_, true) => "Updating…".to_owned(),
        (Some(snapshot), _) => {
            let age = s.now - snapshot.fetched_at;
            if age < 60 { "Updated just now".to_owned() } else { format!("Updated {} ago", time::span(age)) }
        }
        (None, _) => "Not updated yet".to_owned(),
    };
    let button = |key: &'static str, glyph: &'static Icon, name: &'static str, on: bool| {
        row()
            .key(key)
            .role("button")
            .label(name)
            .help(name)
            .size(26.0, 26.0)
            .radius(7.0)
            .center()
            .fill(if on { l.chip } else { Color::TRANSPARENT })
            .hover(l.hover)
            .press_scale(0.94)
            .child(icon(glyph, 15.0).color(if on { l.text } else { l.dim }))
    };
    row()
        .key("footer")
        .w_full()
        .h(40.0)
        .pl(PAD_X)
        .pr(9.0)
        .gap(2.0)
        .center_y()
        .child(text(status).key("status").font(digits(11.0, 400.0)).color(if s.problem.is_some() && s.tab == Tab::Agents { l.warning_text } else { l.faint }))
        .child(spacer())
        // This Mac's measures read on their own: only the limits have something to refresh.
        .child(if s.tab == Tab::Agents { button("refresh", &REFRESH, "Refresh", false).opacity(if s.refreshing { 0.45 } else { 1.0 }).on_press(ui::refresh) } else { el() })
        .child(button("settings", &SETTINGS, "Settings", s.page == Page::Settings).on_press_tx(ui::toggle_settings))
        .child(button("quit", &QUIT, "Quit Nerda Bar", false).on_press_tx(|_: &mut Ui, tx| tx.quit()))
}
