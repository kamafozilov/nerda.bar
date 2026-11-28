//! The Agents tab: a tab per signed-in agent in the context bar, then for an agent with plan
//! limits (Claude, Codex) the limits (a click puts one in the chart), extra usage, free resets
//! and the picked limit's chart (in `burn_view`) in the left column, and in the right one usage
//! from the session logs: totals, a chart split by model, a row per model and the projects that
//! spent the most. DeepSeek's tab is in `deepseek_view`; no agent yet, and adding one, are in
//! `add_view`.

use rune::{Color, El, LineCap, Path, Shape, canvas, col, el, icon, row, spacer, text};

use crate::burn_view;
use crate::icons;
use crate::look::{self, BODY_H, GAP, INNER_W, Look, PAD_X, PAD_Y, ROW_H, TALL_ROW_H, TITLE_H, caption, cell, digits, figure, font, next_section, section, section_title, segment, segments};
use crate::model::{self, ExtraUsage, Limit, Problem, Provider, Snapshot, Subscription};
use crate::stats::{CACHE_READ, CACHE_WRITE_1H, CACHE_WRITE_5M, INPUT, OUTPUT, RangeView};
use crate::time;
use crate::ui::{self, Range, Tab, Ui};
use crate::view::Screen;

const RING: f32 = 32.0;
const RING_WIDTH: f32 = 4.0;
/// Between two limit rows.
const LIMIT_GAP: f32 = 4.0;
/// Extra usage: its line, a gap and the meter under it, with a little room over them.
const EXTRA_H: f32 = 4.0 + 16.0 + 8.0 + 4.0;
/// The usage totals: a caption over a figure.
const TOTALS_H: f32 = 14.0 + 2.0 + 26.0;
const BARS_H: f32 = 48.0;
const AXIS_H: f32 = 14.0;
/// Model rows under the usage chart: the three with their own colour, then the rest.
const MODEL_ROWS: usize = 4;
/// The strip that says why the numbers aren't current.
pub(crate) const BANNER_H: f32 = 40.0;
const NAME_MAX: f32 = INNER_W - 12.0 - 56.0 - 64.0 - 3.0 * 8.0 - 40.0;

/// The Agents tab, on the agent it shows, or how to connect one when there is none.
pub fn screen(s: &Ui, l: &Look) -> Screen {
    let agents = s.agents();
    if agents.is_empty() {
        return crate::add_view::no_agents(s, l);
    }
    let tab = if agents.contains(&s.tab) { s.tab } else { s.current_agent() };
    let context = context(s, l, &agents, tab);
    let body = match tab.provider() {
        Some(Provider::DeepSeek) => crate::deepseek_view::body(s, l),
        Some(provider) => body(s, l, provider),
        None => col(),
    };
    Screen { context, body }
}

pub fn add_popover(s: &Ui, l: &Look) -> El {
    crate::add_view::popover(s, l)
}

/// The agents' tabs and Add, then the shown agent's plan, when it was read and Refresh.
fn context(s: &Ui, l: &Look, agents: &[Tab], shown: Tab) -> El {
    let mut tabs = row().key("agent-tabs").gap(4.0).center_y();
    for &tab in agents {
        tabs = tabs.child(agent_tab(l, tab, tab == shown));
    }
    tabs = tabs.child(el().w(4.0)).child(plus_button(l, s.adding));
    let provider = shown.provider().unwrap_or(Provider::Claude);
    let (plan, (words, warn), refreshing) = match provider {
        Provider::DeepSeek => (Some(plan_line(l, "API balance".to_owned(), None, s.now)), crate::deepseek_view::status_words(s), s.deepseek.refreshing),
        provider => {
            let a = s.agent(provider);
            let limits = a.limits(s.now);
            let stale = limits.is_some() && a.problem.is_some();
            let plan = limits.as_ref().filter(|snapshot| !snapshot.plan.is_empty()).map(|snapshot| plan_line(l, format!("{} plan", snapshot.plan), snapshot.subscription, s.now));
            (plan, status_words(s, provider, limits.as_ref(), stale), a.refreshing)
        }
    };
    let mut lines = col().key("plan").items_end();
    if let Some(plan) = plan {
        lines = lines.child(plan);
    }
    lines = lines.child(text(words).key("updated").font(digits(11.0, if warn { 600.0 } else { 400.0 })).color(if warn { l.warning_text } else { l.faint }));
    let right = row().gap(12.0).center_y().child(lines).child(look::refresh_button(l, refreshing));
    look::context_bar(l, tabs, Some(right)).pl(12.0)
}

/// The plan's name, then when it renews, ends or is paid until while that is ahead: the full
/// date and time in its help. A plan that ends reads as a warning.
fn plan_line(l: &Look, plan: String, subscription: Option<Subscription>, now: i64) -> El {
    let name = text(plan).font(font(11.0, 600.0)).color(l.dim);
    let Some(subscription) = subscription.filter(|s| s.until > now) else { return name };
    let (verb, on, color, weight) = match subscription.renews {
        Some(true) => ("Renews", " on", l.dim, 400.0),
        Some(false) => ("Ends", " on", l.warning_text, 600.0),
        None => ("Paid until", "", l.dim, 400.0),
    };
    let at = subscription.until;
    row()
        .key("plan-date")
        .gap(4.0)
        .center_y()
        .help(format!("{verb}{on} {} at {}", time::date(at), time::clock(at, false)))
        .child(name)
        .child(text("·").font(font(11.0, 400.0)).color(l.faint))
        .child(text(format!("{verb} {}", time::day_clock(at, now))).font(digits(11.0, weight)).color(color))
}

/// An agent's tab: its mark and name, no numbers.
fn agent_tab(l: &Look, tab: Tab, on: bool) -> El {
    let provider = tab.provider().unwrap_or(Provider::Claude);
    let (mark, color, _) = mark(l, provider);
    let mark_color = if on || provider != Provider::Codex { color } else { l.dim };
    let mut tab_el = row()
        .key(format!("agent-{}", provider.id()))
        .role("tab")
        .label(provider.name())
        .help(format!("{}  ⌘⇧[ ⌘⇧]", provider.name()))
        .h(28.0)
        .pl(8.0)
        .pr(10.0)
        .gap(6.0)
        .radius(7.0)
        .continuous_corners()
        .center_y()
        .fill(if on { l.chip_on } else { Color::TRANSPARENT })
        .hover(if on { l.chip_on } else { l.chip })
        .on_press_tx(move |s: &mut Ui, tx| ui::set_tab(s, tab, tx))
        .child(icon(mark, 14.0).color(mark_color))
        .child(text(provider.name()).font(font(12.0, if on { 600.0 } else { 500.0 })).color(if on { l.text } else { l.dim }));
    if on && !l.dark {
        tab_el = tab_el.shadow(1.0, 2.0, Color::BLACK.alpha(0.14));
    }
    tab_el
}

/// Add an agent: a plus on a dashed tile, pressed while its popover is open.
fn plus_button(l: &Look, open: bool) -> El {
    row()
        .key("add-agent")
        .role("button")
        .label("Add an agent")
        .help("Add an agent")
        .size(28.0, 28.0)
        .radius(7.0)
        .continuous_corners()
        .center()
        .fill(if open { l.chip_on } else { Color::TRANSPARENT })
        .border(1.0, l.edge)
        .border_dash(3.0, 3.0)
        .hover(l.chip)
        .on_press_tx(ui::toggle_adding)
        .child(icon(&icons::PLUS, 12.0).color(l.dim))
}

/// The quiet line under an agent's plan: when its limits were read, or why they are not current.
pub(crate) fn status_words(s: &Ui, provider: Provider, limits: Option<&Snapshot>, stale: bool) -> (String, bool) {
    let a = s.agent(provider);
    match (&a.problem, limits, a.refreshing) {
        (_, _, true) => ("Updating…".to_owned(), false),
        (_, Some(snapshot), _) => (updated(s.now, snapshot.fetched_at), stale),
        (Some(_), None, _) => ("Not updated".to_owned(), true),
        (None, None, _) => ("Reading…".to_owned(), false),
    }
}

/// A provider's mark, its colour and its tile's colour.
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

/// A provider with plan limits: the strip saying why its numbers aren't current when they
/// aren't, then the two columns.
fn body(s: &Ui, l: &Look, provider: Provider) -> El {
    let a = s.agent(provider);
    let limits = a.limits(s.now);
    // Signed out with nothing to show is the tab for connecting; with limits from before on
    // screen, it is why they are no longer current.
    let problem = a.problem.as_ref().filter(|p| !matches!(p, Problem::SignedOut) || limits.is_some());
    let height = BODY_H - if problem.is_some() { BANNER_H } else { 0.0 };
    let left = match &limits {
        Some(snapshot) => limits_column(s, l, provider, snapshot, height, problem.is_some()),
        None if problem.is_some() => col().child(section("no-limits").child(section_title(l, "Limits", None)).child(empty_card(l, "No limits to show yet", "They show once the provider answers.", 96.0))),
        None => skeleton_limits(l, height),
    };
    let right = usage_column(s, l, provider, height);
    let mut body = col().key(provider.id()).w_full().h(BODY_H);
    if let Some(problem) = problem {
        body = body.child(problem_banner(s, l, provider, problem, limits.as_ref()));
    }
    body.child(look::columns_in(l, height, left, right))
}

/// Why the numbers on screen are not current, and what happens next: a strip across the panel.
pub(crate) fn problem_banner(s: &Ui, l: &Look, provider: Provider, problem: &Problem, snapshot: Option<&Snapshot>) -> El {
    let retry = |seconds: i64| format!("Trying again in {}.", time::span(seconds.max(60)));
    let problem_at = if provider == Provider::DeepSeek { s.deepseek.problem_at } else { s.agent(provider).problem_at };
    let since = s.now - problem_at;
    let (glyph, detail) = match problem {
        Problem::LoginExpired => (
            &icons::KEY_ROUND,
            match provider {
                Provider::Codex => "Run `codex login` to renew it. Your login is only ever read.".to_owned(),
                Provider::DeepSeek => "DeepSeek turned the key down. Replace it in Settings.".to_owned(),
                Provider::Claude => "Run `claude` once to renew it. Your login is only ever read.".to_owned(),
            },
        ),
        Problem::RateLimited { until } => (&icons::CIRCLE_PAUSE, format!("{} asked for a pause. {}", provider.name(), retry(until - s.now))),
        Problem::Offline(_) => (
            &icons::WIFI_OFF,
            match (provider, snapshot) {
                (Provider::DeepSeek, _) => match s.deepseek.ledger.as_ref().and_then(|ledger| ledger.last.as_ref()) {
                    Some(balance) => format!("The balance is from {}.", time::clock(balance.fetched_at, false)),
                    None => "Your balance shows once the Mac is back online.".to_owned(),
                },
                (_, Some(snapshot)) => format!("Limits are from {}; usage is from this Mac and current.", time::clock(snapshot.fetched_at, false)),
                (_, None) => "Your limits show once the Mac is back online.".to_owned(),
            },
        ),
        Problem::Failed(reason) => (&icons::TRIANGLE_ALERT, format!("{reason} {}", retry(s.config.refresh_seconds() - since))),
        Problem::SignedOut => (
            &icons::LOG_IN,
            match provider {
                Provider::Codex => "Run `codex login` in a terminal; these limits are from before.".to_owned(),
                Provider::DeepSeek => "Add a key with the + button.".to_owned(),
                Provider::Claude => "Run `claude` in a terminal and sign in; these limits are from before.".to_owned(),
            },
        ),
    };
    row()
        .key("problem")
        .w_full()
        .h(BANNER_H)
        .px(PAD_X)
        .gap(10.0)
        .center_y()
        .fill(l.banner)
        .border_sides([0.0, 0.0, 1.0, 0.0], l.stroke)
        .help(detail.clone())
        .child(icon(glyph, 14.0).color(l.warning_text))
        .child(text(problem.title().to_owned()).font(font(12.0, 600.0)).color(l.text))
        .child(text(detail).font(font(12.0, 400.0)).color(l.dim).truncate().grow(1.0).min_w(0.0))
        .child(look::button(l, "retry", "Try again").on_press(ui::refresh))
}

/// What a section of limits takes: its title, a row per limit and extra usage.
fn limits_height(count: usize, extra: bool) -> f32 {
    let rows = count as f32 * TALL_ROW_H + count.saturating_sub(1) as f32 * LIMIT_GAP;
    2.0 * PAD_Y + TITLE_H + GAP + rows + if extra { GAP + EXTRA_H } else { 0.0 }
}

/// The limits, free resets and the picked limit's chart, which takes the rest of the column.
fn limits_column(s: &Ui, l: &Look, provider: Provider, snapshot: &Snapshot, height: f32, stale: bool) -> El {
    let a = s.agent(provider);
    let charted = a.chart_limit(snapshot);
    let any_scoped = snapshot.limits.iter().any(|l| l.scoped);
    let note = if stale { Some(format!("As of {}", time::clock(snapshot.fetched_at, false))) } else { (snapshot.limits.len() > 1).then(|| "Pick one to chart it".to_owned()) };
    let mut rows = col().key("limits").w_full().gap(LIMIT_GAP);
    for limit in &snapshot.limits {
        let on = charted.is_some_and(|c| c.id == limit.id);
        rows = rows.child(limit_row(s, l, provider, limit, any_scoped, on));
    }
    let mut limits = section("limits-section").px(8.0).child(section_title(l, "Limits", note.map(|note| caption(l, note))).px(12.0)).child(rows);
    if let Some(extra) = &snapshot.extra {
        limits = limits.child(extra_row(l, extra));
    }
    let mut column = col().key("limits-column").w_full().child(limits);
    let mut used = limits_height(snapshot.limits.len(), snapshot.extra.is_some());
    if let Some(resets) = snapshot.resets.as_ref().filter(|resets| resets.available > 0) {
        let (resets, resets_h) = crate::resets_view::section(s, l, snapshot, resets);
        column = column.child(resets);
        used += resets_h;
    }
    // A question about a reset takes the room the chart would: it is the thing to read now.
    if let Some(limit) = charted.filter(|_| s.reset.credit().is_none()) {
        column = column.child(burn_view::section(s, l, provider, snapshot, limit, (height - used).max(0.0)));
    }
    column
}

/// A limit: a ring of what is left with a tick at an even pace, its name and when it resets,
/// and the percentage. The one in the chart sits on a card; a click on another charts it.
fn limit_row(s: &Ui, l: &Look, provider: Provider, limit: &Limit, any_scoped: bool, on: bool) -> El {
    let (now, show_used) = (s.now, s.show_used(provider));
    let severity = limit.severity();
    let (name, scope) = model::names(limit, any_scoped);
    let mut title = row().gap(4.0).center_y().min_w(0.0).child(text(name).font(font(13.0, 600.0)).color(l.text));
    if let Some(scope) = scope {
        title = title.child(text(scope).font(font(13.0, 400.0)).color(l.dim).truncate().min_w(0.0));
    }
    let (second, second_color) = match runway(l, limit, now) {
        Some((warning, color)) => (format!("{} · {warning}", reset_note(limit, now)), color),
        None => (reset_note(limit, now), l.faint),
    };
    let words = col().grow(1.0).min_w(0.0).gap(2.0).child(title).child(text(second).font(digits(11.0, 400.0)).color(second_color).truncate().min_w(0.0));
    let share = if show_used { limit.used / 100.0 } else { limit.left() / 100.0 };
    // The tick marks an even pace: where the ring would end if use had kept step with time.
    let elapsed = limit.elapsed(now).filter(|_| !limit.used_up());
    let tick = elapsed.map(|elapsed| if show_used { elapsed } else { 1.0 - elapsed });
    let value = col()
        .items_end()
        .child(text(model::percent(limit, show_used)).font(figure(28.0)).color(l.ink(severity)))
        .child(text(if show_used { "used" } else { "left" }).font(font(11.0, 400.0)).color(l.faint));
    let id = limit.id.clone();
    let mut body = row()
        .key(format!("limit-{}", limit.id))
        .role("button")
        .label(format!("{}: {}", limit.label, model::percent(limit, show_used)))
        .w_full()
        .h(TALL_ROW_H)
        .px(12.0)
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
    if on { body.fill(l.card) } else { body.hover(l.hover).on_press(move |s: &mut Ui| ui::select_limit(s, &id)) }
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
    let (c, r) = (RING / 2.0, RING / 2.0 - RING_WIDTH / 2.0 - 1.0);
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
    let mut value = row().gap(4.0).center_y().child(text(amount(extra.used)).font(digits(12.0, 600.0)).color(l.text));
    if let Some(limit) = extra.limit {
        value = value.child(text(format!("of {}", amount(limit).trim_end_matches(".00"))).font(digits(12.0, 400.0)).color(l.dim));
    }
    let mut name = row().gap(6.0).center_y().child(text("Extra usage").font(font(12.0, 400.0)).color(l.dim));
    if let Some(resets) = extra.resets_at {
        name = name.child(caption(l, format!("resets {}", time::month_day(resets))));
    }
    let spent = extra.limit.map(|limit| extra.used / limit);
    let fill = match spent {
        Some(spent) if spent >= 0.9 => l.critical,
        Some(spent) if spent >= 0.7 => l.warning,
        _ => l.normal,
    };
    col()
        .key("limit-extra")
        .w_full()
        .h(EXTRA_H)
        .pt(4.0)
        .px(12.0)
        .gap(8.0)
        .child(row().w_full().h(16.0).center_y().child(name).child(spacer()).child(value))
        .child(look::meter(l, "meter", INNER_W, 4.0, spent.unwrap_or(0.0), fill))
}

/// The left column while the limits are first read: their places, empty.
fn skeleton_limits(l: &Look, height: f32) -> El {
    let bar = |w: f32, h: f32| el().w(w).h(h).radius(4.0).fill(l.chip);
    let row_of = || row().w_full().h(TALL_ROW_H).px(12.0).gap(12.0).center_y().child(el().size(RING, RING).radius(RING / 2.0).border(RING_WIDTH, l.chip)).child(col().grow(1.0).gap(6.0).child(bar(120.0, 12.0)).child(bar(80.0, 10.0))).child(bar(56.0, 24.0));
    let limits = section("limits-section").px(8.0).child(section_title(l, "Limits", None).px(12.0)).child(col().gap(LIMIT_GAP).child(row_of()).child(row_of()));
    let chart_h = (height - limits_height(2, false) - 2.0 * PAD_Y - 32.0 - 2.0 * GAP - AXIS_H).max(0.0);
    let chart = next_section(l, "chart-skeleton").child(col().gap(6.0).child(bar(140.0, 12.0)).child(bar(200.0, 10.0))).child(el().w_full().h(chart_h).radius(8.0).fill(l.card)).child(el().h(AXIS_H));
    col().key("loading").w_full().child(limits).child(chart)
}

/// A dashed card in place of something that isn't there yet: what is missing and when it comes.
pub(crate) fn empty_card(l: &Look, title: &str, detail: &str, height: f32) -> El {
    col()
        .key("empty")
        .w_full()
        .h(height)
        .gap(4.0)
        .center()
        .radius(8.0)
        .border(1.0, l.edge)
        .border_dash(4.0, 3.0)
        .child(text(title.to_owned()).font(font(12.0, 600.0)).color(l.dim))
        .child(caption(l, detail.to_owned()))
}

/// What the usage section takes with `models` rows under its chart.
fn usage_height(models: usize) -> f32 {
    let rows = if models > 0 { GAP + models as f32 * ROW_H } else { 0.0 };
    2.0 * PAD_Y + TITLE_H + GAP + TOTALS_H + GAP + BARS_H + 4.0 + AXIS_H + rows
}

/// Usage from the session logs, and the projects that spent the most under it.
fn usage_column(s: &Ui, l: &Look, provider: Provider, height: f32) -> El {
    let mut ranges = segments(l, "ranges");
    for range in Range::ALL {
        ranges = ranges.child(segment(l, format!("range-{}", range.label()), range.label(), s.range == range, move |s: &mut Ui| ui::set_range(s, range)));
    }
    let usage = section("usage").child(section_title(l, "Usage", Some(ranges)));
    let tool = if provider == Provider::Codex { "the Codex CLI" } else { "Claude Code" };
    let Some(stats) = &s.agent(provider).stats else {
        let bar = |w: f32| el().w(w).h(10.0).radius(4.0).fill(l.chip);
        let rows = (0..MODEL_ROWS).fold(col(), |rows, _| rows.child(row().h(ROW_H).gap(8.0).center_y().child(el().size(8.0, 8.0).radius(2.0).fill(l.chip)).child(bar(140.0)).child(spacer()).child(bar(48.0))));
        return col()
            .key("usage-loading")
            .w_full()
            .child(usage.child(el().w_full().h(TOTALS_H)).child(el().w_full().h(BARS_H + 4.0 + AXIS_H).radius(6.0).fill(l.card)).child(rows))
            .child(next_section(l, "projects").child(section_title(l, "Projects", None)).child(caption(l, format!("Counting tokens in {tool}'s logs…"))));
    };
    let view = &stats.ranges[s.range.index()];
    if view.total() == 0 {
        let period = if s.range == Range::Today { "today yet".to_owned() } else { format!("in the {}", s.range.span().to_lowercase()) };
        let title = if provider == Provider::Codex { "No Codex CLI sessions" } else { "No Claude Code messages" };
        let usage = usage.child(totals(l, view, s)).child(empty_card(l, &format!("{title} {period}"), &format!("Usage counts from {tool}'s logs as you work."), BARS_H + 4.0 + AXIS_H));
        let projects = next_section(l, "projects")
            .h(height - usage_height(0))
            .child(section_title(l, "Projects", Some(caption(l, "By cost"))))
            .child(col().grow(1.0).gap(4.0).center().child(text("No projects in this period").font(font(12.0, 600.0)).color(l.dim)).child(caption(l, format!("A project shows once {tool} runs in its folder."))));
        return col().key("usage-column").w_full().child(usage).child(projects);
    }
    let models = model_rows(l, view);
    let model_count = view.models.len().min(MODEL_ROWS);
    let usage = usage.child(totals(l, view, s)).child(chart(l, view, s.hovered)).child(models);
    let projects = projects(l, view, height - usage_height(model_count));
    col().key("usage-column").w_full().child(usage).child(projects)
}

/// One reading: the range's totals, or the bar under the pointer. It keeps its place and its
/// shape, so moving across the chart changes only the words in it.
fn totals(l: &Look, view: &RangeView, s: &Ui) -> El {
    let hovered = s.hovered.and_then(|at| view.bars.get(at));
    let (when, tokens, cost) = match hovered {
        Some(bar) => (bar.label.clone(), bar.tokens, model::money(bar.cost)),
        None => ("Tokens".to_owned(), view.total(), view.cost_label()),
    };
    row()
        .key("totals")
        .w_full()
        .h(TOTALS_H)
        .items_end()
        .help(format!("Input {}", model::tokens(view.tokens[INPUT])))
        .help_row(format!("Output {}", model::tokens(view.tokens[OUTPUT])))
        .help_row(format!("Cache write {}", model::tokens(view.tokens[CACHE_WRITE_5M] + view.tokens[CACHE_WRITE_1H])))
        .help_row(format!("Cache read {}", model::tokens(view.tokens[CACHE_READ])))
        .child(col().gap(2.0).child(caption(l, when).key("totals-when")).child(text(model::tokens(tokens)).key("total-tokens").font(figure(22.0)).color(l.text)))
        .child(spacer())
        .child(col().gap(2.0).items_end().child(caption(l, "At API prices")).child(text(cost).key("total-cost").font(figure(22.0)).color(l.text)))
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
                stack = stack.child(el().w_full().h(h).fill(l.series(part)));
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
    let label = |name: &str, strong: bool| text(name.to_owned()).font(digits(11.0, if strong { 600.0 } else { 400.0 })).color(if strong { l.text } else { l.faint });
    let mut axis = row().key("axis").w_full().h(AXIS_H);
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
    col().w_full().gap(4.0).child(bars).child(axis)
}

/// A row per model: the three with their own colour, then the rest together in grey.
fn model_rows(l: &Look, view: &RangeView) -> El {
    let mut list = col().key("models").w_full();
    let shown = if view.models.len() > MODEL_ROWS { MODEL_ROWS - 1 } else { view.models.len() };
    for (at, model) in view.models.iter().take(shown).enumerate() {
        let cost = model.cost.map_or_else(|| "—".to_owned(), model::money);
        list = list.child(data_row(l, format!("model-{at}"), swatch(l.series(at)), text(model.name.clone()).font(font(12.0, 400.0)).color(l.text).truncate_middle().grow(1.0).min_w(0.0), None, model.tokens, cost, model.cost.is_some()));
    }
    if view.models.len() > shown {
        let rest = &view.models[shown..];
        let tokens: u64 = rest.iter().map(|m| m.tokens).sum();
        let cost: f64 = rest.iter().filter_map(|m| m.cost).sum();
        let name = text(format!("{} other models", rest.len())).font(font(12.0, 400.0)).color(l.dim).truncate().grow(1.0).min_w(0.0);
        list = list.child(data_row(l, "model-rest".to_owned(), swatch(l.series_rest), name, None, tokens, model::money(cost), true));
    }
    list
}

fn swatch(color: Color) -> El {
    row().w(12.0).child(look::key_swatch(color, 8.0))
}

/// A list row of usage: a leading slot, a name and a quieter note beside it, tokens and cost.
#[allow(clippy::too_many_arguments)]
fn data_row(l: &Look, key: String, lead: El, name: El, note: Option<El>, tokens: u64, cost: String, priced: bool) -> El {
    let mut names = row().grow(1.0).min_w(0.0).gap(6.0).center_y().child(name);
    if let Some(note) = note {
        names = names.child(note);
    }
    row()
        .key(key)
        .w_full()
        .h(ROW_H)
        .gap(8.0)
        .center_y()
        .child(lead)
        .child(names)
        .child(cell(56.0, text(model::tokens(tokens)).font(digits(12.0, 400.0)).color(l.dim)))
        .child(cell(64.0, text(cost).font(digits(12.0, 600.0)).color(if priced { l.text } else { l.faint })))
}

/// The projects that spent the most in the range: as many as fit, then the rest in a line.
fn projects(l: &Look, view: &RangeView, height: f32) -> El {
    let title = section_title(l, "Projects", Some(caption(l, "By cost")));
    let section = next_section(l, "projects").h(height).child(title);
    if view.projects.is_empty() {
        return section.child(col().grow(1.0).gap(4.0).center().child(text("No projects in this period").font(font(12.0, 600.0)).color(l.dim)).child(caption(l, "A project shows once a session runs in its folder.")));
    }
    let room = height - 2.0 * PAD_Y - TITLE_H - GAP;
    let shown = look::rows_that_fit(room, ROW_H, view.projects.len());
    let mut list = col().key("project-rows").w_full().grow(1.0);
    for (at, project) in view.projects.iter().take(shown).enumerate() {
        let rank = row().w(12.0).child(text(format!("{}", at + 1)).font(digits(11.0, 500.0)).color(l.faint));
        let name = text(project.name.clone()).font(font(12.0, 400.0)).color(l.text).truncate().max_w(NAME_MAX).help(format!("{}/{}", project.path, project.name));
        let path = text(project.path.clone()).font(font(11.0, 400.0)).color(l.faint).truncate().shrink();
        let cost = project.cost.map_or_else(|| "—".to_owned(), model::money);
        list = list.child(data_row(l, format!("project-{at}"), rank, name, Some(path), project.tokens, cost, project.cost.is_some()));
    }
    // The rest in a line at the list's foot, so the list ends at the column's.
    if shown < view.projects.len() {
        let rest = &view.projects[shown..];
        let cost: f64 = rest.iter().filter_map(|p| p.cost).sum();
        let words = format!("{} more project{}, {} together", rest.len(), if rest.len() == 1 { "" } else { "s" }, model::money(cost));
        list = list.child(spacer()).child(look::more_line(l, words));
    }
    section.child(list)
}
