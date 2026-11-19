//! DeepSeek's tab: the balance and what makes it up, how long it lasts at the last week's
//! spend and the top-ups in the left column; the spend per day as the balance fell between
//! refreshes in the right one.

use rune::{El, col, el, icon, row, spacer, text};

use crate::agents_view::{BANNER_H, problem_banner, updated};
use crate::deepseek::{Day, Ledger, money};
use crate::icons;
use crate::look::{self, BODY_H, GAP, INNER_W, Look, PAD_Y, ROW_H, TITLE_H, caption, digits, figure, font, key_swatch, next_section, section, section_title};
use crate::model::{Problem, Provider};
use crate::time::{self, WEEKDAYS};
use crate::ui::Ui;

/// A day's amount over its bar, and the day's name under the bars.
const LABEL_H: f32 = 14.0;
const AXIS_H: f32 = 14.0;
/// The spend's title: what it is, over the week's total.
const SPEND_HEAD_H: f32 = 14.0 + 2.0 + 26.0;

/// The quiet line under DeepSeek's plan in the context bar, and whether it warns.
pub(crate) fn status_words(s: &Ui) -> (String, bool) {
    let d = &s.deepseek;
    let balance = d.ledger.as_ref().and_then(|ledger| ledger.last.as_ref());
    match (d.refreshing, balance, &d.problem) {
        (true, ..) => ("Updating…".to_owned(), false),
        (_, Some(balance), problem) => (updated(s.now, balance.fetched_at), problem.is_some()),
        (_, None, Some(_)) => ("Not updated".to_owned(), true),
        _ => ("Reading…".to_owned(), false),
    }
}

pub fn body(s: &Ui, l: &Look) -> El {
    let d = &s.deepseek;
    let problem = d.problem.as_ref().filter(|p| !matches!(p, Problem::SignedOut));
    let height = BODY_H - if problem.is_some() { BANNER_H } else { 0.0 };
    let (left, right) = match d.ledger.as_ref().filter(|ledger| ledger.last.is_some()) {
        Some(ledger) => {
            let days = ledger.days(s.now, 7);
            let currency = ledger.last.as_ref().map_or("USD", |balance| balance.currency.as_str()).to_owned();
            (balance_column(s, l, ledger, &days, &currency), spend(l, &days, &currency, height))
        }
        None => {
            let reading = section("balance").child(section_title(l, "Balance", None)).child(caption(l, if problem.is_some() { "The balance shows once DeepSeek answers." } else { "Reading your balance…" }));
            (reading, col())
        }
    };
    let mut body = col().key("deepseek").w_full().h(BODY_H);
    if let Some(problem) = problem {
        body = body.child(problem_banner(s, l, Provider::DeepSeek, problem, None));
    }
    body.child(look::columns_in(l, height, left, right))
}

/// The balance: what it is, how long it lasts, what it is made of; then the week's top-ups.
fn balance_column(s: &Ui, l: &Look, ledger: &Ledger, days: &[Day], currency: &str) -> El {
    let Some(balance) = ledger.last.as_ref() else { return col() };
    let mut lasts = col().items_end().gap(4.0);
    if let Some(per_day) = ledger.daily_spend(s.now) {
        let days = (balance.total / per_day).floor().max(0.0) as i64;
        let words = if days < 1 { "Runs out today".to_owned() } else { format!("Lasts about {days} day{}", if days == 1 { "" } else { "s" }) };
        let color = if days < 3 { l.warning_text } else { l.text };
        lasts = lasts.child(row().h(22.0).px(8.0).radius(11.0).center().fill(l.chip).child(text(words).font(font(11.0, 600.0)).color(color))).child(caption(l, "at last week's spend"));
    } else if !balance.available {
        lasts = lasts.child(text("Too low for API calls").font(font(12.0, 600.0)).color(l.critical_text));
    }
    let total = balance.total.max(f64::MIN_POSITIVE);
    let topped_w = (balance.topped_up / total).clamp(0.0, 1.0) as f32 * INNER_W;
    let mut bar = row().key("balance-bar").w_full().h(8.0).gap(2.0);
    if topped_w >= 2.0 {
        bar = bar.child(el().w((topped_w - 2.0).max(2.0)).h(8.0).radius(4.0).fill(l.series[0]));
    }
    if balance.granted > 0.0 {
        bar = bar.child(el().grow(1.0).min_w(4.0).h(8.0).radius(4.0).fill(l.series[1]));
    }
    let legend = |color, name: &str, amount: f64| {
        row().gap(6.0).center_y().child(key_swatch(color, 8.0)).child(text(name.to_owned()).font(font(12.0, 400.0)).color(l.dim)).child(text(money(amount, currency)).font(digits(12.0, 600.0)).color(l.text))
    };
    let now = section("balance")
        .child(
            row()
                .w_full()
                .items_end()
                .child(col().gap(2.0).child(caption(l, "Balance")).child(text(money(balance.total, currency)).key("balance-total").font(figure(34.0)).color(if balance.available { l.text } else { l.critical_text })))
                .child(spacer())
                .child(lasts),
        )
        .child(bar)
        .child(row().gap(16.0).child(legend(l.series[0], "Topped up", balance.topped_up)).child(legend(l.series[1], "Granted", balance.granted)));

    let mut top_ups = col().key("top-ups").w_full();
    let mut any = false;
    for day in days.iter().filter(|day| day.topped_up > 0.0) {
        any = true;
        top_ups = top_ups.child(
            row()
                .key("top-up")
                .w_full()
                .h(ROW_H)
                .gap(8.0)
                .center_y()
                .child(icon(&icons::CIRCLE_PLUS, 13.0).color(l.good))
                .child(text(format!("Topped up on {}", WEEKDAYS[time::local(day.start).weekday as usize])).font(font(12.0, 400.0)).color(l.text))
                .child(spacer())
                .child(text(format!("+{}", money(day.topped_up, currency))).font(digits(12.0, 600.0)).color(l.good_text)),
        );
    }
    if !any {
        top_ups = top_ups.child(row().h(ROW_H).center_y().child(caption(l, "No top-ups in the last 7 days.")));
    }
    let line = |key: &'static str, name: &str, value: String| row().key(key).w_full().h(ROW_H).center_y().border_sides([0.0, 0.0, 1.0, 0.0], l.stroke).child(text(name.to_owned()).font(font(12.0, 400.0)).color(l.dim)).child(spacer()).child(text(value).font(digits(12.0, 600.0)).color(l.text));
    let mut week = col().key("week").w_full();
    if let Some(per_day) = ledger.daily_spend(s.now) {
        week = week.child(line("week-average", "Spent a day, on average", money(per_day, currency)));
    }
    let history = next_section(l, "deepseek-week")
        .grow(1.0)
        .child(section_title(l, "This week", Some(caption(l, "Last 7 days"))))
        .child(week.child(top_ups))
        .child(spacer())
        .child(text("DeepSeek reports only the balance, so there are no limits or per-model usage to show.").font(font(11.0, 400.0)).color(l.faint).w_full().wrap());
    col().key("balance-column").w_full().h_full().child(now).child(history)
}

/// The week's spend, a bar a day with its amount over it, the bars taking the column's height.
fn spend(l: &Look, days: &[Day], currency: &str, height: f32) -> El {
    let total: f64 = days.iter().map(|day| day.spent).sum();
    let peak = days.iter().map(|day| day.spent).fold(0.0, f64::max).max(f64::MIN_POSITIVE);
    let head = col().w_full().h(SPEND_HEAD_H).gap(2.0).child(caption(l, "Last 7 days")).child(text(money(total, currency)).font(figure(22.0)).color(l.text));
    let bars_h = (height - 2.0 * PAD_Y - TITLE_H - GAP - SPEND_HEAD_H - GAP - LABEL_H - 4.0 - 4.0 - AXIS_H).max(40.0);
    let last = days.len().saturating_sub(1);
    let mut bars = row().key("spend").w_full().h(bars_h + LABEL_H + 4.0).gap(8.0).items_end();
    let mut axis = row().w_full().h(AXIS_H).gap(8.0);
    for (at, day) in days.iter().enumerate() {
        let today = at == last;
        let bar_h = if day.spent > 0.0 { ((day.spent / peak) as f32 * bars_h).max(3.0) } else { 2.0 };
        let label = if day.spent > 0.0 { money(day.spent, currency) } else { String::new() };
        bars = bars.child(
            col()
                .key(format!("spend-{at}"))
                .grow(1.0)
                .min_w(0.0)
                .gap(4.0)
                .items_center()
                .child(text(label).font(digits(11.0, if today { 600.0 } else { 400.0 })).color(if today { l.text } else { l.dim }))
                .child(el().w_full().h(bar_h).radius(3.0).fill(if today { l.series[0] } else if day.spent > 0.0 { l.series_rest } else { l.track })),
        );
        let name = if today { "Today".to_owned() } else { WEEKDAYS[time::local(day.start).weekday as usize].to_owned() };
        axis = axis.child(row().grow(1.0).min_w(0.0).center().child(text(name).font(digits(11.0, if today { 600.0 } else { 400.0 })).color(if today { l.text } else { l.faint })));
    }
    section("deepseek-spend").h(height).child(section_title(l, "Spent per day", Some(caption(l, "From balance drops between refreshes")))).child(head).child(col().w_full().gap(4.0).child(bars).child(axis))
}
