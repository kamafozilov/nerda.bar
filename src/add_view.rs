//! Connecting agents: the Agents tab when none is signed in yet, and the Add an agent popover
//! that the context bar's + opens. Claude and Codex are added by signing in to their own tools,
//! which Nerda Bar reads; DeepSeek by an API key, saved to the Keychain from here.

use rune::{Color, El, col, el, icon, row, spacer, text, text_input};

use crate::agents_view::mark;
use crate::look::{self, BAR_H, BODY_H, Look, PANEL_H, PANEL_W, caption, font};
use crate::model::Provider;
use crate::ui::{self, Tab, Ui};
use crate::view::Screen;

const POPOVER_W: f32 = 300.0;

/// How to sign in to each provider, as one line.
fn how(provider: Provider) -> &'static str {
    match provider {
        Provider::Claude => "Run claude in a terminal and sign in",
        Provider::Codex => "Run codex login in a terminal",
        Provider::DeepSeek => "Uses an API key, kept in your Keychain",
    }
}

/// The Agents tab with no agent: each provider, how to connect it, and a button that checks
/// again or asks for the key.
pub fn no_agents(s: &Ui, l: &Look) -> Screen {
    let context = look::context_bar(l, look::context_title(l, "Agents", None, Some("None connected yet".to_owned())), None);
    let mut card = col().key("providers").w_full().radius(12.0).continuous_corners().fill(l.card);
    for (at, provider) in [Provider::Claude, Provider::Codex, Provider::DeepSeek].into_iter().enumerate() {
        let action = match provider {
            Provider::DeepSeek => look::button(l, "add-key", "Add key").on_press_tx(ui::toggle_adding),
            provider => look::button(l, if provider == Provider::Claude { "check-claude" } else { "check-codex" }, "Check again").on_press(move |s: &mut Ui| ui::check_again(s, provider)),
        };
        let (glyph, color, _) = mark(l, provider);
        let mut line = row()
            .key(provider.id())
            .w_full()
            .h(56.0)
            .px(16.0)
            .gap(12.0)
            .center_y()
            .child(row().size(28.0, 28.0).radius(7.0).center().fill(l.chip).child(icon(glyph, 16.0).color(color)))
            .child(col().grow(1.0).min_w(0.0).gap(2.0).child(text(provider.name()).font(font(13.0, 600.0)).color(l.text)).child(text(how(provider)).font(font(11.0, 400.0)).color(l.faint).truncate().min_w(0.0)))
            .child(action);
        if at < 2 {
            line = line.border_sides([0.0, 0.0, 1.0, 0.0], l.stroke);
        }
        card = card.child(line);
    }
    let _ = s;
    let body = col().key("no-agents").w_full().h(BODY_H).center().child(
        col()
            .w(440.0)
            .gap(20.0)
            .child(
                col()
                    .gap(6.0)
                    .child(text("Connect an agent").font(look::figure(22.0)).color(l.text))
                    .child(text("Nerda Bar reads the logins your tools already have. Nothing is sent anywhere but each provider's own service.").font(font(13.0, 400.0)).color(l.dim).w_full().wrap()),
            )
            .child(card)
            .child(caption(l, "Signed-in agents appear here within a minute.")),
    );
    Screen { context, body }
}

/// The Add an agent popover over the panel, under the context bar's +: every provider, the
/// added ones marked, a way to add each of the others. A click outside it closes it.
pub fn popover(s: &Ui, l: &Look) -> El {
    let agents = s.agents();
    let mut card = col().w_full().radius(8.0).continuous_corners().fill(l.card);
    for provider in [Provider::Claude, Provider::Codex] {
        let tab = if provider == Provider::Claude { Tab::Claude } else { Tab::Codex };
        let trailing = if agents.contains(&tab) {
            caption(l, "Added")
        } else {
            look::button(l, if provider == Provider::Claude { "popover-check-claude" } else { "popover-check-codex" }, "Check again").on_press(move |s: &mut Ui| ui::check_again(s, provider)).help(how(provider))
        };
        let (glyph, color, _) = mark(l, provider);
        card = card.child(
            row()
                .key(provider.id())
                .w_full()
                .h(40.0)
                .px(10.0)
                .gap(10.0)
                .center_y()
                .border_sides([0.0, 0.0, 1.0, 0.0], l.stroke)
                .child(icon(glyph, 14.0).color(color))
                .child(text(provider.name()).font(font(12.0, 500.0)).color(l.text))
                .child(spacer())
                .child(trailing),
        );
    }
    card = card.child(deepseek(s, l, agents.contains(&Tab::DeepSeek)));
    let popover = col()
        .key("add-popover")
        .absolute(12.0, 2.0 * BAR_H + 4.0)
        .w(POPOVER_W)
        .padding(12.0)
        .gap(12.0)
        .radius(12.0)
        .continuous_corners()
        .fill(l.float)
        .border(1.0, l.edge)
        .shadow(12.0, 32.0, Color::BLACK.alpha(if l.dark { 0.45 } else { 0.18 }))
        // Presses inside stay inside.
        .on_press(|_: &mut Ui| {})
        .child(col().px(4.0).gap(2.0).child(text("Add an agent").font(font(13.0, 600.0)).color(l.text)).child(caption(l, "Signed-in agents are added for you.")))
        .child(card);
    el().key("add-backdrop").absolute(0.0, 0.0).size(PANEL_W, PANEL_H).on_press_tx(ui::toggle_adding).child(popover)
}

/// DeepSeek's part of the popover: added, or a field for its API key and Add.
fn deepseek(s: &Ui, l: &Look, added: bool) -> El {
    let (glyph, color, _) = mark(l, Provider::DeepSeek);
    let head = row().w_full().h(16.0).gap(10.0).center_y().child(icon(glyph, 14.0).color(color)).child(text("DeepSeek").font(font(12.0, if added { 500.0 } else { 600.0 })).color(l.text)).child(spacer());
    if added {
        return row().key("deepseek").w_full().h(40.0).px(10.0).center_y().child(head.child(caption(l, "Added")));
    }
    let field = row()
        .key("deepseek-key-box")
        .w_full()
        .h(28.0)
        .px(8.0)
        .radius(6.0)
        .continuous_corners()
        .fill(l.panel.alpha(1.0))
        .border(1.0, l.normal)
        .center_y()
        .child(
            text_input(&s.deepseek_key, font(12.0, 400.0))
                .key("deepseek-key")
                .label("DeepSeek API key")
                .single_line()
                .placeholder("Paste your API key", l.faint)
                .caret(l.normal, l.normal.alpha(0.3))
                .autofocus()
                .grow(1.0)
                .min_w(0.0)
                .on_edit(|s: &mut Ui| &mut s.deepseek_key)
                .on_submit(ui::add_deepseek_key),
        );
    let add = row()
        .key("add-deepseek")
        .role("button")
        .label("Add DeepSeek")
        .h(24.0)
        .px(10.0)
        .radius(6.0)
        .continuous_corners()
        .center()
        .fill(l.normal)
        .press_scale(0.96)
        .on_press_tx(ui::add_deepseek_key)
        .child(text("Add").font(font(11.0, 600.0)).color(Color::WHITE));
    let (note, note_color) = match &s.key_error {
        Some(reason) => (reason.clone(), l.critical_text),
        None => ("Kept in your Keychain as \u{201c}Nerda Bar\u{201d}.".to_owned(), l.faint),
    };
    col()
        .key("deepseek")
        .w_full()
        .padding(10.0)
        .gap(8.0)
        .child(head.child(caption(l, "API key")))
        .child(field)
        .child(row().w_full().gap(8.0).center_y().child(text(note).font(font(11.0, 400.0)).color(note_color).w_full().wrap().grow(1.0).min_w(0.0)).child(add))
}
