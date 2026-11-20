//! Sample data for headless renders and tests: a plan part-way through its windows, the last
//! day and a bit of its readings, and a month of usage. Nothing here is read from this Mac.

use std::sync::Arc;

use crate::burn::{self, History};
use crate::model::{ExtraUsage, Limit, Snapshot, Subscription};
use crate::stats::{Activity, Bar, ModelRow, ProjectRow, RangeView, StatsView};
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
    ui.claude.snapshot = Some(Snapshot {
        plan: "Ultra".to_owned(),
        limits: vec![
            limit("weekly", "Weekly", 41.0, 2 * 86_400 + 4 * 3600, 7 * 86_400, false),
            limit("weekly-fable", "Fable weekly", 58.0, 2 * 86_400 + 4 * 3600, 7 * 86_400, true),
        ],
        extra: Some(ExtraUsage { used: 12.4, limit: Some(50.0), currency: "USD".to_owned(), resets_at: None }),
        fetched_at: now - 140,
        resets: None,
        subscription: Some(Subscription { until: now + 9 * 86_400 + 5 * 3600, renews: Some(true) }),
    });
    ui.claude.stats = Some(Arc::new(StatsView { ranges: [range(now, 1), range(now, 7), range(now, 30)], activity: activity(now) }));
    refill_history(ui);
    codex(ui);
    deepseek(ui);
    ui.system = Some(Arc::new(crate::system::demo(now as f64)));
}

/// Codex on ChatGPT Pro, a third of the way into its week, with a reset to spend.
fn codex(ui: &mut Ui) {
    let now = ui.now;
    let limit = |id: &str, label: &str, used: f64, resets_in: i64, window: i64| Limit { id: id.to_owned(), label: label.to_owned(), used, resets_at: Some(now + resets_in), window, scoped: false };
    let snapshot = Snapshot {
        plan: "ChatGPT Pro".to_owned(),
        limits: vec![limit("session", "5-hour", 29.0, 3 * 3600 + 12 * 60, 5 * 3600), limit("weekly", "Weekly", 36.0, 4 * 86_400 + 2 * 3600, 7 * 86_400)],
        extra: None,
        fetched_at: now - 140,
        resets: Some(crate::model::Resets {
            available: 1,
            credits: vec![crate::model::ResetCredit {
                id: "demo-1".to_owned(),
                title: "Weekly reset".to_owned(),
                description: Some("Starts the 5-hour and weekly limits over".to_owned()),
                expires_at: Some(now + 18 * 86_400),
            }],
        }),
        subscription: Some(Subscription { until: now + 23 * 86_400 + 2 * 3600, renews: None }),
    };
    let mut month = range(now, 30);
    let mut week = range(now, 7);
    let mut today = range(now, 1);
    for view in [&mut month, &mut week, &mut today] {
        view.tokens = view.tokens.map(|count| count / 3);
        let total = view.total();
        let cost = view.cost * 0.33;
        view.cost = cost;
        view.models = vec![
            ModelRow { name: "gpt-5.1-codex".to_owned(), tokens: total * 70 / 100, cost: Some(cost * 0.76) },
            ModelRow { name: "gpt-5.1".to_owned(), tokens: total * 23 / 100, cost: Some(cost * 0.2) },
            ModelRow { name: "gpt-5.1-codex-mini".to_owned(), tokens: total * 7 / 100, cost: Some(cost * 0.04) },
        ];
        view.projects = projects(total, cost);
        for bar in &mut view.bars {
            bar.parts = [bar.tokens * 70 / 100, bar.tokens * 23 / 100, bar.tokens - bar.tokens * 93 / 100, 0];
            bar.tokens /= 3;
            bar.parts = bar.parts.map(|part| part / 3);
        }
    }
    let activity = activity(now);
    ui.codex.history = Arc::new(history(&snapshot, &activity));
    ui.codex.snapshot = Some(snapshot);
    ui.codex.stats = Some(Arc::new(StatsView { ranges: [today, week, month], activity }));
}

/// A DeepSeek account with a few dollars, spending a little each day, topped up on Tuesday.
fn deepseek(ui: &mut Ui) {
    let now = ui.now;
    let mut ledger = crate::deepseek::Ledger::default();
    let spends = [0.38, 0.21, 0.62, 0.55, 0.71, 0.44, 0.33];
    let mut total = 2.67 + spends.iter().sum::<f64>();
    let today = time::local(now).day;
    for (at, spend) in spends.iter().enumerate() {
        let start = time::local_hour_start(today - 6 + at as i32, 0);
        ledger.readings.push((start + 3600, total));
        if time::local(start).weekday == 2 {
            total += 5.0;
            ledger.readings.push((start + 7200, total));
        }
        total -= spend;
        ledger.readings.push(((start + 20 * 3600).min(now - 60), total));
    }
    ledger.last = Some(crate::deepseek::Balance { currency: "USD".to_owned(), total: 4.2, granted: 0.2, topped_up: 4.0, available: true, fetched_at: now - 140 });
    ui.deepseek.ledger = Some(Arc::new(ledger));
}

/// The plan as a Pro or Max plan has it: a session limit first, then the weekly ones, each
/// this much used.
pub fn plan(ui: &mut Ui, name: &str, session: f64, weekly: f64, fable: f64) {
    let now = ui.now;
    if let Some(snapshot) = &mut ui.claude.snapshot {
        snapshot.plan = name.to_owned();
        snapshot.limits.retain(|limit| limit.id != "session");
        snapshot.limits.insert(0, Limit { id: "session".to_owned(), label: "Session".to_owned(), used: session, resets_at: Some(now + 6540), window: 5 * 3600, scoped: false });
        for limit in &mut snapshot.limits {
            match limit.id.as_str() {
                "weekly" => limit.used = weekly,
                "weekly-fable" => limit.used = fable,
                _ => {}
            }
        }
    }
    refill_history(ui);
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
    if let (Some(snapshot), Some(stats)) = (&ui.claude.snapshot, &ui.claude.stats) {
        ui.claude.history = Arc::new(history(snapshot, &stats.activity));
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
            let parts = [tokens * 58 / 100, tokens * 29 / 100, tokens * 9 / 100, tokens - tokens * 96 / 100];
            Bar { label, tokens, cost: tokens as f64 * 0.78 / 1e6, parts }
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
        projects: projects(total, cost),
    }
}

/// `total` tokens costing `cost`, spread over seven project folders: three busy ones, then a
/// few small ones, the last taking what rounding left.
fn projects(total: u64, cost: f64) -> Vec<ProjectRow> {
    let shares = [
        ("nerda.bar", "~/Developer", 0.33, 0.348),
        ("gray-area", "~/Developer", 0.23, 0.248),
        ("rune", "~/Developer/oss", 0.16, 0.169),
        ("dotfiles", "~", 0.08, 0.07),
        ("nerda.say", "~/Developer", 0.08, 0.062),
        ("notes", "~/Documents", 0.07, 0.055),
        ("scratch", "/tmp", 0.05, 0.048),
    ];
    let mut rows: Vec<ProjectRow> = shares
        .iter()
        .map(|(name, path, part, cost_part)| ProjectRow { name: (*name).to_owned(), path: (*path).to_owned(), tokens: (total as f64 * part) as u64, cost: Some(cost * cost_part) })
        .collect();
    let (rest, last) = rows.split_at_mut(shares.len() - 1);
    last[0].tokens = total - rest.iter().map(|row| row.tokens).sum::<u64>();
    last[0].cost = Some(cost - rest.iter().filter_map(|row| row.cost).sum::<f64>());
    rows
}
