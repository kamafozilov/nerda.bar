//! The panel's tabs for this Mac. Each has a header, what is in use now as a large number with
//! its parts, its history as an area chart at three resolutions, and the apps behind it.
//!
//! Charts follow one scheme: a share from 0 to 100% on a single axis, series in the fixed order
//! of `Look::series`, a line over a faint fill, hairlines at 50% and 100%, and a readout under
//! the chart for the point under the pointer or, without one, the peak.

use rune::{Color, El, Icon, Shape, canvas, col, el, row, spacer, text};

use crate::icons;
use crate::look::{self, INNER_W, Look, PAD_X, caption, digits, figure, font, hairline, legend_value, section, section_title, segment, segments};
use crate::system::apps::{self, AppUsage};
use crate::system::history::{self, Point, TIERS};
use crate::system::{Pressure, SystemView};
use crate::time;
use crate::ui::{self, Tab, Ui};

const CHART_H: f32 = 64.0;
const LINE: f32 = 1.75;
const CORE_H: f32 = 44.0;
const APP_ROWS: usize = 5;
const ICON: f32 = 22.0;

/// A tab's icon in the rail and on its header's tile.
pub fn glyph(tab: Tab) -> &'static Icon {
    match tab {
        Tab::Memory => &icons::MEMORY_STICK,
        Tab::Disk => &icons::HARD_DRIVE,
        Tab::Network => &icons::ARROW_DOWN_UP,
        Tab::Battery => &icons::BATTERY_MEDIUM,
        Tab::Gpu => &icons::GPU,
        _ => &icons::CPU,
    }
}

pub fn tab(s: &Ui, l: &Look, tab: Tab) -> El {
    let view = s.system.as_deref();
    let machine = view.map(|view| &view.machine);
    let (badge, words) = match tab {
        Tab::Disk | Tab::Network | Tab::Battery | Tab::Gpu => crate::mac_view::header_words(s, tab),
        Tab::Memory => (
            machine.map(|m| ui::system_bytes(m.memory)),
            machine.filter(|m| m.chip.starts_with('M')).map_or(String::new(), |_| "Unified memory, shared with the GPU".to_owned()),
        ),
        _ => (
            machine.map(|m| m.chip.clone()),
            machine.map_or(String::new(), |m| {
                let cores = m.performance_cores + m.efficiency_cores;
                match m.booted_at {
                    Some(at) => format!("{cores} cores · up {}", time::span(s.now - at)),
                    None => format!("{cores} cores"),
                }
            }),
        ),
    };
    let header = look::header(l, glyph(tab), 18.0, l.dim, l.chip, if tab == Tab::Network { tab.label() } else if tab == Tab::Memory { tab.short() } else { tab.label() }, badge.as_deref(), look::subtitle(l, words, None), None);
    let body = match tab {
        Tab::Memory => memory(s, l),
        Tab::Disk => crate::mac_view::disk(s, l),
        Tab::Network => crate::mac_view::network(s, l),
        Tab::Battery => crate::mac_view::battery(s, l),
        Tab::Gpu => crate::mac_view::gpu(s, l),
        _ => cpu(s, l),
    };
    col().key(tab.id()).w_full().child(header).child(body)
}

pub(crate) fn reading(l: &Look, key: &'static str) -> El {
    col().w_full().px(PAD_X).pt(8.0).pb(16.0).child(text("Reading this Mac…").key(key).font(font(12.5, 400.0)).color(l.dim))
}

/// What is in use now: the large number, and its parts under it.
pub(crate) fn now_block(key: &'static str) -> El {
    col().key(key).w_full().px(PAD_X).pt(8.0).pb(12.0).gap(10.0)
}

/// The headline: a large number and what it counts, a step quieter.
pub(crate) fn big_number(l: &Look, number: &str, unit: &str) -> El {
    row()
        .items_end()
        .gap(4.0)
        .child(text(number.to_owned()).font(figure(32.0)).color(l.text))
        .child(text(unit.to_owned()).font(font(13.0, 400.0)).color(l.dim).pb(5.0))
}

pub fn cpu(s: &Ui, l: &Look) -> El {
    let Some((view, cpu)) = s.system.as_ref().and_then(|view| Some((view, view.cpu.as_ref()?))) else {
        return reading(l, "cpu-loading");
    };
    let [apps_color, system_color, _] = l.series;
    let idle = (1.0 - cpu.total()).max(0.0);

    // The share in use as one bar: apps, then the system, on the idle track.
    let split = |share: f32| (INNER_W * share.clamp(0.0, 1.0)).round();
    let mut bar = el().key("cpu-split").w(INNER_W).h(6.0).radius(3.0).fill(l.track);
    let (apps_w, system_w) = (split(cpu.user), split(cpu.system));
    if apps_w + system_w >= 1.0 {
        bar = bar.child(row().absolute(0.0, 0.0).h(6.0).clip_rounded(3.0).child(el().w(apps_w).h(6.0).fill(apps_color)).child(el().w(system_w).h(6.0).fill(system_color)));
    }
    let load = |value: f64, unit: &str| row().items_end().gap(3.0).child(text(format!("{value:.2}")).font(digits(12.5, 700.0)).color(l.text)).child(text(unit.to_owned()).font(font(10.0, 400.0)).color(l.faint));
    let now = now_block("cpu-hero")
        .child(big_number(l, &format!("{:.0}%", cpu.total() * 100.0), "in use"))
        .child(bar)
        .child(
            row()
                .w_full()
                .gap(16.0)
                .child(legend_value(l, apps_color, "Apps", percent(cpu.user)))
                .child(legend_value(l, system_color, "System", percent(cpu.system)))
                .child(legend_value(l, l.track, "Idle", percent(idle))),
        )
        .child(
            row()
                .key("load")
                .w_full()
                .center_y()
                .help("How many tasks were ready to run, on average")
                .child(caption(l, "Load average"))
                .child(spacer())
                .child(row().gap(10.0).items_end().child(load(cpu.load[0], "1 min")).child(load(cpu.load[1], "5 min")).child(load(cpu.load[2], "15 min"))),
        );

    let tier = s.tier;
    let (total, sys) = (&view.cpu_total[tier], &view.cpu_system[tier]);
    let chart = area_chart(l, "cpu-chart", &[Layer { top: total, bottom: Some(sys), color: apps_color }, Layer { top: sys, bottom: None, color: system_color }], s.point, ["100%", "50%"]);
    let readout = match s.point.and_then(|at| Some((at, total[at]?))) {
        Some((at, p)) => {
            let system = sys[at].map_or(0.0, |p| p.avg);
            look::readout(l, None, point_time(view, tier, at), format!("apps {} · system {}", percent(p.avg - system), percent(system)), percent(p.avg))
        }
        None => match peak(total) {
            Some(at) => look::readout(l, Some(apps_color), format!("Peak at {}", point_time(view, tier, at)), String::new(), percent(total[at].map_or(0.0, |p| p.max))),
            None => look::readout(l, None, "No readings yet".to_owned(), String::new(), String::new()),
        },
    };
    let history = section("cpu-history").gap(8.0).child(history_title(s, l)).child(chart).child(axis(view, tier, l)).child(readout);

    let apps = app_list(s, l, "cpu-apps", "share of all cores", |app| app.cpu as f64, |share| cpu_share(share as f32));
    col()
        .w_full()
        .child(now)
        .child(hairline(l))
        .child(history)
        .child(hairline(l))
        .child(apps)
        .child(hairline(l))
        .child(core_bars(l, &cpu.cores, view.machine.efficiency_cores))
}

pub fn memory(s: &Ui, l: &Look) -> El {
    let Some((view, memory)) = s.system.as_ref().and_then(|view| Some((view, view.memory.as_ref()?))) else {
        return reading(l, "memory-loading");
    };
    let [app, wired, compressed] = l.series;
    let cached = l.faint;

    let used = ui::system_bytes(memory.used());
    let (number, unit) = used.split_once(' ').unwrap_or((&used, ""));
    let (status, status_label) = match memory.pressure {
        Pressure::Normal => (l.good, "Pressure normal"),
        Pressure::Warning => (l.warning_text, "Pressure high"),
        Pressure::Critical => (l.critical_text, "Pressure critical"),
    };
    let hero = row()
        .w_full()
        .items_end()
        .child(big_number(l, number, &format!("{unit} used of {}", ui::system_bytes(memory.total))))
        .child(spacer())
        .child(col().pb(8.0).child(look::pill("pressure", status, status_label.to_owned()).help("How hard macOS is working to find memory for apps")));

    // The parts as one bar, each its share of all memory, and what is left free after them.
    let free = memory.total.saturating_sub(memory.used() + memory.cached);
    let parts = [(app, "App", memory.app), (wired, "Wired", memory.wired), (compressed, "Compressed", memory.compressed), (cached, "Cached", memory.cached), (l.track, "Free", free)];
    let total = memory.total.max(1) as f32;
    let shown: Vec<_> = parts.iter().filter(|part| part.2 as f32 / total * INNER_W >= 3.0).collect();
    let room = INNER_W - 2.0 * shown.len().saturating_sub(1) as f32;
    let mut bar = row().key("memory-bar").w_full().h(8.0).gap(2.0);
    for (at, (color, name, bytes)) in shown.iter().enumerate() {
        let w = (*bytes as f32 / total * room).floor();
        let piece = if at + 1 == shown.len() { el().grow(1.0).min_w(0.0) } else { el().w(w) };
        bar = bar.child(piece.key(format!("memory-part-{at}")).h(8.0).radius(2.0).fill(*color).help(format!("{name}: {}", ui::system_bytes(*bytes))));
    }
    let mut legend = row().key("memory-strip").w_full().justify_between();
    for (color, name, bytes) in parts {
        legend = legend.child(
            col()
                .gap(2.0)
                .child(row().gap(4.0).center_y().child(look::key_swatch(color, 7.0)).child(text(name).font(font(10.0, 400.0)).color(l.dim)))
                .child(text(ui::system_bytes(bytes)).font(digits(12.0, 600.0)).color(l.text)),
        );
    }
    let now = now_block("memory-hero")
        .child(hero)
        .child(bar)
        .child(legend)
        .child(text("Used is App + Wired + Compressed, as Activity Monitor counts it. Pressure is how hard macOS works to free memory.").font(font(10.5, 400.0).lh(14.0)).color(l.faint).w_full().wrap());

    let tier = s.tier;
    let used_share = &view.memory_used[tier];
    let chart = area_chart(l, "memory-chart", &[Layer { top: used_share, bottom: None, color: app }], s.point, ["100%", "50%"]);
    let bytes = |share: f32| ui::system_bytes((share as f64 * memory.total as f64) as u64);
    let readout = match s.point.and_then(|at| Some((at, used_share[at]?))) {
        Some((at, p)) => look::readout(l, None, point_time(view, tier, at), percent(p.avg), bytes(p.avg)),
        None => match peak(used_share) {
            Some(at) => {
                let p = used_share[at].map_or(0.0, |p| p.max);
                look::readout(l, Some(app), format!("Peak at {}", point_time(view, tier, at)), percent(p), bytes(p))
            }
            None => look::readout(l, None, "No readings yet".to_owned(), String::new(), String::new()),
        },
    };
    let history = section("memory-history").gap(8.0).child(history_title(s, l)).child(chart).child(axis(view, tier, l)).child(readout);

    let swap = row()
        .key("swap")
        .w_full()
        .py(9.0)
        .px(10.0)
        .gap(12.0)
        .radius(8.0)
        .continuous_corners()
        .fill(l.card)
        .center_y()
        .child(col().grow(1.0).min_w(0.0).gap(1.0).child(text("Swap").font(font(12.5, 600.0)).color(l.text)).child(text("Memory moved to disk to make room").font(font(10.5, 400.0)).color(l.faint)))
        .child(text(if memory.swap_used == 0 { "Not used".to_owned() } else { format!("{} used", ui::system_bytes(memory.swap_used)) }).font(digits(12.0, 600.0)).color(l.text));

    let apps = app_list(s, l, "memory-apps", "memory each app holds", |app| app.memory as f64, |bytes| ui::system_bytes(bytes as u64));
    col()
        .w_full()
        .child(now)
        .child(hairline(l))
        .child(apps)
        .child(hairline(l))
        .child(history.pb(12.0))
        .child(col().w_full().px(PAD_X).pb(14.0).child(swap))
}

/// An app's share of the CPU: a decimal under 10%, where the small ones differ.
fn cpu_share(share: f32) -> String {
    let percent = (share * 100.0).clamp(0.0, 100.0);
    if percent < 10.0 { format!("{percent:.1}%") } else { format!("{percent:.0}%") }
}

/// The apps using the most, one row each: icon, name, how many processes it runs and its
/// figure. Under them, what the rest add up to.
pub(crate) fn app_list(s: &Ui, l: &Look, key: &'static str, note: &str, measure: fn(&AppUsage) -> f64, value: fn(f64) -> String) -> El {
    let mut list = section(key).pb(10.0).gap(2.0).child(section_title(l, "Apps", Some(caption(l, note.to_owned()))).pb(6.0));
    let Some(all) = s.system.as_ref().and_then(|view| view.apps.clone()) else {
        return list.child(text("Reading apps…").key(format!("{key}-loading")).font(font(12.0, 400.0)).color(l.dim));
    };
    let (top, rest, others) = apps::top(&all, APP_ROWS, measure);
    for (at, app) in top.iter().enumerate() {
        let mut name = row().grow(1.0).min_w(0.0).gap(6.0).center_y().child(text(app.name.clone()).font(font(12.5, 500.0)).color(l.text).truncate().min_w(0.0));
        if app.processes > 1 {
            name = name.child(text(format!("{} processes", app.processes)).font(digits(11.0, 400.0)).color(l.faint));
        }
        list = list.child(
            row()
                .key(format!("{key}-{at}"))
                .w_full()
                .py(5.0)
                .gap(10.0)
                .center_y()
                .help(app.path.as_ref().map_or_else(|| app.name.clone(), |p| p.to_string_lossy().into_owned()))
                .child(app_icon(s, l, app))
                .child(name)
                .child(text(value(measure(app))).font(digits(12.5, 600.0)).color(l.text)),
        );
    }
    if others > 0 {
        let noun = if others == 1 { "app" } else { "apps" };
        list = list.child(caption(l, format!("{others} more {noun}, {} together", value(rest))).key(format!("{key}-rest")).pt(4.0));
    }
    list
}

/// An app's Finder icon, or while it loads (or for a process with none), a tile in its place.
fn app_icon(s: &Ui, l: &Look, app: &AppUsage) -> El {
    let tile = el().size(ICON, ICON);
    match app.path.as_deref().and_then(|path| s.icon(path, ICON)) {
        Some(image) => tile.pixels(image.pixels()),
        None => tile.radius(5.0).fill(l.track),
    }
}

pub(crate) fn percent(share: f32) -> String {
    format!("{:.0}%", (share * 100.0).clamp(0.0, 100.0))
}

pub(crate) fn history_title(s: &Ui, l: &Look) -> El {
    let mut tiers = segments(l, "tiers");
    for (at, tier) in TIERS.iter().enumerate() {
        tiers = tiers.child(segment(l, format!("tier-{}", tier.label), tier.label, s.tier == at, move |s: &mut Ui| ui::set_tier(s, at)));
    }
    section_title(l, "History", Some(tiers)).key("history")
}

pub(crate) fn point_time(view: &SystemView, tier: usize, at: usize) -> String {
    time::clock(history::bucket_start(tier, view.at, at), tier == 0)
}

/// The point with the highest reading.
pub(crate) fn peak(points: &[Option<Point>]) -> Option<usize> {
    points.iter().enumerate().filter_map(|(at, p)| Some((at, (*p)?.max))).max_by(|a, b| a.1.total_cmp(&b.1)).map(|(at, _)| at)
}

/// The times under the chart: where it starts, its middle, and now.
pub(crate) fn axis(view: &SystemView, tier: usize, l: &Look) -> El {
    let len = TIERS[tier].len;
    let mut axis = row().key("chart-axis").w_full().justify_between();
    for label in [point_time(view, tier, 0), point_time(view, tier, len / 2), "now".to_owned()] {
        axis = axis.child(text(label).font(digits(10.0, 400.0)).color(l.faint));
    }
    axis
}

/// One filled band of an area chart: from `bottom` (or the baseline) up to `top`, with a line
/// along `top`.
pub(crate) struct Layer<'a> {
    pub top: &'a [Option<Point>],
    pub bottom: Option<&'a [Option<Point>]>,
    pub color: Color,
}

/// The stretches of `points` with a reading in them, as index ranges.
pub(crate) fn runs(points: &[Option<Point>]) -> Vec<std::ops::Range<usize>> {
    let mut runs = Vec::new();
    let mut start = None;
    for (at, point) in points.iter().enumerate() {
        match (point.is_some(), start) {
            (true, None) => start = Some(at),
            (false, Some(from)) => {
                runs.push(from..at);
                start = None;
            }
            _ => {}
        }
    }
    if let Some(from) = start {
        runs.push(from..points.len());
    }
    runs
}

pub(crate) fn area_chart(l: &Look, key: &str, layers: &[Layer], point: Option<usize>, labels: [&str; 2]) -> El {
    let len = layers.first().map_or(0, |layer| layer.top.len());
    let (w, h) = (INNER_W, CHART_H);
    let x = |at: usize| if len > 1 { at as f32 / (len - 1) as f32 * w } else { 0.0 };
    // A hair inside the top and bottom, so a line at 0% or 100% isn't cut in half.
    let y = |share: f32| 1.0 + (1.0 - share.clamp(0.0, 1.0)) * (h - 2.0);

    let mut shape = Shape::new()
        .line([0.0, y(1.0)], [w, y(1.0)], 1.0, rune::LineCap::Butt, l.stroke)
        .line([0.0, y(0.5)], [w, y(0.5)], 1.0, rune::LineCap::Butt, l.stroke)
        .line([0.0, h - 0.5], [w, h - 0.5], 1.0, rune::LineCap::Butt, l.track);
    for layer in layers {
        let value = |points: &[Option<Point>], at: usize| points[at].map_or(0.0, |p| p.avg);
        for run in runs(layer.top) {
            // A lone reading still shows, as a sliver a point wide.
            let xs: Vec<(usize, f32)> = if run.len() == 1 { vec![(run.start, x(run.start) - 0.75), (run.start, x(run.start) + 0.75)] } else { run.clone().map(|at| (at, x(at))).collect() };
            let top: Vec<[f32; 2]> = xs.iter().map(|&(at, px)| [px, y(value(layer.top, at))]).collect();
            let bottom: Vec<[f32; 2]> = xs.iter().rev().map(|&(at, px)| [px, y(layer.bottom.map_or(0.0, |b| value(b, at)))]).collect();
            shape = shape.fill([top.clone(), bottom].concat(), layer.color.alpha(0.22));
            shape = shape.stroke(top, LINE, layer.color);
        }
    }
    // A ring on the point read: the one under the pointer, else the peak.
    let marked = point.filter(|&at| at < len).or_else(|| layers.first().and_then(|layer| peak(layer.top)));
    if let Some(at) = marked {
        let px = x(at);
        if point.is_some() {
            shape = shape.line([px, 0.0], [px, h], 1.0, rune::LineCap::Butt, l.dim);
        }
        if let Some(layer) = layers.first()
            && let Some(p) = layer.top[at]
        {
            let py = y(p.avg);
            shape = shape.line([px, py], [px + 0.01, py], 9.0, rune::LineCap::Round, layer.color).line([px, py], [px + 0.01, py], 5.0, rune::LineCap::Round, l.solid());
        }
    }
    let label = |words: &str, top: f32| row().absolute(w - 80.0, top).w(80.0).justify_end().px(2.0).child(text(words.to_owned()).font(digits(9.0, 400.0)).color(l.faint));
    el().key(format!("{key}-area"))
        .size(w, h)
        .child(
            canvas(shape)
                .key(key.to_owned())
                .size(w, h)
                .label("History chart")
                .on_pointer_move_local(move |s: &mut Ui, p, _| ui::hover_point(s, p.x / w, len))
                .on_hover_change(|s: &mut Ui, inside, _| {
                    if !inside {
                        ui::leave_point(s);
                    }
                }),
        )
        .child(label(labels[0], y(1.0) + 3.0))
        .child(label(labels[1], y(0.5) - 14.0))
}

/// Each core's share in use as a bar, efficiency cores in their own group before the rest.
fn core_bars(l: &Look, cores: &[f32], efficiency: usize) -> El {
    let fill = l.series[0];
    let group = |key: &'static str, label: String, first: usize, shares: &[f32]| {
        let mut bars = row().w_full().h(CORE_H).gap(4.0);
        for (at, &share) in shares.iter().enumerate() {
            let height = (share.clamp(0.0, 1.0) * CORE_H).max(2.0);
            bars = bars.child(
                col()
                    .key(format!("core-{}", first + at))
                    .grow(1.0)
                    .min_w(0.0)
                    .h(CORE_H)
                    .clip_rounded(3.0)
                    .fill(l.track)
                    .justify_end()
                    .help(format!("Core {}: {}", first + at + 1, percent(share)))
                    .child(el().w_full().h(height).fill(fill)),
            );
        }
        col().key(key).grow(shares.len().max(1) as f32).min_w(0.0).gap(5.0).child(bars).child(text(label).font(font(10.5, 400.0)).color(l.faint))
    };
    let efficiency = efficiency.min(cores.len());
    let performance = cores.len() - efficiency;
    let mut groups = row().key("cores").w_full().gap(12.0);
    if efficiency > 0 {
        groups = groups.child(group("cores-efficiency", format!("{efficiency} efficiency"), 0, &cores[..efficiency]));
    }
    groups = groups.child(group("cores-performance", if efficiency > 0 { format!("{performance} performance") } else { format!("{performance} cores") }, efficiency, &cores[efficiency..]));
    section("cores-section").gap(8.0).child(section_title(l, "Cores", Some(caption(l, "each bar: how busy that core is")))).child(groups)
}
