//! Nerda Bar: AI subscription limits in the macOS menu bar, one item per provider, and this
//! Mac's CPU and memory beside them, with a panel of tabs under each: limits and usage, and the
//! measures with their history and the apps behind them. Built with Rune
//! (github.com/anomalyco/rune).
//!
//! ```sh
//! cargo run --release                 # menu bar app
//! cargo run --release -- --show       # also open the panel at launch
//! cargo run --release -- --snap panel.png [--theme light] [--scene used]   # headless render with sample data
//! ```

#[cfg(not(target_os = "macos"))]
compile_error!("Nerda Bar is a macOS app.");

mod add_view;
mod agents;
mod agents_view;
mod alloc;
mod burn;
mod burn_view;
mod codex;
mod codex_logs;
mod claude;
mod claude_desktop;
mod config;
mod deepseek;
mod deepseek_view;
mod demo;
mod icons;
mod look;
mod mac_view;
mod model;
mod prices;
mod resets_view;
mod settings_view;
mod stats;
mod status_item;
mod system;
mod system_view;
mod time;
mod ui;
mod view;
mod worker;

use rune::{App, AppSpec, RunOptions, WindowOptions, WindowSpec};

use crate::model::Problem;
use crate::ui::{Range, Tab, Ui};

/// Rune's headless tooling flags: with one of them the app renders sample data, adds no menu
/// bar item and makes no requests.
const HEADLESS: [&str; 12] = [
    "--snap", "--tree", "--trace", "--script", "--video", "--frames", "--strip", "--out", "--first-pixels",
    "--list-scenes", "--list-windows", "--desktop",
];

#[global_allocator]
static ALLOCATOR: alloc::Returning = alloc::Returning;

/// Prints a diagnostic line when `NERDA_BAR_DEBUG` is set.
fn debug(message: impl FnOnce() -> String) {
    static ON: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| std::env::var_os("NERDA_BAR_DEBUG").is_some());
    if *ON {
        eprintln!("nerda-bar: {}", message());
    }
}

/// An anchor written "x,y,left,right,scale".
fn parse_anchor(text: &str) -> Option<ui::Anchor> {
    let parts: Vec<f32> = text.split(',').filter_map(|part| part.trim().parse().ok()).collect();
    let [x, y, left, right, scale] = parts[..] else { return None };
    Some(ui::Anchor { x, y, screen: [left, right], scale })
}

/// `NERDA_BAR_ANCHOR="x,y,left,right,scale"` stands in for the menu bar item's place with
/// `--show`, to open the panel on a display the menu bar isn't active on.
fn test_anchor() -> Option<ui::Anchor> {
    parse_anchor(&std::env::var("NERDA_BAR_ANCHOR").ok()?)
}

/// `NERDA_BAR_CYCLE="anchor;anchor;…"` with `NERDA_BAR_DEBUG`: clicks the item under each
/// anchor in turn, for watching the panel move between displays. With
/// `NERDA_BAR_CYCLE_CLOSE` it closes between them, as a click elsewhere would.
fn cycle(handle: rune::AppHandle<Ui>) {
    let Some(anchors) = std::env::var("NERDA_BAR_CYCLE").ok().filter(|_| std::env::var_os("NERDA_BAR_DEBUG").is_some()) else { return };
    let anchors: Vec<ui::Anchor> = anchors.split(';').filter_map(parse_anchor).collect();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(1500));
        for at in (0..anchors.len()).cycle() {
            let anchor = anchors[at];
            if std::env::var_os("NERDA_BAR_CYCLE_CLOSE").is_some() {
                handle.update(ui::close);
                std::thread::sleep(std::time::Duration::from_millis(700));
            }
            handle.update(move |s, tx| ui::toggle(s, anchor, Tab::Claude, tx));
            std::thread::sleep(std::time::Duration::from_millis(1300));
        }
    });
}

fn main() {
    rune::prewarm_gpu();
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let show_at_launch = args.iter().any(|a| a == "--show");
    args.retain(|a| a != "--show");
    let headless = args.iter().any(|a| HEADLESS.contains(&a.as_str()));

    let data_dir = config::data_dir();
    let config = if headless { config::Config::default() } else { config::load(&data_dir) };
    // The limits of the last run show at once; the first refresh replaces them.
    let cached = if headless { (None, None) } else { (worker::Cache::load(&data_dir, model::Provider::Claude).snapshot, worker::Cache::load(&data_dir, model::Provider::Codex).snapshot) };

    let spec = AppSpec::<Ui> {
        // The system font, as AppKit draws it; Inter where it can't be found.
        families: (".AppleSystemUIFont, SF Pro Text, SF Pro, Helvetica Neue, Inter", "ui-monospace"),
        poll: Some(ui::poll),
        setup: Some(|s, waker| s.images = Some(rune::ImageCache::new(waker))),
        on_key: Some(ui::on_key),
        scenes: &[
            ("used", |s| s.config.show_used = true),
            ("settings", |s| s.page = ui::Page::Settings),
            ("cpu", |s| s.tab = Tab::Cpu),
            ("cpu-day", |s| {
                s.tab = Tab::Cpu;
                s.tier = 2;
            }),
            ("cpu-hover", |s| {
                s.tab = Tab::Cpu;
                s.point = Some(140);
            }),
            ("memory", |s| s.tab = Tab::Memory),
            ("disk", |s| s.tab = Tab::Disk),
            ("network", |s| s.tab = Tab::Network),
            ("battery", |s| s.tab = Tab::Battery),
            ("battery-charging", |s| {
                s.tab = Tab::Battery;
                if let Some(view) = &mut s.system
                    && let Some(battery) = &mut std::sync::Arc::make_mut(view).battery
                {
                    (battery.charge, battery.plugged, battery.charging, battery.minutes_left) = (0.64, true, true, Some(48));
                }
            }),
            ("battery-low", |s| {
                s.tab = Tab::Battery;
                if let Some(view) = &mut s.system
                    && let Some(battery) = &mut std::sync::Arc::make_mut(view).battery
                {
                    (battery.charge, battery.minutes_left) = (0.12, Some(26));
                }
            }),
            ("gpu", |s| s.tab = Tab::Gpu),
            ("memory-warning", |s| {
                s.tab = Tab::Memory;
                if let Some(view) = &mut s.system
                    && let Some(memory) = &mut std::sync::Arc::make_mut(view).memory
                {
                    memory.pressure = system::Pressure::Warning;
                }
            }),
            ("no-tabs", |s| {
                s.config.cpu.tab = false;
                s.config.memory.tab = false;
            }),
            ("hover", |s| s.hovered = Some(2)),
            ("today", |s| s.range = Range::Today),
            ("month", |s| s.range = Range::Month),
            ("trace", |s| s.trace = Some(0.3)),
            ("session", |s| s.claude.chart = Some("session".to_owned())),
            ("pro", |s| demo::plan(s, "Pro", 76.0, 41.0, 58.0)),
            ("pro-session", |s| {
                demo::plan(s, "Pro", 76.0, 41.0, 58.0);
                s.claude.chart = Some("session".to_owned());
            }),
            ("critical", |s| demo::plan(s, "Pro", 94.0, 73.0, 92.0)),
            ("low", |s| demo::plan(s, "Pro", 93.0, 74.0, 58.0)),
            ("loading", |s| s.claude.snapshot = None),
            ("offline", |s| {
                demo::plan(s, "Pro", 76.0, 41.0, 58.0);
                s.claude.problem = Some(Problem::Offline("Couldn't reach Claude.".to_owned()));
            }),
            ("critical-offline", |s| {
                demo::plan(s, "Pro", 94.0, 73.0, 92.0);
                s.claude.problem = Some(Problem::Offline("Couldn't reach Claude.".to_owned()));
            }),
            ("expired", |s| s.claude.problem = Some(Problem::LoginExpired)),
            ("paused", |s| s.claude.problem = Some(Problem::RateLimited { until: s.now + 720 })),
            ("failed", |s| s.claude.problem = Some(Problem::Failed("The server answered 503.".to_owned()))),
            ("plan-ends", |s| {
                if let Some(snapshot) = &mut s.claude.snapshot {
                    snapshot.subscription = Some(model::Subscription { until: s.now + 2 * 86_400 + 3 * 3600, renews: Some(false) });
                }
            }),
            ("codex", |s| s.tab = Tab::Codex),
            ("codex-confirm", |s| {
                s.tab = Tab::Codex;
                s.reset = agents::ResetFlow::Confirm("demo-1".to_owned());
            }),
            ("codex-signed-out", |s| {
                s.tab = Tab::Codex;
                s.codex.snapshot = None;
                s.codex.problem = Some(Problem::SignedOut);
            }),
            ("deepseek", |s| s.tab = Tab::DeepSeek),
            ("signed-out", |s| {
                s.claude.snapshot = None;
                s.claude.problem = Some(Problem::SignedOut);
            }),
            ("no-agents", |s| {
                s.claude.snapshot = None;
                s.claude.problem = Some(Problem::SignedOut);
                s.codex.snapshot = None;
                s.codex.problem = Some(Problem::SignedOut);
                s.deepseek.ledger = None;
                s.deepseek.problem = Some(Problem::SignedOut);
            }),
            ("add-agent", |s| {
                s.deepseek.ledger = None;
                s.deepseek.problem = Some(Problem::SignedOut);
                s.adding = true;
            }),
            ("deepseek-offline", |s| {
                s.tab = Tab::DeepSeek;
                s.deepseek.problem = Some(Problem::Offline("Couldn't reach DeepSeek.".to_owned()));
            }),
        ],
        ..AppSpec::new("Nerda Bar", (ui::WINDOW_W, ui::WINDOW_H), || unreachable!(), view::view)
    };

    let app = App::new(spec).windowless();
    let me = app.handle();
    let worker = (!headless).then(|| {
        let ui = me.clone();
        worker::spawn(data_dir.clone(), config.refresh_seconds(), move |event| ui.update(move |s, tx| ui::apply(s, event, tx)))
    });

    let sampler = (!headless).then(|| {
        let ui = me.clone();
        system::spawn(Some(data_dir.clone()), move |view| ui.update_if(move |s, tx| ui::apply_system(s, view, tx)))
    });

    let (state_handle, ready_handle) = (me.clone(), me);
    let (state_dir, state_config) = (data_dir.clone(), config.clone());
    let mut installed = false;
    let app = app
        .init(move || Ui::new(state_handle, worker, sampler, state_config, state_dir, cached, headless))
        .on_focus(ui::on_focus)
        // After the first (hidden) frame: the menu bar item, showing the last run's limits.
        .on_ready(move |_| {
            if installed || headless {
                return;
            }
            installed = true;
            status_item::set_show_used(config.show_used);
            // Claude's item first, then this Mac's; Codex and DeepSeek add theirs once signed in.
            status_item::install(ready_handle.clone(), data_dir.clone(), "claude", "Claude", &status_item::CLAUDE_MARK);
            ready_handle.update(|s, _| {
                s.sync_menu_bar();
                s.sync_system_items();
            });
            cycle(ready_handle.clone());
            if show_at_launch {
                // The menu bar gives a new item its place a moment after it is added.
                let handle = ready_handle.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(600));
                    handle.update(|s, tx| {
                        if let Some(anchor) = test_anchor().or_else(|| status_item::anchor_of("claude")) {
                            ui::open(s, anchor, Tab::Claude, tx);
                        }
                    });
                });
            }
        });

    let panel = WindowOptions::launcher_panel().macos(|native| native.shadow(false));
    rune::run_windows_with_args(RunOptions { accessory: true }, args, move |_| vec![WindowSpec::from_app("panel", app).configured_options(panel)]);
}
