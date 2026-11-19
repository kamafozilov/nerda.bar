//! The panel's tabs for this Mac: CPU, Memory, Disk, Network, Battery and GPU. Each has a
//! context bar naming what it measures, and a body of two columns: on the left what is in use
//! now and its history, on the right what is behind it. CPU and Memory are here, the others in
//! `mac_view`; the pieces they share (the headline, a bar of parts, the apps list, the history
//! chart) are here too.
//!
//! The panel has one size, so every section's height is known before layout: fixed parts add
//! up, and a chart takes what its column has left, or a list shows as many rows as fit.
//!
//! Charts follow one scheme: a share from 0 to 100% on a single axis, series in the fixed order
//! of `Look::series`, a line over a faint fill, hairlines at 50% and 100%, and a readout under
//! the chart for the point under the pointer or, without one, the peak.

use rune::{Color, El, Icon, Shape, canvas, col, el, row, spacer, text};

use crate::icons;
use crate::look::{self, BODY_H, GAP, INNER_W, Look, PAD_Y, READOUT_H, ROW_H, TITLE_H, caption, digits, figure, font, legend_line, legend_value, more_line, next_section, rows_that_fit, section, section_title, segment, segments};
use crate::mac_view;
use crate::system::apps::{self, AppUsage};
use crate::system::history::{self, Point, TIERS};
use crate::system::{Pressure, SystemView};
use crate::time;
use crate::ui::{self, Tab, Ui};
use crate::view::Screen;

/// A headline number's line.
pub(crate) const HERO_H: f32 = 40.0;
/// A bar of parts under a headline.
pub(crate) const BAR_H: f32 = 8.0;
/// A line of row text, and a line of a caption, a legend of lines or an axis.
pub(crate) const LINE_H: f32 = 16.0;
pub(crate) const SMALL_H: f32 = 14.0;
/// A chart's line.
const LINE: f32 = 1.75;
const CORE_H: f32 = 44.0;
/// An app's icon, and the column of its value.
pub(crate) const ICON: f32 = 20.0;
const VALUE_W: f32 = 64.0;
/// What an app's name may take of its row before it is cut: the row less the icon, the value
/// and the gaps between them. The process count beside it gives way first.
const NAME_W: f32 = INNER_W - ICON - VALUE_W - 2.0 * 12.0;

/// A tab's icon in the top bar and in Settings.
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

/// What a tab says in its context bar: a badge beside its name, a quiet line after them, and a
/// status at the bar's end.
#[derive(Default)]
pub(crate) struct Heading {
    pub badge: Option<String>,
    pub words: Option<String>,
    pub status: Option<El>,
}

pub fn screen(s: &Ui, l: &Look, tab: Tab) -> Screen {
    let view = s.system.as_deref();
    let heading = match tab {
        Tab::Memory => memory_heading(l, view),
        Tab::Disk | Tab::Network | Tab::Battery | Tab::Gpu => mac_view::heading(l, view, tab),
        _ => cpu_heading(s, view),
    };
    let title = look::context_title(l, tab.label(), heading.badge.as_deref(), heading.words);
    let columns = view.and_then(|view| match tab {
        Tab::Memory => memory(s, l, view),
        Tab::Disk => mac_view::disk(s, l, view),
        Tab::Network => mac_view::network(s, l, view),
        Tab::Battery => mac_view::battery(s, l, view),
        Tab::Gpu => mac_view::gpu(s, l, view),
        _ => cpu(s, l, view),
    });
    let [left, right] = columns.unwrap_or_else(|| [reading(l, tab), col()]);
    Screen { context: look::context_bar(l, title, heading.status), body: look::columns(l, left, right) }
}

/// Until the first reading arrives.
fn reading(l: &Look, tab: Tab) -> El {
    section("reading").child(text("Reading this Mac…").key(format!("{}-loading", tab.id())).font(font(12.0, 400.0)).color(l.dim))
}

/// The height of a section holding `parts`, a gap apart.
pub(crate) fn stack(parts: &[f32]) -> f32 {
    2.0 * PAD_Y + parts.iter().sum::<f32>() + GAP * parts.len().saturating_sub(1) as f32
}

/// What a section `height` tall leaves for one more part after `parts`.
pub(crate) fn rest(height: f32, parts: &[f32]) -> f32 {
    height - stack(parts) - if parts.is_empty() { 0.0 } else { GAP }
}

/// The headline: a large number and what it counts, a step quieter, on one baseline.
pub(crate) fn hero(l: &Look, number: String, unit: String) -> El {
    row()
        .h(HERO_H)
        .items_end()
        .gap(8.0)
        .child(text(number).font(figure(34.0)).color(l.text))
        .child(text(unit).font(font(15.0, 400.0)).color(l.dim).pb(4.0))
}

/// Parts of a whole as one bar, a sliver apart: each part is a colour, its tooltip and its
/// share. A part too thin to see is left out; the last takes what the others leave.
pub(crate) fn parts_bar(key: &'static str, parts: &[(Color, String, f64)]) -> El {
    let shown: Vec<_> = parts.iter().filter(|part| part.2 * INNER_W as f64 >= 3.0).collect();
    let room = INNER_W - 2.0 * shown.len().saturating_sub(1) as f32;
    let mut bar = row().key(key).w(INNER_W).h(BAR_H).gap(2.0);
    for (at, (color, help, share)) in shown.iter().enumerate() {
        let piece = if at + 1 == shown.len() { el().grow(1.0).min_w(0.0) } else { el().w((*share as f32 * room).floor()) };
        bar = bar.child(piece.key(format!("{key}-{at}")).h(BAR_H).radius(BAR_H / 2.0).fill(*color).help(help.clone()));
    }
    bar
}

fn cpu_heading(s: &Ui, view: Option<&SystemView>) -> Heading {
    let machine = view.map(|view| &view.machine);
    Heading {
        badge: machine.map(|m| m.chip.clone()),
        words: machine.map(|m| {
            let cores = m.performance_cores + m.efficiency_cores;
            match m.booted_at {
                Some(at) => format!("{cores} cores · up {}", time::span(s.now - at)),
                None => format!("{cores} cores"),
            }
        }),
        status: None,
    }
}

fn memory_heading(l: &Look, view: Option<&SystemView>) -> Heading {
    let machine = view.map(|view| &view.machine);
    let status = view.and_then(|view| view.memory.as_ref()).map(|memory| {
        let (color, words) = match memory.pressure {
            Pressure::Normal => (l.good_text, "Pressure normal"),
            Pressure::Warning => (l.warning_text, "Pressure high"),
            Pressure::Critical => (l.critical_text, "Pressure critical"),
        };
        look::pill("pressure", color, words.to_owned()).help("How hard macOS is working to find memory for apps")
    });
    Heading {
        badge: machine.map(|m| ui::system_bytes(m.memory)),
        words: machine.filter(|m| m.chip.starts_with('M')).map(|_| "Unified memory, shared with the GPU".to_owned()),
        status,
    }
}

pub fn cpu(s: &Ui, l: &Look, view: &SystemView) -> Option<[El; 2]> {
    let cpu = view.cpu.as_ref()?;
    let [apps_color, system_color, _] = l.series;
    let idle = (1.0 - cpu.total()).max(0.0);

    let load = |value: f64, unit: &str| row().items_end().gap(4.0).child(text(format!("{value:.2}")).font(digits(12.0, 600.0)).color(l.text)).child(text(unit.to_owned()).font(font(12.0, 400.0)).color(l.faint));
    let overview_h = stack(&[HERO_H, BAR_H, LINE_H, LINE_H]);
    let overview = section("cpu-hero")
        .h(overview_h)
        .child(hero(l, percent(cpu.total()), "in use".to_owned()))
        .child(parts_bar("cpu-split", &[(apps_color, format!("Apps: {}", percent(cpu.user)), cpu.user as f64), (system_color, format!("System: {}", percent(cpu.system)), cpu.system as f64), (l.track, format!("Idle: {}", percent(idle)), idle as f64)]))
        .child(
            row()
                .h(LINE_H)
                .gap(16.0)
                .center_y()
                .child(legend_value(l, apps_color, "Apps", percent(cpu.user)))
                .child(legend_value(l, system_color, "System", percent(cpu.system)))
                .child(legend_value(l, l.track, "Idle", percent(idle))),
        )
        .child(
            row()
                .key("load")
                .w_full()
                .h(LINE_H)
                .center_y()
                .help("How many tasks were ready to run, on average")
                .child(text("Load average").font(font(12.0, 400.0)).color(l.dim))
                .child(spacer())
                .child(row().gap(12.0).items_end().child(load(cpu.load[0], "1m")).child(load(cpu.load[1], "5m")).child(load(cpu.load[2], "15m"))),
        );

    let tier = s.tier;
    let (total, sys) = (&view.cpu_total[tier], &view.cpu_system[tier]);
    let history_h = BODY_H - overview_h;
    let chart_h = rest(history_h, &[TITLE_H, 2.0 * SMALL_H + 8.0, READOUT_H]);
    let chart = area_chart(l, "cpu-chart", chart_h, &[Layer { top: total, bottom: Some(sys), color: apps_color }, Layer { top: sys, bottom: None, color: system_color }], s.point);
    let readout = match s.point.and_then(|at| Some((at, total[at]?))) {
        Some((at, p)) => {
            let system = sys[at].map_or(0.0, |p| p.avg);
            look::readout(l, None, point_time(view, tier, at), format!("apps {} · system {}", percent(p.avg - system), percent(system)), percent(p.avg))
        }
        None => match peak(total) {
            Some(at) => look::readout(l, Some(apps_color), format!("Peak at {}", point_time(view, tier, at)), String::new(), percent(total[at].map_or(0.0, |p| p.max))),
            None => no_readings(l),
        },
    };
    let keys = row().h(SMALL_H).gap(16.0).center_y().child(legend_line(l, apps_color, None, "Apps")).child(legend_line(l, system_color, None, "System"));
    let history = next_section(l, "cpu-history")
        .h(history_h)
        .child(tiers_title(s, l, "History"))
        .child(chart)
        .child(col().w_full().gap(8.0).child(axis(view, tier, l)).child(keys))
        .child(readout);

    let cores_h = stack(&[TITLE_H, CORE_H + 8.0 + SMALL_H]);
    let apps = app_list(s, l, "cpu-apps", "Share of all cores", BODY_H - cores_h, |app| app.cpu as f64, |share| cpu_share(share as f32));
    let cores = core_bars(l, &cpu.cores, view.machine.efficiency_cores).h(cores_h);
    Some([col().w_full().child(overview).child(history), col().w_full().child(apps).child(cores)])
}

pub fn memory(s: &Ui, l: &Look, view: &SystemView) -> Option<[El; 2]> {
    let memory = view.memory.as_ref()?;
    let [app, wired, compressed] = l.series;

    let used = ui::system_bytes(memory.used());
    let (number, unit) = used.split_once(' ').unwrap_or((&used, ""));
    // The parts as one bar, each its share of all memory, and what is left free after them.
    let free = memory.total.saturating_sub(memory.used() + memory.cached);
    let parts = [(app, "App", memory.app), (wired, "Wired", memory.wired), (compressed, "Compressed", memory.compressed), (l.series_rest, "Cached", memory.cached), (l.track, "Free", free)];
    let total = memory.total.max(1) as f64;
    let bar: Vec<_> = parts.iter().map(|&(color, name, bytes)| (color, format!("{name}: {}", ui::system_bytes(bytes)), bytes as f64 / total)).collect();
    let mut legend = row().key("memory-strip").w_full().h(2.0 * LINE_H).justify_between();
    for (color, name, bytes) in parts {
        legend = legend.child(
            col()
                .child(row().h(LINE_H).gap(6.0).center_y().child(look::key_swatch(color, 8.0)).child(text(name).font(font(12.0, 400.0)).color(l.dim)))
                .child(text(ui::system_bytes(bytes)).font(digits(13.0, 600.0)).color(l.text)),
        );
    }
    let overview_h = stack(&[HERO_H, BAR_H, 2.0 * LINE_H, SMALL_H]);
    let overview = section("memory-hero")
        .h(overview_h)
        .child(hero(l, number.to_owned(), format!("{unit} used of {}", ui::system_bytes(memory.total))))
        .child(parts_bar("memory-bar", &bar))
        .child(legend)
        .child(caption(l, "Used is App + Wired + Compressed, as in Activity Monitor.").truncate());

    let tier = s.tier;
    let used_share = &view.memory_used[tier];
    let history_h = BODY_H - overview_h;
    let chart = area_chart(l, "memory-chart", rest(history_h, &[TITLE_H, SMALL_H, READOUT_H]), &[Layer { top: used_share, bottom: None, color: app }], s.point);
    let bytes = |share: f32| ui::system_bytes((share as f64 * memory.total as f64) as u64);
    let readout = match s.point.and_then(|at| Some((at, used_share[at]?))) {
        Some((at, p)) => look::readout(l, None, point_time(view, tier, at), percent(p.avg), bytes(p.avg)),
        None => match peak(used_share) {
            Some(at) => {
                let p = used_share[at].map_or(0.0, |p| p.max);
                look::readout(l, Some(app), format!("Peak at {}", point_time(view, tier, at)), percent(p), bytes(p))
            }
            None => no_readings(l),
        },
    };
    let history = next_section(l, "memory-history").h(history_h).child(tiers_title(s, l, "History")).child(chart).child(axis(view, tier, l)).child(readout);

    let swap_h = stack(&[LINE_H + 4.0 + SMALL_H]);
    let swap = next_section(l, "swap").h(swap_h).child(
        col()
            .w_full()
            .gap(4.0)
            .child(
                row()
                    .w_full()
                    .h(LINE_H)
                    .center_y()
                    .child(text("Swap").font(font(13.0, 600.0)).color(l.text))
                    .child(spacer())
                    .child(text(if memory.swap_used == 0 { "Not used".to_owned() } else { format!("{} used", ui::system_bytes(memory.swap_used)) }).font(digits(13.0, 600.0)).color(l.text)),
            )
            .child(caption(l, "Memory moved to disk to make room")),
    );
    let apps = app_list(s, l, "memory-apps", "Memory each app holds", BODY_H - swap_h, |app| app.memory as f64, |bytes| ui::system_bytes(bytes as u64));
    Some([col().w_full().child(overview).child(history), col().w_full().child(apps).child(swap)])
}

/// An app's share of the CPU: a decimal under 10%, where the small ones differ.
fn cpu_share(share: f32) -> String {
    let percent = (share * 100.0).clamp(0.0, 100.0);
    if percent < 10.0 { format!("{percent:.1}%") } else { format!("{percent:.0}%") }
}

/// The apps using the most, as many as fit in a section `height` tall: icon, name, how many
/// processes it runs and its figure. Under them, at the section's bottom, what the rest add up
/// to.
pub(crate) fn app_list(s: &Ui, l: &Look, key: &'static str, note: &str, height: f32, measure: fn(&AppUsage) -> f64, value: fn(f64) -> String) -> El {
    let list = section(key).h(height).child(section_title(l, "Apps", Some(caption(l, note.to_owned()))));
    let Some(all) = s.system.as_ref().and_then(|view| view.apps.clone()) else {
        return list.child(caption(l, "Reading apps…").key(format!("{key}-loading")));
    };
    let room = rest(height, &[TITLE_H]);
    let (top, rest, others) = apps::top(&all, rows_that_fit(room, ROW_H, all.len()), measure);
    let mut rows = col().w_full().h(room);
    for (at, app) in top.iter().enumerate() {
        rows = rows.child(app_row(s, l, format!("{key}-{at}"), app, value(measure(app))));
    }
    if others > 0 {
        let noun = if others == 1 { "app" } else { "apps" };
        rows = rows.child(spacer()).child(more_line(l, format!("{others} more {noun}, {} together", value(rest))).key(format!("{key}-rest")));
    }
    list.child(rows)
}

/// One app: its icon, its name, how many processes it runs, and its value in a column of its
/// own.
fn app_row(s: &Ui, l: &Look, key: String, app: &AppUsage, value: String) -> El {
    let processes = if app.processes == 1 { "1 process".to_owned() } else { format!("{} processes", app.processes) };
    let who = row()
        .grow(1.0)
        .min_w(0.0)
        .gap(8.0)
        .center_y()
        .child(row().max_w(NAME_W).child(text(app.name.clone()).font(font(12.0, 600.0)).color(l.text).truncate()))
        .child(text(processes).font(digits(11.0, 400.0)).color(l.faint).truncate());
    row()
        .key(key)
        .w_full()
        .h(ROW_H)
        .gap(12.0)
        .center_y()
        .help(app.path.as_ref().map_or_else(|| app.name.clone(), |p| p.to_string_lossy().into_owned()))
        .child(app_icon(s, app))
        .child(who)
        .child(look::cell(VALUE_W, text(value).font(digits(12.0, 600.0)).color(l.text)))
}

/// An app's Finder icon; a process outside any app shows the icon every program has. Until it
/// has decoded (the panel asks for the top apps' icons with each reading, so this is rare) its
/// place stays empty rather than flashing a grey tile.
fn app_icon(s: &Ui, app: &AppUsage) -> El {
    let tile = el().size(ICON, ICON);
    // No path is no app: `Ui::icon` gives it the program icon.
    let path = app.path.as_deref().unwrap_or(std::path::Path::new(""));
    match s.icon(path, ICON) {
        Some(image) => tile.pixels(image.pixels()),
        None => tile,
    }
}

pub(crate) fn percent(share: f32) -> String {
    format!("{:.0}%", (share * 100.0).clamp(0.0, 100.0))
}

/// A section's title with the history's three resolutions at its end.
pub(crate) fn tiers_title(s: &Ui, l: &Look, title: &str) -> El {
    let mut tiers = segments(l, "tiers");
    for (at, tier) in TIERS.iter().enumerate() {
        tiers = tiers.child(segment(l, format!("tier-{}", tier.label), tier.label, s.tier == at, move |s: &mut Ui| ui::set_tier(s, at)));
    }
    section_title(l, title, Some(tiers)).key("history")
}

/// A readout for a chart with nothing in it yet.
pub(crate) fn no_readings(l: &Look) -> El {
    look::readout(l, None, "No readings yet".to_owned(), String::new(), String::new())
}

pub(crate) fn point_time(view: &SystemView, tier: usize, at: usize) -> String {
    time::clock(history::bucket_start(tier, view.at, at), tier == 0)
}

/// The point with the highest reading.
pub(crate) fn peak(points: &[Option<Point>]) -> Option<usize> {
    points.iter().enumerate().filter_map(|(at, p)| Some((at, (*p)?.max))).max_by(|a, b| a.1.total_cmp(&b.1)).map(|(at, _)| at)
}

/// The times under a history chart: where it starts, its middle, and now.
pub(crate) fn axis(view: &SystemView, tier: usize, l: &Look) -> El {
    let len = TIERS[tier].len;
    axis_line(l, [point_time(view, tier, 0), point_time(view, tier, len / 2), "now".to_owned()])
}

/// Three times under a chart, at its start, middle and end.
pub(crate) fn axis_line(l: &Look, labels: [String; 3]) -> El {
    let mut axis = row().key("chart-axis").w_full().h(SMALL_H).justify_between();
    for label in labels {
        axis = axis.child(text(label).font(digits(11.0, 400.0)).color(l.faint));
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

/// A chart `h` tall and as wide as a section, that reads the point under the pointer.
pub(crate) fn hover_chart(key: &str, label: &str, shape: Shape, h: f32, len: usize) -> El {
    canvas(shape)
        .key(key.to_owned())
        .size(INNER_W, h)
        .label(label.to_owned())
        .on_pointer_move_local(move |s: &mut Ui, p, _| ui::hover_point(s, p.x / INNER_W, len))
        .on_hover_change(|s: &mut Ui, inside, _| {
            if !inside {
                ui::leave_point(s);
            }
        })
}

/// A ring on a chart's point.
pub(crate) fn ring(shape: Shape, l: &Look, at: [f32; 2], color: Color) -> Shape {
    let [x, y] = at;
    shape.line([x, y], [x + 0.01, y], 9.0, rune::LineCap::Round, color).line([x, y], [x + 0.01, y], 5.0, rune::LineCap::Round, l.solid())
}

/// The hairlines a share chart `h` tall is read against: at 100%, at 50% and the baseline.
pub(crate) fn grid(l: &Look, h: f32) -> Shape {
    let y = |share: f32| share_y(share, h);
    Shape::new()
        .line([0.0, y(1.0)], [INNER_W, y(1.0)], 1.0, rune::LineCap::Butt, l.stroke)
        .line([0.0, y(0.5)], [INNER_W, y(0.5)], 1.0, rune::LineCap::Butt, l.stroke)
        .line([0.0, h - 0.5], [INNER_W, h - 0.5], 1.0, rune::LineCap::Butt, l.track)
}

/// Where a share sits on a chart `h` tall: a hair inside the top and bottom, so a line at 0%
/// or 100% isn't cut in half.
pub(crate) fn share_y(share: f32, h: f32) -> f32 {
    1.0 + (1.0 - share.clamp(0.0, 1.0)) * (h - 2.0)
}

pub(crate) fn area_chart(l: &Look, key: &str, h: f32, layers: &[Layer], point: Option<usize>) -> El {
    let len = layers.first().map_or(0, |layer| layer.top.len());
    let w = INNER_W;
    let x = |at: usize| if len > 1 { at as f32 / (len - 1) as f32 * w } else { 0.0 };
    let y = |share: f32| share_y(share, h);

    let mut shape = grid(l, h);
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
            shape = ring(shape, l, [px, y(p.avg)], layer.color);
        }
    }
    hover_chart(key, "History chart", shape, h, len)
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
                    .clip_rounded(4.0)
                    .fill(l.track)
                    .justify_end()
                    .help(format!("Core {}: {}", first + at + 1, percent(share)))
                    .child(el().w_full().h(height).fill(fill)),
            );
        }
        col().key(key).grow(shares.len().max(1) as f32).min_w(0.0).gap(8.0).child(bars).child(caption(l, label).truncate())
    };
    let efficiency = efficiency.min(cores.len());
    let performance = cores.len() - efficiency;
    let mut groups = row().key("cores").w_full().gap(12.0);
    if efficiency > 0 {
        groups = groups.child(group("cores-efficiency", format!("{efficiency} efficiency"), 0, &cores[..efficiency]));
    }
    groups = groups.child(group("cores-performance", if efficiency > 0 { format!("{performance} performance") } else { format!("{performance} cores") }, efficiency, &cores[efficiency..]));
    next_section(l, "cores-section").child(section_title(l, "Cores", Some(caption(l, "How busy each core is")))).child(groups)
}
