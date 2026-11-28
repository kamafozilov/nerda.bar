//! The settings, in place of a tab. On the left, the agents: a row each with its menu bar item
//! and its tab, how often they are refreshed, and under them the options of the agent picked.
//! On the right, this Mac's measures as a table of their menu bar style and whether they have
//! a tab, then the general settings. A group is a caption over a card of rows, name on the left
//! and its control on the right, as System Settings lays a group out.

use rune::{Color, El, Icon, col, icon, row, text};

use crate::config::{MenuStyle, REFRESH_CHOICES};
use crate::icons;
use crate::look::{Look, PAD_X, PAD_Y, columns, context_bar, context_title, font, segment, segments, switch};
use crate::model::{self, Problem, Provider};
use crate::system_view;
use crate::ui::{self, Tab, Ui};

/// A row of a group's card.
const SETTING_H: f32 = 44.0;
/// The space between two groups of a column.
const GROUP_GAP: f32 = 20.0;
/// The width of This Mac's menu bar styles, the same on every row so the column lines up.
const STYLES_W: f32 = 172.0;
/// The width of a switch, and of the agents' Menu bar column, which its heading is wider than.
const SWITCH_W: f32 = 32.0;
const MENU_BAR_W: f32 = 52.0;
/// The space between a row's controls, and at a card row's ends.
const ROW_PAD: f32 = 12.0;

pub fn screen(s: &Ui, l: &Look) -> crate::view::Screen {
    let version = concat!("Nerda Bar ", env!("CARGO_PKG_VERSION")).to_owned();
    let context = context_bar(l, context_title(l, "Settings", None, Some(version)), Some(quit_button(l)));
    let quiet = text("An agent with no menu bar item and no tab is not asked for anything. Logins are only ever read.").font(font(11.0, 400.0)).color(l.faint).w_full().wrap();
    let left = column("settings-left").child(agents(s, l)).child(options(s, l)).child(row().key("settings-note").w_full().px(4.0).child(quiet));
    let right = column("settings-right").child(this_mac(s, l)).child(general(s, l));
    crate::view::Screen { context, body: columns(l, left.key("settings-page"), right) }
}

/// A column of groups.
fn column(key: &'static str) -> El {
    col().key(key).w_full().px(PAD_X).py(PAD_Y).gap(GROUP_GAP)
}

/// The button that quits the app, with its shortcut.
fn quit_button(l: &Look) -> El {
    row()
        .key("quit")
        .role("button")
        .label("Quit Nerda Bar")
        .h(28.0)
        .px(10.0)
        .gap(8.0)
        .radius(7.0)
        .continuous_corners()
        .center_y()
        .fill(l.chip)
        .hover(l.chip_on)
        .press_scale(0.96)
        .on_press_tx(|_: &mut Ui, tx| tx.quit())
        .child(icon(&icons::POWER, 13.0).color(l.dim))
        .child(text("Quit Nerda Bar").font(font(12.0, 500.0)).color(l.text))
        .child(text("⌘Q").font(font(11.0, 400.0)).color(l.faint))
}

/// A group: its caption, with something at its end, over a card of rows split by hairlines.
/// The caption is `caption_h` tall: 16, or more for one that ends in a control.
#[allow(clippy::too_many_arguments)]
fn group(l: &Look, key: &'static str, mark: &'static Icon, mark_color: Color, name: &str, caption_h: f32, end: Option<El>, rows: Vec<El>) -> El {
    let mut caption = row().w_full().h(caption_h).pl(4.0).pr(if caption_h > 16.0 { 0.0 } else { ROW_PAD }).gap(6.0).center_y().child(icon(mark, 12.0).color(mark_color)).child(text(name.to_owned()).font(font(11.0, 600.0)).color(l.dim));
    if let Some(end) = end {
        caption = caption.child(row().grow(1.0).min_w(0.0).justify_end().child(end));
    }
    let mut card = col().w_full().radius(10.0).continuous_corners().fill(l.card);
    for (at, setting) in rows.into_iter().enumerate() {
        card = card.child(if at > 0 { setting.border_sides([1.0, 0.0, 0.0, 0.0], l.stroke) } else { setting });
    }
    col().key(key).w_full().gap(8.0).child(caption).child(card)
}

/// A row of a card: what leads it (a mark and a name), and its control at the end. The name
/// keeps its width; the control gives way.
fn setting(lead: El, control: El) -> El {
    row().w_full().h(SETTING_H).px(ROW_PAD).gap(ROW_PAD).center_y().child(lead).child(row().grow(1.0).min_w(0.0).justify_end().center_y().child(control))
}

/// A setting's name.
fn name(l: &Look, words: &str) -> El {
    text(words.to_owned()).font(font(12.0, 400.0)).color(l.text)
}

/// Words at the end of a row, which shorten before the name does.
fn note(l: &Look, words: String) -> El {
    text(words).font(font(12.0, 400.0)).color(l.faint).truncate().min_w(0.0)
}

/// A header over a column of a table card, centred over its controls.
fn heading(l: &Look, width: f32, words: &str) -> El {
    row().w(width).justify_center().child(text(words.to_owned()).font(font(11.0, 400.0)).color(l.faint))
}

fn general(s: &Ui, l: &Look) -> El {
    let (can_login, login) = (s.login.0 || s.demo, s.login.1);
    let mut login_switch = switch(l, "login", "Launch at Login", login, can_login);
    if can_login {
        login_switch = login_switch.on_press(move |s: &mut Ui| ui::set_launch_at_login(s, !login));
    } else {
        login_switch = login_switch.help("Available when run as Nerda Bar.app");
    }
    let glass_now = s.material == rune::SystemMaterial::Glass;
    let mut material = segments(l, "material").help("Liquid Glass needs macOS 26 or later");
    for (glass, label) in [(false, "Frosted"), (true, "Glass")] {
        material = material.child(segment(l, format!("material-{label}"), label, glass_now == glass, move |_: &mut Ui| {}).on_press_tx(move |s: &mut Ui, tx| ui::set_material(s, glass, tx)));
    }
    let open_file = crate::look::button(l, "settings-file", "Open").label("Open settings file").help("config.json: edits made there apply the next time the panel opens").on_press_tx(ui::open_settings_file);
    let rows = vec![setting(name(l, "Launch at Login"), login_switch), setting(name(l, "Panel"), material), setting(name(l, "Settings file"), open_file)];
    group(l, "settings-general", &icons::SETTINGS_2, l.dim, "General", 16.0, None, rows)
}

/// An agent's mark and colour.
fn mark(l: &Look, provider: Provider) -> (&'static Icon, Color) {
    match provider {
        Provider::Claude => (&icons::CLAUDE, l.brand),
        Provider::Codex => (&icons::OPENAI, l.text),
        Provider::DeepSeek => (&icons::DEEPSEEK, l.deepseek),
    }
}

/// Where an agent's login stands, in a few words.
fn login_state(s: &Ui, provider: Provider) -> String {
    let (plan, there, problem) = match provider {
        Provider::DeepSeek => (String::new(), s.deepseek.ledger.is_some(), s.deepseek.problem.as_ref()),
        provider => {
            let agent = s.agent(provider);
            (agent.snapshot.as_ref().map(|snapshot| snapshot.plan.clone()).unwrap_or_default(), agent.snapshot.is_some(), agent.problem.as_ref())
        }
    };
    if !s.config.shows(provider) {
        return "Off".to_owned();
    }
    match (there, problem) {
        (_, Some(Problem::SignedOut)) if provider == Provider::DeepSeek => "No API key".to_owned(),
        (_, Some(Problem::SignedOut)) => "Not signed in".to_owned(),
        (_, Some(Problem::LoginExpired)) if provider == Provider::DeepSeek => "Key turned down".to_owned(),
        (_, Some(Problem::LoginExpired)) => "Login expired".to_owned(),
        (true, _) if provider == Provider::DeepSeek => "API key added".to_owned(),
        (true, _) if plan.is_empty() => "Signed in".to_owned(),
        (true, _) => plan,
        (false, Some(problem)) => problem.title().to_owned(),
        (false, None) => "Reading…".to_owned(),
    }
}

/// Every agent: where its login stands, whether it has a menu bar item and whether it has a
/// tab; then how often they are all refreshed.
fn agents(s: &Ui, l: &Look) -> El {
    let head = row().gap(ROW_PAD).center_y().child(heading(l, MENU_BAR_W, "Menu bar")).child(heading(l, SWITCH_W, "Tab"));
    let mut rows: Vec<El> = Provider::ALL.into_iter().map(|provider| agent(s, l, provider)).collect();
    let every = s.config.refresh_minutes.max(REFRESH_CHOICES[0]);
    let mut refresh = segments(l, "refresh-every").help("How often each agent is asked for its limits or balance");
    for minutes in REFRESH_CHOICES {
        refresh = refresh.child(segment(l, format!("refresh-every-{minutes}"), &format!("{minutes} min"), every == minutes, move |s: &mut Ui| ui::set_refresh_minutes(s, minutes)));
    }
    rows.push(setting(name(l, "Refresh every"), refresh).key("settings-refresh"));
    group(l, "settings-agents", &icons::SPARKLES, l.dim, "Agents", 16.0, Some(head), rows)
}

/// An agent's row: its mark, name and login, its menu bar switch and its tab switch.
fn agent(s: &Ui, l: &Look, provider: Provider) -> El {
    let (glyph, color) = mark(l, provider);
    let shown = s.config.shows(provider);
    let lead = row().gap(10.0).center_y().child(icon(glyph, 14.0).color(if shown { color } else { l.faint })).child(name(l, provider.name()));
    let (in_menu_bar, has_tab) = (s.config.in_menu_bar(provider), s.config.has_tab(provider));
    let what = if provider == Provider::DeepSeek { "balance" } else { "limits" };
    let menu_bar = switch(l, &format!("{}-in-menu-bar", provider.id()), &format!("{} in the menu bar", provider.name()), in_menu_bar, true)
        .help(format!("{}'s {what} in the menu bar", provider.name()))
        .on_press(move |s: &mut Ui| ui::set_agent_menu_bar(s, provider, !in_menu_bar));
    let tab = switch(l, &format!("{}-has-tab", provider.id()), &format!("{} tab", provider.name()), has_tab, true)
        .help(format!("{}'s tab in the panel", provider.name()))
        .on_press_tx(move |s: &mut Ui, tx| ui::set_agent_tab(s, provider, !has_tab, tx));
    let state = note(l, login_state(s, provider)).help(if shown { "Read from the login its own tool keeps" } else { "With no menu bar item and no tab, it is not asked for anything" });
    let controls = row().gap(ROW_PAD).center_y().min_w(0.0).child(state).child(row().w(MENU_BAR_W).justify_center().child(menu_bar)).child(tab);
    setting(lead, controls).key(format!("settings-{}", provider.id()))
}

/// The options of the agent picked at the caption's end: what its menu bar item shows, and for
/// DeepSeek its key.
fn options(s: &Ui, l: &Look) -> El {
    let picked = s.settings_agent;
    let mut picker = segments(l, "options-agent");
    for provider in Provider::ALL {
        picker = picker.child(segment(l, format!("options-{}", provider.id()), provider.name(), picked == provider, move |_: &mut Ui| {}).on_press_tx(move |s: &mut Ui, tx| ui::set_settings_agent(s, provider, tx)));
    }
    let (glyph, color) = mark(l, picked);
    let rows = match picked {
        Provider::DeepSeek => deepseek(s, l),
        provider => limits(s, l, provider),
    };
    group(l, "settings-options", glyph, color, &format!("{} options", picked.name()), 24.0, Some(picker), rows)
}

/// Claude's or Codex's options: which limit the menu bar shows, how percentages read, and
/// whether each model's own limit shows beside it.
fn limits(s: &Ui, l: &Look, provider: Provider) -> Vec<El> {
    let id = provider.id();
    let snapshot = s.agent(provider).snapshot.as_ref();
    let (metric, show_used, models) = s.menu_bar(provider);
    let choices = model::menu_bar_choices(snapshot);
    let shown = metric.effective(snapshot);
    // Claude's keys are the plain ones the first settings had.
    let prefix = |part: &str| if provider == Provider::Claude { part.to_owned() } else { format!("{id}-{part}") };
    let mut shows = segments(l, &prefix("menu-bar"));
    for choice in choices.iter().copied() {
        // The session limit under the name the plan gives it: "5-hour", "Monthly".
        let label = match (choice, snapshot.and_then(|snapshot| snapshot.limit("session"))) {
            (model::MenuBarMetric::Session, Some(limit)) => limit.label.clone(),
            _ => choice.label().to_owned(),
        };
        shows = shows.child(segment(l, format!("{}-{}", prefix("menu-bar"), choice.label()), &label, shown == choice, move |s: &mut Ui| ui::set_menu_bar(s, provider, choice)));
    }
    // Where the plan has no session, say so: that is why the choice is missing.
    if let Some(snapshot) = snapshot.filter(|_| !choices.contains(&model::MenuBarMetric::Session)) {
        let plan = if snapshot.plan.is_empty() { "This plan" } else { snapshot.plan.as_str() };
        shows = shows.help(format!("{plan} has no session limit"));
    }
    let mut percent = segments(l, &prefix("percent"));
    for (used, label) in [(false, "Left"), (true, "Used")] {
        percent = percent.child(segment(l, format!("{}-{label}", prefix("percent")), label, show_used == used, move |s: &mut Ui| ui::set_provider_show_used(s, provider, used)));
    }
    let models_switch = switch(l, &prefix("menu-bar-models"), "Model limits in menu bar", models, true)
        .help("Each model's own limit beside the plan's, by name")
        .on_press(move |s: &mut Ui| ui::set_menu_bar_models(s, provider, !models));
    vec![
        setting(name(l, "Menu bar shows"), shows).help("Lowest is the plan-wide limit with the least left"),
        setting(name(l, "Percentages"), percent).help("In the menu bar and in the panel"),
        setting(name(l, "Model limits in menu bar"), models_switch),
    ]
}

/// DeepSeek's options: its API key, and the balance under which it warns.
fn deepseek(s: &Ui, l: &Look) -> Vec<El> {
    let quiet = |key: &'static str, words: &str| crate::look::button(l, key, words);
    let has_key = s.connected(Provider::DeepSeek);
    let key = if !has_key {
        row().gap(8.0).center_y().min_w(0.0).child(note(l, "Kept in your Keychain".to_owned())).child(quiet("settings-add-deepseek", "Add key").on_press_tx(ui::start_adding))
    } else if s.confirm_remove_key {
        row()
            .gap(8.0)
            .center_y()
            .min_w(0.0)
            .child(quiet("deepseek-key-keep", "Keep").on_press(ui::cancel_remove_key))
            .child(crate::look::button(l, "deepseek-key-remove", "Remove key").fill(l.critical.alpha(0.18)).on_press_tx(ui::remove_deepseek_key))
    } else {
        let state = s.key_error.clone().unwrap_or_else(|| "In your Keychain".to_owned());
        row()
            .gap(8.0)
            .center_y()
            .min_w(0.0)
            .child(note(l, state))
            .child(quiet("deepseek-key-replace", "Replace").on_press_tx(ui::replace_deepseek_key))
            .child(quiet("deepseek-key-remove", "Remove").on_press_tx(ui::remove_deepseek_key))
    };
    let currency = s.deepseek.ledger.as_ref().and_then(|ledger| ledger.last.as_ref()).map_or("USD", |balance| balance.currency.as_str()).to_owned();
    let now = s.config.deepseek_warn_below;
    let mut warn = segments(l, "deepseek-warn").help("The balance turns orange under this amount");
    for amount in [0.0, 1.0, 5.0, 10.0] {
        let label = if amount == 0.0 { "Off".to_owned() } else { crate::deepseek::money(amount, &currency).replace(".00", "") };
        warn = warn.child(segment(l, format!("deepseek-warn-{amount}"), &label, now == amount, move |s: &mut Ui| ui::set_deepseek_warn_below(s, amount)));
    }
    // The question takes the name's place: what goes, said before it does.
    let key_name = if has_key && s.confirm_remove_key { "Remove key and history?" } else { "API key" };
    vec![setting(name(l, key_name), key).key("settings-deepseek-key"), setting(name(l, "Warn when under"), warn)]
}

fn this_mac(s: &Ui, l: &Look) -> El {
    let head = row().gap(ROW_PAD).center_y().child(heading(l, STYLES_W, "Menu bar")).child(heading(l, SWITCH_W, "Tab"));
    let no_battery = s.system.as_ref().is_some_and(|view| view.battery.is_none());
    let mut rows: Vec<El> = Tab::SYSTEM.into_iter().map(|tab| measure(s, l, tab, tab == Tab::Battery && no_battery)).collect();
    rows.push(sleep(s, l));
    group(l, "settings-mac", &icons::LAPTOP, l.dim, "This Mac", 16.0, Some(head), rows)
}

/// Sleep's row: a cup in the menu bar while the Mac is kept awake, and whether it has a tab.
fn sleep(s: &Ui, l: &Look) -> El {
    let lead = row().gap(10.0).center_y().child(icon(&icons::MOON, 14.0).color(l.dim)).child(name(l, "Sleep"));
    let on = s.config.sleep.menu_bar;
    let mut styles = segments(l, "sleep-menu-bar").w(STYLES_W).label("Keep awake in the menu bar");
    for (shown, label) in [(false, "Off"), (true, "While awake")] {
        styles = styles.child(segment(l, format!("sleep-menu-bar-{label}"), label, on == shown, move |s: &mut Ui| ui::set_sleep_menu_bar(s, shown)).grow(1.0).px(0.0));
    }
    let tab = s.config.sleep.tab;
    let tab_switch = switch(l, "sleep-tab", "Sleep tab", tab, true).on_press_tx(move |s: &mut Ui, tx| ui::set_sleep_tab(s, !tab, tx));
    setting(lead, row().gap(ROW_PAD).center_y().child(styles.help("A cup and the time left, while this Mac is kept awake")).child(tab_switch)).key("settings-sleep")
}

/// A measure's row: its menu bar style and whether it has a tab, or why it has neither.
fn measure(s: &Ui, l: &Look, tab: Tab, absent: bool) -> El {
    let lead = row().gap(10.0).center_y().child(icon(system_view::glyph(tab), 14.0).color(if absent { l.faint } else { l.dim })).child(name(l, tab.label()));
    if absent {
        return setting(lead, note(l, "This Mac has no battery".to_owned())).key(format!("settings-{}", tab.id()));
    }
    let module = s.module(tab).unwrap_or_default();
    let mut styles = segments(l, &format!("{}-menu-bar", tab.id())).w(STYLES_W).label(format!("{} in the menu bar", tab.label()));
    for style in MenuStyle::ALL {
        let key = format!("{}-menu-bar-{}", tab.id(), style.label());
        styles = styles.child(segment(l, key, style.label(), module.menu_bar == style, move |s: &mut Ui| ui::set_module_menu_bar(s, tab, style)).grow(1.0).px(0.0));
    }
    let shown = module.tab;
    let tab_switch = switch(l, &format!("{}-tab", tab.id()), &format!("{} tab", tab.label()), shown, true).on_press_tx(move |s: &mut Ui, tx| ui::set_module_tab(s, tab, !shown, tx));
    setting(lead, row().gap(ROW_PAD).center_y().child(styles).child(tab_switch)).key(format!("settings-{}", tab.id()))
}
