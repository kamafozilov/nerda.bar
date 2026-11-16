//! How the panel looks: a rail of tabs down its left side (the providers, then this Mac's
//! measures, then Settings and Quit) and the open tab beside it. A provider's tab is in
//! `agents_view`, this Mac's in `system_view`, the settings in `settings_view`.

use rune::{Color, Cx, El, col, el, icon, row, spacer, text};

use crate::agents_view;
use crate::icons;
use crate::look::{Look, font, rail_button};
use crate::settings_view;
use crate::system_view;
use crate::ui::{self, CONTENT_W, Page, RAIL_W, SHADOW_ROOM, Tab, Ui};

const RADIUS: f32 = 18.0;

pub fn view(s: &Ui, cx: &mut Cx<Ui>) -> El {
    let l = Look::new(cx.system_dark());
    // What a display of another scale waits for before it shows the panel.
    s.drawn_scale.set(cx.env().scale);

    let content = match (s.page, s.tab) {
        (Page::Settings, _) => settings_view::page(s, &l),
        (_, Tab::DeepSeek) => crate::deepseek_view::tab(s, &l),
        (_, tab) if tab.provider().is_some() => agents_view::tab(s, &l, tab.provider().unwrap_or(crate::model::Provider::Claude)),
        (_, tab) => system_view::tab(s, &l, tab),
    };
    let content = col().key("content").w(CONTENT_W).child(content);

    let panel = row()
        .key("panel")
        .items_start()
        .radius(RADIUS)
        .continuous_corners()
        .fill(l.panel)
        .border(1.0, l.panel_edge)
        .shadow(18.0, 44.0, l.shadow)
        .system_material(s.material)
        .child(rail(s, &l))
        .child(el().key("rail-edge").w(1.0).self_stretch().fill(l.stroke))
        .child(content)
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

/// The tabs, one above the other: the providers by their marks, a hairline, this Mac's measures
/// each with its icon and its name, and at the foot Settings and Quit.
fn rail(s: &Ui, l: &Look) -> El {
    let mut rail = col().key("rail").w(RAIL_W - 1.0).self_stretch().fill(l.rail).px(8.0).py(10.0).gap(4.0).items_center();
    let tabs = s.tabs();
    for (at, &tab) in tabs.iter().enumerate() {
        let on = s.page == Page::Tab && s.tab == tab;
        let shortcut = if at < 9 { format!(" (⌘{})", at + 1) } else { String::new() };
        let button = if tab.is_system() {
            if at > 0 && !tabs[at - 1].is_system() {
                rail = rail.child(row().key("rail-separator").w_full().py(5.0).center().child(el().size(24.0, 1.0).fill(l.track)));
            }
            col()
                .w_full()
                .pt(8.0)
                .pb(6.0)
                .gap(5.0)
                .center()
                .child(icon(system_view::glyph(tab), 18.0).color(if on { l.text } else { l.dim }))
                .child(text(tab.short()).font(font(10.0, if on { 600.0 } else { 500.0 })).color(if on { l.text } else { l.dim }))
        } else {
            let (mark, color, _) = agents_view::mark(l, tab.provider().unwrap_or(crate::model::Provider::Claude));
            row().size(40.0, 40.0).center().child(icon(mark, 20.0).color(color))
        };
        rail = rail.child(
            button
                .key(format!("tab-{}", tab.id()))
                .role("tab")
                .label(tab.label())
                .help(format!("{}{shortcut}", tab.label()))
                .radius(11.0)
                .continuous_corners()
                .fill(if on { l.chip } else { Color::TRANSPARENT })
                .hover(l.hover)
                .press_scale(0.96)
                .on_press_tx(move |s: &mut Ui, tx| ui::set_tab(s, tab, tx)),
        );
    }
    rail.child(spacer())
        .child(rail_button(l, "settings", &icons::SETTINGS_2, "Settings (⌘,)", s.page == Page::Settings).on_press_tx(ui::toggle_settings))
        .child(rail_button(l, "quit", &icons::POWER, "Quit Nerda Bar (⌘Q)", false).on_press_tx(|_: &mut Ui, tx| tx.quit()))
}
