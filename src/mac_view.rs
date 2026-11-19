//! This Mac's other tabs: the disk, the network, the battery and the GPU. Each follows the CPU
//! and Memory tabs' shape in `system_view`: what is in use now and its history on the left,
//! what is behind it on the right.

use rune::{Color, Easing, El, LineCap, LoopSpec, Path, Shape, canvas, col, el, icon, row, spacer, text};

use crate::burn_view::dashed;
use crate::icons;
use crate::look::{self, BODY_H, INNER_W, Look, READOUT_H, ROW_H, TALL_ROW_H, TITLE_H, caption, digits, figure, font, legend_line, legend_value, more_line, next_section, rows_that_fit, section, section_title, segment, segments};
use crate::system::history::{Point, TIERS};
use crate::system::net::{Interface, Kind};
use crate::system::{ChargePoint, SystemView};
use crate::system_view::{BAR_H, HERO_H, Heading, LINE_H, Layer, SMALL_H, app_list, area_chart, axis, axis_line, grid, hero, hover_chart, no_readings, parts_bar, peak, percent, point_time, rest, ring, runs, share_y, stack, tiers_title};
use crate::time;
use crate::ui::{self, Tab, Ui};

/// Another volume's row: its name over a meter of how full it is.
const VOLUME_H: f32 = 44.0;
/// The most other volumes listed; the apps keep the rest of their column.
const VOLUMES_SHOWN: usize = 3;
const EJECT_W: f32 = 68.0;
/// A row of the GPU's last hour.
const STAT_H: f32 = 36.0;
/// The battery drawn as a battery: its body, the room inside it, and the terminal on its end.
const CELL_W: f32 = 132.0;
const CELL_H: f32 = 64.0;
const CELL_PAD: f32 = 5.0;
const NUB_W: f32 = 6.0;
/// The band of light that runs through the charge while it charges.
const SHEEN_W: f32 = 48.0;
/// A line of a figure over its caption.
const FACT_H: f32 = LINE_H + 2.0 + SMALL_H;
/// The GPU's rings: busy outside, then rendering, then geometry.
const RINGS: f32 = 140.0;
const RING_W: f32 = 11.0;

/// Bytes as a disk's maker counts them, in powers of a thousand: "612 GB", "1.4 TB".
pub fn disk_bytes(bytes: u64) -> String {
    let (value, unit) = scaled(bytes as f64, &["bytes", "KB", "MB", "GB", "TB", "PB"]);
    unit_string(value, unit)
}

/// A rate in powers of a thousand: "2.4 MB/s", "380 KB/s".
pub fn rate(bytes_per_second: f64) -> (String, &'static str) {
    let (value, unit) = scaled(bytes_per_second.max(0.0), &["B/s", "KB/s", "MB/s", "GB/s", "TB/s", "PB/s"]);
    (number(value), unit)
}

pub fn rate_string(bytes_per_second: f64) -> String {
    let (value, unit) = rate(bytes_per_second);
    format!("{value} {unit}")
}

fn scaled(mut value: f64, units: &[&'static str]) -> (f64, &'static str) {
    let mut at = 0;
    while value >= 999.5 && at + 1 < units.len() {
        value /= 1000.0;
        at += 1;
    }
    (value, units[at])
}

/// Two significant figures under ten, whole from there: "2.4", "48", "612".
fn number(value: f64) -> String {
    if value < 9.95 && value.fract() >= 0.05 { format!("{value:.1}") } else { format!("{value:.0}") }
}

fn unit_string(value: f64, unit: &str) -> String {
    format!("{} {unit}", number(value))
}

/// The size a drive is sold as, from what it holds: 994 GB is a "1 TB" drive.
fn nominal(bytes: u64) -> String {
    let gb = bytes as f64 / 1e9;
    let sizes = [64.0, 128.0, 256.0, 500.0, 512.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0];
    match sizes.iter().find(|&&size| gb <= size * 1.02 && gb >= size * 0.85) {
        Some(&size) if size >= 1000.0 => format!("{} TB", size / 1000.0),
        Some(&size) => format!("{size} GB"),
        None => disk_bytes(bytes),
    }
}

/// A nice scale for a chart of rates: the peak rounded up to 1, 2 or 5 times a power of ten.
fn scale_for(peak: f64) -> f64 {
    let peak = peak.max(1e6);
    let magnitude = 10f64.powf(peak.log10().floor());
    [1.0, 2.0, 5.0, 10.0].iter().map(|step| step * magnitude).find(|&top| top >= peak).unwrap_or(peak)
}

/// `points` as shares of `scale`, for a chart drawn on a 0 to 1 axis.
fn shares(points: &[Option<Point>], scale: f64) -> Vec<Option<Point>> {
    points.iter().map(|p| p.map(|p| Point { avg: (p.avg as f64 / scale) as f32, max: (p.max as f64 / scale) as f32 })).collect()
}

fn highest(series: &[&[Option<Point>]]) -> f64 {
    series.iter().flat_map(|points| points.iter().flatten()).map(|p| p.max as f64).fold(0.0, f64::max)
}

/// The context bar's words for the disk, the network, the battery and the GPU.
pub(crate) fn heading(l: &Look, view: Option<&SystemView>, tab: Tab) -> Heading {
    match tab {
        Tab::Disk => {
            let boot = view.and_then(|view| view.disk.as_ref()).and_then(|disk| disk.volumes.first());
            Heading {
                badge: boot.map(|v| nominal(v.total)),
                words: boot.map(|v| format!("{} · {}", v.name, v.format)),
                status: boot.map(|v| look::chip_label(l, "available", format!("{} available", disk_bytes(v.available))).help("Free space, and what macOS can clear when it needs room")),
            }
        }
        Tab::Network => {
            let net = view.and_then(|view| view.net.as_ref());
            let primary = net.and_then(|net| net.interfaces.iter().find(|i| i.active && Some(&i.bsd) == net.primary.as_ref()).or_else(|| net.interfaces.iter().find(|i| i.active)));
            let words = match primary {
                Some(interface) => Some(match &interface.address {
                    Some(address) => format!("Using {} · {address}", interface.name),
                    None => format!("Using {}", interface.name),
                }),
                None => net.map(|_| "Not connected".to_owned()),
            };
            let offline = net.is_some() && primary.is_none();
            Heading { badge: None, words, status: offline.then(|| look::pill("offline", l.warning_text, "Offline".to_owned())) }
        }
        Tab::Battery => {
            let words = view.and_then(|view| view.battery.as_ref()).map(|b| {
                match (b.plugged, b.charging, b.full) {
                    (false, ..) => "Running on battery",
                    (true, true, _) => "Charging",
                    (true, false, true) => "Plugged in, fully charged",
                    (true, false, false) => "Plugged in, not charging",
                }
                .to_owned()
            });
            Heading { badge: None, words, status: None }
        }
        _ => {
            let cores = view.and_then(|view| view.gpu.as_ref()).and_then(|gpu| gpu.cores);
            Heading { badge: view.map(|view| view.machine.chip.clone()), words: cores.map(|cores| format!("{cores} GPU cores")), status: None }
        }
    }
}

pub fn disk(s: &Ui, l: &Look, view: &SystemView) -> Option<[El; 2]> {
    let disk = view.disk.as_ref()?;
    let boot = disk.volumes.first();
    let overview_h = stack(&[HERO_H, BAR_H, LINE_H, 2.0 * SMALL_H]);
    let mut overview = section("disk-hero").h(overview_h);
    if let Some(boot) = boot {
        let used = boot.used();
        let used_words = disk_bytes(used);
        let (number, unit) = used_words.split_once(' ').unwrap_or((&used_words, ""));
        let parts = [(l.normal, "Used", used), (l.series_rest, "Purgeable", boot.purgeable()), (l.track, "Free", boot.free)];
        let total = boot.total.max(1) as f64;
        let bar: Vec<_> = parts.iter().map(|&(color, name, bytes)| (color, format!("{name}: {}", disk_bytes(bytes)), bytes as f64 / total)).collect();
        let mut legend = row().h(LINE_H).gap(16.0).center_y();
        for (color, name, bytes) in parts {
            legend = legend.child(legend_value(l, color, name, disk_bytes(bytes)));
        }
        overview = overview
            .child(hero(l, number.to_owned(), format!("{unit} used of {}", disk_bytes(boot.total))))
            .child(parts_bar("disk-bar", &bar))
            .child(legend)
            .child(caption(l, "Available is Free + Purgeable. macOS clears purgeable files, such as caches, when it needs the room.").w_full().h(2.0 * SMALL_H).wrap());
    }

    // Activity: read and write rates, on a scale that fits the busiest moment shown.
    let tier = s.tier;
    let (read, write) = (&view.disk_read[tier], &view.disk_write[tier]);
    let scale = scale_for(highest(&[read, write]));
    let (read_share, write_share) = (shares(read, scale), shares(write, scale));
    let [read_color, _, write_color] = l.series;
    let activity_h = BODY_H - overview_h;
    let chart_h = rest(activity_h, &[TITLE_H, LINE_H, SMALL_H, READOUT_H]);
    let chart = area_chart(l, "disk-chart", chart_h, &[Layer { top: &read_share, bottom: None, color: read_color }, Layer { top: &write_share, bottom: None, color: write_color }], s.point);
    let readout = match s.point.and_then(|at| Some((at, read[at]?, write[at].map_or(0.0, |p| p.avg)))) {
        Some((at, r, w)) => look::readout(l, None, point_time(view, tier, at), format!("write {}", rate_string(w as f64)), format!("read {}", rate_string(r.avg as f64))),
        None => match peak(read) {
            Some(at) => look::readout(l, Some(read_color), format!("Read peak at {}", point_time(view, tier, at)), String::new(), rate_string(read[at].map_or(0.0, |p| p.max) as f64)),
            None => no_readings(l),
        },
    };
    let rates = row().key("disk-rates").h(LINE_H).gap(16.0).center_y().child(legend_value(l, read_color, "Read", rate_string(disk.read))).child(legend_value(l, write_color, "Write", rate_string(disk.write)));
    let activity = next_section(l, "disk-activity").h(activity_h).child(tiers_title(s, l, "Activity")).child(rates).child(chart).child(axis(view, tier, l)).child(readout);

    let others: Vec<_> = disk.volumes.iter().skip(1).take(VOLUMES_SHOWN).collect();
    let volumes_h = if others.is_empty() { 0.0 } else { stack(&[TITLE_H, VOLUME_H * others.len() as f32]) };
    let mut right = col().w_full().child(app_list(s, l, "disk-apps", "Read + written, last hour", BODY_H - volumes_h, |app| app.disk as f64, |bytes| disk_bytes(bytes as u64)));
    if !others.is_empty() {
        let external = others.iter().filter(|v| !v.internal).count();
        let internal = others.len() - external;
        let count = match (internal, external) {
            (0, n) => format!("{n} external"),
            (n, 0) => format!("{n} internal"),
            (i, e) => format!("{i} internal, {e} external"),
        };
        let mut list = col().w_full();
        for (at, volume) in others.into_iter().enumerate() {
            list = list.child(volume_row(l, at, volume));
        }
        right = right.child(next_section(l, "volumes").h(volumes_h).child(section_title(l, "Other volumes", Some(caption(l, count)))).child(list));
    }
    Some([col().w_full().child(overview).child(activity), right])
}

/// Another volume: its name, how much is free, a meter of how full it is, and Eject when it
/// can be ejected.
fn volume_row(l: &Look, at: usize, volume: &crate::system::disk::Volume) -> El {
    let share = volume.used() as f64 / volume.total.max(1) as f64;
    let meter_w = INNER_W - 20.0 - 12.0 - if volume.ejectable { 12.0 + EJECT_W } else { 0.0 };
    let line = row()
        .w_full()
        .h(LINE_H)
        .gap(8.0)
        .center_y()
        .child(text(volume.name.clone()).font(font(12.0, 600.0)).color(l.text).truncate())
        .child(text(format!("{} free of {}", disk_bytes(volume.available), disk_bytes(volume.total))).font(digits(11.0, 400.0)).color(l.faint));
    let mut item = row()
        .key(format!("volume-{at}"))
        .w_full()
        .h(VOLUME_H)
        .gap(12.0)
        .center_y()
        .help(volume.mount.to_string_lossy().into_owned())
        .child(row().w(20.0).center().child(icon(&icons::HARD_DRIVE_DOWNLOAD, 16.0).color(l.dim)))
        .child(col().grow(1.0).min_w(0.0).gap(8.0).child(line).child(look::meter(l, "volume-meter", meter_w, 4.0, share, l.normal)));
    if volume.ejectable {
        let mount = volume.mount.clone();
        item = item.child(
            row()
                .key(format!("eject-{at}"))
                .role("button")
                .label(format!("Eject {}", volume.name))
                .w(EJECT_W)
                .h(24.0)
                .gap(6.0)
                .radius(6.0)
                .continuous_corners()
                .center()
                .fill(l.chip)
                .hover(l.chip_on)
                .press_scale(0.96)
                .on_press(move |s: &mut Ui| ui::eject(s, &mount))
                .child(icon(&icons::ARROW_UP_FROM_LINE, 12.0).color(l.text))
                .child(text("Eject").font(font(11.0, 600.0)).color(l.text)),
        );
    }
    item
}

pub fn network(s: &Ui, l: &Look, view: &SystemView) -> Option<[El; 2]> {
    let net = view.net.as_ref()?;
    let [down_color, up_color, _] = l.series;
    let half = INNER_W / 2.0;
    let big = |glyph: &'static rune::Icon, color: Color, name: &str, bytes_per_second: f64| {
        let (value, unit) = rate(bytes_per_second);
        col()
            .w(half)
            .gap(4.0)
            .child(row().h(LINE_H).gap(6.0).center_y().child(icon(glyph, 12.0).color(color)).child(text(name.to_owned()).font(font(13.0, 400.0)).color(l.dim)))
            .child(hero(l, value, unit.to_owned()))
    };
    let overview_h = stack(&[LINE_H + 4.0 + HERO_H]);
    let overview = section("network-hero").h(overview_h).child(row().w_full().child(big(&icons::ARROW_DOWN, down_color, "Download", net.down)).child(big(&icons::ARROW_UP, up_color, "Upload", net.up)));

    let tier = s.tier;
    let (down, up) = (&view.net_down[tier], &view.net_up[tier]);
    let scale = scale_for(highest(&[down, up]));
    let readout = match s.point.and_then(|at| Some((at, down[at]?, up[at].map_or(0.0, |p| p.avg)))) {
        Some((at, d, u)) => look::readout(l, None, point_time(view, tier, at), format!("{} up", rate_string(u as f64)), format!("{} down", rate_string(d.avg as f64))),
        None => match peak(down) {
            Some(at) => look::readout(l, Some(down_color), format!("Download peak at {}", point_time(view, tier, at)), String::new(), rate_string(down[at].map_or(0.0, |p| p.max) as f64)),
            None => no_readings(l),
        },
    };
    let history_h = BODY_H - overview_h;
    let chart_h = rest(history_h, &[TITLE_H, SMALL_H, SMALL_H, READOUT_H]);
    let keys = row().h(SMALL_H).gap(16.0).center_y().child(legend_line(l, down_color, None, "Download, above the line")).child(legend_line(l, up_color, None, "Upload, below"));
    let history = next_section(l, "network-history")
        .h(history_h)
        .child(tiers_title(s, l, "History"))
        .child(keys)
        .child(mirrored(l, view, tier, scale, s.point, chart_h))
        .child(axis(view, tier, l))
        .child(readout);

    let figure_h = SMALL_H + 4.0 + 26.0;
    let today_h = stack(&[TITLE_H, figure_h]);
    let total = |name: &str, bytes: u64| col().w(half).gap(4.0).child(caption(l, name.to_owned())).child(text(disk_bytes(bytes)).font(figure(22.0)).color(l.text));
    let today = next_section(l, "network-today")
        .h(today_h)
        .child(section_title(l, "Today", Some(caption(l, format!("Since {}", time::clock(net.since, false))))))
        .child(row().w_full().h(figure_h).child(total("Received", net.received_today)).child(total("Sent", net.sent_today)));

    let interfaces_h = BODY_H - today_h;
    let room = rest(interfaces_h, &[TITLE_H]);
    let shown = rows_that_fit(room, TALL_ROW_H, net.interfaces.len());
    let mut rows = col().w_full().h(room);
    for (at, interface) in net.interfaces.iter().take(shown).enumerate() {
        rows = rows.child(interface_row(l, at, interface));
    }
    let hidden = net.interfaces.len() - shown;
    if net.interfaces.is_empty() {
        rows = rows.child(row().h(ROW_H).center_y().child(caption(l, "No network interfaces.")));
    } else if hidden > 0 {
        rows = rows.child(spacer()).child(more_line(l, format!("{hidden} more {}", if hidden == 1 { "interface" } else { "interfaces" })));
    }
    let interfaces = section("interfaces").h(interfaces_h).child(section_title(l, "Interfaces", Some(caption(l, "Traffic since the Mac started")))).child(rows);
    Some([col().w_full().child(overview).child(history), col().w_full().child(interfaces).child(today)])
}

/// Download over a middle line and upload under it, each on the same scale.
fn mirrored(l: &Look, view: &SystemView, tier: usize, scale: f64, point: Option<usize>, h: f32) -> El {
    let (down, up) = (&view.net_down[tier], &view.net_up[tier]);
    let len = down.len();
    let w = INNER_W;
    let mid = h / 2.0;
    let x = |at: usize| if len > 1 { at as f32 / (len - 1) as f32 * w } else { 0.0 };
    let reach = |value: f32| ((value as f64 / scale) as f32).clamp(0.0, 1.0) * (mid - 2.0);
    let [down_color, up_color, _] = l.series;
    let mut shape = Shape::new()
        .line([0.0, 0.5], [w, 0.5], 1.0, LineCap::Butt, l.stroke)
        .line([0.0, h - 0.5], [w, h - 0.5], 1.0, LineCap::Butt, l.stroke)
        .line([0.0, mid], [w, mid], 1.0, LineCap::Butt, l.track);
    for (points, color, sign) in [(down, down_color, -1.0f32), (up, up_color, 1.0)] {
        for run in runs(points) {
            let line: Vec<[f32; 2]> = run.clone().map(|at| [x(at), mid + sign * reach(points[at].map_or(0.0, |p| p.avg))]).collect();
            if line.len() < 2 {
                continue;
            }
            let mut area = line.clone();
            area.extend([[line[line.len() - 1][0], mid], [line[0][0], mid]]);
            shape = shape.fill(area, color.alpha(0.22)).stroke(line, 1.75, color);
        }
    }
    let marked = point.filter(|&at| at < len).or_else(|| peak(down));
    if let Some(at) = marked
        && let Some(p) = down[at]
    {
        let at_x = x(at);
        if point.is_some() {
            shape = shape.line([at_x, 0.0], [at_x, h], 1.0, LineCap::Butt, l.dim);
        }
        shape = ring(shape, l, [at_x, mid - reach(p.avg)], down_color);
    }
    hover_chart("network-chart", "Network history chart", shape, h, len)
}

/// A network port: whether it is up, its name and address, and its traffic in words.
fn interface_row(l: &Look, at: usize, interface: &Interface) -> El {
    let detail = match &interface.address {
        Some(address) => format!("{} · {address}", interface.bsd),
        None => interface.bsd.clone(),
    };
    let status = match (interface.active, interface.kind) {
        (true, _) => "Connected",
        (false, Kind::Ethernet) => "Cable unplugged",
        (false, _) => "Not connected",
    };
    let mut end = col().items_end().gap(4.0);
    if interface.active {
        end = end
            .child(text(status).font(font(12.0, 600.0)).color(l.good_text))
            .child(text(format!("{} in · {} out", disk_bytes(interface.received), disk_bytes(interface.sent))).font(digits(12.0, 400.0)).color(l.dim));
    } else {
        end = end.child(text(status).font(font(12.0, 400.0)).color(l.faint));
    }
    row()
        .key(format!("interface-{at}"))
        .w_full()
        .h(TALL_ROW_H)
        .gap(8.0)
        .center_y()
        .child(row().w(12.0).center().child(el().size(8.0, 8.0).radius(4.0).fill(if interface.active { l.good } else { l.track })))
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(4.0)
                .child(text(interface.name.clone()).font(font(12.0, 600.0)).color(l.text).truncate())
                .child(text(detail).font(digits(11.0, 400.0)).color(l.faint).truncate()),
        )
        .child(end)
}

pub fn battery(s: &Ui, l: &Look, view: &SystemView) -> Option<[El; 2]> {
    let battery = view.battery.as_ref()?;
    let left = battery.charge.clamp(0.0, 1.0);
    let low = !battery.plugged && left <= 0.2;
    let fill = if !battery.plugged && left <= 0.1 {
        l.critical
    } else if low {
        l.warning
    } else {
        l.good
    };
    let state = match (battery.charging, battery.plugged, battery.full || left >= 0.995) {
        (true, ..) => "Charging",
        (false, true, true) => "Fully charged",
        (false, true, false) => "On power adapter, not charging",
        (false, false, _) => "On battery",
    };
    let words = col()
        .grow(1.0)
        .min_w(0.0)
        .gap(2.0)
        .child(hero(l, percent(left), "charged".to_owned()))
        .child(text(state).key("battery-state").font(font(12.0, 500.0)).color(if battery.charging { l.good_text } else { l.dim }).truncate().min_w(0.0));
    let top = row().w_full().h(CELL_H).gap(20.0).center_y().child(battery_cell(l, left, fill, battery.charging, battery.plugged, low)).child(words);
    let fact = |key: &'static str, value: String, meaning: &str| {
        col().key(key).grow(1.0).min_w(0.0).h(FACT_H).gap(2.0).child(text(value).font(digits(13.0, 600.0)).color(l.text)).child(caption(l, meaning.to_owned()).truncate().min_w(0.0))
    };
    let mut facts = row().key("battery-power").w_full().h(FACT_H).gap(16.0);
    if let Some(minutes) = battery.minutes_left {
        facts = facts.child(fact("battery-time", time::span(minutes as i64 * 60), if battery.charging { "until full" } else { "left at this draw" }));
    }
    facts = match (battery.watts, battery.plugged) {
        (Some(watts), false) => facts.child(fact("battery-draw", format!("{watts:.1} W"), "drawn from the battery")),
        (_, true) => facts.child(fact("battery-source", "Adapter".to_owned(), "powers this Mac")),
        (None, false) => facts,
    };
    let overview_h = stack(&[CELL_H, FACT_H]);
    let overview = section("battery-hero").h(overview_h).child(top).child(facts);

    let stamp = |at: i64| if view.at as i64 - at > 20 * 3600 { time::weekday_clock(at) } else { time::clock(at, false) };
    let mut facts: Vec<(&str, String, String, Color)> = Vec::new();
    let capacity = battery.health.map(|health| {
        let worn = (1.0 - health).max(0.0);
        col()
            .key("battery-capacity")
            .w_full()
            .h(look::TALL_ROW_H)
            .gap(6.0)
            .justify_center()
            .child(row().w_full().h(LINE_H).center_y().child(text("Maximum capacity").font(font(12.0, 600.0)).color(l.text)).child(spacer()).child(text(percent(health)).font(digits(13.0, 600.0)).color(l.text)))
            .child(look::meter(l, "capacity", INNER_W, 6.0, health as f64, l.good))
            .child(caption(l, format!("Of what it held new · {} worn", percent(worn))))
    });
    if let Some(cycles) = battery.cycle_count {
        facts.push(("Cycle count", "Full drain-and-charge equivalents".to_owned(), cycles.to_string(), l.text));
    }
    if let Some(condition) = &battery.condition {
        let good = condition.eq_ignore_ascii_case("normal") || condition.eq_ignore_ascii_case("good");
        facts.push(("Condition", "As macOS reports it".to_owned(), condition.clone(), if good { l.good_text } else { l.warning_text }));
    }
    if let Some(celsius) = battery.temperature {
        facts.push(("Temperature", "Of the battery cells".to_owned(), format!("{celsius:.0} °C"), l.text));
    }
    let charged = last_charge(&view.charge);
    if let Some(charge) = &charged {
        let words = if charge.reached_at > charge.from_at {
            format!("From {} to {} in {}", percent(charge.from), percent(charge.to), time::span(charge.reached_at - charge.from_at))
        } else {
            format!("To {}", percent(charge.to))
        };
        facts.push(("Last charged", words, stamp(charge.reached_at), l.text));
    }
    if let Some((at, level)) = charged.and_then(|charge| charge.unplugged).filter(|_| !battery.plugged) {
        facts.push(("On battery since", format!("Unplugged at {}", percent(level)), stamp(at), l.text));
    }
    let mut health = next_section(l, "battery-health").child(section_title(l, "Health and charging", None));
    let mut rows = col().key("health").w_full();
    if let Some(capacity) = capacity {
        rows = rows.child(capacity);
    }
    // Each row but the column's first has a hairline over it; the capacity gauge comes first.
    let after_capacity = battery.health.is_some();
    for (at, (name, meaning, value, color)) in facts.into_iter().enumerate() {
        let mut item = row()
            .w_full()
            .h(look::TALL_ROW_H)
            .gap(12.0)
            .center_y()
            .child(col().grow(1.0).min_w(0.0).gap(4.0).child(text(name).font(font(12.0, 600.0)).color(l.text).truncate()).child(caption(l, meaning).truncate()))
            .child(text(value).font(digits(13.0, 600.0)).color(color));
        if at > 0 || after_capacity {
            item = item.border_sides([1.0, 0.0, 0.0, 0.0], l.stroke);
        }
        rows = rows.child(item);
    }
    health = health.child(rows);

    // The charge over the last day or week, with the stretches it was plugged in shaded.
    let span = if s.battery_week { 7 * 86_400 } else { 86_400 };
    let mut ranges = segments(l, "charge-ranges");
    for (week, label) in [(false, "24h"), (true, "7d")] {
        ranges = ranges.child(segment(l, format!("charge-{label}"), label, s.battery_week == week, move |s: &mut Ui| ui::set_battery_week(s, week)));
    }
    let to = view.at as i64;
    let from = to - span;
    let points: Vec<&ChargePoint> = view.charge.iter().filter(|p| p.at >= from).collect();
    // When it last reached 100%: the start of the latest stretch at full charge.
    let full = points.iter().rposition(|p| p.charge >= 0.995).map(|end| points[..end].iter().rposition(|p| p.charge < 0.995).map_or(points[0], |at| points[at + 1]));
    let low = points.iter().min_by(|a, b| a.charge.total_cmp(&b.charge)).copied();
    let marked = full.or(low);
    let readout = match (full, low) {
        (Some(p), _) => look::readout(l, Some(l.good), format!("Reached 100% at {}", stamp(p.at)), String::new(), "100%".to_owned()),
        (None, Some(p)) => look::readout(l, Some(l.good), format!("Lowest at {}", stamp(p.at)), String::new(), percent(p.charge)),
        (None, None) => no_readings(l),
    };
    let label = |at: i64| if s.battery_week { time::weekday_clock(at).split(' ').next().unwrap_or("").to_owned() } else { time::clock(at, false) };
    let keys = row()
        .h(SMALL_H)
        .gap(16.0)
        .center_y()
        .child(legend_line(l, l.good, None, "Charge"))
        .child(row().gap(6.0).center_y().child(look::key_swatch(plugged_wash(l), 10.0)).child(text("Plugged in").font(font(11.0, 400.0)).color(l.dim)));
    let chart_h = rest(BODY_H, &[TITLE_H, SMALL_H, SMALL_H, READOUT_H]);
    let charge = section("charge")
        .h(BODY_H)
        .child(section_title(l, "Charge", Some(ranges)))
        .child(keys)
        .child(charge_chart(l, &points, marked, from, to, chart_h))
        .child(axis_line(l, [label(from), label(from + span / 2), "now".to_owned()]))
        .child(readout);
    Some([col().w_full().child(overview).child(health), charge])
}

/// The last stretch on the charger in the battery's history.
struct Charge {
    /// When it was plugged in, at what charge.
    from_at: i64,
    from: f32,
    /// When it reached its highest charge on the charger, and that charge.
    reached_at: i64,
    to: f32,
    /// When it was unplugged after, at what charge; none while it still is plugged in.
    unplugged: Option<(i64, f32)>,
}

fn last_charge(points: &[ChargePoint]) -> Option<Charge> {
    let end = points.iter().rposition(|p| p.plugged)?;
    let start = points[..end].iter().rposition(|p| !p.plugged).map_or(0, |at| at + 1);
    let mut top = &points[start];
    for point in &points[start..=end] {
        if point.charge > top.charge {
            top = point;
        }
    }
    Some(Charge {
        from_at: points[start].at,
        from: points[start].charge,
        reached_at: top.at,
        to: top.charge,
        unplugged: points.get(end + 1).map(|p| (p.at, p.charge)),
    })
}

/// The wash behind the stretches the Mac was plugged in.
fn plugged_wash(l: &Look) -> Color {
    l.good.alpha(0.16)
}

/// The charge as steps, each reading held until the next, over washes where it was plugged in.
fn charge_chart(l: &Look, points: &[&ChargePoint], marked: Option<&ChargePoint>, from: i64, to: i64, h: f32) -> El {
    let w = INNER_W;
    let span = (to - from).max(1) as f32;
    let x = |at: i64| ((at - from) as f32 / span * w).clamp(0.0, w);
    let y = |charge: f32| share_y(charge, h);
    let mut shape = grid(l, h);
    // Plugged-in stretches as a wash behind the line, from the first reading on the charger to
    // the first one off it.
    let mut start: Option<i64> = None;
    for (at, point) in points.iter().enumerate() {
        if point.plugged && start.is_none() {
            start = Some(point.at);
        }
        let last = at + 1 == points.len();
        if let Some(begun) = start
            && (!point.plugged || last)
        {
            let (a, b) = (x(begun), x(point.at));
            shape = shape.fill(vec![[a, 0.0], [b, 0.0], [b, h], [a, h]], plugged_wash(l));
            start = None;
        }
    }
    // The line, broken where readings stop for longer than a few of their steps.
    let mut line: Vec<[f32; 2]> = Vec::new();
    for pair in points.windows(2) {
        if pair[1].at - pair[0].at > 3 * 3600 {
            shape = flush(shape, std::mem::take(&mut line), l.good);
            continue;
        }
        if line.is_empty() {
            line.push([x(pair[0].at), y(pair[0].charge)]);
        }
        line.push([x(pair[1].at), y(pair[0].charge)]);
        line.push([x(pair[1].at), y(pair[1].charge)]);
    }
    shape = flush(shape, line, l.good);
    if points.is_empty() {
        shape = dashed(shape, [0.0, h / 2.0], [w, h / 2.0], 3.0, 3.0, 1.0, LineCap::Butt, l.faint);
    }
    if let Some(point) = marked {
        shape = ring(shape, l, [x(point.at), y(point.charge)], l.good);
    }
    canvas(shape).key("charge-chart").size(w, h).label("Charge history")
}

fn flush(shape: Shape, line: Vec<[f32; 2]>, color: Color) -> Shape {
    if line.len() < 2 { shape } else { shape.stroke(line, 2.0, color) }
}

pub fn gpu(s: &Ui, l: &Look, view: &SystemView) -> Option<[El; 2]> {
    let gpu = view.gpu.as_ref()?;
    let [busy_color, rendering_color, geometry_color] = l.series;
    let stage = |key: &'static str, color: Color, name: &str, meaning: &str, share: f32| {
        row()
            .key(key)
            .h(LINE_H)
            .gap(8.0)
            .center_y()
            .help(meaning.to_owned())
            .child(look::key_swatch(color, 8.0))
            .child(text(name.to_owned()).font(font(12.0, 400.0)).color(l.dim))
            .child(spacer())
            .child(text(percent(share)).font(digits(12.0, 600.0)).color(l.text))
    };
    let words = col()
        .grow(1.0)
        .min_w(0.0)
        .gap(10.0)
        .child(hero(l, percent(gpu.busy), "busy".to_owned()))
        .child(stage("gpu-rendering", rendering_color, "Rendering", "Drawing pixels: windows, video, games", gpu.rendering))
        .child(stage("gpu-geometry", geometry_color, "Geometry", "Shaping 3D scenes before they are drawn", gpu.geometry));
    let overview = section("gpu-hero")
        .h(stack(&[RINGS]))
        .child(row().w_full().h(RINGS).gap(20.0).center_y().child(gpu_rings(l, [gpu.busy, gpu.rendering, gpu.geometry], [busy_color, rendering_color, geometry_color])).child(words));

    let memory_total = view.machine.memory.max(1);
    let memory = next_section(l, "gpu-memory")
        .h(stack(&[TITLE_H, HERO_H - 14.0, 8.0]))
        .child(section_title(l, "Memory", Some(caption(l, "Unified, shared with the CPU"))))
        .child(
            row()
                .h(HERO_H - 14.0)
                .items_end()
                .gap(6.0)
                .child(text(ui::system_bytes(gpu.memory)).font(figure(22.0)).color(l.text))
                .child(text(format!("of {}", ui::system_bytes(memory_total))).font(font(13.0, 400.0)).color(l.dim).pb(3.0)),
        )
        .child(look::meter(l, "gpu-memory-meter", INNER_W, 8.0, gpu.memory as f64 / memory_total as f64, busy_color));

    // The last hour, from the history's one-hour resolution.
    let hour = &view.gpu_busy[1];
    let readings: Vec<(usize, Point)> = hour.iter().enumerate().filter_map(|(at, p)| Some((at, (*p)?))).collect();
    let mut last_hour = next_section(l, "gpu-hour").child(section_title(l, "Last hour", None));
    if readings.is_empty() {
        last_hour = last_hour.child(caption(l, "No readings yet"));
    } else {
        let average = readings.iter().map(|(_, p)| p.avg).sum::<f32>() / readings.len() as f32;
        let (peak_at, top) = readings.iter().map(|&(at, p)| (at, p.max)).fold((0, f32::MIN), |best, next| if next.1 > best.1 { next } else { best });
        let minutes = |seconds: i64| if seconds < 3600 { format!("{} min", (seconds + 30) / 60) } else { time::span(seconds) };
        let above = minutes(readings.iter().filter(|(_, p)| p.avg > 0.5).count() as i64 * TIERS[1].step);
        let idle = minutes(readings.iter().filter(|(_, p)| p.max < 0.05).count() as i64 * TIERS[1].step);
        let stats = [("Average", percent(average)), ("Peak", format!("{} at {}", percent(top), point_time(view, 1, peak_at))), ("Above 50%", above), ("Idle, under 5%", idle)];
        let mut rows = col().key("gpu-stats").w_full();
        for (at, (name, value)) in stats.into_iter().enumerate() {
            let mut item = row().w_full().h(STAT_H).center_y().child(text(name).font(font(12.0, 400.0)).color(l.dim)).child(spacer()).child(text(value).font(digits(12.0, 600.0)).color(l.text));
            if at > 0 {
                item = item.border_sides([1.0, 0.0, 0.0, 0.0], l.stroke);
            }
            rows = rows.child(item);
        }
        last_hour = last_hour.child(rows);
    }

    let tier = s.tier;
    let busy = &view.gpu_busy[tier];
    let color = l.series[0];
    let chart = area_chart(l, "gpu-chart", rest(BODY_H, &[TITLE_H, SMALL_H, READOUT_H]), &[Layer { top: busy, bottom: None, color }], s.point);
    let readout = match s.point.and_then(|at| Some((at, busy[at]?))) {
        Some((at, p)) => look::readout(l, None, point_time(view, tier, at), String::new(), percent(p.avg)),
        None => match peak(busy) {
            Some(at) => look::readout(l, Some(color), format!("Peak at {}", point_time(view, tier, at)), String::new(), percent(busy[at].map_or(0.0, |p| p.max))),
            None => no_readings(l),
        },
    };
    let history = section("gpu-history").h(BODY_H).child(tiers_title(s, l, "History")).child(chart).child(axis(view, tier, l)).child(readout);
    Some([col().w_full().child(overview).child(memory).child(last_hour), history])
}

/// The GPU as three rings filled clockwise from the top: busy outside, rendering and geometry
/// inside it. A light runs round the outer ring, faster the busier the GPU is, so the load
/// shows at a glance; it stops when the GPU is idle.
fn gpu_rings(l: &Look, shares: [f32; 3], colors: [Color; 3]) -> El {
    let c = RINGS / 2.0;
    let radius = |at: usize| c - RING_W / 2.0 - 1.0 - at as f32 * (RING_W + 3.0);
    let mut shape = Shape::new();
    for (at, (share, color)) in shares.into_iter().zip(colors).enumerate() {
        let r = radius(at);
        shape = shape.stroke_path(&arc(c, r, 0.0, 1.0), RING_W, LineCap::Butt, color.alpha(0.18));
        let share = share.clamp(0.0, 1.0);
        if share > 0.0 {
            shape = shape.stroke_path(&arc(c, r, 0.0, share.max(0.01)), RING_W, LineCap::Round, color);
        }
    }
    let mut rings = el().key("gpu-rings").size(RINGS, RINGS).child(canvas(shape).size(RINGS, RINGS).label(format!("GPU {} busy", percent(shares[0]))));
    if shares[0] >= 0.02 {
        let glint = Shape::new().stroke_path(&arc(c, radius(0), 0.0, 0.07), RING_W - 3.0, LineCap::Round, Color::WHITE.alpha(if l.dark { 0.55 } else { 0.7 }));
        let period = (1.4 / shares[0] as f64).clamp(1.2, 14.0);
        rings = rings.child(canvas(glint).key("gpu-glint").absolute(0.0, 0.0).size(RINGS, RINGS).looping(LoopSpec::rotation(period)));
    }
    rings
}

/// A circle's arc of radius `r` round (`c`, `c`), clockwise from the top, from turn `from` to `to`.
fn arc(c: f32, r: f32, from: f32, to: f32) -> Path {
    let at = |turn: f32| {
        let angle = turn * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
        [c + r * angle.cos(), c + r * angle.sin()]
    };
    let steps = ((to - from) * 120.0).ceil().max(1.0) as usize;
    let mut path = Path::new().move_to(at(from));
    for step in 1..=steps {
        path = path.line_to(at(from + (to - from) * step as f32 / steps as f32));
    }
    path
}

/// The battery drawn as a battery: a body filled to `charge` in the state's colour, its terminal
/// on the end. While it charges a band of light runs through the charge and a bolt sits on it;
/// on the adapter, the bolt alone; running low on battery, the charge pulses slowly.
fn battery_cell(l: &Look, charge: f32, fill: Color, charging: bool, plugged: bool, low: bool) -> El {
    let (inner_w, inner_h) = (CELL_W - 2.0 * CELL_PAD, CELL_H - 2.0 * CELL_PAD);
    let level_w = if charge > 0.0 { (inner_w * charge).max(10.0) } else { 0.0 };
    let mut level = el().key("battery-level").w(level_w).h(inner_h).radius(10.0).continuous_corners().fill(fill).clip_rounded(10.0).animate_layout();
    if charging && level_w > SHEEN_W / 2.0 {
        let sheen = el()
            .key("battery-sheen")
            .absolute(0.0, 0.0)
            .size(SHEEN_W, inner_h)
            .linear_gradient(std::f32::consts::FRAC_PI_2, [(0.0, Color::WHITE.alpha(0.0)), (0.5, Color::WHITE.alpha(0.38)), (1.0, Color::WHITE.alpha(0.0))])
            .looping(LoopSpec::slide(2.0, [-1.0, 0.0], [level_w / SHEEN_W, 0.0]).easing(Easing::LINEAR));
        level = level.child(sheen);
    }
    if low {
        level = level.looping(LoopSpec::opacity(1.6, 0.0, 0.5, 1.0));
    }
    let mut body = el()
        .key("battery-cell")
        .size(CELL_W, CELL_H)
        .padding(CELL_PAD)
        .radius(16.0)
        .continuous_corners()
        .border(2.5, l.dim)
        .label(format!("Battery {} charged{}", percent(charge), if charging { ", charging" } else { "" }))
        .child(level);
    if plugged {
        // The bolt is white on the charge, and the text colour where the body is still empty.
        let bolt_color = if charge >= 0.58 { Color::WHITE } else { l.text };
        body = body.child(row().absolute(0.0, 0.0).size(CELL_W, CELL_H).center().child(icon(&icons::ZAP, 26.0).color(bolt_color)));
    }
    row().gap(3.0).center_y().child(body).child(el().size(NUB_W, 22.0).corners([0.0, 4.0, 4.0, 0.0]).fill(l.dim))
}
