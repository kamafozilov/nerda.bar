//! The Sleep tab. On the left, what can be done for a while: keep this Mac awake for a length
//! of time or until turned off, with its display on or not and its lid closed or not, and the
//! apps that keep it awake already. On the right, what macOS does on its own: when the display
//! turns off and the Mac sleeps on each power source, the lid and the last sleep and wake, and
//! putting the display, the Mac or the screen to sleep now.
//!
//! Like every tab, its regions have known heights: the fixed sections add up, and the two
//! lists (the apps holding sleep off, the lid and wake) show as many rows as fit.

use rune::{Color, El, Icon, LineCap, Path, Shape, canvas, col, el, icon, row, spacer, text};

use crate::agents_view::BANNER_H;
use crate::awake::{self, DURATIONS, LidHold};
use crate::icons;
use crate::look::{self, BODY_H, INNER_W, Look, PAD_X, ROW_H, TALL_ROW_H, TITLE_H, caption, digits, figure, font, more_line, next_section, rows_that_fit, section, section_title, segment, segments, switch};
use crate::system::power::{Holder, PowerView, Setting, Source};
use crate::system_view::{HERO_H, SMALL_H, rest, stack};
use crate::time;
use crate::ui::Ui;
use crate::view::Screen;

/// The keep-awake ring, and the block it leads.
const RING: f32 = 72.0;
const RING_WIDTH: f32 = 6.0;
/// A row of the idle timers.
const IDLE_ROW_H: f32 = 36.0;
/// The Now section's buttons.
const ACTION_H: f32 = 28.0;
/// An app's icon in the list, and the column of how long it has held sleep off.
const ICON: f32 = 20.0;
const VALUE_W: f32 = 64.0;
/// The idle timers' choices, in minutes: 0 is never. A value set elsewhere takes the place of
/// the nearest.
const TIMER_CHOICES: [u32; 6] = [2, 5, 10, 30, 60, 0];

pub fn screen(s: &Ui, l: &Look) -> Screen {
    let view = s.awake.view.as_deref();
    let title = look::context_title(l, "Sleep", None, view.map(|view| summary(view)));
    let context = look::context_bar(l, title, Some(status(s, l, view)));
    let banner = banner(s, l);
    let height = BODY_H - if banner.is_some() { BANNER_H } else { 0.0 };
    let (left, right) = match view {
        Some(view) => (left_column(s, l, view, height), right_column(s, l, view, height)),
        None => (section("sleep-reading").child(text("Reading this Mac…").key("sleep-loading").font(font(12.0, 400.0)).color(l.dim)), col()),
    };
    let mut body = col().key("sleep-body").w_full();
    if let Some(banner) = banner {
        body = body.child(banner);
    }
    Screen { context, body: body.child(look::columns_in(l, height, left.key("sleep-left"), right.key("sleep-right"))) }
}

/// The context bar's quiet line: the power source and its timers.
fn summary(view: &PowerView) -> String {
    let source = view.source();
    let on = match source {
        Source::Battery => "On battery",
        Source::Adapter => "On power adapter",
    };
    let Some(timers) = view.timers(source).filter(|_| view.timers_read) else { return on.to_owned() };
    let display = match timers.display {
        Some(0) => Some("display stays on".to_owned()),
        Some(m) => Some(format!("display off after {}", minutes(m))),
        None => None,
    };
    let sleep = match timers.sleep {
        Some(0) => Some("never sleeps idle".to_owned()),
        Some(m) => Some(format!("sleeps after {}", minutes(m))),
        None => None,
    };
    [Some(on.to_owned()), display, sleep].into_iter().flatten().collect::<Vec<_>>().join(" · ")
}

/// The context bar's status: whether this Mac is kept awake, and by whom.
fn status(s: &Ui, l: &Look, view: Option<&PowerView>) -> El {
    if let Some(session) = &s.awake.session {
        let words = match s.awake.left(s.now) {
            Some(left) => format!("Kept awake · {} left", awake::left_words(left)),
            None => "Kept awake".to_owned(),
        };
        let help = match session.until {
            Some(until) => format!("Until {}", time::clock(until, false)),
            None => "Until you turn it off".to_owned(),
        };
        return look::pill("sleep-status", l.good_text, words).help(help);
    }
    if s.awake.left_off || view.is_some_and(|view| view.sleep_disabled) {
        return look::pill("sleep-status", l.warning_text, "Sleep is off".to_owned()).help("This Mac won't sleep, not even with its lid closed");
    }
    let others = view.map_or(0, |view| view.holders.len());
    let words = match others {
        0 => "Sleeps when idle".to_owned(),
        1 => "1 app keeps it awake".to_owned(),
        n => format!("{n} apps keep it awake"),
    };
    look::chip_label(l, "sleep-status", words)
}

/// A strip under the context bar for sleep left off by an earlier run, or a change that
/// didn't happen.
fn banner(s: &Ui, l: &Look) -> Option<El> {
    let strip = |glyph: &'static Icon, title: &str, detail: String, button: El| {
        row()
            .key("sleep-banner")
            .w_full()
            .h(BANNER_H)
            .px(PAD_X)
            .gap(10.0)
            .center_y()
            .fill(l.banner)
            .border_sides([0.0, 0.0, 1.0, 0.0], l.stroke)
            .help(detail.clone())
            .child(icon(glyph, 14.0).color(l.warning_text))
            .child(text(title.to_owned()).font(font(12.0, 600.0)).color(l.text))
            .child(text(detail).font(font(12.0, 400.0)).color(l.dim).truncate().grow(1.0).min_w(0.0))
            .child(button)
    };
    if s.awake.left_off && s.awake.session.is_none() {
        let asking = s.awake.asking == Some(awake::Asking::AllowSleep);
        let button = look::button(l, "allow-sleep", if asking { "Waiting…" } else { "Turn sleep on" }).opacity(if asking { 0.6 } else { 1.0 }).on_press(awake::allow_sleep);
        return Some(strip(&icons::TRIANGLE_ALERT, "Sleep is off", "An earlier session left it off: closing the lid won't sleep this Mac.".to_owned(), button));
    }
    let error = s.awake.error.clone()?;
    Some(strip(&icons::TRIANGLE_ALERT, "Not changed", error, look::button(l, "dismiss-error", "Dismiss").on_press(awake::dismiss_error)))
}

fn left_column(s: &Ui, l: &Look, view: &PowerView, height: f32) -> El {
    let keep_h = stack(&[TITLE_H, RING, 24.0]);
    let rows = if view.lid.is_some() { 2.0 } else { 1.0 };
    let options_h = stack(&[TITLE_H, rows * TALL_ROW_H]);
    col().w_full().child(keep_awake(s, l).h(keep_h)).child(while_awake(s, l, view).h(options_h)).child(holders(s, l, view, height - keep_h - options_h))
}

/// The keep-awake: a ring of the time left and what it means, and the durations to pick from.
fn keep_awake(s: &Ui, l: &Look) -> El {
    let session = s.awake.session.as_ref();
    let left = s.awake.left(s.now);
    let display = if s.config.sleep.display { "display stays on" } else { "display may turn off" };
    let (number, unit, line, share, color) = match session {
        Some(session) => match (session.until, left) {
            (Some(until), Some(left)) => {
                let (number, unit) = if left >= 3600 { (format!("{}:{:02}", left / 3600, left % 3600 / 60), "hours left") } else { (((left + 59) / 60).max(1).to_string(), "min left") };
                let total = (until - session.started).max(1) as f64;
                (number, unit, format!("Until {} · {display}", time::clock(until, false)), left as f64 / total, l.normal)
            }
            _ => ("On".to_owned(), "until you turn it off", format!("Since {} · {display}", time::clock(session.started, false)), 1.0, l.good),
        },
        None => ("Off".to_owned(), "sleeps as set", "Pick how long to keep this Mac awake".to_owned(), 0.0, l.normal),
    };
    let ring = row()
        .key("awake-ring")
        .size(RING, RING)
        .center()
        .child(ring(l, share, color).absolute(0.0, 0.0))
        .child(icon(&icons::COFFEE, 24.0).color(if session.is_some() { color } else { l.faint }));
    let words = col()
        .grow(1.0)
        .min_w(0.0)
        .gap(4.0)
        .child(
            row()
                .key("awake-hero")
                .h(HERO_H)
                .items_end()
                .gap(8.0)
                .child(text(number).font(figure(34.0)).color(l.text))
                .child(text(unit.to_owned()).font(font(15.0, 400.0)).color(l.dim).pb(4.0)),
        )
        .child(text(line).key("awake-line").font(digits(12.0, 400.0)).color(l.dim).truncate().min_w(0.0));
    let picked = session.map(|session| session.minutes);
    let mut durations = segments(l, "durations").w_full();
    durations = durations.child(segment(l, "duration-off".to_owned(), "Off", picked.is_none(), |s: &mut Ui| awake::pick(s, None)).grow(1.0).px(0.0));
    for (minutes, label) in DURATIONS {
        let help = if minutes == 0 { "Until you turn it off".to_owned() } else { format!("For {}", self::minutes(minutes)) };
        durations = durations.child(segment(l, format!("duration-{label}"), label, picked == Some(minutes), move |s: &mut Ui| awake::pick(s, Some(minutes))).grow(1.0).px(0.0).help(help));
    }
    let end = session.map(|session| caption(l, format!("Since {}", time::clock(session.started, false))));
    section("keep-awake").child(section_title(l, "Keep awake", end)).child(row().w_full().h(RING).gap(16.0).center_y().child(ring).child(words)).child(durations)
}

/// A ring filled to `share` from twelve o'clock, as the limits' rings are.
fn ring(l: &Look, share: f64, color: Color) -> El {
    let (c, r) = (RING / 2.0, RING / 2.0 - RING_WIDTH / 2.0 - 1.0);
    let at = |turn: f32| {
        let angle = turn * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
        [c + r * angle.cos(), c + r * angle.sin()]
    };
    let arc = |to: f32| {
        let steps = (to * 128.0).ceil().max(1.0) as usize;
        let mut path = Path::new().move_to(at(0.0));
        for step in 1..=steps {
            path = path.line_to(at(to * step as f32 / steps as f32));
        }
        path
    };
    let mut shape = Shape::new().stroke_path(&arc(1.0), RING_WIDTH, LineCap::Butt, l.track);
    let share = share.clamp(0.0, 1.0) as f32;
    if share > 0.0 {
        shape = shape.stroke_path(&arc(share.max(0.005)), RING_WIDTH, LineCap::Round, color);
    }
    canvas(shape).size(RING, RING)
}

/// What a keep-awake keeps on: the display, and on a notebook, the Mac with its lid closed.
fn while_awake(s: &Ui, l: &Look, view: &PowerView) -> El {
    let display = s.config.sleep.display;
    let display_switch = switch(l, "keep-display", "Keep the display on", display, true).on_press(move |s: &mut Ui| awake::set_display(s, !display));
    let mut card = col().w_full().radius(10.0).continuous_corners().fill(l.card).child(option(l, "option-display", &icons::MONITOR, "Keep the display on", "Off lets it turn off; apps keep running".to_owned(), display_switch));
    if view.lid.is_some() {
        let lid = s.config.sleep.lid;
        let note = match s.awake.session.as_ref().map(|session| session.lid) {
            Some(LidHold::Asking) => "Waiting for your password…".to_owned(),
            // What macOS reads, not what was asked: the watcher may still be starting.
            Some(LidHold::On) if view.sleep_disabled => "Sleep is off until this ends".to_owned(),
            Some(LidHold::On) => "Turning sleep off…".to_owned(),
            _ if lid => "Asks for your password when it starts".to_owned(),
            _ => "Asks for your password · uses more battery".to_owned(),
        };
        let lid_switch = switch(l, "keep-lid", "Stay awake with the lid closed", lid, true).on_press(move |s: &mut Ui| awake::set_lid(s, !lid));
        card = card.child(option(l, "option-lid", &icons::LAPTOP_LID, "Stay awake with the lid closed", note, lid_switch).border_sides([1.0, 0.0, 0.0, 0.0], l.stroke));
    }
    next_section(l, "while-awake").child(section_title(l, "While kept awake", None)).child(card)
}

/// A switch in a card: its icon, its name and what it means under it, and the switch.
fn option(l: &Look, key: &'static str, glyph: &'static Icon, name: &str, note: String, control: El) -> El {
    row()
        .key(key)
        .w_full()
        .h(TALL_ROW_H)
        .px(12.0)
        .gap(12.0)
        .center_y()
        .child(icon(glyph, 16.0).color(l.dim))
        .child(col().grow(1.0).min_w(0.0).gap(2.0).child(text(name.to_owned()).font(font(12.0, 500.0)).color(l.text)).child(text(note).font(font(11.0, 400.0)).color(l.faint).truncate().min_w(0.0)))
        .child(control)
}

/// The apps holding sleep off, as many as fit, the longest-held first.
fn holders(s: &Ui, l: &Look, view: &PowerView, height: f32) -> El {
    let list = next_section(l, "holders").h(height).child(section_title(l, "Keeping it awake", Some(caption(l, "Apps that stop it sleeping"))));
    if view.holders.is_empty() {
        let room = rest(height, &[TITLE_H]).min(TALL_ROW_H + 12.0);
        let empty = crate::agents_view::empty_card(l, "Nothing keeps this Mac awake now", "It sleeps when idle, as set on the right", room).key("holders-empty");
        return list.child(empty);
    }
    let room = rest(height, &[TITLE_H]);
    let shown = rows_that_fit(room, ROW_H, view.holders.len());
    let mut rows = col().w_full().h(room);
    for (at, holder) in view.holders.iter().take(shown).enumerate() {
        rows = rows.child(holder_row(s, l, at, holder));
    }
    let others = view.holders.len() - shown;
    if others > 0 {
        rows = rows.child(spacer()).child(more_line(l, format!("{others} more {}", if others == 1 { "app" } else { "apps" })).key("holders-rest"));
    }
    list.child(rows)
}

fn holder_row(s: &Ui, l: &Look, at: usize, holder: &Holder) -> El {
    let what = if holder.display { "Keeps the display on" } else { "Keeps the Mac awake" };
    let tile = el().size(ICON, ICON);
    let tile = match holder.path.as_deref().and_then(|path| s.icon(path, ICON)) {
        Some(image) => tile.pixels(image.pixels()),
        None => tile,
    };
    let held = holder.since.map(|since| time::span(s.now - since)).unwrap_or_default();
    let mut help = holder.name.clone();
    if !holder.reasons.is_empty() {
        help = format!("{help}: {}", holder.reasons.join(", "));
    }
    row()
        .key(format!("holder-{at}"))
        .w_full()
        .h(ROW_H)
        .gap(12.0)
        .center_y()
        .help(help)
        .child(tile)
        .child(
            row()
                .grow(1.0)
                .min_w(0.0)
                .gap(8.0)
                .center_y()
                .child(row().max_w(INNER_W - ICON - VALUE_W - 24.0 - 80.0).child(text(holder.name.clone()).font(font(12.0, 600.0)).color(l.text).truncate()))
                .child(text(what.to_owned()).font(font(11.0, 400.0)).color(l.faint).truncate().min_w(0.0)),
        )
        .child(look::cell(VALUE_W, text(held).font(digits(12.0, 600.0)).color(l.text)))
}

fn right_column(s: &Ui, l: &Look, view: &PowerView, height: f32) -> El {
    let idle = when_idle(s, l, view);
    let idle_h = idle.1;
    let now_h = stack(&[TITLE_H, ACTION_H]);
    col().w_full().child(idle.0.h(idle_h)).child(lid_and_wake(s, l, view, height - idle_h - now_h)).child(now(l).h(now_h))
}

/// When the display turns off and the Mac sleeps on its own, for the source picked, with the
/// height it takes.
fn when_idle(s: &Ui, l: &Look, view: &PowerView) -> (El, f32) {
    let source = s.awake.shown_source();
    let end = if view.battery.is_some() {
        let mut sources = segments(l, "idle-source");
        for each in [Source::Battery, Source::Adapter] {
            sources = sources.child(segment(l, format!("idle-{}", if each == Source::Battery { "battery" } else { "adapter" }), each.label(), each == source, move |s: &mut Ui| awake::set_source(s, each)));
        }
        sources
    } else {
        caption(l, "Power adapter")
    };
    let title = section_title(l, "When idle", Some(end));
    let timers = view.timers(source).filter(|_| view.timers_read);
    let Some(timers) = timers else {
        let reading = caption(l, "Reading the timers…").key("idle-loading");
        return (section("when-idle").child(title).child(reading), stack(&[TITLE_H, SMALL_H]));
    };
    let asking = s.awake.asking.is_some();
    let mut rows = col().key("idle-rows").w_full();
    let mut count = 0;
    let mut add = |rows: El, setting: El| {
        count += 1;
        rows.child(if count > 1 { setting.border_sides([1.0, 0.0, 0.0, 0.0], l.stroke) } else { setting })
    };
    if let Some(display) = timers.display {
        rows = add(rows, idle_row(l, "Display off", "How long idle before the display turns off", minutes_choices(l, "display-after", source, display, Setting::Display)));
    }
    if let Some(sleep) = timers.sleep {
        rows = add(rows, idle_row(l, "Mac sleeps", "How long idle before the Mac sleeps", minutes_choices(l, "sleep-after", source, sleep, Setting::Sleep)));
    }
    let flags: [(Option<bool>, &'static str, &str, &str, fn(bool) -> Setting); 3] = [
        (timers.low_power, "low-power", "Low Power Mode", "Slows the Mac a little to save energy", Setting::LowPower),
        (timers.wake_network, "wake-network", "Wake for network access", "Wakes for a shared file or screen", Setting::WakeNetwork),
        (timers.power_nap, "power-nap", "Power Nap", "Checks mail and updates while asleep", Setting::PowerNap),
    ];
    for (value, key, name, help, make) in flags {
        if let Some(on) = value {
            let toggle = switch(l, key, name, on, true).on_press(move |s: &mut Ui| awake::set_timer(s, source, make(!on)));
            rows = add(rows, idle_row(l, name, help, toggle));
        }
    }
    let rows = rows.opacity(if asking { 0.6 } else { 1.0 });
    let note = if asking { "Waiting for your password…" } else { "Changes ask for your password" };
    let foot = row().key("idle-note").h(SMALL_H).gap(6.0).center_y().child(icon(&icons::LOCK, 11.0).color(l.faint)).child(caption(l, note));
    let height = stack(&[TITLE_H, count as f32 * IDLE_ROW_H, SMALL_H]);
    (section("when-idle").child(title).child(rows).child(foot), height)
}

/// A row of the idle timers: its name, with what it means in its tooltip, and its control.
fn idle_row(l: &Look, name: &str, help: &str, control: El) -> El {
    row().w_full().h(IDLE_ROW_H).gap(12.0).center_y().help(help.to_owned()).child(text(name.to_owned()).font(font(12.0, 400.0)).color(l.text)).child(spacer()).child(control)
}

/// The minutes to pick from for a timer now at `current`.
fn minutes_choices(l: &Look, key: &'static str, source: Source, current: u32, make: fn(u32) -> Setting) -> El {
    let mut choices = segments(l, key);
    for minutes in choices_for(current) {
        let on = minutes == current;
        let label = self::minutes_short(minutes);
        choices = choices.child(
            segment(l, format!("{key}-{label}"), &label, on, move |s: &mut Ui| {
                if !on {
                    awake::set_timer(s, source, make(minutes));
                }
            })
            .px(6.0),
        );
    }
    choices
}

/// [`TIMER_CHOICES`], with `current` in the place of the nearest when it isn't one of them.
fn choices_for(current: u32) -> Vec<u32> {
    let mut choices = TIMER_CHOICES.to_vec();
    if !choices.contains(&current) {
        let nearest = choices.iter().enumerate().filter(|(_, m)| **m != 0).min_by_key(|(_, m)| m.abs_diff(current)).map_or(0, |(at, _)| at);
        choices[nearest] = current;
        choices.sort_by_key(|&m| if m == 0 { u32::MAX } else { m });
    }
    choices
}

/// Minutes in words: "5 min", "1 h", "1 h 30 min".
fn minutes(m: u32) -> String {
    match m {
        0 => "never".to_owned(),
        m if m < 60 => format!("{m} min"),
        m if m % 60 == 0 => format!("{} h", m / 60),
        m => format!("{} h {} min", m / 60, m % 60),
    }
}

/// Minutes as a choice's label: "5m", "1h", "Never".
fn minutes_short(m: u32) -> String {
    match m {
        0 => "Never".to_owned(),
        m if m < 60 => format!("{m}m"),
        m if m % 60 == 0 => format!("{}h", m / 60),
        m => format!("{}h{}", m / 60, m % 60),
    }
}

/// The lid, and when the Mac last slept and woke, as many rows as fit.
fn lid_and_wake(s: &Ui, l: &Look, view: &PowerView, height: f32) -> El {
    let mut lines: Vec<(&'static str, String, String)> = Vec::new();
    if let Some(lid) = view.lid {
        let state = if lid.closed { "Closed" } else { "Open" };
        let note = if view.sleep_disabled {
            "closing it keeps the Mac awake"
        } else if lid.causes_sleep {
            "closing it sleeps the Mac"
        } else {
            "closing it keeps it on the display"
        };
        lines.push(("lid", state.to_owned(), note.to_owned()));
    }
    if let Some(at) = view.woke_at {
        lines.push(("woke", format!("Woke {}", moment(at, s.now)), view.wake_reason.clone().unwrap_or_default()));
    }
    if let Some(at) = view.slept_at {
        lines.push(("slept", format!("Slept {}", moment(at, s.now)), format!("{} ago", time::span(s.now - at))));
    }
    let list = next_section(l, "lid-wake").h(height).child(section_title(l, if view.lid.is_some() { "Lid and wake" } else { "Wake" }, None));
    let room = rest(height, &[TITLE_H]);
    let shown = (room / ROW_H).floor().max(0.0) as usize;
    let mut rows = col().w_full();
    for (key, value, note) in lines.into_iter().take(shown) {
        let lead = match key {
            "lid" => icon(&icons::LAPTOP_LID, 14.0),
            "woke" => icon(&icons::COFFEE, 14.0),
            _ => icon(&icons::MOON, 14.0),
        };
        rows = rows.child(
            row()
                .key(format!("power-{key}"))
                .w_full()
                .h(ROW_H)
                .gap(10.0)
                .center_y()
                .child(lead.color(l.dim))
                .child(text(if key == "lid" { format!("Lid {}", value.to_lowercase()) } else { value }).font(digits(12.0, 600.0)).color(l.text))
                .child(spacer())
                .child(text(note).font(font(11.0, 400.0)).color(l.faint).truncate().min_w(0.0)),
        );
    }
    list.child(rows)
}

/// A moment today as its time, and before today with its weekday.
fn moment(at: i64, now: i64) -> String {
    if time::local(at).day == time::local(now).day { format!("at {}", time::clock(at, false)) } else { time::weekday_clock(at) }
}

/// Putting the display, the Mac or the screen to sleep now.
fn now(l: &Look) -> El {
    let action = |key: &'static str, glyph: &'static Icon, words: &str| {
        row()
            .key(key)
            .role("button")
            .label(words.to_owned())
            .grow(1.0)
            .h(ACTION_H)
            .gap(6.0)
            .radius(7.0)
            .continuous_corners()
            .center()
            .fill(l.chip)
            .hover(l.chip_on)
            .press_scale(0.96)
            .child(icon(glyph, 13.0).color(l.dim))
            .child(text(words.to_owned()).font(font(12.0, 500.0)).color(l.text))
    };
    let buttons = row()
        .w_full()
        .gap(8.0)
        .child(action("now-display-off", &icons::MONITOR_OFF, "Display off").help("Turns the display off; the Mac stays awake").on_press_tx(awake::display_off))
        .child(action("now-sleep", &icons::MOON, "Sleep").help("Puts the Mac to sleep now").on_press_tx(awake::sleep_now))
        .child(action("now-lock", &icons::LOCK, "Lock screen").help("Locks the screen, as ⌃⌘Q does").on_press_tx(awake::lock_screen));
    next_section(l, "now").child(section_title(l, "Now", None)).child(buttons)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_timer_set_elsewhere_takes_the_nearest_choices_place() {
        assert_eq!(choices_for(10), [2, 5, 10, 30, 60, 0]);
        assert_eq!(choices_for(20), [2, 5, 20, 30, 60, 0]);
        assert_eq!(choices_for(180), [2, 5, 10, 30, 180, 0]);
        assert_eq!(choices_for(1), [1, 5, 10, 30, 60, 0]);
    }

    #[test]
    fn minutes_read_short_and_long() {
        assert_eq!((minutes(5), minutes(60), minutes(90), minutes(0)), ("5 min".to_owned(), "1 h".to_owned(), "1 h 30 min".to_owned(), "never".to_owned()));
        assert_eq!((minutes_short(30), minutes_short(120), minutes_short(0)), ("30m".to_owned(), "2h".to_owned(), "Never".to_owned()));
    }
}
