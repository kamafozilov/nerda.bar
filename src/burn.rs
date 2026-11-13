//! Each limit's history: what was used at every refresh, kept for a week and a day, and the
//! curve the panel's chart draws from it for the window under way.
//!
//! Readings start when Nerda Bar first ran and miss the hours it wasn't running or the Mac
//! slept. Such a stretch is filled in from Claude Code's logs: what the limit gained between
//! the readings on either side is spread over its hours by what this Mac's messages to the
//! models the limit counts would have cost in each. That part of the curve is an estimate and
//! is drawn as one.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::model::{self, Limit, Snapshot};
use crate::stats::Activity;
use crate::time;

const FILE: &str = "history.json";
/// How long readings are kept: the longest window and a day.
const KEEP: i64 = 8 * 86_400;
/// Readings further apart than this, with use between them, leave a gap to estimate. Refreshes
/// come every 5 minutes, so this is a few missed in a row.
const GAP: i64 = 20 * 60;
const HOUR: i64 = 3600;
const DAY: i64 = 86_400;

/// Every limit's readings.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct History {
    /// By limit id: Unix seconds and percent used, oldest first.
    pub limits: BTreeMap<String, Vec<(i64, f32)>>,
}

impl History {
    pub fn load(dir: &Path) -> History {
        std::fs::read_to_string(dir.join(FILE)).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
    }

    pub fn save(&self, dir: &Path) {
        if std::fs::create_dir_all(dir).is_ok()
            && let Ok(text) = serde_json::to_string(self)
        {
            let _ = std::fs::write(dir.join(FILE), text);
        }
    }

    /// Adds a refresh's readings and forgets those older than a week and a day.
    pub fn record(&mut self, snapshot: &Snapshot) {
        let at = snapshot.fetched_at;
        for limit in &snapshot.limits {
            let readings = self.limits.entry(limit.id.clone()).or_default();
            if readings.last().is_some_and(|(last, _)| *last >= at) {
                continue;
            }
            let used = limit.used as f32;
            // A run of equal readings keeps its first and its last: the line between them is
            // flat either way.
            match readings.as_mut_slice() {
                [.., (_, before), (last_at, last)] if *before == used && *last == used => *last_at = at,
                _ => readings.push((at, used)),
            }
        }
        for readings in self.limits.values_mut() {
            readings.retain(|(t, _)| *t >= at - KEEP);
        }
        self.limits.retain(|_, readings| !readings.is_empty());
    }

    pub fn readings(&self, id: &str) -> &[(i64, f32)] {
        self.limits.get(id).map_or(&[], Vec::as_slice)
    }
}

/// How a stretch of the curve is known.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// Read from the provider, refresh by refresh.
    Recorded,
    /// Spread between two readings by this Mac's activity.
    Estimated,
    /// Where the pace so far leads.
    Projected,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub at: i64,
    /// Percent used.
    pub used: f64,
    /// How the stretch that ends here is known.
    pub source: Source,
}

/// A limit's window from its start to now: what was used along it, and where it is heading.
#[derive(Clone, Debug, PartialEq)]
pub struct Curve {
    pub start: i64,
    pub end: i64,
    /// From the window's start to the last reading.
    pub points: Vec<Point>,
    /// Where the pace so far leads: the reset, or the moment the limit is used up before it.
    pub projection: Option<Point>,
    pub runs_out: bool,
}

/// Whether messages to `model` count toward `limit`: a plan-wide limit counts them all, a
/// model's own limit those whose name has the model's in it ("weekly-fable", "claude-fable-5-2").
pub fn counts(limit: &Limit, model: &str) -> bool {
    if !limit.scoped {
        return true;
    }
    let name = limit.id.strip_prefix("weekly-").unwrap_or(&limit.id);
    !name.is_empty() && model.to_ascii_lowercase().contains(&name.to_ascii_lowercase())
}

/// The curve of `limit`'s window as of the refresh at `at`, or `None` when no window is under
/// way.
pub fn curve(limit: &Limit, at: i64, readings: &[(i64, f32)], activity: &[Activity]) -> Option<Curve> {
    let end = limit.resets_at?;
    let start = end - limit.window;
    if limit.window <= 0 || at <= start || at >= end {
        return None;
    }

    // This window's readings, then the refresh itself. Use never goes down within a window:
    // a reading under the one before it is the provider rounding, and one well under it means
    // the earlier readings belong to the window before.
    let mut known: Vec<(i64, f64)> = vec![(start, 0.0)];
    let current = (at, limit.used as f32);
    for &(t, used) in readings.iter().filter(|(t, _)| *t > start && *t < at).chain(std::iter::once(&current)) {
        let used = used as f64;
        if used + 1.0 < known.last().map_or(0.0, |k| k.1) {
            known.truncate(1);
        }
        let floor = known.last().map_or(0.0, |k| k.1);
        known.push((t, used.max(floor)));
    }

    let weights = hourly(limit, activity, start, at);
    let mut points = vec![Point { at: start, used: 0.0, source: Source::Recorded }];
    for pair in known.windows(2) {
        let ((a, used_a), (b, used_b)) = (pair[0], pair[1]);
        if used_b <= used_a || b - a <= GAP {
            points.push(Point { at: b, used: used_b, source: Source::Recorded });
            continue;
        }
        let total = weight(&weights, a, b);
        if total > 0.0 {
            let mut hour = a - a.rem_euclid(HOUR) + HOUR;
            while hour < b {
                let used = used_a + (used_b - used_a) * weight(&weights, a, hour) / total;
                points.push(Point { at: hour, used, source: Source::Estimated });
                hour += HOUR;
            }
        }
        // With no activity here to go by, the use came from elsewhere: a straight line.
        points.push(Point { at: b, used: used_b, source: Source::Estimated });
    }

    let (projection, runs_out) = project(limit, start, end, at);
    Some(Curve { start, end, points, projection, runs_out })
}

/// The activity that counts toward `limit` from `from` to `to`, summed by the hour.
pub(crate) fn hourly(limit: &Limit, activity: &[Activity], from: i64, to: i64) -> Vec<(i64, f64)> {
    let mut hours: Vec<(i64, f64)> = Vec::new();
    for entry in activity.iter().filter(|a| a.at + HOUR > from && a.at < to && counts(limit, &a.model)) {
        match hours.last_mut() {
            Some((at, weight)) if *at == entry.at => *weight += entry.weight,
            _ => hours.push((entry.at, entry.weight)),
        }
    }
    hours
}

/// The activity from `from` to `to`, an hour's weight shared out by how much of it is inside.
pub(crate) fn weight(hours: &[(i64, f64)], from: i64, to: i64) -> f64 {
    hours
        .iter()
        .map(|&(at, weight)| {
            let inside = (at + HOUR).min(to) - at.max(from);
            if inside > 0 { weight * inside as f64 / HOUR as f64 } else { 0.0 }
        })
        .sum()
}

/// Where the pace from the window's start to `at` leads, as [`Limit::runs_out_in`] reads it.
fn project(limit: &Limit, start: i64, end: i64, at: i64) -> (Option<Point>, bool) {
    let elapsed = at - start;
    if (elapsed as f64) < model::PACE_MIN_ELAPSED * limit.window as f64 || limit.used < model::PACE_MIN_USED || limit.used_up() {
        return (None, false);
    }
    let rate = limit.used / elapsed as f64;
    let out = at + ((100.0 - limit.used) / rate) as i64;
    if out < end {
        (Some(Point { at: out, used: 100.0, source: Source::Projected }), true)
    } else {
        let used = (limit.used + rate * (end - at) as f64).min(100.0);
        (Some(Point { at: end, used, source: Source::Projected }), false)
    }
}

impl Curve {
    pub fn last(&self) -> Point {
        *self.points.last().expect("a curve starts with its window")
    }

    /// Whether some of the curve is estimated.
    pub fn estimated(&self) -> bool {
        self.points.iter().any(|p| p.source == Source::Estimated)
    }

    /// What was used at `t`, and how that is known. Past the last reading it follows the
    /// projection, and without one it is unknown.
    pub fn at(&self, t: i64) -> Option<(f64, Source)> {
        let last = self.last();
        if t <= last.at {
            let next = self.points.partition_point(|p| p.at < t).max(1);
            let (a, b) = (self.points[next - 1], self.points[next.min(self.points.len() - 1)]);
            return Some((lerp(a, b, t), if t <= a.at { a.source } else { b.source }));
        }
        let ahead = self.projection?;
        Some((if t >= ahead.at { ahead.used } else { lerp(last, ahead, t) }, Source::Projected))
    }

    /// What was used from `from` to `to`, as far as the readings go.
    pub fn used_between(&self, from: i64, to: i64) -> Option<f64> {
        let (from, to) = (from.max(self.start), to.min(self.last().at));
        (to > from).then(|| self.at(to).map_or(0.0, |v| v.0) - self.at(from).map_or(0.0, |v| v.0))
    }

    /// Whether the window is long enough to read by the day rather than by the hour.
    pub fn daily(&self) -> bool {
        self.end - self.start >= DAY
    }

    /// The calendar days the window crosses, or its hours when it is shorter than a day, each
    /// as its start and end within the window.
    pub fn buckets(&self) -> Vec<(i64, i64)> {
        let mut edges = vec![self.start];
        let here = time::local(self.start);
        if self.daily() {
            let mut day = here.day + 1;
            loop {
                let midnight = time::local_hour_start(day, 0);
                if midnight >= self.end {
                    break;
                }
                edges.push(midnight);
                day += 1;
            }
        } else {
            let mut hour = time::local_hour_start(here.day, here.hour) + HOUR;
            while hour < self.end {
                edges.push(hour);
                hour += HOUR;
            }
        }
        edges.push(self.end);
        edges.windows(2).map(|w| (w[0], w[1])).filter(|(a, b)| b > a).collect()
    }

    /// The bucket `t` is in.
    pub fn bucket(&self, t: i64) -> Option<(i64, i64)> {
        self.buckets().into_iter().find(|(from, to)| t >= *from && t < *to)
    }
}

/// The straight line from `a` to `b`, at `t`.
fn lerp(a: Point, b: Point, t: i64) -> f64 {
    if b.at <= a.at {
        return b.used;
    }
    let share = ((t - a.at) as f64 / (b.at - a.at) as f64).clamp(0.0, 1.0);
    a.used + (b.used - a.used) * share
}

#[cfg(test)]
mod tests {
    use super::*;

    const WEEK: i64 = 7 * DAY;
    /// A whole hour, so hour edges fall on round numbers.
    const START: i64 = 1_794_441_600;

    fn weekly(used: f64) -> Limit {
        Limit { id: "weekly".to_owned(), label: "Weekly".to_owned(), used, resets_at: Some(START + WEEK), window: WEEK, scoped: false }
    }

    fn activity(hours: &[(i64, &str, f64)]) -> Vec<Activity> {
        hours.iter().map(|&(hour, model, weight)| Activity { at: START + hour * HOUR, model: model.to_owned(), weight }).collect()
    }

    #[test]
    fn readings_keep_the_ends_of_a_flat_run_and_a_week_and_a_day() {
        let mut history = History::default();
        let mut snapshot = Snapshot { plan: String::new(), limits: vec![weekly(10.0)], extra: None, fetched_at: START };
        for step in 0..5 {
            snapshot.fetched_at = START + step * 300;
            history.record(&snapshot);
        }
        assert_eq!(history.readings("weekly"), [(START, 10.0), (START + 1200, 10.0)]);
        snapshot.limits[0].used = 12.0;
        snapshot.fetched_at += 300;
        history.record(&snapshot);
        assert_eq!(history.readings("weekly").len(), 3);
        // The same refresh twice counts once.
        history.record(&snapshot);
        assert_eq!(history.readings("weekly").len(), 3);
        snapshot.fetched_at = START + 9 * DAY;
        history.record(&snapshot);
        assert_eq!(history.readings("weekly"), [(START + 9 * DAY, 12.0)]);
    }

    #[test]
    fn without_readings_the_window_is_spread_by_activity() {
        let limit = weekly(30.0);
        let at = START + 3 * DAY;
        // A third of the cost in the second hour, two thirds on the second day; a model
        // the plan-wide limit counts as much as any other.
        let curve = curve(&limit, at, &[], &activity(&[(1, "claude-opus-5-5", 1.0), (30, "claude-fable-5-2", 2.0)])).unwrap();
        assert_eq!((curve.start, curve.end), (START, START + WEEK));
        assert!(curve.estimated());
        assert_eq!(curve.at(START + HOUR), Some((0.0, Source::Estimated)));
        assert_eq!(curve.at(START + 2 * HOUR).map(|v| v.0.round()), Some(10.0));
        assert_eq!(curve.at(START + 20 * HOUR).map(|v| v.0.round()), Some(10.0));
        assert_eq!(curve.at(START + 31 * HOUR).map(|v| v.0.round()), Some(30.0));
        assert_eq!(curve.last().used, 30.0);
        assert_eq!(curve.used_between(START + DAY, START + 2 * DAY).map(f64::round), Some(20.0));
    }

    #[test]
    fn readings_close_together_are_the_record() {
        let limit = weekly(14.0);
        let at = START + 2 * DAY;
        let readings = [(START + 2 * DAY - 900, 12.0), (START + 2 * DAY - 600, 13.0), (START + 2 * DAY - 300, 13.0)];
        let curve = curve(&limit, at, &readings, &activity(&[(5, "claude-opus-5-5", 1.0)])).unwrap();
        let tail: Vec<Source> = curve.points.iter().rev().take(4).map(|p| p.source).collect();
        assert_eq!(tail, [Source::Recorded, Source::Recorded, Source::Recorded, Source::Estimated]);
        // Before the first reading: estimated, all of it put in the one busy hour.
        assert_eq!(curve.at(START + 6 * HOUR), Some((12.0, Source::Estimated)));
    }

    #[test]
    fn readings_from_the_window_before_are_dropped() {
        let limit = weekly(5.0);
        let at = START + DAY;
        // The provider still reported last week's 80% for a moment after the reset.
        let readings = [(START + 60, 80.0), (START + DAY - 300, 4.0)];
        let curve = curve(&limit, at, &readings, &[]).unwrap();
        assert!(curve.points.iter().all(|p| p.used <= 5.0));
        assert_eq!(curve.at(at - 300).map(|v| v.0), Some(4.0));
    }

    #[test]
    fn a_fast_pace_projects_to_where_the_limit_runs_out() {
        let limit = weekly(60.0);
        let at = START + 2 * DAY;
        let curve = curve(&limit, at, &[], &[]).unwrap();
        // 60% in two days: the rest lasts a day and a third.
        let ahead = curve.projection.unwrap();
        assert!(curve.runs_out);
        assert_eq!(ahead.used, 100.0);
        assert!((ahead.at - (at + 32 * HOUR)).abs() <= 1);
        assert_eq!(curve.at(at + 16 * HOUR).map(|v| (v.0.round(), v.1)), Some((80.0, Source::Projected)));
        // A slow one reaches the reset with some left.
        let slow = super::curve(&weekly(10.0), at, &[], &[]).unwrap();
        assert!(!slow.runs_out);
        assert_eq!(slow.projection.map(|p| (p.at, p.used.round())), Some((START + WEEK, 35.0)));
    }

    #[test]
    fn a_model_limit_counts_only_its_model() {
        let mut fable = weekly(20.0);
        fable.id = "weekly-fable".to_owned();
        fable.scoped = true;
        assert!(counts(&fable, "claude-fable-5-2"));
        assert!(!counts(&fable, "claude-opus-5-5"));
        assert!(counts(&weekly(0.0), "claude-opus-5-5"));
        // Opus was busy early, Fable late: Fable's limit follows Fable.
        let curve = curve(&fable, START + DAY, &[], &activity(&[(1, "claude-opus-5-5", 9.0), (20, "claude-fable-5-2", 1.0)])).unwrap();
        assert_eq!(curve.at(START + 10 * HOUR).map(|v| v.0), Some(0.0));
    }

    #[test]
    fn a_week_splits_into_its_days_and_a_session_into_its_hours() {
        let curve = curve(&weekly(10.0), START + DAY, &[], &[]).unwrap();
        let days = curve.buckets();
        assert!((7..=8).contains(&days.len()));
        assert_eq!(days.first().unwrap().0, curve.start);
        assert_eq!(days.last().unwrap().1, curve.end);
        assert!(days.windows(2).all(|w| w[0].1 == w[1].0));
        let session = Limit { id: "session".to_owned(), label: "Session".to_owned(), used: 10.0, resets_at: Some(START + 5 * HOUR + 1200), window: 5 * HOUR, scoped: false };
        let hours = super::curve(&session, START + HOUR, &[], &[]).unwrap().buckets();
        assert!((5..=6).contains(&hours.len()));
    }
}
