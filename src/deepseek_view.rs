//! DeepSeek's tab: the balance and what makes it up, how long it lasts at the last week's
//! spend, and the spend per day as the balance fell between refreshes, with top-ups.

use rune::{El, col, el, icon, row, spacer, text};

use crate::agents_view::{mark, problem_banner, signed_out, updated};
use crate::deepseek::{Day, money};
use crate::icons;
use crate::look::{self, INNER_W, Look, PAD_X, caption, digits, figure, font, hairline, key_swatch, section};
use crate::model::{Problem, Provider};
use crate::time::{self, WEEKDAYS};
use crate::ui::Ui;

const BARS_H: f32 = 56.0;

pub fn tab(s: &Ui, l: &Look) -> El {
    let d = &s.deepseek;
    let balance = d.ledger.as_ref().and_then(|ledger| ledger.last.as_ref());
    let words = match (d.refreshing, balance, &d.problem) {
        (true, ..) => "Updating…".to_owned(),
        (_, _, Some(Problem::SignedOut)) => "No API key yet".to_owned(),
        (_, Some(balance), _) => updated(s.now, balance.fetched_at),
        _ => "Not updated yet".to_owned(),
    };
    let stale = balance.is_some() && d.problem.is_some();
    let (glyph, color, tile) = mark(l, Provider::DeepSeek);
    let header = look::header(l, glyph, 20.0, color, tile, "DeepSeek", Some("API"), look::subtitle(l, words, stale.then_some(l.warning_text)), Some(look::refresh_button(l, d.refreshing)));
    let mut body = col().key("deepseek").w_full().child(header);

    match (&d.problem, d.ledger.as_ref(), balance) {
        (Some(Problem::SignedOut), ..) => return body.child(col().w_full().px(PAD_X).pt(8.0).pb(16.0).child(signed_out(l, Provider::DeepSeek))),
        (problem, Some(ledger), Some(balance)) => {
            if let Some(problem) = problem {
                body = body.child(col().w_full().px(PAD_X).pb(4.0).child(problem_banner(s, l, Provider::DeepSeek, problem, None)));
            }
            let currency = balance.currency.as_str();
            let mut lasts = col().items_end().gap(4.0);
            if let Some(per_day) = ledger.daily_spend(s.now) {
                let days = (balance.total / per_day).floor().max(0.0) as i64;
                let words = if days < 1 { "Runs out today".to_owned() } else { format!("Lasts about {days} day{}", if days == 1 { "" } else { "s" }) };
                let chip = if days < 3 { l.warning_text } else { l.text };
                lasts = lasts
                    .child(row().h(22.0).px(9.0).radius(11.0).center().fill(l.chip).child(text(words).font(font(11.5, 600.0)).color(chip)))
                    .child(caption(l, "at last week's spend"));
            } else if !balance.available {
                lasts = lasts.child(text("Too low for API calls").font(font(11.5, 600.0)).color(l.critical_text));
            }
            let total = balance.total.max(f64::MIN_POSITIVE);
            let topped_w = (balance.topped_up / total).clamp(0.0, 1.0) as f32 * INNER_W;
            let mut bar = row().key("balance-bar").w_full().h(8.0).gap(2.0);
            if topped_w >= 2.0 {
                bar = bar.child(el().w((topped_w - 2.0).max(2.0)).h(8.0).radius(3.0).fill(l.series[0]));
            }
            if balance.granted > 0.0 {
                bar = bar.child(el().grow(1.0).min_w(4.0).h(8.0).radius(3.0).fill(l.series[1]));
            }
            let legend = |color, name: &str, amount: f64| {
                row().gap(6.0).center_y().child(key_swatch(color, 8.0)).child(text(name.to_owned()).font(font(11.5, 400.0)).color(l.dim)).child(text(money(amount, currency)).font(digits(11.5, 600.0)).color(l.text))
            };
            let now = col()
                .key("balance")
                .w_full()
                .px(PAD_X)
                .pt(8.0)
                .pb(14.0)
                .gap(10.0)
                .child(
                    row()
                        .w_full()
                        .items_end()
                        .child(col().gap(1.0).child(caption(l, "Balance")).child(text(money(balance.total, currency)).key("balance-total").font(figure(40.0)).color(if balance.available { l.text } else { l.critical_text })))
                        .child(spacer())
                        .child(lasts.pb(6.0)),
                )
                .child(bar)
                .child(row().gap(18.0).child(legend(l.series[0], "Topped up", balance.topped_up)).child(legend(l.series[1], "Granted", balance.granted)));
            body = body.child(now).child(hairline(l)).child(spend(s, l, &ledger.days(s.now, 7), currency));
        }
        (Some(problem), ..) => body = body.child(col().w_full().px(PAD_X).pt(4.0).pb(14.0).child(problem_banner(s, l, Provider::DeepSeek, problem, None))),
        _ => {
            body = body.child(
                row()
                    .key("loading")
                    .w_full()
                    .pt(26.0)
                    .pb(36.0)
                    .gap(8.0)
                    .center()
                    .child(icon(&icons::LOADER_CIRCLE, 14.0).color(l.dim))
                    .child(text("Reading your balance…").font(font(12.5, 400.0)).color(l.dim)),
            )
        }
    }
    body
}

/// The week's spend, a bar a day with its amount over it, and the top-ups under it.
fn spend(s: &Ui, l: &Look, days: &[Day], currency: &str) -> El {
    let total: f64 = days.iter().map(|day| day.spent).sum();
    let peak = days.iter().map(|day| day.spent).fold(0.0, f64::max).max(f64::MIN_POSITIVE);
    let title = row()
        .w_full()
        .items_start()
        .child(col().gap(2.0).child(text("Spent per day").font(font(13.0, 700.0)).color(l.text)).child(caption(l, "From balance drops between refreshes")))
        .child(spacer())
        .child(col().items_end().gap(1.0).child(caption(l, "Last 7 days")).child(text(money(total, currency)).font(figure(18.0)).color(l.text)));
    let last = days.len().saturating_sub(1);
    let mut bars = row().key("spend").w_full().h(BARS_H + 18.0).gap(8.0).items_end();
    let mut axis = row().w_full().gap(8.0);
    for (at, day) in days.iter().enumerate() {
        let today = at == last;
        let height = if day.spent > 0.0 { ((day.spent / peak) as f32 * BARS_H).max(3.0) } else { 2.0 };
        let label = if day.spent > 0.0 { money(day.spent, currency) } else { String::new() };
        bars = bars.child(
            col()
                .key(format!("spend-{at}"))
                .grow(1.0)
                .min_w(0.0)
                .gap(4.0)
                .items_center()
                .child(text(label).font(digits(10.0, if today { 600.0 } else { 400.0 })).color(if today { l.text } else { l.dim }))
                .child(el().w_full().h(height).radius(3.0).fill(if today { l.series[0] } else if day.spent > 0.0 { l.faint } else { l.track })),
        );
        let name = if today { "Today".to_owned() } else { WEEKDAYS[time::local(day.start).weekday as usize].to_owned() };
        axis = axis.child(row().grow(1.0).min_w(0.0).center().child(text(name).font(digits(10.5, if today { 700.0 } else { 400.0 })).color(if today { l.text } else { l.faint })));
    }
    let mut section = section("deepseek-spend").child(title).child(col().w_full().gap(5.0).child(bars).child(axis));
    for day in days.iter().filter(|day| day.topped_up > 0.0) {
        section = section.child(
            row()
                .key("top-up")
                .w_full()
                .gap(8.0)
                .center_y()
                .child(icon(&icons::CIRCLE_PLUS, 13.0).color(l.good))
                .child(text(format!("Topped up on {}", WEEKDAYS[time::local(day.start).weekday as usize])).font(font(12.5, 400.0)).color(l.text))
                .child(spacer())
                .child(text(format!("+{}", money(day.topped_up, currency))).font(digits(12.5, 600.0)).color(l.good)),
        );
    }
    let _ = s;
    section.child(text("DeepSeek reports only the balance, so there are no limits or per-model usage to show.").font(font(11.0, 400.0).lh(15.0)).color(l.faint).w_full().wrap().pt(6.0))
}
