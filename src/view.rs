//! How the panel looks: a top bar of tabs (the agents, then this Mac's measures, then Settings),
//! the context bar of the open tab under it, and the tab's body in two columns. The panel has
//! one size on every tab. An agent's tab is in `agents_view`, this Mac's in `system_view`, the
//! settings in `settings_view`.

use rune::{Color, Cx, El, col, el, row, spacer};

use crate::agents_view;
use crate::icons;
use crate::look::{BAR_H, Look, PANEL_H, PANEL_W, nav_tile};
use crate::settings_view;
use crate::system_view;
use crate::ui::{self, Page, SHADOW_ROOM, Tab, Ui};

const RADIUS: f32 = 18.0;

/// What a tab puts under the top bar: its context bar and its body.
pub struct Screen {
    pub context: El,
    pub body: El,
}

pub fn view(s: &Ui, cx: &mut Cx<Ui>) -> El {
    let l = Look::new(cx.system_dark());
    // What a display of another scale waits for before it shows the panel.
    s.drawn_scale.set(cx.env().scale);

    let screen = match (s.page, s.tab) {
        (Page::Settings, _) => settings_view::screen(s, &l),
        (_, tab) if tab.provider().is_some() => agents_view::screen(s, &l),
        (_, tab) => system_view::screen(s, &l, tab),
    };

    let mut panel = col()
        .key("panel")
        .w(PANEL_W)
        .h(PANEL_H)
        .radius(RADIUS)
        .continuous_corners()
        .fill(l.panel)
        .border(1.0, l.panel_edge)
        .shadow(18.0, 44.0, l.shadow)
        .system_material(s.material)
        .child(top_bar(s, &l))
        .child(screen.context)
        .child(screen.body)
        // Presses on the panel stay with it; only the empty window around it closes.
        .on_press(|_: &mut Ui| {});
    if s.adding && s.page == Page::Tab {
        panel = panel.child(agents_view::add_popover(s, &l));
    }

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

/// The tabs, side by side: Agents, a hairline, this Mac's measures, and Settings at the end.
/// Each is an icon; its name and shortcut are in its tooltip.
fn top_bar(s: &Ui, l: &Look) -> El {
    let on_tab = s.page == Page::Tab;
    let agents_on = on_tab && s.tab.provider().is_some();
    let mut bar = row()
        .key("top-bar")
        .w_full()
        .h(BAR_H)
        .px(8.0)
        .gap(4.0)
        .center_y()
        .fill(l.bar)
        .corners([RADIUS, RADIUS, 0.0, 0.0])
        .border_sides([0.0, 0.0, 1.0, 0.0], l.stroke)
        .child(nav_tile(l, "tab-agents", &icons::SPARKLES, "Agents", Some("⌘1"), agents_on).on_press_tx(ui::show_agents));
    let macs = s.mac_tabs();
    if !macs.is_empty() {
        bar = bar.child(el().key("bar-separator").size(1.0, 16.0).mx(4.0).fill(l.track));
    }
    for tab in macs {
        let shortcut = s.shortcut(tab);
        bar = bar.child(nav_tile(l, tab_key(tab), tab.icon(), tab.label(), shortcut.as_deref(), on_tab && s.tab == tab).on_press_tx(move |s: &mut Ui, tx| ui::set_tab(s, tab, tx)));
    }
    bar.child(spacer()).child(nav_tile(l, "tab-settings", &icons::SETTINGS_2, "Settings", Some("⌘,"), s.page == Page::Settings).on_press_tx(ui::toggle_settings))
}

fn tab_key(tab: Tab) -> &'static str {
    match tab {
        Tab::Cpu => "tab-cpu",
        Tab::Memory => "tab-memory",
        Tab::Disk => "tab-disk",
        Tab::Network => "tab-network",
        Tab::Battery => "tab-battery",
        Tab::Gpu => "tab-gpu",
        Tab::Claude | Tab::Codex | Tab::DeepSeek => "tab-agents",
    }
}

