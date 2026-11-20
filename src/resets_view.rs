//! A plan's free limit resets, where the provider reports them (Codex): each kind with how
//! many are left and when they expire, and a "Use…" that asks before spending one, says what
//! it will do to each limit, locks while the request runs and reports how it went.

use rune::{Color, El, col, el, icon, row, text};

use crate::agents::{self, ResetFlow};
use crate::icons;
use crate::look::{GAP, Look, PAD_Y, TALL_ROW_H, TITLE_H, caption, digits, font, next_section, section_title};
use crate::model::{self, Resets, Snapshot};
use crate::time;
use crate::ui::Ui;

/// The line under the resets saying one was just used.
const DONE_H: f32 = 20.0;

/// One kind of reset: its title, the ids of the credits of that kind, and when the soonest
/// expires.
struct Kind<'a> {
    title: &'a str,
    description: Option<&'a str>,
    credits: Vec<&'a str>,
    expires: Option<i64>,
    same_expiry: bool,
}

fn kinds(resets: &Resets) -> Vec<Kind<'_>> {
    let mut kinds: Vec<Kind> = Vec::new();
    for credit in &resets.credits {
        match kinds.iter_mut().find(|kind| kind.title == credit.title) {
            Some(kind) => {
                kind.credits.push(&credit.id);
                kind.same_expiry &= kind.expires == credit.expires_at;
                kind.expires = kind.expires.min(credit.expires_at).or(credit.expires_at);
            }
            None => kinds.push(Kind { title: &credit.title, description: credit.description.as_deref(), credits: vec![&credit.id], expires: credit.expires_at, same_expiry: true }),
        }
    }
    kinds
}

/// The plan's free resets, a fixed-height row per kind, and the height the section takes. While
/// a reset is being asked about, its question takes the row's place and the column's room.
pub fn section(s: &Ui, l: &Look, snapshot: &Snapshot, resets: &Resets) -> (El, f32) {
    let mut body = next_section(l, "resets").child(section_title(l, "Free resets", Some(caption(l, format!("{} available", resets.available)))));
    let mut height = 2.0 * PAD_Y + TITLE_H;
    if resets.credits.is_empty() {
        return (body.child(caption(l, "Use them in ChatGPT, or refresh to list them here.")), height + GAP + 14.0);
    }
    for (at, kind) in kinds(resets).into_iter().enumerate() {
        height += GAP + TALL_ROW_H;
        let asked = s.reset.credit().is_some_and(|credit| kind.credits.contains(&credit));
        if asked {
            body = body.child(confirm(s, l, snapshot, &kind));
            continue;
        }
        let count = kind.credits.len();
        let expires = kind.expires.map(|at| format!("{} {}", if count > 1 && kind.same_expiry { "All expire" } else { "Expires" }, time::day_clock(at, s.now)));
        let second = match (kind.description, expires) {
            (Some(description), Some(expires)) => format!("{description} · {expires}"),
            (Some(words), None) => words.to_owned(),
            (None, Some(words)) => words,
            (None, None) => String::new(),
        };
        let first = kind.credits[0].to_owned();
        let words = col()
            .grow(1.0)
            .min_w(0.0)
            .gap(2.0)
            .child(
                row()
                    .gap(6.0)
                    .center_y()
                    .child(text(kind.title.to_owned()).font(font(12.0, 600.0)).color(l.text).truncate().min_w(0.0))
                    .child(row().h(16.0).px(6.0).radius(5.0).center().fill(l.chip).child(text(format!("{count} left")).font(digits(11.0, 600.0)).color(l.dim))),
            )
            .child(text(second).font(digits(11.0, 400.0)).color(l.faint).truncate().min_w(0.0));
        let busy = matches!(s.reset, ResetFlow::Resetting { .. });
        body = body.child(
            row()
                .key(format!("reset-{at}"))
                .w_full()
                .h(TALL_ROW_H)
                .gap(10.0)
                .center_y()
                .child(tile(l, l.normal, false))
                .child(words)
                .child(button(l, "Use…", false).key(format!("use-{at}")).opacity(if busy { 0.45 } else { 1.0 }).on_press(move |s: &mut Ui| agents::ask_reset(s, &first))),
        );
    }
    if s.reset == ResetFlow::Done {
        height += GAP + DONE_H;
        body = body.child(row().key("reset-done").h(DONE_H).gap(6.0).center_y().child(el().size(6.0, 6.0).radius(3.0).fill(l.good)).child(text("Reset just now. The limits read again.").font(font(12.0, 500.0)).color(l.good_text)));
    }
    (body, height)
}

fn tile(l: &Look, color: Color, failed: bool) -> El {
    row().size(28.0, 28.0).radius(8.0).center().fill(l.chip).child(icon(if failed { &icons::CIRCLE_X } else { &icons::ROTATE_CCW }, 14.0).color(color))
}

fn button(l: &Look, label: &str, primary: bool) -> El {
    row()
        .role("button")
        .label(label.to_owned())
        .h(24.0)
        .px(10.0)
        .radius(6.0)
        .continuous_corners()
        .center()
        .fill(if primary { l.normal } else { l.chip })
        .hover(if primary { l.normal } else { l.chip_on })
        .child(text(label.to_owned()).font(font(11.0, 600.0)).color(if primary { rune::Color::WHITE } else { l.text }))
}

/// The question, in place of the reset's row: what each limit becomes, that it can't be
/// undone, and the buttons. While the request runs both buttons lock; when it fails, why.
fn confirm(s: &Ui, l: &Look, snapshot: &Snapshot, kind: &Kind) -> El {
    let count = kind.credits.len();
    let failed = match &s.reset {
        ResetFlow::Failed { reason, certain, .. } => Some((reason.as_str(), *certain)),
        _ => None,
    };
    let busy = matches!(s.reset, ResetFlow::Resetting { .. });
    let title = match (failed, count) {
        (Some(_), _) => format!("Couldn't use the {}", kind.title.to_lowercase()),
        (None, 1) => format!("Use your last {}?", kind.title.to_lowercase()),
        (None, _) => format!("Use 1 of {count} {}s?", kind.title),
    };
    let what = format!("{} {}", kind.title, kind.description.unwrap_or(""));
    let edge = if failed.is_some() { l.critical } else { l.normal };
    let mut effects = col().w_full().padding(10.0).gap(6.0).radius(8.0).fill(l.card);
    for limit in snapshot.limits.iter().filter(|limit| covers(&what, limit)) {
        let (name, scope) = model::names(limit, snapshot.limits.iter().any(|l| l.scoped));
        let label = match scope {
            Some(scope) => format!("{name} · {scope}"),
            None => name,
        };
        effects = effects.child(
            row()
                .w_full()
                .gap(6.0)
                .center_y()
                .child(text(label).font(font(12.0, 400.0)).color(l.dim).truncate().grow(1.0).min_w(0.0))
                .child(text(model::percent(limit, false)).font(digits(12.0, 400.0)).color(l.dim))
                .child(icon(&icons::ARROW_RIGHT, 11.0).color(l.faint))
                .child(text(match failed { Some((_, true)) => "unchanged".to_owned(), Some((_, false)) => "unknown".to_owned(), None => "100% left".to_owned() }).font(digits(12.0, 600.0)).color(if failed.is_some() { l.dim } else { l.good_text })),
        );
    }
    let note = match failed {
        Some((reason, true)) => format!("{reason} No reset was used; you still have {count}."),
        Some((reason, false)) => format!("{reason} Whether the reset was used isn't known yet; the limits are being read again to show it. Trying again can't use a second one."),
        None => format!("This can't be undone. {} will be left.", if count == 1 { "None".to_owned() } else { format!("{}", count - 1) }),
    };
    let mut buttons = row().w_full().gap(8.0).justify_end();
    if failed.is_some() {
        buttons = buttons.child(button(l, "Close", false).key("reset-cancel").on_press(agents::cancel_reset)).child(button(l, "Try again", true).key("reset-confirm").on_press(agents::confirm_reset));
    } else {
        let cancel = button(l, "Cancel", false).key("reset-cancel").opacity(if busy { 0.45 } else { 1.0 });
        let go = button(l, if busy { "Resetting…" } else { "Reset now" }, true).key("reset-confirm").opacity(if busy { 0.6 } else { 1.0 });
        buttons = if busy { buttons.child(cancel).child(go) } else { buttons.child(cancel.on_press(agents::cancel_reset)).child(go.on_press(agents::confirm_reset)) };
    }
    col()
        .key("reset-confirm-card")
        .w_full()
        .padding(12.0)
        .gap(10.0)
        .radius(10.0)
        .continuous_corners()
        .fill(l.card)
        .border(1.5, edge)
        .child(row().gap(10.0).center_y().child(tile(l, edge, failed.is_some())).child(text(title).font(font(13.0, 600.0)).color(l.text)))
        .child(effects)
        .child(text(note).font(font(11.0, 400.0)).color(l.faint).w_full().wrap())
        .child(buttons)
}

/// Whether a reset of this kind starts `limit` over, read from its title and description:
/// "Full reset (Weekly + 5 hr)" covers both, a weekly reset the week's limits, a five-hour one
/// the session.
fn covers(title: &str, limit: &model::Limit) -> bool {
    let title = title.to_lowercase();
    let weekly = title.contains("week") || title.contains("full");
    let session = title.contains("5 hr") || title.contains("5-hour") || title.contains("5 hour") || title.contains("session") || title.contains("full");
    if !weekly && !session {
        return true;
    }
    if limit.window <= 86_400 { session } else { weekly }
}
