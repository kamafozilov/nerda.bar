//! Sample data for headless renders and tests: a plan part-way through its windows, the last
//! day and a bit of its readings, and a month of usage. Nothing here is read from this Mac.

use std::sync::Arc;

use crate::burn::{self, History};
use crate::model::{ExtraUsage, Limit, Snapshot};
use crate::stats::{Activity, Bar, ModelRow, RangeView, StatsView};
use crate::time::{self, MONTHS, WEEKDAYS};
use crate::ui::Ui;

pub fn fill(ui: &mut Ui) {
    let now = ui.now;
    let limit = |id: &str, label: &str, used: f64, resets_in: i64, window: i64, scoped: bool| Limit {
        id: id.to_owned(),
        label: label.to_owned(),
        used,
        resets_at: Some(now + resets_in),
        window,
        scoped,
    };
    ui.snapshot = Some(Snapshot {
        plan: "Ultra".to_owned(),
        limits: vec![
            limit("session", "Session", 76.0, 6540, 5 * 3600, false),
            limit("weekly", "Weekly", 41.0, 2 * 86_400 + 4 * 3600, 7 * 86_400, false),
            limit("weekly-fable", "Fable weekly", 58.0, 2 * 86_400 + 4 * 3600, 7 * 86_400, true),
        ],
        extra: Some(ExtraUsage { used: 12.4, limit: Some(50.0), currency: "USD".to_owned() }),
        fetched_at: now - 140,
    });
    ui.stats = Some(Arc::new(StatsView { ranges: [range(now, 1), range(now, 7), range(now, 30)], activity: activity(now) }));
    refill_history(ui);
    ui.system = Some(Arc::new(crate::system::demo(now as f64)));
}

/// Eight days of work by the hour: busy from nine to eleven at night and the last few hours,
/// mostly Fable.
fn activity(now: i64) -> Vec<Activity> {
    let mut activity = Vec::new();
    let mut hour = now - now.rem_euclid(3600) - 8 * 86_400;
    while hour <= now {
        if (9..23).contains(&time::local(hour).hour) || now - hour < 3 * 3600 {
            let busy = ((hour / 3600 * 37 + 11) % 17) as f64 / 17.0 + 0.15;
            for (model, share) in [("claude-fable-5-2", 0.62), ("claude-opus-5-5", 0.28), ("claude-sonnet-5-5", 0.1)] {
                activity.push(Activity { at: hour, model: model.to_owned(), weight: busy * share * 12.0 });
            }
        }
        hour += 3600;
    }
    activity
}

/// The readings again, for limits a scene has changed.
pub fn refill_history(ui: &mut Ui) {
    if let (Some(snapshot), Some(stats)) = (&ui.snapshot, &ui.stats) {
        ui.history = Arc::new(history(snapshot, &stats.activity));
    }
}

/// Readings every ten minutes for the last 30 hours, as if Nerda Bar was installed then: each
/// limit's use rising with the activity it counts to where it is now. What came before is left
/// for the chart to estimate.
fn history(snapshot: &Snapshot, activity: &[Activity]) -> History {
    let at = snapshot.fetched_at;
    let mut history = History::default();
    for limit in &snapshot.limits {
        let Some(end) = limit.resets_at else { continue };
        let start = end - limit.window;
        let hours = burn::hourly(limit, activity, start, at);
        let total = burn::weight(&hours, start, at);
        let mut readings = Vec::new();
        let mut t = (at - 30 * 3600).max(start) + 600;
        while t < at {
            let share = if total > 0.0 { burn::weight(&hours, start, t) / total } else { (t - start) as f64 / (at - start) as f64 };
            readings.push((t, (limit.used * share).floor() as f32));
            t += 600;
        }
        history.limits.insert(limit.id.clone(), readings);
    }
    history
}

fn range(now: i64, days: i32) -> RangeView {
    let today = time::local(now).day;
    let first = today - days + 1;
    let count = if days == 1 { 24 } else { days };
    let bars: Vec<Bar> = (0..count)
        .map(|at| {
            // A repeatable, uneven shape: busy weekdays, quiet nights.
            let wave = ((at * 37 + days * 11) % 17) as u64;
            let tokens = if days == 1 { if (9..22).contains(&at) { wave * 2_300_000 + 900_000 } else { 0 } } else { wave * 21_000_000 + 4_000_000 };
            let label = if days == 1 {
                format!("{at:02}:00")
            } else {
                let date = time::local_day(first + at);
                format!("{} {} {}", WEEKDAYS[date.weekday as usize], date.day_of_month, MONTHS[date.month as usize - 1])
            };
            Bar { label, tokens, cost: tokens as f64 * 0.78 / 1e6 }
        })
        .collect();
    let total: u64 = bars.iter().map(|bar| bar.tokens).sum();
    let cost: f64 = bars.iter().map(|bar| bar.cost).sum();
    let share = |name: &str, part: f64, cost_part: f64| ModelRow { name: name.to_owned(), tokens: (total as f64 * part) as u64, cost: Some(cost * cost_part) };
    let (axis, axis_per_bar) = if days == 1 {
        (["00:00", "06:00", "12:00", "18:00", "24:00"].map(str::to_owned).to_vec(), false)
    } else if days <= 7 {
        ((0..days).map(|at| WEEKDAYS[time::local_day(first + at).weekday as usize].to_owned()).collect(), true)
    } else {
        let date = |day: i32| {
            let date = time::local_day(day);
            format!("{} {}", date.day_of_month, MONTHS[date.month as usize - 1])
        };
        (vec![date(first), date(first + days / 2), date(today)], false)
    };
    RangeView {
        tokens: [total / 400, total / 90, total / 60, total / 25, total - total / 400 - total / 90 - total / 60 - total / 25],
        cost,
        unpriced: false,
        bars,
        axis,
        axis_per_bar,
        models: vec![
            share("claude-fable-5-2", 0.58, 0.71),
            share("claude-opus-5-5", 0.29, 0.22),
            share("claude-sonnet-5-5", 0.09, 0.06),
            share("claude-haiku-5-5", 0.04, 0.01),
        ],
    }
}
