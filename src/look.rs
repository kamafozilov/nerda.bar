//! The panel's colours, type, geometry and the pieces every tab is built from: the top bar's
//! tiles, the context bar under it, the two columns of a tab's body, a section with its title,
//! segmented choices, a switch, a meter, a legend key, the line that reads a chart's point, list
//! rows and the "N more" line under a list.
//!
//! The panel has one size on every tab and in every state, so each region's height is known
//! before layout: a section's charts take what is left of their column, and a list shows as
//! many rows as fit. `DESIGN.md` at the repository's root describes the system these follow.

use rune::{Color, El, Icon, TextStyle, col, el, icon, row, spacer, text};

use crate::icons;
use crate::model::Severity;
use crate::ui::{self, Ui};

/// The panel, the same size on every tab and in every state.
pub(crate) const PANEL_W: f32 = 760.0;
pub(crate) const PANEL_H: f32 = 640.0;
/// The top bar of tabs, and the context bar under it.
pub(crate) const BAR_H: f32 = 48.0;
/// What the two bars leave for a tab's body.
pub(crate) const BODY_H: f32 = PANEL_H - 2.0 * BAR_H;
/// The left column; the right one takes the rest, which is the same.
pub(crate) const COL_W: f32 = PANEL_W / 2.0;
/// A section's side padding, and what it leaves for the section's content.
pub(crate) const PAD_X: f32 = 20.0;
pub(crate) const INNER_W: f32 = COL_W - 2.0 * PAD_X;
/// A section's top and bottom padding, its title row, and the gap between its parts.
pub(crate) const PAD_Y: f32 = 16.0;
pub(crate) const TITLE_H: f32 = 24.0;
pub(crate) const GAP: f32 = 12.0;
/// A one-line list row, a two-line one, and the "N more …" line under a list.
pub(crate) const ROW_H: f32 = 32.0;
pub(crate) const TALL_ROW_H: f32 = 52.0;
pub(crate) const MORE_H: f32 = 20.0;
/// A chart's point readout under it.
pub(crate) const READOUT_H: f32 = 32.0;

/// The colours of one appearance, as the design's tokens name them.
#[derive(Clone, Copy)]
pub(crate) struct Look {
    pub dark: bool,
    pub text: Color,
    pub dim: Color,
    pub faint: Color,
    pub hover: Color,
    pub stroke: Color,
    pub edge: Color,
    pub track: Color,
    pub tick: Color,
    pub chip: Color,
    /// The picked tab, segment or tile.
    pub chip_on: Color,
    /// Behind a group of rows, and behind the picked limit.
    pub card: Color,
    pub panel: Color,
    pub panel_edge: Color,
    /// The top bar of tabs.
    pub bar: Color,
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
    pub good_text: Color,
    /// The chart series, in this fixed order: blue, teal, purple. Checked for colour-blind
    /// separation and against the panel in both appearances.
    pub series: [Color; 3],
    /// The fourth series and the rest: a grey.
    pub series_rest: Color,
    /// A recorded line's estimated stretches, per severity.
    pub estimated: [Color; 3],
    /// A problem banner's wash.
    pub banner: Color,
    /// DeepSeek's whale.
    pub deepseek: Color,
    /// A popover or tooltip, over the panel.
    pub float: Color,
}

impl Look {
    pub(crate) fn new(dark: bool) -> Look {
        let (white, black) = (Color::WHITE, Color::BLACK);
        let hex = |s: &str| rune::color::hex(s).unwrap_or(Color::WHITE);
        if dark {
            Look {
                dark: true,
                text: white.alpha(0.92),
                dim: white.alpha(0.66),
                // The floor for any text: 4.5:1 on the panel at 11 points.
                faint: white.alpha(0.52),
                hover: white.alpha(0.06),
                stroke: white.alpha(0.09),
                edge: white.alpha(0.14),
                track: white.alpha(0.12),
                tick: white.alpha(0.92),
                chip: white.alpha(0.08),
                chip_on: white.alpha(0.16),
                card: white.alpha(0.05),
                panel: hex("#1c1c1f").alpha(0.86),
                panel_edge: white.alpha(0.14),
                bar: black.alpha(0.18),
                shadow: black.alpha(0.5),
                normal: hex("#0a84ff"),
                warning: hex("#ff9f0a"),
                critical: hex("#ff453a"),
                warning_text: hex("#ff9f0a"),
                critical_text: hex("#ff453a"),
                brand: hex("#d97757"),
                good: hex("#30d158"),
                good_text: hex("#30d158"),
                series: [hex("#0a84ff"), hex("#1aa3b5"), hex("#bf5af2")],
                series_rest: white.alpha(0.32),
                estimated: [hex("#7db9ff"), hex("#ffc266"), hex("#ff8f88")],
                banner: hex("#ff9f0a").alpha(0.12),
                deepseek: hex("#4d6bfe"),
                float: hex("#2a2a2e"),
            }
        } else {
            Look {
                dark: false,
                text: black.alpha(0.88),
                dim: black.alpha(0.66),
                faint: black.alpha(0.55),
                hover: black.alpha(0.05),
                stroke: black.alpha(0.08),
                edge: black.alpha(0.1),
                track: black.alpha(0.09),
                tick: black.alpha(0.88),
                chip: black.alpha(0.05),
                chip_on: white,
                card: white.alpha(0.85),
                panel: hex("#f5f5f7").alpha(0.9),
                panel_edge: white.alpha(0.7),
                bar: black.alpha(0.035),
                shadow: black.alpha(0.22),
                normal: hex("#007aff"),
                warning: hex("#ff9500"),
                critical: hex("#ff3b30"),
                warning_text: hex("#b25e00"),
                critical_text: hex("#d6001a"),
                brand: hex("#d97757"),
                good: hex("#28a745"),
                good_text: hex("#1e7b34"),
                series: [hex("#007aff"), hex("#1597ab"), hex("#a64fe0")],
                series_rest: black.alpha(0.25),
                estimated: [hex("#7fb6ff"), hex("#ffbe5c"), hex("#ff8a84")],
                banner: hex("#ff9500").alpha(0.12),
                deepseek: hex("#4d6bfe"),
                float: hex("#ffffff"),
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

    /// A list's colour for its `at`th series: three colours, then grey for the rest.
    pub(crate) fn series(&self, at: usize) -> Color {
        self.series.get(at).copied().unwrap_or(self.series_rest)
    }
}

/// Text at `size` points: its line height is the design's for that size, so every region's
/// height is known before layout.
pub(crate) fn font(size: f32, weight: f32) -> TextStyle {
    TextStyle::new(size, weight).lh(line_height(size))
}

/// Digits that keep their width, so a changing number doesn't shift what is beside it.
pub(crate) fn digits(size: f32, weight: f32) -> TextStyle {
    font(size, weight).features("tnum")
}

/// A headline number: bold, tight and with digits of one width.
pub(crate) fn figure(size: f32) -> TextStyle {
    let tracking = match size {
        s if s >= 34.0 => -0.025,
        s if s >= 28.0 => -0.02,
        _ => -0.01,
    };
    font(size, 700.0).features("tnum").tracking(size * tracking)
}

/// The design's line height for each type size.
pub(crate) fn line_height(size: f32) -> f32 {
    match size {
        s if s <= 11.0 => 14.0,
        s if s <= 13.0 => 16.0,
        s if s <= 15.0 => 20.0,
        s if s <= 22.0 => 26.0,
        s if s <= 28.0 => 32.0,
        _ => 40.0,
    }
}

/// A tab of the top bar: an icon on a square tile, its name and shortcut in the tooltip.
pub(crate) fn nav_tile(l: &Look, key: &'static str, glyph: &'static Icon, name: &str, shortcut: Option<&str>, on: bool) -> El {
    let help = match shortcut {
        Some(keys) => format!("{name}  {keys}"),
        None => name.to_owned(),
    };
    row()
        .key(key)
        .role("tab")
        .label(name.to_owned())
        .help(help)
        .size(32.0, 32.0)
        .radius(8.0)
        .continuous_corners()
        .center()
        .fill(if on { l.chip_on } else { Color::TRANSPARENT })
        .hover(if on { l.chip_on } else { l.chip })
        .press_scale(0.94)
        .child(icon(glyph, 16.0).color(if on { l.text } else { l.dim }))
}

/// The bar under the top bar: what this tab is about, and what can be done to it.
pub(crate) fn context_bar(l: &Look, left: El, right: Option<El>) -> El {
    let mut bar = row()
        .key("context")
        .w_full()
        .h(BAR_H)
        .pl(PAD_X)
        .pr(PAD_X)
        .gap(12.0)
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], l.stroke)
        .child(row().key("context-left").grow(1.0).min_w(0.0).gap(8.0).center_y().child(left));
    if let Some(right) = right {
        bar = bar.child(right);
    }
    bar
}

/// A tab's title in the context bar, a badge beside it and a quiet line after them.
pub(crate) fn context_title(l: &Look, title: &str, badge: Option<&str>, subtitle: Option<String>) -> El {
    let mut line = row().gap(8.0).center_y().min_w(0.0).child(text(title.to_owned()).key("title").font(font(15.0, 600.0)).color(l.text));
    if let Some(badge) = badge.filter(|badge| !badge.is_empty()) {
        line = line.child(
            row()
                .key("badge")
                .h(18.0)
                .px(6.0)
                .radius(5.0)
                .border(1.0, l.edge)
                .center()
                .child(text(badge.to_owned()).font(font(11.0, 600.0)).color(l.dim)),
        );
    }
    if let Some(subtitle) = subtitle {
        line = line.child(text(subtitle).key("subtitle").font(digits(11.0, 400.0)).color(l.faint).truncate().min_w(0.0));
    }
    line
}

/// A tab's body: two columns of [`COL_W`], a hairline between them.
pub(crate) fn columns(l: &Look, left: El, right: El) -> El {
    columns_in(l, BODY_H, left, right)
}

/// [`columns`] `height` tall, for a body that gives some of its height to a strip over them.
pub(crate) fn columns_in(l: &Look, height: f32, left: El, right: El) -> El {
    row()
        .key("body")
        .w_full()
        .h(height)
        .child(column("left", height).w(COL_W).border_sides([0.0, 1.0, 0.0, 0.0], l.stroke).child(left))
        .child(column("right", height).grow(1.0).min_w(0.0).child(right))
}

/// A column of sections, `height` tall, cutting off anything that runs past its foot.
fn column(key: &'static str, height: f32) -> El {
    col().key(key).h(height).clip_rounded(0.0)
}

/// A section of a column: its padding, and its parts a gap apart. Any section but a column's
/// first has a hairline over it.
pub(crate) fn section(key: &'static str) -> El {
    col().key(key).w_full().px(PAD_X).py(PAD_Y).gap(GAP)
}

/// A section under another: a hairline over it.
pub(crate) fn next_section(l: &Look, key: &'static str) -> El {
    section(key).border_sides([1.0, 0.0, 0.0, 0.0], l.stroke)
}

/// The height a section's content has when it is `height` tall in all.
pub(crate) fn inside(height: f32) -> f32 {
    height - 2.0 * PAD_Y
}

/// How many rows of `row_h` fit in `room`, leaving the "N more" line its place when not all
/// `count` fit.
pub(crate) fn rows_that_fit(room: f32, row_h: f32, count: usize) -> usize {
    let all = (room / row_h).floor().max(0.0) as usize;
    if count <= all { count } else { ((room - MORE_H) / row_h).floor().max(0.0) as usize }
}

/// A section's title, with something at its end: a caption, or segmented choices.
pub(crate) fn section_title(l: &Look, title: &str, end: Option<El>) -> El {
    let mut bar = row().w_full().h(TITLE_H).center_y().gap(12.0).child(text(title.to_owned()).font(font(13.0, 600.0)).color(l.text)).child(spacer());
    if let Some(end) = end {
        bar = bar.child(end);
    }
    bar
}

/// A section title's quiet caption.
pub(crate) fn caption(l: &Look, words: impl Into<String>) -> El {
    text(words.into()).font(font(11.0, 400.0)).color(l.faint)
}

/// The "N more apps, 1.2 GB together" line under a list, at the bottom of its place.
pub(crate) fn more_line(l: &Look, words: impl Into<String>) -> El {
    row().key("more").w_full().h(MORE_H).items_end().child(caption(l, words).truncate().min_w(0.0))
}

/// A row of choices where one is on, as AppKit's segmented control.
pub(crate) fn segments(l: &Look, key: &str) -> El {
    row().key(key).padding(2.0).gap(2.0).radius(7.0).continuous_corners().fill(l.chip).center_y()
}

pub(crate) fn segment(l: &Look, key: String, label: &str, on: bool, pick: impl Fn(&mut Ui) + 'static) -> El {
    let mut segment = row()
        .key(key)
        .role("button")
        .label(label)
        .h(20.0)
        .px(8.0)
        .radius(5.0)
        .continuous_corners()
        .center()
        .fill(if on { l.chip_on } else { Color::TRANSPARENT })
        .on_press(pick)
        .child(text(label.to_owned()).font(font(11.0, if on { 600.0 } else { 500.0 })).color(if on { l.text } else { l.dim }));
    if on && !l.dark {
        segment = segment.shadow(1.0, 2.0, Color::BLACK.alpha(0.14));
    }
    segment
}

/// A switch, as System Settings draws a small one.
pub(crate) fn switch(l: &Look, key: &str, label: &str, on: bool, enabled: bool) -> El {
    let mut switch = row()
        .key(key)
        .role("switch")
        .label(label)
        .size(32.0, 18.0)
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
        .child(text(name.to_owned()).font(font(12.0, 400.0)).color(l.dim))
        .child(text(value).font(digits(12.0, 600.0)).color(l.text))
}

/// A line's key: a short stroke, solid or dashed, and its name.
pub(crate) fn legend_line(l: &Look, color: Color, dash: Option<(f32, f32)>, name: &str) -> El {
    let mut shape = rune::Shape::new();
    shape = match dash {
        Some((on, off)) => crate::burn_view::dashed(shape, [0.0, 1.5], [12.0, 1.5], on, off, 2.0, rune::LineCap::Butt, color),
        None => shape.line([0.0, 1.5], [12.0, 1.5], 2.0, rune::LineCap::Butt, color),
    };
    row().gap(6.0).center_y().child(rune::canvas(shape).size(12.0, 3.0)).child(text(name.to_owned()).font(font(11.0, 400.0)).color(l.dim))
}

/// The line under a chart reading one point: a ring as on the chart, when that is, and its
/// value. It keeps its shape as the pointer moves, so only the words change.
pub(crate) fn readout(l: &Look, ring: Option<Color>, name: String, side: String, value: String) -> El {
    let mut when = row().gap(8.0).center_y().min_w(0.0);
    if let Some(color) = ring {
        when = when.child(el().size(8.0, 8.0).radius(4.0).border(2.0, color));
    }
    when = when.child(text(name).font(digits(12.0, 400.0)).color(l.dim).truncate().min_w(0.0));
    let mut line = row().key("chart-readout").w_full().h(READOUT_H).px(12.0).gap(8.0).radius(8.0).continuous_corners().fill(l.card).center_y().child(when).child(spacer());
    if !side.is_empty() {
        line = line.child(text(side).font(digits(12.0, 400.0)).color(l.dim));
    }
    line.child(text(value).font(digits(12.0, 600.0)).color(l.text))
}

/// A pill with a dot: a status in a word or two.
pub(crate) fn pill(key: &'static str, color: Color, words: String) -> El {
    row()
        .key(key)
        .h(22.0)
        .px(8.0)
        .gap(6.0)
        .radius(11.0)
        .center_y()
        .fill(color.alpha(0.14))
        .child(el().size(6.0, 6.0).radius(3.0).fill(color))
        .child(text(words).font(digits(11.0, 600.0)).color(color))
}

/// A quiet pill with no dot: a fact about the tab, "382 GB available".
pub(crate) fn chip_label(l: &Look, key: &'static str, words: String) -> El {
    row().key(key).h(22.0).px(8.0).radius(11.0).center_y().fill(l.chip).child(text(words).font(digits(11.0, 600.0)).color(l.text))
}

/// A button: a word on a chip.
pub(crate) fn button(l: &Look, key: &'static str, words: &str) -> El {
    row()
        .key(key)
        .role("button")
        .label(words.to_owned())
        .h(24.0)
        .px(10.0)
        .gap(6.0)
        .radius(6.0)
        .continuous_corners()
        .center()
        .fill(l.chip)
        .hover(l.chip_on)
        .press_scale(0.96)
        .child(text(words.to_owned()).font(font(11.0, 600.0)).color(l.text))
}

/// A square button with an icon, on a chip.
pub(crate) fn icon_button(l: &Look, key: &'static str, glyph: &'static Icon, name: &'static str, size: f32, glyph_size: f32) -> El {
    row()
        .key(key)
        .role("button")
        .label(name)
        .help(name)
        .size(size, size)
        .radius(7.0)
        .continuous_corners()
        .center()
        .fill(l.chip)
        .hover(l.chip_on)
        .press_scale(0.94)
        .child(icon(glyph, glyph_size).color(l.dim))
}

/// The refresh button of a provider's context bar, quieter while a refresh runs.
pub(crate) fn refresh_button(l: &Look, refreshing: bool) -> El {
    icon_button(l, "refresh", &icons::ROTATE_CW, "Refresh (⌘R)", 28.0, 14.0).opacity(if refreshing { 0.45 } else { 1.0 }).on_press(ui::refresh)
}

/// A value in a list row's trailing column of `width`, right-aligned.
pub(crate) fn cell(width: f32, value: El) -> El {
    row().w(width).justify_end().child(value)
}
