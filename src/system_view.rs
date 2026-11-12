//! The panel's tabs for this Mac: the CPU and memory, each as a headline number, its history as
//! an area chart at three resolutions, its parts, and the apps that use the most of it.
//!
//! Charts follow one scheme: a share from 0 to 100% on a single axis, series in the fixed order
//! of `Look::series`, a 1.75-point line over a faint fill, hairlines at 50% and 100%, and a
//! readout under the chart for the point under the pointer or, without one, the peak.

use rune::{Color, El, Shape, canvas, col, el, row, spacer, text};

use crate::system::apps::{self, AppUsage};
use crate::system::history::{self, Point, TIERS};
use crate::system::{Pressure, SystemView};
use crate::time;
use crate::ui::{self, Tab, Ui};
use crate::view::{INNER_W, Look, PAD_X, digits, font, segment, segments, title_bar};

const CHART_H: f32 = 76.0;
const LINE: f32 = 1.75;
const CORE_H: f32 = 40.0;
const APP_ROWS: usize = 5;
const ICON: f32 = 22.0;

pub fn header(s: &Ui, l: &Look, tab: Tab) -> El {
    let machine = s.system.as_ref().map(|view| view.machine.clone());
    let chip = match tab {
        Tab::Memory => machine.as_ref().map(|m| ui::system_bytes(m.memory)),
        _ => machine.as_ref().map(|m| m.chip.clone()),
    };
    title_bar(l, tab.label(), chip.as_deref())
}

pub fn cpu(s: &Ui, l: &Look) -> El {
    let body = col().key("cpu").w_full().px(PAD_X).pb(14.0).gap(14.0);
    let Some((view, cpu)) = s.system.as_ref().and_then(|view| Some((view, view.cpu.as_ref()?))) else {
        return body.child(text("Reading this Mac…").key("cpu-loading").font(font(12.5, 400.0)).color(l.dim));
    };
    let [apps, system, _] = l.series;
    let machine = &view.machine;
    let cores = machine.performance_cores + machine.efficiency_cores;

    let hero = row()
        .key("cpu-hero")
        .w_full()
        .items_end()
        .child(
            big_number(l, &format!("{:.0}", cpu.total() * 100.0), "%", &format!("in use · {cores} cores · load {:.2}", cpu.load[0]))
                .help(format!("Load average: {:.2} over 1 minute, {:.2} over 5, {:.2} over 15", cpu.load[0], cpu.load[1], cpu.load[2])),
        )
        .child(spacer())
        .child(col().gap(5.0).pb(3.0).child(legend_row(l, apps, "Apps", percent(cpu.user))).child(legend_row(l, system, "System", percent(cpu.system))));

    let tier = s.tier;
    let (total, sys) = (&view.cpu_total[tier], &view.cpu_system[tier]);
    let chart = area_chart(l, "cpu-chart", &[Layer { top: total, bottom: Some(sys), color: apps }, Layer { top: sys, bottom: None, color: system }], s.point);
    let read = |at: usize| total[at].map(|p| (p, sys[at].map_or(0.0, |p| p.avg)));
    let readout = match s.point.and_then(|at| Some((at, read(at)?))) {
        Some((at, (p, sys))) => readout(l, point_time(view, tier, at), true, format!("apps {} · system {}", percent(p.avg - sys), percent(sys)), percent(p.avg)),
        None => match peak(total) {
            Some(at) => readout(l, format!("Peak · {}", point_time(view, tier, at)), false, String::new(), percent(total[at].map_or(0.0, |p| p.max))),
            None => readout(l, "No readings yet".to_owned(), false, String::new(), String::new()),
        },
    };

    let apps = app_list(s, l, "cpu-apps", "share of all cores", |app| app.cpu as f64, |share| cpu_share(share as f32));
    body.child(hero)
        .child(history_bar(s, l))
        .child(col().w_full().gap(5.0).child(chart).child(axis(view, tier)(l)).child(el().h(3.0)).child(readout))
        .child(apps)
        .child(core_bars(l, &cpu.cores, machine.efficiency_cores))
}

pub fn memory(s: &Ui, l: &Look) -> El {
    let body = col().key("memory").w_full().px(PAD_X).pb(14.0).gap(14.0);
    let Some((view, memory)) = s.system.as_ref().and_then(|view| Some((view, view.memory.as_ref()?))) else {
        return body.child(text("Reading this Mac…").key("memory-loading").font(font(12.5, 400.0)).color(l.dim));
    };
    let [app, wired, compressed] = l.series;
    let cached = l.dim.alpha(0.5);

    let used = ui::system_bytes(memory.used());
    let (number, unit) = used.split_once(' ').unwrap_or((&used, ""));
    let (status, status_label) = match memory.pressure {
        Pressure::Normal => (l.good, "Pressure normal"),
        Pressure::Warning => (l.warning, "Pressure high"),
        Pressure::Critical => (l.critical, "Pressure critical"),
    };
    let pressure = row()
        .key("pressure")
        .h(22.0)
        .px(8.0)
        .gap(6.0)
        .radius(11.0)
        .center_y()
        .fill(status.alpha(0.16))
        .help("How hard macOS is working to find memory for apps")
        .child(el().size(7.0, 7.0).radius(3.5).fill(status))
        .child(text(status_label).font(font(11.0, 600.0)).color(l.text));
    let hero = row()
        .key("memory-hero")
        .w_full()
        .items_end()
        .child(big_number(l, number, unit, &format!("used of {}", ui::system_bytes(memory.total))))
        .child(spacer())
        .child(col().pb(3.0).child(pressure));

    // The parts as one bar, each its share of all memory, and what is left free as the track.
    let total = memory.total.max(1) as f32;
    let parts = [(app, "App", memory.app), (wired, "Wired", memory.wired), (compressed, "Compressed", memory.compressed), (cached, "Cached", memory.cached)];
    let mut bar = row().key("memory-bar").w_full().h(10.0).gap(2.0);
    let mut left = INNER_W;
    for (at, (color, name, bytes)) in parts.iter().enumerate() {
        let w = (bytes.to_owned() as f32 / total * INNER_W).floor();
        if w < 2.0 {
            continue;
        }
        left -= w + 2.0;
        bar = bar.child(el().key(format!("memory-part-{at}")).w(w - 2.0).h(10.0).radius(3.0).fill(*color).help(format!("{name}: {}", ui::system_bytes(*bytes))));
    }
    if left > 2.0 {
        bar = bar.child(el().key("memory-free").grow(1.0).min_w(0.0).h(10.0).radius(3.0).fill(l.track).help("Free"));
    }
    let mut strip = row().key("memory-strip").w_full();
    for (color, name, bytes) in parts {
        strip = strip.child(cell(l, Some(color), name, ui::system_bytes(bytes)));
    }

    let tier = s.tier;
    let used_share = &view.memory_used[tier];
    let chart = area_chart(l, "memory-chart", &[Layer { top: used_share, bottom: None, color: app }], s.point);
    let bytes = |share: f32| ui::system_bytes((share as f64 * memory.total as f64) as u64);
    let readout = match s.point.and_then(|at| Some((at, used_share[at]?))) {
        Some((at, p)) => readout(l, point_time(view, tier, at), true, percent(p.avg), bytes(p.avg)),
        None => match peak(used_share) {
            Some(at) => {
                let p = used_share[at].map_or(0.0, |p| p.max);
                readout(l, format!("Peak · {}", point_time(view, tier, at)), false, percent(p), bytes(p))
            }
            None => readout(l, "No readings yet".to_owned(), false, String::new(), String::new()),
        },
    };

    let swap = row()
        .key("swap")
        .w_full()
        .h(32.0)
        .px(10.0)
        .radius(8.0)
        .fill(l.card)
        .center_y()
        .help("Memory macOS moved to disk to make room")
        .child(text("Swap").font(font(12.0, 500.0)).color(l.text))
        .child(spacer())
        .child(text(if memory.swap_used == 0 { "Not used".to_owned() } else { format!("{} used", ui::system_bytes(memory.swap_used)) }).font(digits(12.0, 500.0)).color(l.dim));

    let apps = app_list(s, l, "memory-apps", "by footprint", |app| app.memory as f64, |bytes| ui::system_bytes(bytes as u64));
    body.child(hero)
        .child(col().w_full().gap(10.0).child(bar).child(strip))
        .child(apps)
        .child(history_bar(s, l))
        .child(col().w_full().gap(5.0).child(chart).child(axis(view, tier)(l)).child(el().h(3.0)).child(readout))
        .child(swap)
}

/// An app's share of the CPU: a decimal under 10%, where the small ones differ.
fn cpu_share(share: f32) -> String {
    let percent = (share * 100.0).clamp(0.0, 100.0);
    if percent < 10.0 { format!("{percent:.1}%") } else { format!("{percent:.0}%") }
}

/// The apps using the most, one row each: icon, name, how many processes it runs and its
/// figure, over a slim meter of its share of the busiest app's. Under them, what the rest add
/// up to.
fn app_list(s: &Ui, l: &Look, key: &'static str, caption: &str, measure: fn(&AppUsage) -> f64, value: fn(f64) -> String) -> El {
    let mut list = col().key(key).w_full().gap(2.0);
    let heading = row().w_full().center_y().pb(6.0).child(text("Apps").font(font(13.0, 600.0)).color(l.text)).child(spacer()).child(text(caption.to_owned()).font(font(11.0, 400.0)).color(l.dim));
    list = list.child(heading);
    let Some(all) = s.system.as_ref().and_then(|view| view.apps.clone()) else {
        return list.child(text("Reading apps…").key(format!("{key}-loading")).font(font(12.0, 400.0)).color(l.dim).px(8.0));
    };
    let (top, rest, others) = apps::top(&all, APP_ROWS, measure);
    let most = top.first().map_or(0.0, |app| measure(app)).max(f64::MIN_POSITIVE);
    // The meter runs under the name and the figure, from the icon's edge to the panel's.
    let meter_w = INNER_W - ICON - 10.0;
    for (at, app) in top.iter().enumerate() {
        let share = (measure(app) / most).clamp(0.0, 1.0) as f32;
        let mut line = row().w_full().gap(6.0).center_y().child(text(app.name.clone()).font(font(12.0, 500.0)).color(l.text).truncate().min_w(0.0));
        if app.processes > 1 {
            line = line.child(text(format!("{} processes", app.processes)).font(digits(10.5, 400.0)).color(l.faint));
        }
        line = line.child(spacer()).child(text(value(measure(app))).font(digits(12.0, 500.0)).color(l.text));
        let mut meter = el().w(meter_w).h(3.0).radius(1.5).fill(l.track);
        if share > 0.0 {
            meter = meter.child(el().absolute(0.0, 0.0).size((meter_w * share).max(3.0), 3.0).radius(1.5).fill(l.series[0]));
        }
        list = list.child(
            row()
                .key(format!("{key}-{at}"))
                .w_full()
                .h(34.0)
                .gap(10.0)
                .center_y()
                .help(app.path.as_ref().map_or_else(|| app.name.clone(), |p| p.to_string_lossy().into_owned()))
                .child(app_icon(s, l, app))
                .child(col().grow(1.0).min_w(0.0).gap(5.0).child(line).child(meter)),
        );
    }
    if others > 0 {
        list = list.child(text(format!("{others} more, {} together", value(rest))).key(format!("{key}-rest")).font(digits(11.0, 400.0)).color(l.faint).pl(ICON + 10.0).pt(4.0));
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

fn percent(share: f32) -> String {
    format!("{:.0}%", (share * 100.0).clamp(0.0, 100.0))
}

/// The headline: a large number, its unit a step quieter, and a caption under them.
fn big_number(l: &Look, number: &str, unit: &str, caption: &str) -> El {
    col()
        .gap(1.0)
        .child(
            row()
                .items_end()
                .gap(2.0)
                .child(text(number.to_owned()).font(digits(30.0, 700.0).tracking(-0.6)).color(l.text))
                .child(text(unit.to_owned()).font(font(15.0, 600.0)).color(l.dim).pb(4.0)),
        )
        .child(text(caption.to_owned()).font(font(11.0, 400.0)).color(l.dim))
}

/// A series' swatch, its name and its value: the legend, so colour never names a series alone.
fn legend_row(l: &Look, color: Color, name: &str, value: String) -> El {
    row()
        .gap(6.0)
        .center_y()
        .child(el().size(8.0, 8.0).radius(2.0).fill(color))
        .child(text(name.to_owned()).font(font(11.5, 400.0)).color(l.dim))
        .child(row().w(36.0).justify_end().child(text(value).font(digits(11.5, 600.0)).color(l.text)))
}

/// One figure in a row of four: its name small above, its value below.
fn cell(l: &Look, swatch: Option<Color>, name: &str, value: String) -> El {
    let mut label = row().gap(5.0).center_y();
    if let Some(color) = swatch {
        label = label.child(el().size(7.0, 7.0).radius(2.0).fill(color));
    }
    label = label.child(text(name.to_owned()).font(font(10.5, 400.0)).color(l.faint));
    col().grow(1.0).min_w(0.0).gap(2.0).child(label).child(text(value).font(digits(12.0, 500.0)).color(l.text))
}

fn history_bar(s: &Ui, l: &Look) -> El {
    let mut tiers = segments(l, "tiers");
    for (at, tier) in TIERS.iter().enumerate() {
        tiers = tiers.child(segment(l, format!("tier-{}", tier.label), tier.label, s.tier == at, move |s: &mut Ui| ui::set_tier(s, at)));
    }
    row().key("history").w_full().center_y().child(text("History").font(font(13.0, 600.0)).color(l.text)).child(spacer()).child(tiers)
}

fn point_time(view: &SystemView, tier: usize, at: usize) -> String {
    time::clock(history::bucket_start(tier, view.at, at), tier == 0)
}

/// The point with the highest reading.
fn peak(points: &[Option<Point>]) -> Option<usize> {
    points.iter().enumerate().filter_map(|(at, p)| Some((at, (*p)?.max))).max_by(|a, b| a.1.total_cmp(&b.1)).map(|(at, _)| at)
}

/// The times under the chart: where it starts, its middle, and now.
fn axis(view: &SystemView, tier: usize) -> impl Fn(&Look) -> El {
    let len = TIERS[tier].len;
    let labels = [point_time(view, tier, 0), point_time(view, tier, len / 2), "now".to_owned()];
    move |l: &Look| {
        let mut axis = row().key("chart-axis").w_full().justify_between();
        for label in &labels {
            axis = axis.child(text(label.clone()).font(digits(10.0, 400.0)).color(l.faint));
        }
        axis
    }
}

/// The line under a chart reading one point: its time, then its values. It keeps its shape as
/// the pointer moves, so only the words change.
fn readout(l: &Look, name: String, live: bool, side: String, value: String) -> El {
    row()
        .key("chart-readout")
        .w_full()
        .h(26.0)
        .px(8.0)
        .radius(6.0)
        .fill(l.card)
        .center_y()
        .child(text(name).font(digits(11.5, 500.0)).color(if live { l.text } else { l.dim }))
        .child(spacer())
        .child(text(side).font(digits(11.5, 400.0)).color(l.dim))
        .child(row().w(64.0).justify_end().child(text(value).font(digits(11.5, 600.0)).color(l.text)))
}

/// One filled band of an area chart: from `bottom` (or the baseline) up to `top`, with a line
/// along `top`.
struct Layer<'a> {
    top: &'a [Option<Point>],
    bottom: Option<&'a [Option<Point>]>,
    color: Color,
}

/// The stretches of `points` with a reading in them, as index ranges.
fn runs(points: &[Option<Point>]) -> Vec<std::ops::Range<usize>> {
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

fn area_chart(l: &Look, key: &str, layers: &[Layer], point: Option<usize>) -> El {
    let len = layers.first().map_or(0, |layer| layer.top.len());
    let (w, h) = (INNER_W, CHART_H);
    let x = |at: usize| if len > 1 { at as f32 / (len - 1) as f32 * w } else { 0.0 };
    // A hair inside the top and bottom, so a line at 0% or 100% isn't cut in half.
    let y = |share: f32| 1.0 + (1.0 - share.clamp(0.0, 1.0)) * (h - 2.0);

    let mut shape = Shape::new()
        .line([0.0, y(1.0)], [w, y(1.0)], 1.0, rune::LineCap::Butt, l.stroke)
        .line([0.0, y(0.5)], [w, y(0.5)], 1.0, rune::LineCap::Butt, l.stroke)
        .line([0.0, y(0.0)], [w, y(0.0)], 1.0, rune::LineCap::Butt, l.track);
    for layer in layers {
        let value = |points: &[Option<Point>], at: usize| points[at].map_or(0.0, |p| p.avg);
        for run in runs(layer.top) {
            // A lone reading still shows, as a sliver a point wide.
            let xs: Vec<(usize, f32)> = if run.len() == 1 { vec![(run.start, x(run.start) - 0.75), (run.start, x(run.start) + 0.75)] } else { run.clone().map(|at| (at, x(at))).collect() };
            let top: Vec<[f32; 2]> = xs.iter().map(|&(at, px)| [px, y(value(layer.top, at))]).collect();
            let bottom: Vec<[f32; 2]> = xs.iter().rev().map(|&(at, px)| [px, y(layer.bottom.map_or(0.0, |b| value(b, at)))]).collect();
            shape = shape.fill([top.clone(), bottom].concat(), layer.color.alpha(0.2));
            shape = shape.stroke(top, LINE, layer.color);
        }
    }
    if let Some(at) = point.filter(|&at| at < len) {
        let px = x(at);
        shape = shape.line([px, 0.0], [px, h], 1.0, rune::LineCap::Butt, l.faint);
        for layer in layers {
            if let Some(p) = layer.top[at] {
                // A dot on the line, ringed in the panel's colour so it lifts off the fill.
                let py = y(p.avg);
                shape = shape.line([px, py], [px + 0.01, py], 9.0, rune::LineCap::Round, l.panel.alpha(1.0)).line([px, py], [px + 0.01, py], 6.0, rune::LineCap::Round, layer.color);
            }
        }
    }
    canvas(shape)
        .key(key.to_owned())
        .size(w, h)
        .label("History chart")
        .on_pointer_move_local(move |s: &mut Ui, p, _| ui::hover_point(s, p.x / w, len))
        .on_hover_change(|s: &mut Ui, inside, _| {
            if !inside {
                ui::leave_point(s);
            }
        })
}

/// Each core's share in use as a bar, efficiency cores in their own group before the rest.
fn core_bars(l: &Look, cores: &[f32], efficiency: usize) -> El {
    let fill = l.series[0];
    let group = |key: &'static str, label: &str, first: usize, shares: &[f32]| {
        let mut bars = row().w_full().h(CORE_H).gap(3.0);
        for (at, &share) in shares.iter().enumerate() {
            let height = (share.clamp(0.0, 1.0) * CORE_H).max(2.0);
            bars = bars.child(
                col()
                    .key(format!("core-{}", first + at))
                    .grow(1.0)
                    .min_w(0.0)
                    .h(CORE_H)
                    .radius(3.0)
                    .fill(l.track)
                    .justify_end()
                    .help(format!("Core {}: {}", first + at + 1, percent(share)))
                    .child(el().w_full().h(height).radius(3.0).fill(fill)),
            );
        }
        col().key(key).grow(shares.len().max(1) as f32).min_w(0.0).gap(5.0).child(bars).child(text(label.to_owned()).font(font(10.0, 400.0)).color(l.faint))
    };
    let efficiency = efficiency.min(cores.len());
    let performance = cores.len() - efficiency;
    let summary = if efficiency > 0 { format!("{performance} performance · {efficiency} efficiency") } else { format!("{} cores", cores.len()) };
    let mut groups = row().key("cores").w_full().gap(10.0);
    if efficiency > 0 {
        groups = groups.child(group("cores-efficiency", "Efficiency", 0, &cores[..efficiency]));
    }
    groups = groups.child(group("cores-performance", if efficiency > 0 { "Performance" } else { "" }, efficiency, &cores[efficiency..]));
    col()
        .w_full()
        .gap(8.0)
        .child(row().w_full().center_y().child(text("Cores").font(font(13.0, 600.0)).color(l.text)).child(spacer()).child(text(summary).font(font(11.0, 400.0)).color(l.dim)))
        .child(groups)
}
