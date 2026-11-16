//! A provider's tab: its header, what stands between it and current numbers, a ring per limit
//! (a click puts that limit in the chart), extra usage, the picked limit's chart (in
//! `burn_view`), then usage from the session logs: totals, a chart split by model and a row per
//! model.

use rune::{Color, El, LineCap, Path, Shape, canvas, col, el, icon, row, spacer, text};

use crate::burn_view;
use crate::icons;
use crate::look::{self, INNER_W, Look, PAD_X, caption, digits, figure, font, hairline, section, section_title, segment, segments};
use crate::model::{self, ExtraUsage, Limit, Problem, Provider, Snapshot};
use crate::stats::{CACHE_READ, CACHE_WRITE_1H, CACHE_WRITE_5M, INPUT, OUTPUT, RangeView, SERIES};
use crate::time;
use crate::ui::{self, Range, Ui};

const RING: f32 = 34.0;
const RING_WIDTH: f32 = 4.5;
const BARS_H: f32 = 52.0;
const MODEL_ROWS: usize = 5;
/// What stale numbers fade to while a banner says why they are not current.
const STALE: f32 = 0.45;

/// A provider with plan limits: Claude or Codex.
pub fn tab(s: &Ui, l: &Look, provider: Provider) -> El {
    let a = s.agent(provider);
    let limits = a.limits(s.now);
    let stale = limits.is_some() && a.problem.as_ref().is_some_and(|p| !matches!(p, Problem::SignedOut));
    let mut tab = col().key(provider.id()).w_full().child(header(s, l, provider, limits.as_ref(), stale));

    match (&limits, &a.problem) {
        (_, Some(Problem::SignedOut)) => tab = tab.child(col().w_full().px(PAD_X).pt(8.0).pb(16.0).child(signed_out(l, provider))),
        (None, None) => tab = tab.child(loading(l)),
        (None, Some(problem)) => tab = tab.child(col().w_full().px(PAD_X).pt(4.0).pb(14.0).child(problem_banner(s, l, provider, problem, None))),
        (Some(snapshot), problem) => {
            if let Some(problem) = problem {
                tab = tab.child(col().w_full().px(PAD_X).pb(4.0).child(problem_banner(s, l, provider, problem, Some(snapshot))));
            }
            let fade = if stale { STALE } else { 1.0 };
            let charted = a.chart_limit(snapshot);
            let any_scoped = snapshot.limits.iter().any(|l| l.scoped);
            let mut rows = col().key("limits").w_full().px(PAD_X - 8.0).pt(8.0).pb(6.0).gap(2.0).opacity(fade);
            for limit in &snapshot.limits {
                let on = charted.is_some_and(|c| c.id == limit.id);
                rows = rows.child(limit_row(s, l, provider, limit, any_scoped, on));
            }
            tab = tab.child(rows);
            if let Some(extra) = &snapshot.extra {
                tab = tab.child(extra_row(l, extra).opacity(fade));
            }
            if let Some(resets) = snapshot.resets.as_ref().filter(|resets| resets.available > 0) {
                tab = tab.child(hairline(l)).child(crate::resets_view::section(s, l, snapshot, resets).opacity(fade));
            }
            if let Some(limit) = charted {
                tab = tab.child(hairline(l)).child(burn_view::section(s, l, provider, snapshot, limit).opacity(fade));
            }
        }
    }
    tab.child(hairline(l)).child(usage(s, l, provider))
}

fn header(s: &Ui, l: &Look, provider: Provider, limits: Option<&Snapshot>, stale: bool) -> El {
    let a = s.agent(provider);
    let plan = limits.map(|snapshot| snapshot.plan.as_str());
    let (words, warn) = match (&a.problem, limits, a.refreshing) {
        (_, _, true) => ("Updating…".to_owned(), false),
        (Some(Problem::SignedOut), ..) => ("Usage from this Mac's logs".to_owned(), false),
        (_, Some(snapshot), _) => (updated(s.now, snapshot.fetched_at), stale),
        (_, None, _) => ("Not updated yet".to_owned(), false),
    };
    let subtitle = look::subtitle(l, words, warn.then_some(l.warning_text));
    let (mark, color, tile) = mark(l, provider);
    look::header(l, mark, 20.0, color, tile, provider.name(), plan, subtitle, Some(look::refresh_button(l, a.refreshing)))
}

/// A provider's mark, its colour and its header tile's colour.
pub(crate) fn mark(l: &Look, provider: Provider) -> (&'static rune::Icon, Color, Color) {
    match provider {
        Provider::Claude => (&icons::CLAUDE, l.brand, l.brand.alpha(0.12)),
        Provider::Codex => (&icons::OPENAI, l.text, l.chip),
        Provider::DeepSeek => (&icons::DEEPSEEK, l.deepseek, l.deepseek.alpha(0.14)),
    }
}

/// How long ago the limits were read: "Updated just now", "Updated 2m ago".
pub(crate) fn updated(now: i64, at: i64) -> String {
    let age = now - at;
    if age < 60 { "Updated just now".to_owned() } else { format!("Updated {} ago", time::span(age)) }
}

fn loading(l: &Look) -> El {
    row()
        .key("loading")
        .w_full()
        .pt(26.0)
        .pb(36.0)
        .gap(8.0)
        .center()
        .child(icon(&icons::LOADER_CIRCLE, 14.0).color(l.dim))
        .child(text("Reading your limits…").font(font(12.5, 400.0)).color(l.dim))
}

/// No login to read: what to run, with a button that copies it.
pub(crate) fn signed_out(l: &Look, provider: Provider) -> El {
    let (detail, command, note) = match provider {
        Provider::Codex => ("Nerda Bar reads the Codex CLI's ChatGPT login to show your plan's limits. Sign in once in a terminal:", "codex login", "Your login is only read, never changed."),
        Provider::DeepSeek => ("Nerda Bar reads your DeepSeek API key from the Keychain to show your balance. Add it once in a terminal:", "security add-generic-password -s \"Nerda Bar\" -a deepseek -w", "The key stays in your Keychain and is sent only to DeepSeek."),
        Provider::Claude => ("Nerda Bar reads Claude Code's login to show your plan's limits. Sign in once in a terminal:", "claude", "Your login is only read, never changed."),
    };
    let copy = row()
        .key("copy-command")
        .role("button")
        .label("Copy the command")
        .gap(4.0)
        .center_y()
        .hover(l.hover)
        .on_press_tx(move |_: &mut Ui, tx| tx.copy(command))
        .child(icon(&icons::COPY, 12.0).color(l.dim))
        .child(text("Copy").font(font(11.0, 400.0)).color(l.dim));
    col()
        .key("problem")
        .w_full()
        .padding(14.0)
        .gap(8.0)
        .radius(12.0)
        .continuous_corners()
        .fill(l.card)
        .border(1.0, l.stroke)
        .child(row().gap(8.0).center_y().child(icon(&icons::LOG_IN, 15.0).color(l.warning)).child(text("Not signed in").font(font(13.0, 700.0)).color(l.text)))
        .child(
            text(detail)
                .font(font(11.5, 400.0).lh(16.0))
                .color(l.dim)
                .w_full()
                .wrap(),
        )
        .child(
            row()
                .key("command")
                .w_full()
                .py(7.0)
                .px(10.0)
                .radius(7.0)
                .fill(Color::BLACK.alpha(if l.dark { 0.25 } else { 0.06 }))
                .center_y()
                .child(text(command).font(font(12.0, 400.0).mono()).color(l.text).truncate().grow(1.0).min_w(0.0))
                .child(spacer())
                .child(copy),
        )
        .child(text(note).font(font(11.0, 400.0)).color(l.faint))
}

/// Why the numbers on screen are not current, and when they will be.
pub(crate) fn problem_banner(s: &Ui, l: &Look, provider: Provider, problem: &Problem, snapshot: Option<&Snapshot>) -> El {
    let retry = |seconds: i64| format!("Trying again in {}.", time::span(seconds.max(60)));
    let problem_at = if provider == Provider::DeepSeek { s.deepseek.problem_at } else { s.agent(provider).problem_at };
    let since = s.now - problem_at;
    let (glyph, detail) = match problem {
        Problem::LoginExpired => (
            &icons::KEY_ROUND,
            match provider {
                Provider::Codex => "Run `codex login` to renew it. Nerda Bar never changes your login.".to_owned(),
                Provider::DeepSeek => "DeepSeek turned the key down. Add a valid key to the Keychain.".to_owned(),
                Provider::Claude => problem.detail(s.now),
            },
        ),
        Problem::RateLimited { until } => (&icons::CIRCLE_PAUSE, format!("{} asked for a pause. {}", provider.name(), retry(until - s.now))),
        Problem::Offline(_) => (
            &icons::WIFI_OFF,
            match snapshot {
                Some(snapshot) => format!("The limits below are from {}. They update when the Mac is back online.", time::clock(snapshot.fetched_at, false)),
                None => "Your limits show once the Mac is back online.".to_owned(),
            },
        ),
        Problem::Failed(reason) => (&icons::TRIANGLE_ALERT, format!("{reason} {}", retry(s.config.refresh_seconds() - since))),
        Problem::SignedOut => (&icons::LOG_IN, problem.detail(s.now)),
    };
    look::banner(l, "problem", glyph, problem.title(), detail)
}

/// A limit: a ring of what is left with a tick at an even pace, its name and when it resets,
/// and the percentage. The one in the chart sits on a card; a click on another charts it.
fn limit_row(s: &Ui, l: &Look, provider: Provider, limit: &Limit, any_scoped: bool, on: bool) -> El {
    let (now, show_used) = (s.now, s.show_used(provider));
    let severity = limit.severity();
    let (name, scope) = model::names(limit, any_scoped);
    let mut title = row().gap(4.0).center_y().child(text(name).font(font(12.5, 600.0)).color(l.text));
    if let Some(scope) = scope {
        title = title.child(text(scope).font(font(12.5, 400.0)).color(l.dim));
    }
    let mut words = col().grow(1.0).min_w(0.0).gap(2.0).child(title).child(text(reset_note(limit, now)).font(digits(11.0, 400.0)).color(l.faint));
    if let Some((warning, color)) = runway(l, limit, now) {
        words = words.child(text(warning).font(digits(11.0, 600.0)).color(color));
    }
    let share = if show_used { limit.used / 100.0 } else { limit.left() / 100.0 };
    // The tick marks an even pace: where the ring would end if use had kept step with time.
    let elapsed = limit.elapsed(now).filter(|_| !limit.used_up());
    let tick = elapsed.map(|elapsed| if show_used { elapsed } else { 1.0 - elapsed });
    let value = col()
        .items_end()
        .child(text(model::percent(limit, show_used)).font(figure(16.0).tracking(-0.3)).color(l.ink(severity)))
        .child(text(if show_used { "used" } else { "left" }).font(font(10.5, 400.0)).color(l.faint));
    let id = limit.id.clone();
    let mut body = row()
        .key(format!("limit-{}", limit.id))
        .role("button")
        .label(format!("{}: {}", limit.label, model::percent(limit, show_used)))
        .w_full()
        .padding(8.0)
        .gap(12.0)
        .center_y()
        .radius(10.0)
        .continuous_corners()
        .child(ring(l, share, l.fill(severity), tick))
        .child(words)
        .child(value);
    if let Some(elapsed) = elapsed {
        let pace = (1.0 - elapsed) * 100.0;
        let even = if show_used { 100.0 - pace } else { pace };
        let word = if show_used { "used" } else { "left" };
        let verdict = match (limit.left() < pace - 1.0, limit.left() > pace + 1.0) {
            (true, _) => "so you're using it faster than that.",
            (_, true) => "so you're using it slower than that.",
            _ => "about right.",
        };
        let window = if limit.window <= 86_400 { "the session" } else { "it" };
        body = body.help(format!("Even pace: {:.0}% {word} by now", even)).help_row(format!(
            "Using {window} evenly until it resets would leave {:.0}% now. You have {:.0}%, {verdict}",
            pace,
            limit.left().round(),
        ));
    }
    if on {
        body.fill(l.card).border(1.0, l.stroke)
    } else {
        body.hover(l.hover).on_press(move |s: &mut Ui| ui::select_limit(s, &id))
    }
}

/// When a limit resets: "Resets in 2d 4h".
pub(crate) fn reset_note(limit: &Limit, now: i64) -> String {
    match limit.resets_in(now) {
        Some(left) => format!("Resets in {}", time::span(left)),
        None if limit.used <= 0.0 => "Starts with your next message".to_owned(),
        None => "Resetting".to_owned(),
    }
}

/// A warning that the limit is used up, or will be before it resets at the pace so far: within
/// a day as a countdown, further off as the day and time.
pub(crate) fn runway(l: &Look, limit: &Limit, now: i64) -> Option<(String, Color)> {
    if limit.used_up() {
        return Some(("Used up".to_owned(), l.critical_text));
    }
    let runway = limit.runs_out_in(now)?;
    let words = if runway < 86_400 && limit.window <= 86_400 { format!("Runs out in {}", time::span(runway)) } else { format!("Runs out {}", time::weekday_clock(now + runway)) };
    Some((words, if limit.severity() == model::Severity::Critical { l.critical_text } else { l.warning_text }))
}

/// A ring filled clockwise from the top to `share`, with a tick across it at `tick`.
fn ring(l: &Look, share: f64, color: Color, tick: Option<f64>) -> El {
    let (c, r) = (RING / 2.0, RING / 2.0 - 3.5);
    let at = |turn: f32, radius: f32| {
        let angle = turn * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
        [c + radius * angle.cos(), c + radius * angle.sin()]
    };
    let arc = |from: f32, to: f32| {
        let steps = ((to - from) * 96.0).ceil().max(1.0) as usize;
        let mut path = Path::new().move_to(at(from, r));
        for step in 1..=steps {
            path = path.line_to(at(from + (to - from) * step as f32 / steps as f32, r));
        }
        path
    };
    let mut shape = Shape::new().stroke_path(&arc(0.0, 1.0), RING_WIDTH, LineCap::Butt, l.track);
    let share = share.clamp(0.0, 1.0) as f32;
    if share > 0.0 {
        shape = shape.stroke_path(&arc(0.0, share.max(0.005)), RING_WIDTH, LineCap::Round, color);
    }
    if let Some(tick) = tick {
        let turn = tick.clamp(0.0, 1.0) as f32;
        shape = shape.line(at(turn, r - 4.0), at(turn, r + 4.0), 1.5, LineCap::Round, l.tick);
    }
    canvas(shape).key("ring").size(RING, RING)
}

fn extra_row(l: &Look, extra: &ExtraUsage) -> El {
    let amount = |value: f64| if extra.currency == "USD" { model::money(value) } else { format!("{value:.2} {}", extra.currency) };
    let mut value = row().gap(3.0).center_y().child(text(amount(extra.used)).font(digits(12.0, 700.0)).color(l.text));
    if let Some(limit) = extra.limit {
        value = value.child(text(format!("of {}", amount(limit).trim_end_matches(".00"))).font(digits(12.0, 400.0)).color(l.dim));
    }
    let mut name = row().gap(6.0).center_y().child(text("Extra usage").font(font(12.0, 600.0)).color(l.dim));
    if let Some(resets) = extra.resets_at {
        name = name.child(text(format!("resets {}", time::month_day(resets))).font(font(11.0, 400.0)).color(l.faint));
    }
    let mut body = col().key("limit-extra").w_full().px(PAD_X).pt(6.0).pb(12.0).gap(6.0).child(row().w_full().center_y().child(name).child(spacer()).child(value));
    if let Some(limit) = extra.limit {
        let spent = extra.used / limit;
        let fill = if spent >= 0.9 { l.critical } else if spent >= 0.7 { l.warning } else { l.normal };
        body = body.child(look::meter(l, "meter", INNER_W, 4.0, spent, fill));
    }
    body
}

fn usage(s: &Ui, l: &Look, provider: Provider) -> El {
    let mut ranges = segments(l, "ranges");
    for range in Range::ALL {
        ranges = ranges.child(segment(l, format!("range-{}", range.label()), range.label(), s.range == range, move |s: &mut Ui| ui::set_range(s, range)));
    }
    let body = section("usage").child(section_title(l, "Usage", Some(ranges)));

    let tool = if provider == Provider::Codex { "the Codex CLI" } else { "Claude Code" };
    let Some(stats) = &s.agent(provider).stats else {
        return body.child(text(format!("Reading {tool}'s session logs…")).key("usage-loading").font(font(12.0, 400.0)).color(l.dim));
    };
    let view = &stats.ranges[s.range.index()];
    if view.total() == 0 {
        return body.child(text(format!("No {tool} activity in this period.")).key("usage-empty").font(font(12.0, 400.0)).color(l.dim));
    }

    // One reading: the range's totals, or the bar under the pointer. It keeps its place and its
    // shape, so moving across the chart changes only the words in it.
    let hovered = s.hovered.and_then(|at| view.bars.get(at));
    let (when, tokens, cost) = match hovered {
        Some(bar) => (bar.label.clone(), bar.tokens, model::money(bar.cost)),
        None => (format!("Tokens, {}", s.range.span().to_lowercase()), view.total(), view.cost_label()),
    };
    let totals = row()
        .key("totals")
        .w_full()
        .items_end()
        .help(format!("Input {}", model::tokens(view.tokens[INPUT])))
        .help_row(format!("Output {}", model::tokens(view.tokens[OUTPUT])))
        .help_row(format!("Cache write {}", model::tokens(view.tokens[CACHE_WRITE_5M] + view.tokens[CACHE_WRITE_1H])))
        .help_row(format!("Cache read {}", model::tokens(view.tokens[CACHE_READ])))
        .child(col().child(caption(l, when).key("totals-when")).child(text(model::tokens(tokens)).key("total-tokens").font(figure(24.0)).color(l.text)))
        .child(spacer())
        .child(col().items_end().child(caption(l, "At API prices")).child(text(cost).key("total-cost").font(figure(24.0)).color(l.text)));
    body.child(totals).child(chart(l, view, s.hovered)).child(models(l, view))
}

/// A model's colour: the first three in the list each have one, the rest share grey.
fn series(l: &Look, at: usize) -> Color {
    if at < SERIES { l.series[at] } else { l.faint }
}

fn chart(l: &Look, view: &RangeView, hovered: Option<usize>) -> El {
    let peak = view.bars.iter().map(|bar| bar.tokens).max().unwrap_or(0).max(1) as f32;
    let few = view.bars.len() <= 7;
    let gap = if few { 8.0 } else { 2.0 };
    let mut bars = row()
        .key("chart")
        .w_full()
        .h(BARS_H)
        .gap(gap)
        // Leaving the chart lets go of the bar; crossing the gap between two bars doesn't.
        .on_hover_change(|s: &mut Ui, inside, _| {
            if !inside {
                ui::leave_chart(s);
            }
        });
    let last = view.bars.len().saturating_sub(1);
    for (at, bar) in view.bars.iter().enumerate() {
        // With a bar picked out, the others step back.
        let fade = if hovered.is_some_and(|h| h != at) { 0.4 } else { 1.0 };
        let mut stack = col().w_full().gap(1.0).clip_rounded(if few { 3.0 } else { 1.5 }).opacity(fade);
        if bar.tokens == 0 {
            stack = stack.child(el().w_full().h(2.0).fill(l.track));
        } else {
            let height = (bar.tokens as f32 / peak * BARS_H).max(3.0);
            // Top down: the rest, then the third, second and first model at the base.
            let parts: Vec<(usize, u64)> = bar.parts.iter().copied().enumerate().rev().filter(|(_, tokens)| *tokens > 0).collect();
            let gaps = parts.len().saturating_sub(1) as f32;
            for (part, tokens) in parts {
                let h = ((tokens as f32 / bar.tokens as f32) * (height - gaps)).max(1.0);
                stack = stack.child(el().w_full().h(h).fill(series(l, part)));
            }
        }
        // The whole column answers the pointer, so a short bar is as easy to reach as a tall one.
        bars = bars.child(
            col()
                .key(format!("bar-{at}"))
                .grow(1.0)
                .min_w(0.0)
                .h(BARS_H)
                .justify_end()
                .on_hover_change(move |s: &mut Ui, inside, _| {
                    if inside {
                        ui::hover_bar(s, at);
                    }
                })
                .child(stack),
        );
    }
    let label = |name: &str, strong: bool| text(name.to_owned()).font(digits(10.0, if strong { 700.0 } else { 400.0 })).color(if strong { l.text } else { l.faint });
    let mut axis = row().key("axis").w_full();
    if view.axis_per_bar {
        axis = axis.gap(gap);
        for (at, name) in view.axis.iter().enumerate() {
            let (name, strong) = if at == last && hovered.is_none_or(|h| h == at) { ("Today", true) } else { (name.as_str(), hovered == Some(at)) };
            axis = axis.child(row().grow(1.0).min_w(0.0).center().child(label(name, strong)));
        }
    } else {
        axis = axis.justify_between();
        let count = view.axis.len();
        for (at, name) in view.axis.iter().enumerate() {
            // The last date of a month is today: it reads as the bars' own "Today" does.
            axis = axis.child(label(name, at + 1 == count && count == 3));
        }
    }
    col().w_full().gap(5.0).child(bars).child(axis)
}

fn models(l: &Look, view: &RangeView) -> El {
    let mut list = col().key("models").w_full();
    for (at, model) in view.models.iter().take(MODEL_ROWS).enumerate() {
        list = list.child(
            row()
                .key(format!("model-{at}"))
                .w_full()
                .py(5.0)
                .gap(8.0)
                .center_y()
                .child(look::key_swatch(series(l, at), 8.0))
                .child(text(model.name.as_str()).font(font(12.5, 400.0)).color(l.text).truncate().grow(1.0).min_w(0.0))
                .child(row().w(52.0).justify_end().child(text(model::tokens(model.tokens)).font(digits(12.0, 400.0)).color(l.dim)))
                .child(row().w(56.0).justify_end().child(text(model.cost.map_or_else(|| "—".to_owned(), model::money)).font(digits(12.5, 600.0)).color(if model.cost.is_some() { l.text } else { l.faint }))),
        );
    }
    if view.models.len() > MODEL_ROWS {
        let rest = &view.models[MODEL_ROWS..];
        let tokens: u64 = rest.iter().map(|m| m.tokens).sum();
        list = list.child(caption(l, format!("{} more models, {} tokens", rest.len(), model::tokens(tokens))).pt(2.0));
    }
    list
}
