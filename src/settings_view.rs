//! The settings, in place of a tab: the general ones and the agents on the left, then Claude's;
//! this Mac's measures as a table of their menu bar style and whether they have a tab on the
//! right, then Codex's. A group is a caption over a card of rows, name on the left and its
//! control on the right, as System Settings lays a group out.

use rune::{Color, El, Icon, col, icon, row, text};

use crate::config::MenuStyle;
use crate::icons;
use crate::look::{Look, PAD_X, PAD_Y, columns, context_bar, context_title, font, segment, segments, switch};
use crate::model;
use crate::system_view;
use crate::ui::{self, Tab, Ui};

/// A row of a group's card.
const SETTING_H: f32 = 48.0;
/// The space between two groups of a column.
const GROUP_GAP: f32 = 20.0;
/// The width of This Mac's menu bar styles, the same on every row so the column lines up.
const STYLES_W: f32 = 172.0;
/// The width of a switch.
const SWITCH_W: f32 = 32.0;
/// The space between a row's controls, and at a card row's ends.
const ROW_PAD: f32 = 12.0;

pub fn screen(s: &Ui, l: &Look) -> crate::view::Screen {
    let version = concat!("Nerda Bar ", env!("CARGO_PKG_VERSION")).to_owned();
    let context = context_bar(l, context_title(l, "Settings", None, Some(version)), Some(quit_button(l)));
    let left = column("settings-left").child(general(s, l)).child(agents(s, l)).child(claude(s, l));
    let right = column("settings-right").child(this_mac(s, l)).child(codex(s, l));
    crate::view::Screen { context, body: columns(l, left.key("settings-page"), right) }
}

/// A column of groups.
fn column(key: &'static str) -> El {
    col().key(key).w_full().px(PAD_X).py(PAD_Y).gap(GROUP_GAP)
}

/// The button that quits the app, with its shortcut.
fn quit_button(l: &Look) -> El {
    row()
        .key("quit")
        .role("button")
        .label("Quit Nerda Bar")
        .h(28.0)
        .px(10.0)
        .gap(8.0)
        .radius(7.0)
        .continuous_corners()
        .center_y()
        .fill(l.chip)
        .hover(l.chip_on)
        .press_scale(0.96)
        .on_press_tx(|_: &mut Ui, tx| tx.quit())
        .child(icon(&icons::POWER, 13.0).color(l.dim))
        .child(text("Quit Nerda Bar").font(font(12.0, 500.0)).color(l.text))
        .child(text("⌘Q").font(font(11.0, 400.0)).color(l.faint))
}

/// A group: its caption, with something at its end, over a card of rows split by hairlines.
fn group(l: &Look, key: &'static str, mark: &'static Icon, mark_color: Color, name: &str, end: Option<El>, rows: Vec<El>) -> El {
    let mut caption = row().w_full().h(16.0).pl(4.0).pr(ROW_PAD).gap(6.0).center_y().child(icon(mark, 12.0).color(mark_color)).child(text(name.to_owned()).font(font(11.0, 600.0)).color(l.dim));
    if let Some(end) = end {
        caption = caption.child(row().grow(1.0).min_w(0.0).justify_end().child(end));
    }
    let mut card = col().w_full().radius(10.0).continuous_corners().fill(l.card);
    for (at, setting) in rows.into_iter().enumerate() {
        card = card.child(if at > 0 { setting.border_sides([1.0, 0.0, 0.0, 0.0], l.stroke) } else { setting });
    }
    col().key(key).w_full().gap(8.0).child(caption).child(card)
}

/// A row of a card: what leads it (a mark and a name), and its control at the end. The name
/// keeps its width; the control gives way.
fn setting(lead: El, control: El) -> El {
    row().w_full().h(SETTING_H).px(ROW_PAD).gap(ROW_PAD).center_y().child(lead).child(row().grow(1.0).min_w(0.0).justify_end().center_y().child(control))
}

/// A setting's name.
fn name(l: &Look, words: &str) -> El {
    text(words.to_owned()).font(font(12.0, 400.0)).color(l.text)
}

/// Words at the end of a row, which shorten before the name does.
fn note(l: &Look, words: String) -> El {
    text(words).font(font(12.0, 400.0)).color(l.faint).truncate().min_w(0.0)
}

/// Left and Used, for how the percentages read.
fn percentages(l: &Look, key: &'static str, used_now: bool, pick: fn(&mut Ui, bool)) -> El {
    let mut percent = segments(l, key);
    for (used, label) in [(false, "Left"), (true, "Used")] {
        percent = percent.child(segment(l, format!("{key}-{label}"), label, used_now == used, move |s: &mut Ui| pick(s, used)));
    }
    percent
}

fn general(s: &Ui, l: &Look) -> El {
    let (can_login, login) = (s.login.0 || s.demo, s.login.1);
    let mut login_switch = switch(l, "login", "Launch at Login", login, can_login);
    if can_login {
        login_switch = login_switch.on_press(move |s: &mut Ui| ui::set_launch_at_login(s, !login));
    } else {
        login_switch = login_switch.help("Available when run as Nerda Bar.app");
    }
    let open_file = crate::look::button(l, "settings-file", "Open").label("Open settings file").on_press_tx(ui::open_settings_file);
    let rows = vec![setting(name(l, "Launch at Login"), login_switch), setting(name(l, "Settings file"), open_file)];
    group(l, "settings-general", &icons::SETTINGS_2, l.dim, "General", None, rows)
}

/// The agents and whether each is signed in; DeepSeek, without a key, offers to add one.
fn agents(s: &Ui, l: &Look) -> El {
    let signed_in = |plan: &str| if plan.is_empty() { "Signed in".to_owned() } else { format!("Signed in · {plan}") };
    let state = |snapshot: Option<&model::Snapshot>| note(l, snapshot.map_or_else(|| "Not signed in".to_owned(), |snapshot| signed_in(&snapshot.plan)));
    let deepseek = if s.deepseek.ledger.is_some() {
        deepseek_menu_bar(s, l)
    } else {
        row()
            .key("settings-add-deepseek")
            .role("button")
            .label("Add DeepSeek")
            .h(24.0)
            .px(10.0)
            .radius(6.0)
            .continuous_corners()
            .center()
            .border(1.0, l.edge)
            .hover(l.chip)
            .press_scale(0.96)
            .on_press_tx(ui::start_adding)
            .child(text("Add").font(font(12.0, 600.0)).color(l.text))
    };
    let rows = vec![
        setting(agent(l, &icons::CLAUDE, l.brand, "Claude"), state(s.claude.snapshot.as_ref())),
        setting(agent(l, &icons::OPENAI, l.text, "Codex"), state(s.codex.snapshot.as_ref())),
        setting(agent(l, &icons::DEEPSEEK, l.deepseek, "DeepSeek"), deepseek),
    ];
    group(l, "settings-agents", &icons::SPARKLES, l.dim, "Agents", None, rows)
}

/// An agent's mark in its colour, and its name.
fn agent(l: &Look, mark: &'static Icon, color: Color, words: &str) -> El {
    row().gap(10.0).center_y().child(icon(mark, 14.0).color(color)).child(name(l, words))
}

/// Whether DeepSeek's balance is in the menu bar. It sits in the Agents card, on DeepSeek's
/// row, since a group of its own would not fit under This Mac and Codex.
fn deepseek_menu_bar(s: &Ui, l: &Look) -> El {
    let on = s.config.deepseek_menu_bar;
    let toggle = switch(l, "deepseek-menu-bar", "DeepSeek balance in menu bar", on, true).on_press(move |s: &mut Ui| ui::set_deepseek_menu_bar(s, !on));
    row().gap(8.0).center_y().min_w(0.0).child(note(l, "In menu bar".to_owned())).child(toggle)
}

fn claude(s: &Ui, l: &Look) -> El {
    let snapshot = s.claude.snapshot.as_ref();
    let choices = model::menu_bar_choices(snapshot);
    let shown = s.config.menu_bar.effective(snapshot);
    let mut menu_bar = segments(l, "menu-bar");
    for metric in choices.iter().copied() {
        menu_bar = menu_bar.child(segment(l, format!("menu-bar-{}", metric.label()), metric.label(), shown == metric, move |s: &mut Ui| ui::set_menu_bar(s, metric)));
    }
    let models = s.config.menu_bar_models;
    let models_switch = switch(l, "menu-bar-models", "Model limits in menu bar", models, true).on_press(move |s: &mut Ui| ui::set_menu_bar_models(s, !models));
    // Where the plan has no session, say so: that is why the choice is missing.
    let missing = snapshot.filter(|_| !choices.contains(&model::MenuBarMetric::Session)).map(|snapshot| {
        let plan = if snapshot.plan.is_empty() { "This plan" } else { snapshot.plan.as_str() };
        text(format!("{plan} has no session limit")).font(font(11.0, 400.0)).color(l.faint).truncate().min_w(0.0)
    });
    let rows = vec![
        setting(name(l, "Menu bar shows"), menu_bar),
        setting(name(l, "Percentages"), percentages(l, "percent", s.config.show_used, ui::set_show_used)),
        setting(name(l, "Model limits in menu bar"), models_switch),
    ];
    group(l, "settings-claude", &icons::CLAUDE, l.brand, "Claude", missing, rows)
}

fn codex(s: &Ui, l: &Look) -> El {
    let snapshot = s.codex.snapshot.as_ref();
    let shown = s.config.codex.menu_bar.effective(snapshot);
    let mut menu_bar = segments(l, "codex-menu-bar");
    for metric in model::menu_bar_choices(snapshot) {
        let label = if metric == model::MenuBarMetric::Session { "5-hour" } else { metric.label() };
        menu_bar = menu_bar.child(segment(l, format!("codex-menu-bar-{}", metric.label()), label, shown == metric, move |s: &mut Ui| ui::set_codex_menu_bar(s, metric)));
    }
    let rows = vec![
        setting(name(l, "Menu bar shows"), menu_bar),
        setting(name(l, "Percentages"), percentages(l, "codex-percent", s.config.codex.show_used, ui::set_codex_show_used)),
    ];
    group(l, "settings-codex", &icons::OPENAI, l.text, "Codex", None, rows)
}

/// A header over a column of This Mac's card, centred over its controls.
fn heading(l: &Look, width: f32, words: &str) -> El {
    row().w(width).justify_center().child(text(words.to_owned()).font(font(11.0, 400.0)).color(l.faint))
}

fn this_mac(s: &Ui, l: &Look) -> El {
    let head = row().gap(ROW_PAD).center_y().child(heading(l, STYLES_W, "Menu bar")).child(heading(l, SWITCH_W, "Tab"));
    let no_battery = s.system.as_ref().is_some_and(|view| view.battery.is_none());
    let mut rows: Vec<El> = Tab::SYSTEM.into_iter().map(|tab| measure(s, l, tab, tab == Tab::Battery && no_battery)).collect();
    rows.push(sleep(s, l));
    group(l, "settings-mac", &icons::LAPTOP, l.dim, "This Mac", Some(head), rows)
}

/// Sleep's row: a cup in the menu bar while the Mac is kept awake, and whether it has a tab.
fn sleep(s: &Ui, l: &Look) -> El {
    let lead = row().gap(10.0).center_y().child(icon(&icons::MOON, 14.0).color(l.dim)).child(name(l, "Sleep"));
    let on = s.config.sleep.menu_bar;
    let mut styles = segments(l, "sleep-menu-bar").w(STYLES_W).label("Keep awake in the menu bar");
    for (shown, label) in [(false, "Off"), (true, "While awake")] {
        styles = styles.child(segment(l, format!("sleep-menu-bar-{label}"), label, on == shown, move |s: &mut Ui| ui::set_sleep_menu_bar(s, shown)).grow(1.0).px(0.0));
    }
    let tab = s.config.sleep.tab;
    let tab_switch = switch(l, "sleep-tab", "Sleep tab", tab, true).on_press_tx(move |s: &mut Ui, tx| ui::set_sleep_tab(s, !tab, tx));
    setting(lead, row().gap(ROW_PAD).center_y().child(styles.help("A cup and the time left, while this Mac is kept awake")).child(tab_switch)).key("settings-sleep")
}

/// A measure's row: its menu bar style and whether it has a tab, or why it has neither.
fn measure(s: &Ui, l: &Look, tab: Tab, absent: bool) -> El {
    let lead = row().gap(10.0).center_y().child(icon(system_view::glyph(tab), 14.0).color(if absent { l.faint } else { l.dim })).child(name(l, tab.label()));
    if absent {
        return setting(lead, note(l, "This Mac has no battery".to_owned())).key(format!("settings-{}", tab.id()));
    }
    let module = s.module(tab).unwrap_or_default();
    let mut styles = segments(l, &format!("{}-menu-bar", tab.id())).w(STYLES_W).label(format!("{} in the menu bar", tab.label()));
    for style in MenuStyle::ALL {
        let key = format!("{}-menu-bar-{}", tab.id(), style.label());
        styles = styles.child(segment(l, key, style.label(), module.menu_bar == style, move |s: &mut Ui| ui::set_module_menu_bar(s, tab, style)).grow(1.0).px(0.0));
    }
    let shown = module.tab;
    let tab_switch = switch(l, &format!("{}-tab", tab.id()), &format!("{} tab", tab.label()), shown, true).on_press_tx(move |s: &mut Ui, tx| ui::set_module_tab(s, tab, !shown, tx));
    setting(lead, row().gap(ROW_PAD).center_y().child(styles).child(tab_switch)).key(format!("settings-{}", tab.id()))
}
