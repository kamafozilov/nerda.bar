//! The picked limit's chart on a provider's tab: how it went from nothing used to now across its
//! window, beside an even pace, and where the pace so far leads. The rows over it pick which
//! limit it shows. Over the chart, the limit's name and what was used today, and a pill saying
//! whether the pace lasts until the reset; under the pointer, what was left at that moment and
//! what was used that day.
//!
//! The chart follows the panel's percentages: what is left falls from 100%, or what is used
//! climbs to it. Recorded stretches are a solid line, estimated ones a paler line, the even pace
//! dashed and the projection dotted, each named in the legend under the chart. Only the area
//! under the line is filled, fading to the baseline: the chart sits on the panel itself.

use rune::{Color, El, LineCap, Shape, canvas, col, el, row, spacer, text};

use crate::agents_view::runway;
use crate::burn::{self, Curve, Source};
use crate::look::{INNER_W, Look, PAD_X, digits, font, pill};
use crate::model::{self, Limit, Provider, Severity, Snapshot};
use crate::time::{self, WEEKDAYS};
use crate::ui::{self, Ui};

const CHART_H: f32 = 80.0;
/// Room under the baseline for a ring that sits on it.
const BELOW: f32 = 5.0;
const AXIS_H: f32 = 12.0;
const LINE: f32 = 2.0;
/// The narrowest column that still gets its name under the chart.
const LABEL_MIN_W: f32 = 22.0;
const HOUR_LABEL_MIN_W: f32 = 32.0;
/// Points closer together than this along the chart are drawn as one.
const THIN: f32 = 0.6;
/// Steps of the area's fade from the line to the baseline.
const FADE_STEPS: usize = 16;

/// The chart of `limit`, the one picked, with its name and reading over it. A limit with no
/// window under way says when one starts in the chart's place, so the panel keeps its height.
pub fn section(s: &Ui, l: &Look, provider: Provider, snapshot: &Snapshot, limit: &Limit) -> El {
    let a = s.agent(provider);
    let show_used = s.show_used(provider);
    let activity = a.stats.as_ref().map_or(&[][..], |stats| stats.activity.as_slice());
    let curve = burn::curve(limit, snapshot.fetched_at, a.history.readings(&limit.id), activity);
    let moment = curve.as_ref().and_then(|curve| s.trace.map(|at| curve.start + ((curve.end - curve.start) as f64 * at as f64) as i64));
    let any_scoped = snapshot.limits.iter().any(|l| l.scoped);
    let body = col().key("limit-history").w_full().px(PAD_X).py(12.0).gap(8.0);
    let (name, scope) = model::names(limit, any_scoped);
    let mut names = row().gap(4.0).center_y().child(text(name).font(font(12.5, 600.0)).color(l.text));
    if let Some(scope) = scope {
        names = names.child(text(scope).font(font(12.5, 400.0)).color(l.dim));
    }
    let Some(curve) = curve else {
        let empty = row().key("limit-chart-empty").w_full().h(CHART_H + BELOW + 8.0 + AXIS_H).center().child(text(crate::agents_view::reset_note(limit, s.now)).font(font(11.5, 400.0)).color(l.faint));
        return body.child(row().key("limit-title").w_full().child(names)).child(empty);
    };
    let severity = limit.severity();
    let color = l.fill(severity);
    let title = row()
        .key("limit-title")
        .w_full()
        .h(30.0)
        .gap(8.0)
        .center_y()
        .child(col().grow(1.0).min_w(0.0).gap(1.0).child(names).child(text(detail(&curve, moment)).key("limit-readout").font(digits(11.0, 400.0)).color(l.faint)))
        .child(status(s, l, limit, &curve, moment, show_used));
    body.child(title)
        .child(col().w_full().gap(4.0).child(chart(l, limit, &curve, severity, moment, show_used)).child(axis(l, &curve, moment, s.now)))
        .child(legend(l, &curve, color, severity))
}

/// The title's other end: a pill for how the pace goes, or under the pointer, what was left
/// (or used) at that moment.
fn status(s: &Ui, l: &Look, limit: &Limit, curve: &Curve, moment: Option<i64>, show_used: bool) -> El {
    let word = if show_used { "used" } else { "left" };
    if let Some(t) = moment {
        let Some((used, source)) = curve.at(t) else { return el() };
        let shown = if show_used { used } else { 100.0 - used }.clamp(0.0, 100.0);
        let estimated = source != Source::Recorded;
        let note = match source {
            Source::Recorded => "recorded",
            Source::Estimated => "estimated",
            Source::Projected => "projected",
        };
        return col()
            .key("limit-value")
            .items_end()
            .child(text(format!("{}{}% {word}", if estimated { "~" } else { "" }, shown.round() as i64)).font(digits(13.0, 700.0)).color(l.ink(model::severity(100.0 - used))))
            .child(text(note).font(font(10.5, 400.0)).color(l.faint));
    }
    // The pill names the moment; the row over the chart counts down to it.
    let when = |runway: i64| {
        let at = s.now + runway;
        if limit.window <= 86_400 { time::clock(at, false) } else { time::weekday_clock(at) }
    };
    match runway(l, limit, s.now) {
        Some((words, color)) => pill("limit-pill", color, limit.runs_out_in(s.now).map_or(words, |runway| format!("Runs out {}", when(runway)))),
        None if curve.projection.is_some_and(|_| !curve.runs_out) => pill("limit-pill", l.good, "Lasts until reset".to_owned()),
        None => el(),
    }
}

fn approx(on: bool) -> &'static str {
    if on { "~" } else { "" }
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
    let spent = format!("{}{}%", approx(estimated), spent.round().max(0.0) as i64);
    match (moment, curve.daily()) {
        (None, true) => format!("{spent} used today"),
        (None, false) => format!("{spent} used {}–{}", time::clock(from, false), time::clock(to, false)),
        (Some(_), true) => format!("{} · {spent} used that day", WEEKDAYS[time::local(from).weekday as usize]),
        (Some(_), false) => format!("{}–{} · {spent} used", time::clock(from, false), time::clock(to, false)),
    }
}

fn chart(l: &Look, limit: &Limit, curve: &Curve, severity: Severity, moment: Option<i64>, show_used: bool) -> El {
    let color = l.fill(severity);
    let (w, h) = (INNER_W, CHART_H);
    let (start, span) = (curve.start, (curve.end - curve.start) as f32);
    let x = |t: i64| ((t - start) as f32 / span * w).clamp(0.0, w);
    // A hair inside the top, so a line at 100% isn't cut in half.
    let y = |used: f64| {
        let shown = if show_used { used } else { 100.0 - used };
        1.0 + (1.0 - (shown / 100.0).clamp(0.0, 1.0) as f32) * (h - 1.0)
    };

    let mut shape = Shape::new();
    if let Some(t) = moment
        && let Some((from, to)) = curve.bucket(t)
    {
        // The column read, on a quiet wash.
        shape = shape.fill(vec![[x(from), 0.0], [x(to), 0.0], [x(to), h], [x(from), h]], l.card);
    }
    for (from, _) in curve.buckets().into_iter().skip(1) {
        shape = shape.line([x(from), 0.0], [x(from), h], 1.0, LineCap::Butt, l.stroke);
    }
    shape = shape.line([0.0, h - 0.5], [w, h - 0.5], 1.0, LineCap::Butt, l.track);
    shape = dashed(shape, [0.0, y(0.0)], [w, y(100.0)], 4.0, 4.0, 1.25, LineCap::Butt, l.faint);

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
    let line: Vec<[f32; 2]> = points.iter().map(|p| p.0).collect();
    shape = faded_area(shape, &line, h, color, if l.dark { 0.3 } else { 0.22 });
    // A polyline per stretch of one source, each starting where the one before ended. The
    // estimated stretches are paler but opaque: a translucent line darkens where it overlaps
    // itself at every joint.
    let faded = l.estimated(severity);
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

    let last = line.last().copied().unwrap_or([0.0, y(0.0)]);
    let mut label: Option<(String, [f32; 2])> = None;
    if let Some(ahead) = curve.projection {
        let end = [x(ahead.at), y(ahead.used)];
        shape = dashed(shape, last, end, 0.01, 5.0, 2.5, LineCap::Round, color);
        // A ring where the projection ends: the reset, or the moment the limit runs out.
        shape = shape.line(end, [end[0] + 0.01, end[1]], 10.0, LineCap::Round, color).line(end, [end[0] + 0.01, end[1]], 6.0, LineCap::Round, l.solid());
        let shown = if show_used { ahead.used } else { 100.0 - ahead.used };
        let words = if curve.runs_out { "Used up".to_owned() } else { format!("{}% at reset", shown.round().clamp(0.0, 100.0) as i64) };
        label = Some((words, end));
    }
    shape = shape.line(last, [last[0] + 0.01, last[1]], 8.0, LineCap::Round, color);

    if let Some(t) = moment {
        let px = x(t);
        shape = shape.line([px, 0.0], [px, h], 1.0, LineCap::Butt, l.dim);
        if let Some((used, _)) = curve.at(t) {
            let at = [px, y(used)];
            shape = shape.line(at, [at[0] + 0.01, at[1]], 10.0, LineCap::Round, color).line(at, [at[0] + 0.01, at[1]], 6.0, LineCap::Round, l.solid());
        }
    }
    let mut area = el().key("limit-chart-area").size(w, h + BELOW).child(
        canvas(shape)
            .key("limit-chart")
            .size(w, h)
            .label(format!("{} history chart", limit.label))
            .on_pointer_move_local(move |s: &mut Ui, p, _| ui::trace_limit(s, p.x / w))
            .on_hover_change(|s: &mut Ui, inside, _| {
                if !inside {
                    ui::leave_trace(s);
                }
            }),
    );
    // What the projection comes to, over the ring at its end, kept inside the chart.
    if let Some((words, [ex, ey])) = label.filter(|_| moment.is_none()) {
        let label_w = 90.0;
        let lx = (ex - label_w / 2.0).clamp(0.0, w - label_w);
        let ly = if curve.runs_out { ey - 30.0 } else { ey - 34.0 }.max(0.0);
        let anchor = if ex > w - label_w / 2.0 { row().justify_end() } else { row().center() };
        area = area.child(anchor.absolute(lx, ly).w(label_w).child(text(words).key("limit-label").font(digits(10.0, 600.0)).color(color)));
    }
    area
}

/// The area under `line` down to the baseline, its colour fading from `top` alpha at the
/// chart's top to nothing at its foot. Translucent bands that meet would leave a seam where
/// their antialiased edges overlap, so each layer instead runs from the line down to a step
/// and the layers stack: a band shows through every layer that reaches it, and each layer's
/// alpha is what makes the stack come to that band's share.
fn faded_area(mut shape: Shape, line: &[[f32; 2]], h: f32, color: Color, top: f32) -> Shape {
    if line.len() < 2 {
        return shape;
    }
    let target = |step: usize| top * (1.0 - (step as f32 + 0.5) / FADE_STEPS as f32);
    for step in 0..FADE_STEPS {
        let below = if step + 1 < FADE_STEPS { target(step + 1) } else { 0.0 };
        let alpha = 1.0 - (1.0 - target(step)) / (1.0 - below);
        let foot = h * (step + 1) as f32 / FADE_STEPS as f32;
        // The layer's top edge: the line, held above the foot, with a point where it crosses.
        let mut edge: Vec<[f32; 2]> = Vec::with_capacity(line.len() + 8);
        for pair in line.windows(2) {
            let ([ax, ay], [bx, by]) = (pair[0], pair[1]);
            if edge.is_empty() {
                edge.push([ax, ay.min(foot)]);
            }
            if (ay - foot) * (by - foot) < 0.0 {
                let t = (foot - ay) / (by - ay);
                edge.push([ax + (bx - ax) * t, foot]);
            }
            edge.push([bx, by.min(foot)]);
        }
        if edge.iter().all(|p| p[1] >= foot) {
            continue;
        }
        let (first, last) = (edge[0][0], edge[edge.len() - 1][0]);
        edge.extend([[last, foot], [first, foot]]);
        shape = shape.fill(edge, color.alpha(alpha));
    }
    shape
}

fn stroke_run(shape: Shape, run: Vec<[f32; 2]>, source: Source, color: Color, faded: Color) -> Shape {
    if run.len() < 2 {
        return shape;
    }
    shape.stroke(run, LINE, if source == Source::Estimated { faded } else { color })
}

/// A broken line from `a` to `b`: `dash` long pieces `gap` apart; with round caps and a tiny
/// dash, a line of dots.
#[allow(clippy::too_many_arguments)]
pub(crate) fn dashed(mut shape: Shape, a: [f32; 2], b: [f32; 2], dash: f32, gap: f32, width: f32, cap: LineCap, color: Color) -> Shape {
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

/// Each day's name under its column, or each hour's for a window shorter than a day: today's
/// (or the one read) bold.
fn axis(l: &Look, curve: &Curve, moment: Option<i64>, now: i64) -> El {
    let span = (curve.end - curve.start) as f32;
    let strong = curve.bucket(moment.unwrap_or(now));
    let daily = curve.daily();
    let mut axis = row().key("limit-axis").w_full().h(AXIS_H);
    for (from, to) in curve.buckets() {
        let width = (to - from) as f32 / span * INNER_W;
        let mut cell = row().w(width).min_w(0.0).h(AXIS_H).center();
        let name = if daily { WEEKDAYS[time::local(from).weekday as usize].to_owned() } else { format!("{:02}:00", time::local(from).hour) };
        if width >= if daily { LABEL_MIN_W } else { HOUR_LABEL_MIN_W } {
            let on = strong == Some((from, to));
            cell = cell.child(text(name).font(digits(10.0, if on { 700.0 } else { 400.0 })).color(if on { l.text } else { l.faint }));
        }
        axis = axis.child(cell);
    }
    axis
}

/// What each line is: colour and dashes never name a series alone.
fn legend(l: &Look, curve: &Curve, color: Color, severity: Severity) -> El {
    let (w, mid) = (13.0, 1.5);
    let item = |key: &'static str, swatch: Shape, name: &'static str| {
        row().key(key).gap(5.0).center_y().child(canvas(swatch).size(w, 3.0)).child(text(name).font(font(10.5, 400.0)).color(l.dim))
    };
    let mut items = Vec::new();
    if curve.points.iter().skip(1).any(|p| p.source == Source::Recorded) {
        items.push(item("legend-recorded", Shape::new().line([1.0, mid], [11.0, mid], 2.0, LineCap::Round, color), "Recorded"));
    }
    if curve.estimated() {
        items.push(item("legend-estimated", Shape::new().line([1.0, mid], [11.0, mid], 2.0, LineCap::Round, l.estimated(severity)), "Estimated"));
    }
    items.push(item("legend-pace", dashed(Shape::new(), [0.0, mid], [w, mid], 3.0, 2.0, 2.0, LineCap::Butt, l.faint), "Even pace"));
    if curve.projection.is_some() {
        items.push(item("legend-projection", dashed(Shape::new(), [1.0, mid], [w, mid], 0.01, 4.0, 2.0, LineCap::Round, color), "Projection"));
    }
    // Spread across the width, as the design's legend is.
    let mut legend = row().key("limit-legend").w_full().center_y();
    let count = items.len();
    for (at, item) in items.into_iter().enumerate() {
        legend = legend.child(item);
        if at + 1 < count {
            legend = legend.child(spacer());
        }
    }
    legend
}
