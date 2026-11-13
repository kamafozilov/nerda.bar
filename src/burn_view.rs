//! The picked limit's chart at the top of the Agents tab: how it went from nothing used to now
//! across its window, beside an even pace, and where the pace so far leads. The rows under it
//! pick which limit it shows. Over the chart, the limit's name and what is left, then what was
//! used today; under the pointer, what was left at that moment and what was used that day.
//!
//! The chart follows the panel's percentages: what is left falls from 100%, or what is used
//! climbs to it. Recorded stretches are a solid line, estimated ones a fainter line, the even
//! pace dashed and the projection dotted, each named in the legend under the chart. Only the
//! area under the line is filled: the chart sits on the panel itself.

use rune::{Color, El, LineCap, Shape, canvas, col, row, spacer, text};

use crate::burn::{self, Curve, Source};
use crate::model::{self, Limit, Severity, Snapshot};
use crate::time::{self, WEEKDAYS};
use crate::ui::{self, Ui};
use crate::view::{INNER_W, Look, PAD_X, digits, font, reset_note, runway_note};

const CHART_H: f32 = 64.0;
const AXIS_H: f32 = 12.0;
const LINE: f32 = 1.75;
/// The narrowest column that still gets its name under the chart.
const LABEL_MIN_W: f32 = 22.0;
const HOUR_LABEL_MIN_W: f32 = 32.0;
/// Points closer together than this along the chart are drawn as one.
const THIN: f32 = 0.6;

/// The chart of `limit`, the one picked, with its name and reading over it. A limit with no
/// window under way says when one starts in the chart's place, so the panel keeps its height.
pub fn section(s: &Ui, l: &Look, snapshot: &Snapshot, limit: &Limit) -> El {
    let show_used = s.config.show_used;
    let activity = s.stats.as_ref().map_or(&[][..], |stats| stats.activity.as_slice());
    let curve = burn::curve(limit, snapshot.fetched_at, s.history.readings(&limit.id), activity);
    let moment = curve.as_ref().and_then(|curve| s.trace.map(|at| curve.start + ((curve.end - curve.start) as f64 * at as f64) as i64));
    let any_scoped = snapshot.limits.iter().any(|l| l.scoped);
    let body = col().key("limit-history").w_full().px(PAD_X).pt(2.0).pb(12.0).gap(8.0);
    let Some(curve) = curve else {
        let empty = row().key("limit-chart-empty").w_full().h(CHART_H + 5.0 + AXIS_H).center().child(text(reset_note(limit, s.now)).font(font(11.5, 400.0)).color(l.faint));
        return body.child(title(l, limit, any_scoped, None, None, show_used)).child(empty);
    };
    let color = l.fill(limit.severity());
    let head = col().w_full().gap(2.0).child(title(l, limit, any_scoped, Some(&curve), moment, show_used)).child(reading(l, limit, &curve, moment, s.now));
    body.child(head)
        .child(col().w_full().gap(5.0).child(chart(l, &curve, color, moment, show_used)).child(axis(l, &curve, moment)))
        .child(legend(l, &curve, color))
}

/// The limit's name, and what is left of it now or at the moment under the pointer.
fn title(l: &Look, limit: &Limit, any_scoped: bool, curve: Option<&Curve>, moment: Option<i64>, show_used: bool) -> El {
    let (name, scope) = model::names(limit, any_scoped);
    let mut names = row().gap(6.0).center_y().child(text(name).font(font(13.0, 600.0)).color(l.text));
    if let Some(scope) = scope {
        names = names.child(text(scope).font(font(13.0, 400.0)).color(l.dim));
    }
    let word = if show_used { "used" } else { "left" };
    let read = curve.zip(moment).and_then(|(curve, t)| curve.at(t));
    let (value, severity) = match read {
        Some((used, source)) => {
            let shown = if show_used { used } else { 100.0 - used }.clamp(0.0, 100.0);
            (format!("{}{}% {word}", approx(source != Source::Recorded), shown.round() as i64), model::severity(100.0 - used))
        }
        None if moment.is_some() => (String::new(), Severity::Normal),
        None => (format!("{} {word}", model::percent(limit, show_used)), limit.severity()),
    };
    row().key("limit-title").w_full().h(18.0).center_y().child(names).child(spacer()).child(text(value).font(digits(13.0, 600.0)).color(l.ink(severity)))
}

/// The line under the title. Now: what was used today, and whether the pace so far lasts
/// until the reset. Under the pointer: when that is and what was used that day.
fn reading(l: &Look, limit: &Limit, curve: &Curve, moment: Option<i64>, now: i64) -> El {
    let line = row().key("limit-readout").w_full().h(14.0).gap(6.0).center_y();
    let Some(t) = moment else {
        let mut line = line.child(text(detail(curve, None)).font(digits(11.0, 400.0)).color(l.dim)).child(spacer());
        if let Some(warning) = runway_note(l, limit, now) {
            line = line.child(warning);
        } else if curve.projection.is_some_and(|_| !curve.runs_out) {
            line = line.child(text("Lasts until reset").font(font(11.0, 400.0)).color(l.dim));
        }
        return line;
    };
    let when = if curve.daily() { time::weekday_clock(t) } else { time::clock(t, false) };
    line.child(text(when).font(digits(11.0, 600.0)).color(l.text)).child(text(detail(curve, Some(t))).font(digits(11.0, 400.0)).color(l.dim)).child(spacer())
}

fn approx(on: bool) -> &'static str {
    if on { "≈" } else { "" }
}

/// What was used in the day (or hour) of the moment read, or where the pace leads past now.
fn detail(curve: &Curve, moment: Option<i64>) -> String {
    let last = curve.last();
    let t = moment.unwrap_or(last.at);
    if t > last.at {
        return match curve.projection {
            None => "Too early to project".to_owned(),
            Some(ahead) if curve.runs_out && t >= ahead.at => "Used up at this pace".to_owned(),
            Some(_) => "Projected at this pace".to_owned(),
        };
    }
    let Some((from, to)) = curve.bucket(t).or_else(|| curve.bucket(t - 1)) else { return String::new() };
    let Some(spent) = curve.used_between(from, to) else { return String::new() };
    // Estimated if any of the stretch read is.
    let estimated = curve.points.windows(2).any(|pair| pair[1].source == Source::Estimated && pair[1].at > from && pair[0].at < to);
    let when = match (moment, curve.daily()) {
        (None, true) => "today".to_owned(),
        (None, false) => "this hour".to_owned(),
        (Some(_), true) => format!("on {}", WEEKDAYS[time::local(from).weekday as usize]),
        (Some(_), false) => format!("{}–{}", time::clock(from, false), time::clock(to, false)),
    };
    format!("{}{}% used {when}", approx(estimated), spent.round().max(0.0) as i64)
}

fn chart(l: &Look, curve: &Curve, color: Color, moment: Option<i64>, show_used: bool) -> El {
    let (w, h) = (INNER_W, CHART_H);
    let (start, span) = (curve.start, (curve.end - curve.start) as f32);
    let x = |t: i64| ((t - start) as f32 / span * w).clamp(0.0, w);
    // A hair inside the top and bottom, so a line at 0% or 100% isn't cut in half.
    let y = |used: f64| {
        let shown = if show_used { used } else { 100.0 - used };
        1.0 + (1.0 - (shown / 100.0).clamp(0.0, 1.0) as f32) * (h - 2.0)
    };

    let mut shape = Shape::new().line([0.0, 0.5], [w, 0.5], 1.0, LineCap::Butt, l.stroke).line([0.0, h - 0.5], [w, h - 0.5], 1.0, LineCap::Butt, l.track);
    for (from, _) in curve.buckets().into_iter().skip(1) {
        shape = shape.line([x(from), 0.0], [x(from), h], 1.0, LineCap::Butt, l.stroke);
    }
    shape = dashed(shape, [0.0, y(0.0)], [w, y(100.0)], 3.0, 3.0, 1.0, LineCap::Butt, l.faint);

    // The curve, thinned to what a point can show.
    let mut points: Vec<([f32; 2], Source)> = Vec::new();
    for (at, p) in curve.points.iter().enumerate() {
        let here = [x(p.at), y(p.used)];
        let keep = match points.last() {
            None => true,
            Some((before, source)) => here[0] - before[0] >= THIN || *source != p.source || at == curve.points.len() - 1,
        };
        if keep {
            points.push((here, p.source));
        }
    }
    let last = points.last().map_or([0.0, y(0.0)], |p| p.0);
    let mut area: Vec<[f32; 2]> = points.iter().map(|p| p.0).collect();
    area.extend([[last[0], h], [0.0, h]]);
    shape = shape.fill(area, color.alpha(0.16));
    // A polyline per stretch of one source, each starting where the one before ended. The
    // estimated stretches are paler but opaque: a translucent line darkens where it overlaps
    // itself at every joint.
    let faded = estimated_color(l, color);
    let mut run: Vec<[f32; 2]> = Vec::new();
    let mut run_source = Source::Recorded;
    for pair in points.windows(2) {
        let (a, (b, source)) = (pair[0].0, pair[1]);
        if source != run_source && !run.is_empty() {
            shape = stroke_run(shape, std::mem::take(&mut run), run_source, color, faded);
        }
        if run.is_empty() {
            run.push(a);
        }
        run.push(b);
        run_source = source;
    }
    shape = stroke_run(shape, run, run_source, color, faded);

    if let Some(ahead) = curve.projection {
        let end = [x(ahead.at), y(ahead.used)];
        shape = dashed(shape, last, end, 0.01, 4.0, 2.0, LineCap::Round, color.alpha(0.85));
        // A ring where the projection ends: the reset, or the moment the limit runs out.
        shape = shape.line(end, [end[0] + 0.01, end[1]], 8.0, LineCap::Round, color).line(end, [end[0] + 0.01, end[1]], 4.5, LineCap::Round, l.panel.alpha(1.0));
    }
    shape = dot(shape, l, last, color);

    if let Some(t) = moment {
        let px = x(t);
        shape = shape.line([px, 0.0], [px, h], 1.0, LineCap::Butt, l.faint);
        if let Some((used, _)) = curve.at(t) {
            shape = dot(shape, l, [px, y(used)], color);
        }
    }
    canvas(shape)
        .key("limit-chart")
        .size(w, h)
        .label("Limit history chart")
        .on_pointer_move_local(move |s: &mut Ui, p, _| ui::trace_limit(s, p.x / w))
        .on_hover_change(|s: &mut Ui, inside, _| {
            if !inside {
                ui::leave_trace(s);
            }
        })
}

fn stroke_run(shape: Shape, run: Vec<[f32; 2]>, source: Source, color: Color, faded: Color) -> Shape {
    if run.len() < 2 {
        return shape;
    }
    shape.stroke(run, LINE, if source == Source::Estimated { faded } else { color })
}

/// The line's colour part way to the panel's: further on a dark panel, where the full colour
/// stands out more.
fn estimated_color(l: &Look, color: Color) -> Color {
    color.mix(l.panel.alpha(1.0), if l.dark { 0.55 } else { 0.35 })
}

/// A dot on the line, ringed in the panel's colour so it lifts off the fill.
fn dot(shape: Shape, l: &Look, at: [f32; 2], color: Color) -> Shape {
    shape.line(at, [at[0] + 0.01, at[1]], 9.0, LineCap::Round, l.panel.alpha(1.0)).line(at, [at[0] + 0.01, at[1]], 6.0, LineCap::Round, color)
}

/// A broken line from `a` to `b`: `dash` long pieces `gap` apart; with round caps and a tiny
/// dash, a line of dots.
#[allow(clippy::too_many_arguments)]
fn dashed(mut shape: Shape, a: [f32; 2], b: [f32; 2], dash: f32, gap: f32, width: f32, cap: LineCap, color: Color) -> Shape {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1.0 {
        return shape;
    }
    let (ux, uy) = (dx / len, dy / len);
    let mut along = 0.0;
    while along < len {
        let to = (along + dash).min(len);
        shape = shape.line([a[0] + ux * along, a[1] + uy * along], [a[0] + ux * to, a[1] + uy * to], width, cap, color);
        along += dash + gap;
    }
    shape
}

/// Each day's name under its column, or each hour's for a window shorter than a day; the one
/// read brighter.
fn axis(l: &Look, curve: &Curve, moment: Option<i64>) -> El {
    let span = (curve.end - curve.start) as f32;
    let read = moment.and_then(|t| curve.bucket(t));
    let daily = curve.daily();
    let mut axis = row().key("limit-axis").w_full().h(AXIS_H);
    for (from, to) in curve.buckets() {
        let width = (to - from) as f32 / span * INNER_W;
        let mut cell = row().w(width).min_w(0.0).h(AXIS_H).center();
        let name = if daily { WEEKDAYS[time::local(from).weekday as usize].to_owned() } else { format!("{:02}:00", time::local(from).hour) };
        if width >= if daily { LABEL_MIN_W } else { HOUR_LABEL_MIN_W } {
            cell = cell.child(text(name).font(digits(10.0, 400.0)).color(if read == Some((from, to)) { l.text } else { l.faint }));
        }
        axis = axis.child(cell);
    }
    axis
}

/// What each line is: colour and dashes never name a series alone.
fn legend(l: &Look, curve: &Curve, color: Color) -> El {
    let (w, mid) = (14.0, 3.0);
    let item = |key: &'static str, swatch: Shape, name: &'static str| {
        row().key(key).gap(5.0).center_y().child(canvas(swatch).size(w, 6.0)).child(text(name).font(font(10.5, 400.0)).color(l.dim))
    };
    let mut legend = row().key("limit-legend").w_full().gap(12.0).center_y();
    if curve.points.iter().skip(1).any(|p| p.source == Source::Recorded) {
        legend = legend.child(item("legend-recorded", Shape::new().line([1.0, mid], [w - 1.0, mid], LINE, LineCap::Round, color), "Recorded"));
    }
    if curve.estimated() {
        legend = legend.child(item("legend-estimated", Shape::new().line([1.0, mid], [w - 1.0, mid], LINE, LineCap::Round, estimated_color(l, color)), "Estimated"));
    }
    legend = legend.child(item("legend-pace", dashed(Shape::new(), [0.0, mid], [w, mid], 3.0, 3.0, 1.0, LineCap::Butt, l.faint), "Even pace"));
    if curve.projection.is_some() {
        legend = legend.child(item("legend-projection", dashed(Shape::new(), [1.0, mid], [w, mid], 0.01, 4.0, 2.0, LineCap::Round, color.alpha(0.85)), "Projection"));
    }
    legend
}
