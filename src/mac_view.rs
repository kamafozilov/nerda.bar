//! This Mac's other tabs: the disk, the network, the battery and the GPU. Each follows the CPU
//! and RAM tabs' shape in `system_view`: what is in use now, its history, then what is behind it.

use rune::{Color, El, LineCap, Shape, canvas, col, el, icon, row, rule, spacer, text};

use crate::burn_view::dashed;
use crate::icons;
use crate::look::{self, INNER_W, Look, caption, digits, figure, font, hairline, key_swatch, section, section_title, segment, segments};
use crate::system::history::{Point, TIERS};
use crate::system::net::{Interface, Kind};
use crate::system::{ChargePoint, SystemView};
use crate::system_view::{Layer, app_list, area_chart, axis, big_number, history_title, now_block, peak, percent, point_time, reading, runs};
use crate::time;
use crate::ui::{self, Ui};

const NET_CHART_H: f32 = 80.0;
const CHARGE_CHART_H: f32 = 64.0;

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

pub fn disk(s: &Ui, l: &Look) -> El {
    let Some((view, disk)) = s.system.as_ref().and_then(|view| Some((view, view.disk.as_ref()?))) else {
        return reading(l, "disk-loading");
    };
    let mut body = col().w_full();
    if let Some(boot) = disk.volumes.first() {
        let used = boot.used();
        let (number, unit) = disk_bytes(used).split_once(' ').map(|(n, u)| (n.to_owned(), u.to_owned())).unwrap_or_default();
        let hero = row()
            .w_full()
            .items_end()
            .child(big_number(l, &number, &format!("{unit} used of {}", disk_bytes(boot.total))))
            .child(spacer())
            .child(
                row()
                    .key("available")
                    .h(19.0)
                    .px(8.0)
                    .radius(10.0)
                    .center_y()
                    .fill(l.chip)
                    .help("Free space, and what macOS can clear when it needs room")
                    .child(text(format!("{} available", disk_bytes(boot.available))).font(digits(11.0, 600.0)).color(l.text))
                    .mb(8.0),
            );
        let parts = [(l.normal, "Used", used), (l.faint, "Purgeable", boot.purgeable()), (l.track, "Free", boot.free)];
        let total = boot.total.max(1) as f32;
        let mut bar = row().key("disk-bar").w_full().h(8.0).gap(2.0);
        let shown: Vec<_> = parts.iter().filter(|part| part.2 as f32 / total * INNER_W >= 3.0).collect();
        for (at, (color, name, bytes)) in shown.iter().enumerate() {
            let piece = if at + 1 == shown.len() { el().grow(1.0).min_w(0.0) } else { el().w((*bytes as f32 / total * INNER_W).floor()) };
            bar = bar.child(piece.h(8.0).radius(2.0).fill(*color).help(format!("{name}: {}", disk_bytes(*bytes))));
        }
        let mut legend = row().w_full().gap(14.0);
        for (color, name, bytes) in parts {
            legend = legend.child(
                col()
                    .gap(2.0)
                    .child(row().gap(4.0).center_y().child(key_swatch(color, 7.0)).child(text(name).font(font(10.0, 400.0)).color(l.dim)))
                    .child(text(disk_bytes(bytes)).font(digits(12.0, 600.0)).color(l.text)),
            );
        }
        body = body.child(
            now_block("disk-hero")
                .child(hero)
                .child(bar)
                .child(legend)
                .child(text("Available is Free + Purgeable. macOS clears purgeable files, such as caches, when it needs the room.").font(font(10.5, 400.0).lh(14.0)).color(l.faint).w_full().wrap()),
        );
    }

    // Activity: read and write rates, on a scale that fits the busiest moment shown.
    let tier = s.tier;
    let (read, write) = (&view.disk_read[tier], &view.disk_write[tier]);
    let scale = scale_for(highest(&[read, write]));
    let (read_share, write_share) = (shares(read, scale), shares(write, scale));
    let [read_color, _, write_color] = l.series;
    let chart = area_chart(l, "disk-chart", &[Layer { top: &read_share, bottom: None, color: read_color }, Layer { top: &write_share, bottom: None, color: write_color }], s.point, [&rate_string(scale), &rate_string(scale / 2.0)]);
    let readout = match s.point.and_then(|at| Some((at, read[at]?, write[at].map_or(0.0, |p| p.avg)))) {
        Some((at, r, w)) => look::readout(l, None, point_time(view, tier, at), format!("write {}", rate_string(w as f64)), format!("read {}", rate_string(r.avg as f64))),
        None => match peak(read) {
            Some(at) => look::readout(l, Some(read_color), format!("Read peak at {}", point_time(view, tier, at)), String::new(), rate_string(read[at].map_or(0.0, |p| p.max) as f64)),
            None => look::readout(l, None, "No readings yet".to_owned(), String::new(), String::new()),
        },
    };
    let rates = row()
        .key("disk-rates")
        .w_full()
        .gap(16.0)
        .child(rate_key(l, read_color, "Read", disk.read))
        .child(rate_key(l, write_color, "Write", disk.write));
    let activity = section("disk-activity").gap(8.0).child(tiers_title(s, l, "Activity")).child(rates).child(chart).child(axis(view, tier, l)).child(readout);
    body = body.child(hairline(l)).child(activity);

    if view.apps.is_some() {
        body = body.child(hairline(l)).child(app_list(s, l, "disk-apps", "read + written, last hour", |app| app.disk as f64, |bytes| disk_bytes(bytes as u64)));
    }

    let others: Vec<_> = disk.volumes.iter().skip(1).collect();
    if !others.is_empty() {
        let mut list = section("volumes").gap(8.0).child(section_title(l, "Other volumes", None));
        for (at, volume) in others.into_iter().enumerate() {
            let mount = volume.mount.clone();
            let line = row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(text(volume.name.clone()).font(font(12.5, 500.0)).color(l.text).truncate().min_w(0.0))
                .child(text(if volume.internal { "internal" } else { "external" }).font(font(11.0, 400.0)).color(l.faint));
            let share = volume.used() as f64 / volume.total.max(1) as f64;
            let meter_w = INNER_W - 28.0 - if volume.ejectable { 72.0 } else { 0.0 } - 110.0;
            let usage = row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(look::meter(l, "volume-meter", meter_w, 4.0, share, l.normal))
                .child(spacer())
                .child(text(format!("{} free of {}", disk_bytes(volume.available), disk_bytes(volume.total))).font(digits(11.0, 400.0)).color(l.dim));
            let mut item = row()
                .key(format!("volume-{at}"))
                .w_full()
                .gap(10.0)
                .center_y()
                .child(icon(&icons::HARD_DRIVE_DOWNLOAD, 18.0).color(l.dim))
                .child(col().grow(1.0).min_w(0.0).gap(4.0).child(line).child(usage));
            if volume.ejectable {
                item = item.child(
                    row()
                        .key(format!("eject-{at}"))
                        .role("button")
                        .label(format!("Eject {}", volume.name))
                        .h(28.0)
                        .px(9.0)
                        .gap(5.0)
                        .radius(8.0)
                        .continuous_corners()
                        .fill(l.chip)
                        .hover(l.hover)
                        .center()
                        .on_press(move |s: &mut Ui| ui::eject(s, &mount))
                        .child(icon(&icons::ARROW_UP_FROM_LINE, 12.0).color(l.text))
                        .child(text("Eject").font(font(11.5, 600.0)).color(l.text)),
                );
            }
            list = list.child(item);
        }
        body = body.child(hairline(l)).child(list);
    }
    body
}

/// A rate with its key: "■ Read 48 MB/s".
fn rate_key(l: &Look, color: Color, name: &str, bytes_per_second: f64) -> El {
    row()
        .gap(6.0)
        .center_y()
        .child(key_swatch(color, 8.0))
        .child(text(name.to_owned()).font(font(11.5, 400.0)).color(l.dim))
        .child(text(rate_string(bytes_per_second)).font(digits(12.5, 700.0)).color(l.text))
}

fn tiers_title(s: &Ui, l: &Look, title: &str) -> El {
    let mut tiers = segments(l, "tiers");
    for (at, tier) in TIERS.iter().enumerate() {
        tiers = tiers.child(segment(l, format!("tier-{}", tier.label), tier.label, s.tier == at, move |s: &mut Ui| ui::set_tier(s, at)));
    }
    section_title(l, title, Some(tiers)).key("history")
}

pub fn network(s: &Ui, l: &Look) -> El {
    let Some((view, net)) = s.system.as_ref().and_then(|view| Some((view, view.net.as_ref()?))) else {
        return reading(l, "network-loading");
    };
    let [down_color, up_color, _] = l.series;
    let big = |glyph: &'static rune::Icon, color: Color, name: &str, bytes_per_second: f64| {
        let (value, unit) = rate(bytes_per_second);
        col()
            .grow(1.0)
            .gap(2.0)
            .child(row().gap(4.0).center_y().child(icon(glyph, 12.0).color(color)).child(text(name.to_owned()).font(font(11.0, 400.0)).color(l.dim)))
            .child(row().items_end().gap(4.0).child(text(value).font(figure(28.0)).color(l.text)).child(text(unit).font(font(12.0, 400.0)).color(l.dim).pb(4.0)))
    };
    let now = now_block("network-hero").gap(8.0).child(row().w_full().gap(12.0).child(big(&icons::ARROW_DOWN, down_color, "Download", net.down)).child(big(&icons::ARROW_UP, up_color, "Upload", net.up)));

    let tier = s.tier;
    let (down, up) = (&view.net_down[tier], &view.net_up[tier]);
    let scale = scale_for(highest(&[down, up]));
    let readout = match s.point.and_then(|at| Some((at, down[at]?, up[at].map_or(0.0, |p| p.avg)))) {
        Some((at, d, u)) => look::readout(l, None, point_time(view, tier, at), format!("↑ {}", rate_string(u as f64)), format!("↓ {}", rate_string(d.avg as f64))),
        None => match peak(down) {
            Some(at) => look::readout(l, Some(down_color), format!("Download peak at {}", point_time(view, tier, at)), String::new(), rate_string(down[at].map_or(0.0, |p| p.max) as f64)),
            None => look::readout(l, None, "No readings yet".to_owned(), String::new(), String::new()),
        },
    };
    let today = row()
        .key("network-today")
        .w_full()
        .pt(2.0)
        .gap(16.0)
        .center_y()
        .child(text(format!("Since {}", time::clock(net.since, false))).font(digits(11.5, 400.0)).color(l.faint))
        .child(row().gap(4.0).child(text(disk_bytes(net.received_today)).font(digits(11.5, 700.0)).color(l.text)).child(text("received").font(font(11.5, 400.0)).color(l.dim)))
        .child(row().gap(4.0).child(text(disk_bytes(net.sent_today)).font(digits(11.5, 700.0)).color(l.text)).child(text("sent").font(font(11.5, 400.0)).color(l.dim)));
    let history = section("network-history")
        .gap(8.0)
        .child(history_title(s, l))
        .child(mirrored(l, view, tier, scale, s.point))
        .child(axis(view, tier, l))
        .child(caption(l, "Download above the line, upload below it."))
        .child(readout)
        .child(today);

    let mut list = section("interfaces").gap(8.0).child(section_title(l, "Interfaces", None));
    for (at, interface) in net.interfaces.iter().enumerate() {
        list = list.child(interface_row(l, at, interface));
    }
    if net.interfaces.is_empty() {
        list = list.child(caption(l, "No network interfaces."));
    }
    col().w_full().child(now).child(hairline(l)).child(history).child(hairline(l)).child(list)
}

/// Download over a middle line and upload under it, each on the same scale.
fn mirrored(l: &Look, view: &SystemView, tier: usize, scale: f64, point: Option<usize>) -> El {
    let (down, up) = (&view.net_down[tier], &view.net_up[tier]);
    let len = down.len();
    let (w, h) = (INNER_W, NET_CHART_H);
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
        let py = mid - reach(p.avg);
        shape = shape.line([at_x, py], [at_x + 0.01, py], 9.0, LineCap::Round, down_color).line([at_x, py], [at_x + 0.01, py], 5.0, LineCap::Round, l.solid());
    }
    let label = |words: String, top: f32| row().absolute(0.0, top).px(2.0).child(text(words).font(digits(9.0, 400.0)).color(l.faint));
    el().key("network-chart-area")
        .size(w, h)
        .child(
            canvas(shape)
                .key("network-chart")
                .size(w, h)
                .label("Network history chart")
                .on_pointer_move_local(move |s: &mut Ui, p, _| ui::hover_point(s, p.x / w, len))
                .on_hover_change(|s: &mut Ui, inside, _| {
                    if !inside {
                        ui::leave_point(s);
                    }
                }),
        )
        .child(label(format!("↓ {}", rate_string(scale)), 3.0))
        .child(label(format!("↑ {}", rate_string(scale)), h - 15.0))
}

fn interface_row(l: &Look, at: usize, interface: &Interface) -> El {
    let detail = match &interface.address {
        Some(address) => format!("{} · {address}", interface.bsd),
        None => interface.bsd.clone(),
    };
    let (status, color) = match (interface.active, interface.kind) {
        (true, _) => ("Connected", l.good),
        (false, Kind::Ethernet) => ("Cable unplugged", l.dim),
        (false, Kind::WiFi) => ("Not connected", l.dim),
        (false, _) => ("Inactive", l.dim),
    };
    row()
        .key(format!("interface-{at}"))
        .w_full()
        .gap(10.0)
        .center_y()
        .child(el().size(7.0, 7.0).radius(3.5).fill(if interface.active { l.good } else { l.track }))
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(1.0)
                .child(text(interface.name.clone()).font(font(12.5, 500.0)).color(l.text).truncate())
                .child(text(detail).font(digits(11.0, 400.0)).color(l.faint).truncate()),
        )
        .child(text(status).font(font(11.5, 400.0)).color(color))
}

pub fn battery(s: &Ui, l: &Look) -> El {
    let Some((view, battery)) = s.system.as_ref().and_then(|view| Some((view, view.battery.as_ref()?))) else {
        return reading(l, "battery-loading");
    };
    let left = battery.charge.clamp(0.0, 1.0);
    let fill = if left <= 0.1 && !battery.plugged {
        l.critical
    } else if left <= 0.2 && !battery.plugged {
        l.warning
    } else {
        l.good
    };
    let time_left = battery.minutes_left.map(|minutes| time::span(minutes as i64 * 60));
    let mut hero = row().w_full().items_end().child(big_number(l, &percent(left), "charged")).child(spacer());
    if let Some(time_left) = time_left {
        let note = if battery.charging { "until full" } else { "left at this draw" };
        hero = hero.child(col().items_end().gap(1.0).pb(5.0).child(text(time_left).font(digits(13.0, 700.0)).color(l.text)).child(text(note).font(font(11.0, 400.0)).color(l.faint)));
    }
    let power = match (battery.watts, battery.plugged) {
        (Some(watts), false) => row().gap(6.0).child(text("Drawing").font(font(11.5, 400.0)).color(l.dim)).child(text(format!("{watts:.1} W from the battery")).font(digits(11.5, 600.0)).color(l.text)),
        (_, true) => row().gap(6.0).child(text("On power adapter").font(font(11.5, 400.0)).color(l.dim)),
        (None, false) => el(),
    };
    let now = now_block("battery-hero").child(hero).child(look::meter(l, "battery-meter", INNER_W, 6.0, left as f64, fill)).child(power.key("battery-power"));

    // The charge over the last day or week, with the stretches it was plugged in shaded.
    let span = if s.battery_week { 7 * 86_400 } else { 86_400 };
    let mut ranges = segments(l, "charge-ranges");
    for (week, label) in [(false, "24h"), (true, "7d")] {
        ranges = ranges.child(segment(l, format!("charge-{label}"), label, s.battery_week == week, move |s: &mut Ui| ui::set_battery_week(s, week)));
    }
    let to = view.at as i64;
    let from = to - span;
    let points: Vec<&ChargePoint> = view.charge.iter().filter(|p| p.at >= from).collect();
    let full = points.iter().rev().find(|p| p.charge >= 0.995).map(|p| p.at);
    let readout = match full {
        Some(at) => look::readout(l, None, format!("Reached 100% at {}", if s.battery_week { time::weekday_clock(at) } else { time::clock(at, false) }), String::new(), "100%".to_owned()),
        None => match points.iter().min_by(|a, b| a.charge.total_cmp(&b.charge)) {
            Some(low) => look::readout(l, None, format!("Lowest at {}", time::clock(low.at, false)), String::new(), percent(low.charge)),
            None => look::readout(l, None, "No readings yet".to_owned(), String::new(), String::new()),
        },
    };
    let mid = from + span / 2;
    let stamp = |at: i64| if s.battery_week { time::weekday_clock(at).split(' ').next().unwrap_or("").to_owned() } else { time::clock(at, false) };
    let mut axis = row().w_full().justify_between();
    for label in [stamp(from), stamp(mid), "now".to_owned()] {
        axis = axis.child(text(label).font(digits(10.0, 400.0)).color(l.faint));
    }
    let legend = row()
        .gap(14.0)
        .child(row().gap(5.0).center_y().child(el().size(10.0, 2.0).radius(1.0).fill(l.good)).child(text("Charge").font(font(10.5, 400.0)).color(l.dim)))
        .child(row().gap(5.0).center_y().child(el().size(9.0, 9.0).radius(2.0).fill(l.good.alpha(0.16))).child(text("Plugged in").font(font(10.5, 400.0)).color(l.dim)));
    let history = section("battery-history").gap(8.0).child(section_title(l, "Charge", Some(ranges))).child(charge_chart(l, &points, from, to)).child(axis).child(legend).child(readout);

    let mut facts: Vec<(&str, Option<&str>, String, Color)> = Vec::new();
    if let Some(health) = battery.health {
        facts.push(("Maximum capacity", Some("Compared with when it was new"), percent(health), l.text));
    }
    if let Some(cycles) = battery.cycle_count {
        facts.push(("Cycle count", Some("Full drain-and-charge equivalents"), cycles.to_string(), l.text));
    }
    if let Some(condition) = &battery.condition {
        let good = condition.eq_ignore_ascii_case("normal") || condition.eq_ignore_ascii_case("good");
        facts.push(("Condition", None, condition.clone(), if good { l.good } else { l.warning_text }));
    }
    if let Some(celsius) = battery.temperature {
        facts.push(("Temperature", None, format!("{celsius:.0} °C"), l.text));
    }
    let mut card = col().key("health").w_full().radius(10.0).continuous_corners().fill(l.card).border(1.0, l.stroke);
    for (at, (name, meaning, value, color)) in facts.into_iter().enumerate() {
        if at > 0 {
            card = card.child(rule(1.0, l.stroke));
        }
        let mut label = col().grow(1.0).min_w(0.0).gap(1.0).child(text(name).font(font(12.0, 400.0)).color(l.text));
        if let Some(meaning) = meaning {
            label = label.child(text(meaning).font(font(10.5, 400.0)).color(l.faint));
        }
        card = card.child(row().w_full().py(8.0).px(10.0).gap(10.0).center_y().child(label).child(text(value).font(digits(12.0, 600.0)).color(color)));
    }
    let health = section("battery-health").gap(8.0).child(section_title(l, "Health", None)).child(card);
    col().w_full().child(now).child(hairline(l)).child(history).child(hairline(l)).child(health)
}

fn charge_chart(l: &Look, points: &[&ChargePoint], from: i64, to: i64) -> El {
    let (w, h) = (INNER_W, CHARGE_CHART_H);
    let span = (to - from).max(1) as f32;
    let x = |at: i64| ((at - from) as f32 / span * w).clamp(0.0, w);
    let y = |charge: f32| 1.0 + (1.0 - charge.clamp(0.0, 1.0)) * (h - 2.0);
    let mut shape = Shape::new()
        .line([0.0, y(1.0)], [w, y(1.0)], 1.0, LineCap::Butt, l.stroke)
        .line([0.0, y(0.5)], [w, y(0.5)], 1.0, LineCap::Butt, l.stroke)
        .line([0.0, h - 0.5], [w, h - 0.5], 1.0, LineCap::Butt, l.track);
    // Plugged-in stretches as a wash behind the line.
    let mut start: Option<i64> = None;
    for (at, point) in points.iter().enumerate() {
        let end_here = !point.plugged || at + 1 == points.len();
        match (point.plugged, start) {
            (true, None) => start = Some(point.at),
            _ if end_here && start.is_some() => {
                let a = x(start.take().unwrap_or(point.at));
                let b = x(point.at);
                shape = shape.fill(vec![[a, 0.0], [b, 0.0], [b, h], [a, h]], l.good.alpha(0.12));
            }
            _ => {}
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
        line.push([x(pair[1].at), y(pair[1].charge)]);
    }
    shape = flush(shape, line, l.good);
    if points.is_empty() {
        shape = dashed(shape, [0.0, h / 2.0], [w, h / 2.0], 3.0, 3.0, 1.0, LineCap::Butt, l.faint);
    }
    let label = |words: &'static str, top: f32| row().absolute(w - 60.0, top).w(60.0).justify_end().px(2.0).child(text(words).font(digits(9.0, 400.0)).color(l.faint));
    el().key("charge-chart").size(w, h).child(canvas(shape).size(w, h).label("Charge history")).child(label("100%", y(1.0) + 3.0)).child(label("50%", y(0.5) - 14.0))
}

fn flush(shape: Shape, line: Vec<[f32; 2]>, color: Color) -> Shape {
    if line.len() < 2 { shape } else { shape.stroke(line, 2.0, color) }
}

pub fn gpu(s: &Ui, l: &Look) -> El {
    let Some((view, gpu)) = s.system.as_ref().and_then(|view| Some((view, view.gpu.as_ref()?))) else {
        return reading(l, "gpu-loading");
    };
    let memory_total = view.machine.memory.max(1);
    let stage = |key: &'static str, name: &str, meaning: String, value: String, share: f64| {
        col()
            .key(key)
            .w_full()
            .gap(5.0)
            .child(
                row()
                    .w_full()
                    .items_end()
                    .child(col().grow(1.0).min_w(0.0).gap(1.0).child(text(name.to_owned()).font(font(12.0, 600.0)).color(l.text)).child(text(meaning).font(font(10.5, 400.0)).color(l.faint)))
                    .child(text(value).font(digits(12.0, 700.0)).color(l.text)),
            )
            .child(look::meter(l, "meter", INNER_W, 6.0, share, l.normal))
    };
    let now = now_block("gpu-hero")
        .child(big_number(l, &percent(gpu.busy), "busy"))
        .child(stage("gpu-rendering", "Rendering", "Drawing pixels: windows, video, games".to_owned(), percent(gpu.rendering), gpu.rendering as f64))
        .child(stage("gpu-geometry", "Geometry", "Shaping 3D scenes before they are drawn".to_owned(), percent(gpu.geometry), gpu.geometry as f64))
        .child(stage(
            "gpu-memory",
            "Memory",
            format!("Taken from the {} of unified memory", ui::system_bytes(memory_total)),
            ui::system_bytes(gpu.memory),
            gpu.memory as f64 / memory_total as f64,
        ));

    let tier = s.tier;
    let busy = &view.gpu_busy[tier];
    let color = l.series[0];
    let chart = area_chart(l, "gpu-chart", &[Layer { top: busy, bottom: None, color }], s.point, ["100%", "50%"]);
    let readout = match s.point.and_then(|at| Some((at, busy[at]?))) {
        Some((at, p)) => look::readout(l, None, point_time(view, tier, at), String::new(), percent(p.avg)),
        None => match peak(busy) {
            Some(at) => look::readout(l, Some(color), format!("Peak at {}", point_time(view, tier, at)), String::new(), percent(busy[at].map_or(0.0, |p| p.max))),
            None => look::readout(l, None, "No readings yet".to_owned(), String::new(), String::new()),
        },
    };
    let history = section("gpu-history").gap(8.0).child(history_title(s, l)).child(chart).child(axis(view, tier, l)).child(readout);
    col().w_full().child(now).child(hairline(l)).child(history)
}

/// The header's line under each of these tabs' names, and its badge.
pub fn header_words(s: &Ui, tab: ui::Tab) -> (Option<String>, String) {
    let view = s.system.as_deref();
    match tab {
        ui::Tab::Disk => {
            let boot = view.and_then(|view| view.disk.as_ref()).and_then(|disk| disk.volumes.first());
            (boot.map(|v| nominal(v.total)), boot.map_or(String::new(), |v| format!("{} · {}", v.name, v.format)))
        }
        ui::Tab::Network => {
            let net = view.and_then(|view| view.net.as_ref());
            let primary = net.and_then(|net| net.interfaces.iter().find(|i| Some(&i.bsd) == net.primary.as_ref()).or_else(|| net.interfaces.iter().find(|i| i.active)));
            let words = match primary {
                Some(interface) => match &interface.address {
                    Some(address) => format!("Using {} · {address}", interface.name),
                    None => format!("Using {}", interface.name),
                },
                None if net.is_some() => "Not connected".to_owned(),
                None => String::new(),
            };
            (None, words)
        }
        ui::Tab::Battery => {
            let words = view.and_then(|view| view.battery.as_ref()).map_or(String::new(), |b| {
                match (b.plugged, b.charging, b.full) {
                    (false, ..) => "Running on battery",
                    (true, true, _) => "Charging",
                    (true, false, true) => "Plugged in, fully charged",
                    (true, false, false) => "Plugged in, not charging",
                }
                .to_owned()
            });
            (None, words)
        }
        _ => {
            let machine = view.map(|view| &view.machine);
            let cores = view.and_then(|view| view.gpu.as_ref()).and_then(|gpu| gpu.cores);
            (machine.map(|m| m.chip.clone()), cores.map_or(String::new(), |cores| format!("{cores} GPU cores")))
        }
    }
}
