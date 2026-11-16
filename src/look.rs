//! The panel's colours, type and the small pieces every tab is built from: a tab's header, a
//! section with its title, segmented choices, a switch, a meter, a legend key and the line that
//! reads a chart's point.

use rune::{Color, El, Icon, TextStyle, col, el, icon, row, rule, spacer, text};

use crate::icons;
use crate::model::Severity;
use crate::ui::{self, CONTENT_W, Ui};

pub(crate) const PAD_X: f32 = 16.0;
pub(crate) const INNER_W: f32 = CONTENT_W - 2.0 * PAD_X;

/// The colours of one appearance, as the design's tokens name them.
#[derive(Clone, Copy)]
pub(crate) struct Look {
    pub dark: bool,
    pub text: Color,
    pub dim: Color,
    pub faint: Color,
    pub hover: Color,
    pub stroke: Color,
    pub track: Color,
    pub tick: Color,
    pub chip: Color,
    /// Behind a group of rows, and behind the picked limit.
    pub card: Color,
    pub panel: Color,
    pub panel_edge: Color,
    pub rail: Color,
    pub shadow: Color,
    pub normal: Color,
    pub warning: Color,
    pub critical: Color,
    /// Warning and critical as text, darker in light mode where the fills are too pale to read.
    pub warning_text: Color,
    pub critical_text: Color,
    pub brand: Color,
    /// Status: all is well.
    pub good: Color,
    /// The chart series, in this fixed order: blue, teal, purple. Checked for colour-blind
    /// separation and against the panel in both appearances.
    pub series: [Color; 3],
    /// A recorded line's estimated stretches, per severity.
    pub estimated: [Color; 3],
    /// A problem banner's wash.
    pub banner: Color,
    /// DeepSeek's whale.
    pub deepseek: Color,
}

impl Look {
    pub(crate) fn new(dark: bool) -> Look {
        let (white, black) = (Color::WHITE, Color::BLACK);
        let hex = |s: &str| rune::color::hex(s).unwrap_or(Color::WHITE);
        if dark {
            Look {
                dark: true,
                text: white.alpha(0.92),
                dim: white.alpha(0.58),
                faint: white.alpha(0.36),
                hover: white.alpha(0.08),
                stroke: white.alpha(0.09),
                track: white.alpha(0.12),
                tick: white.alpha(0.92),
                chip: white.alpha(0.1),
                card: white.alpha(0.055),
                panel: hex("#1c1c1f").alpha(0.82),
                panel_edge: white.alpha(0.14),
                rail: black.alpha(0.15),
                shadow: black.alpha(0.5),
                normal: hex("#0a84ff"),
                warning: hex("#ff9f0a"),
                critical: hex("#ff453a"),
                warning_text: hex("#ff9f0a"),
                critical_text: hex("#ff453a"),
                brand: hex("#d97757"),
                good: hex("#30d158"),
                series: [hex("#0a84ff"), hex("#1aa3b5"), hex("#bf5af2")],
                estimated: [hex("#7db9ff"), hex("#ffc266"), hex("#ff8f88")],
                banner: hex("#ff9f0a").alpha(0.16),
                deepseek: hex("#4d6bfe"),
            }
        } else {
            Look {
                dark: false,
                text: black.alpha(0.88),
                dim: black.alpha(0.55),
                faint: black.alpha(0.36),
                hover: black.alpha(0.06),
                stroke: black.alpha(0.08),
                track: black.alpha(0.09),
                tick: black.alpha(0.88),
                chip: black.alpha(0.07),
                card: black.alpha(0.04),
                panel: hex("#f5f5f7").alpha(0.85),
                panel_edge: white.alpha(0.7),
                rail: black.alpha(0.04),
                shadow: black.alpha(0.22),
                normal: hex("#007aff"),
                warning: hex("#ff9500"),
                critical: hex("#ff3b30"),
                warning_text: hex("#b25e00"),
                critical_text: hex("#d6001a"),
                brand: hex("#d97757"),
                good: hex("#28a745"),
                series: [hex("#007aff"), hex("#1597ab"), hex("#a64fe0")],
                estimated: [hex("#7fb6ff"), hex("#ffbe5c"), hex("#ff8a84")],
                banner: hex("#ff9500").alpha(0.14),
                deepseek: hex("#4d6bfe"),
            }
        }
    }

    pub(crate) fn fill(&self, severity: Severity) -> Color {
        match severity {
            Severity::Normal => self.normal,
            Severity::Warning => self.warning,
            Severity::Critical => self.critical,
        }
    }

    pub(crate) fn ink(&self, severity: Severity) -> Color {
        match severity {
            Severity::Normal => self.text,
            Severity::Warning => self.warning_text,
            Severity::Critical => self.critical_text,
        }
    }

    pub(crate) fn estimated(&self, severity: Severity) -> Color {
        self.estimated[severity as usize]
    }

    /// The panel's colour without its translucency, for rings drawn over a chart.
    pub(crate) fn solid(&self) -> Color {
        self.panel.alpha(1.0)
    }
}

pub(crate) const fn font(size: f32, weight: f32) -> TextStyle {
    TextStyle::new(size, weight)
}

/// Digits that keep their width, so a changing number doesn't shift what is beside it.
pub(crate) const fn digits(size: f32, weight: f32) -> TextStyle {
    TextStyle::new(size, weight).features("tnum")
}

/// A headline number: large, tight and with digits of one width.
pub(crate) const fn figure(size: f32) -> TextStyle {
    TextStyle::new(size, 700.0).features("tnum").tracking(-size / 40.0)
}

/// A tab's header: its mark on a tile, its name and a badge, a line under them, and a button
/// at the end when there is something to do.
#[allow(clippy::too_many_arguments)]
pub(crate) fn header(l: &Look, mark: &'static Icon, mark_size: f32, mark_color: Color, tile: Color, name: &str, badge: Option<&str>, subtitle: El, action: Option<El>) -> El {
    let mut line = row().gap(6.0).center_y().child(text(name.to_owned()).font(font(15.0, 700.0)).color(l.text));
    if let Some(badge) = badge.filter(|badge| !badge.is_empty()) {
        line = line.child(
            row()
                .key("badge")
                .h(14.0)
                .px(5.0)
                .radius(5.0)
                .border(1.0, l.track)
                .center()
                .child(text(badge.to_owned()).font(font(10.0, 600.0)).color(l.dim)),
        );
    }
    let mut bar = row()
        .key("header")
        .w_full()
        .pt(14.0)
        .pb(6.0)
        .pl(PAD_X)
        .pr(12.0)
        .gap(10.0)
        .center_y()
        .child(row().key("mark").size(32.0, 32.0).radius(9.0).continuous_corners().fill(tile).center().child(icon(mark, mark_size).color(mark_color)))
        .child(col().grow(1.0).min_w(0.0).gap(1.0).child(line).child(subtitle));
    if let Some(action) = action {
        bar = bar.child(action);
    }
    bar
}

/// The quieter line under a tab's name.
pub(crate) fn subtitle(l: &Look, words: impl Into<String>, color: Option<Color>) -> El {
    text(words.into()).key("subtitle").font(digits(11.0, 400.0)).color(color.unwrap_or(l.faint)).truncate()
}

/// A square button with an icon, on a chip.
pub(crate) fn icon_button(l: &Look, key: &'static str, glyph: &'static Icon, name: &'static str, size: f32, glyph_size: f32) -> El {
    row()
        .key(key)
        .role("button")
        .label(name)
        .help(name)
        .size(size, size)
        .radius(8.0)
        .continuous_corners()
        .center()
        .fill(l.chip)
        .hover(l.hover)
        .press_scale(0.94)
        .child(icon(glyph, glyph_size).color(l.dim))
}

/// A section of a tab under a hairline: its padding, and its parts a gap apart.
pub(crate) fn section(key: &'static str) -> El {
    col().key(key).w_full().px(PAD_X).pt(12.0).pb(14.0).gap(10.0)
}

pub(crate) fn hairline(l: &Look) -> El {
    rule(1.0, l.stroke)
}

/// A section's title, with something at its end: a caption, or segmented choices.
pub(crate) fn section_title(l: &Look, title: &str, end: Option<El>) -> El {
    let mut bar = row().w_full().center_y().child(text(title.to_owned()).font(font(13.0, 700.0)).color(l.text)).child(spacer());
    if let Some(end) = end {
        bar = bar.child(end);
    }
    bar
}

/// A section title's quiet caption.
pub(crate) fn caption(l: &Look, words: impl Into<String>) -> El {
    text(words.into()).font(font(11.0, 400.0)).color(l.faint)
}

/// A row of choices where one is on, as AppKit's segmented control.
pub(crate) fn segments(l: &Look, key: &str) -> El {
    row().key(key).padding(2.0).gap(2.0).radius(8.0).continuous_corners().fill(l.chip).center_y()
}

pub(crate) fn segment(l: &Look, key: String, label: &str, on: bool, pick: impl Fn(&mut Ui) + 'static) -> El {
    row()
        .key(key)
        .role("button")
        .label(label)
        .h(19.0)
        .px(10.0)
        .radius(6.0)
        .continuous_corners()
        .center()
        .fill(if on { l.chip } else { Color::TRANSPARENT })
        .on_press(pick)
        .child(text(label.to_owned()).font(font(11.0, if on { 700.0 } else { 500.0 })).color(if on { l.text } else { l.dim }))
}

/// A switch, as System Settings draws a small one.
pub(crate) fn switch(l: &Look, key: &str, label: &str, on: bool, enabled: bool) -> El {
    let mut switch = row()
        .key(key)
        .role("switch")
        .label(label)
        .size(30.0, 18.0)
        .radius(9.0)
        .padding(2.0)
        .center_y()
        .fill(if on { l.normal } else { l.track })
        .opacity(if enabled { 1.0 } else { 0.45 })
        .child(el().size(14.0, 14.0).radius(7.0).fill(Color::WHITE));
    if on {
        switch = switch.justify_end();
    }
    switch
}

/// A track with its filled share.
pub(crate) fn meter(l: &Look, key: &'static str, width: f32, height: f32, share: f64, fill: Color) -> El {
    let share = share.clamp(0.0, 1.0) as f32;
    let mut track = el().key(key).w(width).h(height).radius(height / 2.0).fill(l.track);
    if share > 0.0 {
        // Never narrower than a dot: a sliver still shows.
        track = track.child(el().absolute(0.0, 0.0).size((width * share).max(height), height).radius(height / 2.0).fill(fill));
    }
    track
}

/// A series' square key.
pub(crate) fn key_swatch(color: Color, size: f32) -> El {
    el().size(size, size).radius(2.0).fill(color)
}

/// A key, a name and a value: a legend entry, so colour never names a series alone.
pub(crate) fn legend_value(l: &Look, color: Color, name: &str, value: String) -> El {
    row()
        .gap(6.0)
        .center_y()
        .child(key_swatch(color, 8.0))
        .child(text(name.to_owned()).font(font(11.5, 400.0)).color(l.dim))
        .child(text(value).font(digits(11.5, 600.0)).color(l.text))
}

/// The line under a chart reading one point: a ring as on the chart, when that is, and its
/// value. It keeps its shape as the pointer moves, so only the words change.
pub(crate) fn readout(l: &Look, ring: Option<Color>, name: String, side: String, value: String) -> El {
    let mut when = row().gap(6.0).center_y();
    if let Some(color) = ring {
        when = when.child(el().size(9.0, 9.0).radius(4.5).border(2.0, color));
    }
    when = when.child(text(name).font(digits(11.5, 400.0)).color(l.dim));
    let mut line = row().key("chart-readout").w_full().h(28.0).px(10.0).radius(8.0).continuous_corners().fill(l.card).center_y().child(when).child(spacer());
    if !side.is_empty() {
        line = line.child(text(side).font(digits(11.5, 400.0)).color(l.dim).pr(10.0));
    }
    line.child(text(value).font(digits(12.0, 700.0)).color(l.text))
}

/// A pill with a dot: a status in a word or two.
pub(crate) fn pill(key: &'static str, color: Color, words: String) -> El {
    row()
        .key(key)
        .h(19.0)
        .px(8.0)
        .gap(5.0)
        .radius(10.0)
        .center_y()
        .fill(color.alpha(0.13))
        .child(el().size(6.0, 6.0).radius(3.0).fill(color))
        .child(text(words).font(digits(11.0, 600.0)).color(color))
}

/// A warning in a tinted card: an icon, a title and what it means.
pub(crate) fn banner(l: &Look, key: &'static str, glyph: &'static Icon, title: &str, detail: String) -> El {
    row()
        .key(key)
        .w_full()
        .padding(12.0)
        .gap(10.0)
        .items_start()
        .radius(10.0)
        .continuous_corners()
        .fill(l.banner)
        .child(icon(glyph, 15.0).color(l.warning_text).pt(1.0))
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(2.0)
                .child(text(title.to_owned()).font(font(12.5, 700.0)).color(l.warning_text))
                .child(text(detail).font(font(11.5, 400.0).lh(16.0)).color(l.dim).w_full().wrap()),
        )
}

/// The settings button and Quit at the foot of the rail.
pub(crate) fn rail_button(l: &Look, key: &'static str, glyph: &'static Icon, name: &'static str, on: bool) -> El {
    row()
        .key(key)
        .role("button")
        .label(name)
        .help(name)
        .size(32.0, 32.0)
        .radius(8.0)
        .continuous_corners()
        .center()
        .fill(if on { l.chip } else { Color::TRANSPARENT })
        .hover(l.hover)
        .press_scale(0.94)
        .child(icon(glyph, 16.0).color(if on { l.text } else { l.dim }))
}

/// The refresh button of a provider's header, quieter while a refresh runs.
pub(crate) fn refresh_button(l: &Look, refreshing: bool) -> El {
    icon_button(l, "refresh", &icons::ROTATE_CW, "Refresh (⌘R)", 28.0, 14.0).opacity(if refreshing { 0.45 } else { 1.0 }).on_press(ui::refresh)
}
