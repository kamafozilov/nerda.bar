//! The settings, in place of a tab: a group per provider, this Mac's measures as a table of
//! their menu bar style and whether they have a tab, and the general ones. One line each,
//! name on the left and its control on the right, as System Settings lays a group out.

use rune::{Color, El, Icon, PopupMenu, col, el, icon, row, rule, spacer, text};

use crate::config::MenuStyle;
use crate::icons;
use crate::look::{Look, PAD_X, digits, font, segment, segments, switch};
use crate::model;
use crate::system_view;
use crate::ui::{self, Tab, Ui};

const ROW_H: f32 = 37.0;

pub fn page(s: &Ui, l: &Look) -> El {
    let back = row()
        .key("back")
        .role("button")
        .label("Back")
        .help("Back (esc)")
        .size(26.0, 26.0)
        .radius(7.0)
        .center()
        .hover(l.hover)
        .on_press_tx(ui::toggle_settings)
        .child(icon(&icons::CHEVRON_LEFT, 16.0).color(l.dim));
    let header = row()
        .key("header")
        .w_full()
        .pt(14.0)
        .pb(8.0)
        .pl(10.0)
        .pr(PAD_X)
        .gap(6.0)
        .center_y()
        .child(back)
        .child(text("Settings").font(font(15.0, 700.0)).color(l.text))
        .child(spacer())
        .child(text(concat!("Nerda Bar ", env!("CARGO_PKG_VERSION"))).font(digits(11.0, 400.0)).color(l.faint));

    col()
        .key("settings-page")
        .w_full()
        .child(header)
        .child(col().w_full().px(PAD_X).pt(4.0).pb(16.0).gap(14.0).child(claude(s, l)).child(codex(s, l)).child(deepseek(s, l)).child(this_mac(s, l)).child(general(s, l)))
}

/// A row of a group: its name, and its control at the end.
fn setting(l: &Look, name: &str, control: El) -> El {
    row().w_full().h(ROW_H).pl(12.0).pr(10.0).center_y().child(text(name.to_owned()).font(font(12.5, 400.0)).color(l.text)).child(spacer()).child(control)
}

/// Rows on a card, split by hairlines.
fn card(l: &Look, key: &'static str, rows: Vec<El>) -> El {
    let mut card = col().key(key).w_full().radius(10.0).continuous_corners().fill(l.card);
    for (at, setting) in rows.into_iter().enumerate() {
        if at > 0 {
            card = card.child(rule(1.0, l.stroke));
        }
        card = card.child(setting);
    }
    card
}

/// A group's caption: its mark, its name and a quieter note.
fn caption(l: &Look, mark: &'static Icon, mark_color: Color, name: &str, note: Option<String>) -> El {
    let mut caption = row().px(4.0).gap(6.0).center_y().child(icon(mark, 12.0).color(mark_color)).child(text(name.to_owned()).font(font(11.0, 600.0)).color(l.dim));
    if let Some(note) = note {
        caption = caption.child(text(note).font(font(11.0, 400.0)).color(l.faint));
    }
    caption
}

fn claude(s: &Ui, l: &Look) -> El {
    let snapshot = s.claude.snapshot.as_ref();
    let choices = model::menu_bar_choices(snapshot);
    let shown = s.config.menu_bar.effective(snapshot);
    let mut menu_bar = segments(l, "menu-bar");
    for metric in choices.iter().copied() {
        menu_bar = menu_bar.child(segment(l, format!("menu-bar-{}", metric.label()), metric.label(), shown == metric, move |s: &mut Ui| ui::set_menu_bar(s, metric)));
    }
    let mut percent = segments(l, "percent");
    for (used, label) in [(false, "Left"), (true, "Used")] {
        percent = percent.child(segment(l, format!("percent-{label}"), label, s.config.show_used == used, move |s: &mut Ui| ui::set_show_used(s, used)));
    }
    let models = s.config.menu_bar_models;
    let models_switch = switch(l, "menu-bar-models", "Model limits in menu bar", models, true).on_press(move |s: &mut Ui| ui::set_menu_bar_models(s, !models));
    // Where the plan has no session, say so: that is why the choice is missing.
    let note = snapshot.filter(|_| !choices.contains(&model::MenuBarMetric::Session)).map(|snapshot| {
        let plan = if snapshot.plan.is_empty() { "This plan" } else { snapshot.plan.as_str() };
        format!("· {plan} has no session limit")
    });
    col()
        .key("settings-claude")
        .w_full()
        .gap(6.0)
        .child(caption(l, &icons::CLAUDE, l.brand, "Claude", note))
        .child(card(l, "settings-group", vec![setting(l, "Menu bar shows", menu_bar), setting(l, "Percentages", percent), setting(l, "Model limits in menu bar", models_switch)]))
}

fn codex(s: &Ui, l: &Look) -> El {
    let snapshot = s.codex.snapshot.as_ref();
    let shown = s.config.codex.menu_bar.effective(snapshot);
    let mut menu_bar = segments(l, "codex-menu-bar");
    for metric in model::menu_bar_choices(snapshot) {
        let label = if metric == model::MenuBarMetric::Session { "5-hour" } else { metric.label() };
        menu_bar = menu_bar.child(segment(l, format!("codex-menu-bar-{}", metric.label()), label, shown == metric, move |s: &mut Ui| ui::set_codex_menu_bar(s, metric)));
    }
    let mut percent = segments(l, "codex-percent");
    for (used, label) in [(false, "Left"), (true, "Used")] {
        percent = percent.child(segment(l, format!("codex-percent-{label}"), label, s.config.codex.show_used == used, move |s: &mut Ui| ui::set_codex_show_used(s, used)));
    }
    let note = snapshot.is_none().then(|| "· not signed in".to_owned());
    col()
        .key("settings-codex")
        .w_full()
        .gap(6.0)
        .child(caption(l, &icons::OPENAI, l.text, "Codex", note))
        .child(card(l, "settings-codex-card", vec![setting(l, "Menu bar shows", menu_bar), setting(l, "Percentages", percent)]))
}

fn deepseek(s: &Ui, l: &Look) -> El {
    let on = s.config.deepseek_menu_bar;
    let mut menu_bar = segments(l, "deepseek-menu-bar");
    for (value, label) in [(true, "Balance"), (false, "Off")] {
        menu_bar = menu_bar.child(segment(l, format!("deepseek-menu-bar-{label}"), label, on == value, move |s: &mut Ui| ui::set_deepseek_menu_bar(s, value)));
    }
    let note = s.deepseek.ledger.is_none().then(|| "· no API key".to_owned());
    col()
        .key("settings-deepseek")
        .w_full()
        .gap(6.0)
        .child(caption(l, &icons::DEEPSEEK, l.deepseek, "DeepSeek", note))
        .child(card(l, "settings-deepseek-card", vec![setting(l, "Menu bar shows", menu_bar)]))
}

fn this_mac(s: &Ui, l: &Look) -> El {
    let head = row()
        .w_full()
        .pl(4.0)
        .pr(10.0)
        .gap(10.0)
        .center_y()
        .child(caption(l, &icons::LAPTOP, l.dim, "This Mac", None).px(0.0))
        .child(spacer())
        .child(row().w(74.0).child(text("Menu bar").font(font(10.5, 400.0)).color(l.faint)))
        .child(row().w(30.0).child(text("Tab").font(font(10.5, 400.0)).color(l.faint)));
    let mut rows = Vec::new();
    for tab in Tab::SYSTEM {
        let module = s.module(tab).unwrap_or_default();
        let style = module.menu_bar;
        let key = format!("{}-menu-bar", tab.id());
        let anchor = key.clone();
        let picker = row()
            .key(key)
            .role("button")
            .label(format!("{} in the menu bar: {}", tab.short(), style.label()))
            .w(74.0)
            .py(3.0)
            .px(8.0)
            .radius(6.0)
            .continuous_corners()
            .fill(l.chip)
            .hover(l.hover)
            .center_y()
            .on_press_tx(move |_: &mut Ui, tx| {
                let menu = PopupMenu::picker(MenuStyle::ALL.map(|style| (style.label(), style.label())), style.label());
                tx.popup_menu(menu, anchor.clone(), move |s: &mut Ui, picked, _| {
                    if let Some(style) = MenuStyle::ALL.into_iter().find(|style| Some(style.label()) == picked) {
                        ui::set_module_menu_bar(s, tab, style);
                    }
                });
            })
            .child(text(style.label()).font(font(11.0, 600.0)).color(if style == MenuStyle::Off { l.dim } else { l.text }))
            .child(spacer())
            .child(icon(&icons::CHEVRONS_UP_DOWN, 10.0).color(l.dim));
        let shown = module.tab;
        let tab_switch = switch(l, &format!("{}-tab", tab.id()), &format!("{} tab", tab.short()), shown, true).on_press_tx(move |s: &mut Ui, tx| ui::set_module_tab(s, tab, !shown, tx));
        rows.push(
            row()
                .key(format!("settings-{}", tab.id()))
                .w_full()
                .h(ROW_H - 1.5)
                .pl(12.0)
                .pr(10.0)
                .gap(10.0)
                .center_y()
                .child(icon(system_view::glyph(tab), 14.0).color(l.dim))
                .child(text(tab.short()).font(font(12.5, 400.0)).color(l.text))
                .child(spacer())
                .child(picker)
                .child(tab_switch),
        );
    }
    col()
        .key("settings-mac")
        .w_full()
        .gap(6.0)
        .child(head)
        .child(card(l, "settings-mac-card", rows))
        .child(text("Menu bar styles: Off, Text, Ring, Graph. Tab shows the item in the panel's rail.").font(font(11.0, 400.0).lh(15.0)).color(l.faint).w_full().wrap().px(4.0))
}

fn general(s: &Ui, l: &Look) -> El {
    let (can_login, login) = (s.login.0 || s.demo, s.login.1);
    let mut login_switch = switch(l, "login", "Launch at Login", login, can_login);
    if can_login {
        login_switch = login_switch.on_press(move |s: &mut Ui| ui::set_launch_at_login(s, !login));
    } else {
        login_switch = login_switch.help("Available when run as Nerda Bar.app");
    }
    let open_file = row()
        .key("settings-file")
        .role("button")
        .label("Open settings file")
        .h(24.0)
        .px(12.0)
        .radius(7.0)
        .continuous_corners()
        .center()
        .fill(l.chip)
        .hover(l.hover)
        .on_press_tx(ui::open_settings_file)
        .child(text("Open").font(font(11.5, 600.0)).color(l.text));
    col()
        .key("settings-general")
        .w_full()
        .gap(6.0)
        .child(caption(l, &icons::SETTINGS_2, l.dim, "General", None))
        .child(card(l, "settings-general-card", vec![setting(l, "Launch at Login", login_switch), setting(l, "Settings file", open_file)]))
        .child(el())
}
