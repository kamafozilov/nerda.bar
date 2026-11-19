---
name: Nerda Bar
description: AI plan limits and this Mac's load, in one fixed-size panel under the menu bar.
colors:
  panel: "#1C1C1F"
  bar: "#161618"
  text: "rgba(255,255,255,0.92)"
  dim: "rgba(255,255,255,0.66)"
  faint: "rgba(255,255,255,0.52)"
  stroke: "rgba(255,255,255,0.09)"
  edge: "rgba(255,255,255,0.14)"
  track: "rgba(255,255,255,0.12)"
  chip: "rgba(255,255,255,0.08)"
  chip-on: "rgba(255,255,255,0.16)"
  card: "rgba(255,255,255,0.05)"
  normal: "#0A84FF"
  series-2: "#1AA3B5"
  series-3: "#BF5AF2"
  series-4: "rgba(255,255,255,0.32)"
  good: "#30D158"
  good-wash: "rgba(48,209,88,0.14)"
  warning: "#FF9F0A"
  critical: "#FF453A"
  claude: "#D97757"
  deepseek: "#4D6BFE"
  light-panel: "#F5F5F7"
  light-bar: "#ECECEE"
  light-text: "rgba(0,0,0,0.88)"
  light-dim: "rgba(0,0,0,0.66)"
  light-faint: "rgba(0,0,0,0.55)"
  light-stroke: "rgba(0,0,0,0.08)"
  light-edge: "rgba(0,0,0,0.10)"
  light-track: "rgba(0,0,0,0.09)"
  light-chip: "rgba(0,0,0,0.05)"
  light-chip-on: "#FFFFFF"
  light-card: "#FFFFFF"
  light-normal: "#007AFF"
  light-series-2: "#1597AB"
  light-series-3: "#A64FE0"
  light-series-4: "rgba(0,0,0,0.25)"
  light-good: "#28A745"
  light-good-text: "#1E7B34"
  light-warning: "#B25E00"
  light-critical: "#D6001A"
typography:
  hero:
    fontFamily: "SF Pro"
    fontSize: "34px"
    fontWeight: 700
    lineHeight: "40px"
    letterSpacing: "-0.025em"
    fontFeature: "tnum"
  figure-lg:
    fontFamily: "SF Pro"
    fontSize: "28px"
    fontWeight: 700
    lineHeight: "32px"
    letterSpacing: "-0.02em"
    fontFeature: "tnum"
  figure:
    fontFamily: "SF Pro"
    fontSize: "22px"
    fontWeight: 700
    lineHeight: "26px"
    letterSpacing: "-0.01em"
    fontFeature: "tnum"
  page-title:
    fontFamily: "SF Pro"
    fontSize: "15px"
    fontWeight: 600
    lineHeight: "20px"
  section-title:
    fontFamily: "SF Pro"
    fontSize: "13px"
    fontWeight: 600
    lineHeight: "16px"
  row:
    fontFamily: "SF Pro"
    fontSize: "12px"
    fontWeight: 400
    lineHeight: "16px"
  caption:
    fontFamily: "SF Pro"
    fontSize: "11px"
    fontWeight: 400
    lineHeight: "14px"
rounded:
  xs: "5px"
  sm: "6px"
  control: "7px"
  md: "8px"
  card: "10px"
  popover: "12px"
  panel: "18px"
spacing:
  "1": "4px"
  "2": "8px"
  "3": "12px"
  "4": "16px"
  "5": "20px"
components:
  panel:
    backgroundColor: "{colors.panel}"
    rounded: "{rounded.panel}"
    width: "760px"
    height: "640px"
  top-bar:
    backgroundColor: "{colors.bar}"
    height: "48px"
    padding: "0 8px"
  nav-tile:
    size: "32px"
    rounded: "{rounded.md}"
  nav-tile-active:
    backgroundColor: "{colors.chip-on}"
    textColor: "{colors.text}"
  context-bar:
    height: "48px"
    padding: "0 20px"
  provider-tab-active:
    backgroundColor: "{colors.chip-on}"
    textColor: "{colors.text}"
    rounded: "{rounded.control}"
    height: "28px"
    padding: "0 10px 0 8px"
  section:
    padding: "16px 20px"
  segmented:
    backgroundColor: "{colors.chip}"
    rounded: "{rounded.control}"
    padding: "2px"
  segmented-item-active:
    backgroundColor: "{colors.chip-on}"
    textColor: "{colors.text}"
    rounded: "{rounded.xs}"
    height: "20px"
  limit-row:
    height: "52px"
    rounded: "{rounded.card}"
    padding: "0 12px"
  limit-row-picked:
    backgroundColor: "{colors.card}"
  data-row:
    height: "32px"
    typography: "{typography.row}"
  app-row:
    height: "32px"
    typography: "{typography.row}"
  detail-row:
    height: "52px"
  settings-row:
    height: "48px"
    padding: "0 12px"
  settings-group:
    backgroundColor: "{colors.card}"
    rounded: "{rounded.card}"
  readout:
    backgroundColor: "{colors.card}"
    rounded: "{rounded.md}"
    height: "32px"
    padding: "0 12px"
  pill-good:
    backgroundColor: "{colors.good-wash}"
    textColor: "{colors.good}"
    height: "22px"
    padding: "0 8px"
  tooltip:
    backgroundColor: "#2F2F33"
    rounded: "{rounded.sm}"
    height: "24px"
    padding: "0 8px"
  popover:
    backgroundColor: "#26262A"
    rounded: "{rounded.popover}"
    width: "300px"
    padding: "12px"
---

## Overview

An instrument panel under the menu bar: one fixed 760×640 panel, read in a few seconds and closed. The design reference is the Paper file "Nerda Bar — Redesign": page **02 · Reviewed** holds every tab in dark and light; page **03 · States** holds first run, loading, error and empty, the add-agent popover, the nav tooltip, offline, a Mac without a battery, and worst-case data.

Structure, top to bottom:

1. **Top bar (48pt):** icon-only tabs. Agents, a hairline, CPU, Memory, Disk, Network, Battery, GPU; Settings at the trailing end. A Mac without a battery has no Battery tab.
2. **Context bar (48pt):** what this tab is about. In Agents: one tab per signed-in provider, then `+`, then the plan, its last update and Refresh. In a Mac tab: its name, a badge (chip, capacity) and a quiet subtitle, with one status at the trailing end. In Settings: the version and Quit.
3. **Body:** two columns of 380pt, divided by a hairline, each a stack of sections.

## Colors

Restrained: neutrals from white or black at fixed alphas, one accent (system blue, `normal`) for data and the current selection, green only for "fine" states and the battery, amber for warnings. Provider marks keep their own colours (`claude`, `deepseek`) and appear only on the marks.

- Text has three levels: `text`, `dim`, `faint`. `faint` is the floor for any text: 0.52 in dark and 0.55 in light both clear 4.5:1 at 11px. Nothing readable sits below it.
- Every dark colour has a `light-` twin; both appearances are required.
- Chart series keep their order everywhere: `normal`, `series-2`, `series-3`, `series-4`.

## Typography

SF Pro, the system font, in the app (Inter stands in for it in Paper). Seven roles only: hero 34, figure-lg 28, figure 22, page-title 15, section-title 13, row 12, caption 11. Every number uses tabular figures. No text below 11px.

- The number that answers the tab's question is its largest element: the percentage left of a limit (28), the CPU or memory in use (34).
- Totals beside a chart use `figure` (22).

## Layout

- **Fixed panel.** 760×640 on every tab and in every state; content never resizes the window. 640 fits a 13" MacBook under the menu bar.
- **Spacing scale:** 4, 8, 12, 16, 20 only.
- **Section:** padding 16 top and bottom, 20 at the sides; title row 24 high; 12 from title to content; sections divided by a hairline.
- **Fill rule.** Columns end at the panel's bottom edge. What grows into spare height, in order:
  1. a chart (limit chart, History, Activity, Charge) takes all remaining height;
  2. otherwise a list shows as many fixed-height rows as fit (`floor(available / row height)`), then one "N more …, X together" line pinned to the list's bottom.
  Rows never stretch and gaps never grow to fill space.
- **Row heights by role:** list row 32 for every one-line list (models, projects, apps with a 20pt icon), detail row 52 (two lines: a title and an explanation), settings row 48, limit row 52.
- **Lanes:** leading slots are fixed width (12 for a swatch, rank or status dot; 20 for an app icon); trailing values are right-aligned in fixed columns (tokens 56, cost or value 64).
- **Truncation:** names truncate at the end with a tooltip; the secondary text beside a name (folder, process count, sign-in status) gives way first. Model IDs truncate in the middle, because they differ at the end. Numbers, amounts and times never truncate.

## Elevation & Depth

Flat, tonal. The panel carries the only window shadow. Inside it, depth is one layer of `card` behind a group or a picked row. In light appearance the selected segment and the active tab are white with a 0 1px 2px shadow, as AppKit draws them. Popovers and tooltips float with a soft shadow (0 12px 32px at 45% for the popover, 0 4px 12px for the tooltip).

## Shapes

Radii step down as things get smaller: panel 18, popover 12, card and limit row 10, nav tile and readout 8, segmented control and provider tab 7, button 6, segment 5. Pills (22 high) are the only fully round controls.

## Components

- **Nav tile:** 32×32, glyph 16. Active: `chip-on`. Hover: `chip`. Each has a tooltip with its name and shortcut ("Disk ⌘4").
- **Provider tab:** mark 14 + name 12; no numbers in the tab. Only signed-in providers appear; `+` (28×28, dashed edge) opens the Add an agent popover.
- **Add an agent popover:** lists every provider; added ones say "Added", and DeepSeek takes an API key in a secure field saved to the Keychain item "Nerda Bar".
- **Segmented control:** chip track, padding 2, items 20 high. Fixed width (172) where it lines up in a column, as in Settings › This Mac.
- **Limit row:** ring 32, "Weekly" + scope, "Resets in …", the percentage at figure-lg with "left" under it. The picked row has `card` behind it and drives the chart.
- **Readout:** 32 high card under a chart: a ring like the chart's marker, what the point is, and its value.
- **Pills:** green "Lasts until reset", "Pressure normal"; amber "Offline".
- **Banner:** a 40 high amber-washed strip under the context bar for a provider that failed: what failed, what the data below is now, and Try again. The panel keeps its size; the body gives up the 40.

States every tab must cover:

- **Loading:** skeleton bars in the exact slots of the real content; the Refresh button dims and shows "Updating…".
- **Stale or failed:** the banner, the last-update time in `warning`, and the limits' "As of" time.
- **Empty:** a dashed card with what is missing and when it appears ("No Claude Code messages today yet"); an empty list says what will fill it.
- **No agents:** a centred card listing each provider with how to sign in and a Check again or Add key button.
- **Absent hardware:** no Battery tab; its Settings row says "This Mac has no battery".

Keyboard: ⌘1 Agents, ⌘2–⌘7 the Mac tabs in order, ⌘⇧[ and ⌘⇧] the previous and next provider, ⌘R refresh, ⌘, Settings, ⌘Q quit, Esc closes.

## Do's and Don'ts

- Do keep the panel 760×640 in every state.
- Do let the chart, or more rows, take spare height.
- Do give every number tabular figures and room to show in full.
- Don't stretch rows or gaps to fill a column.
- Don't add a font size, spacing step or radius outside the scales above.
- Don't put a percentage or value in a provider tab or nav tile.
- Don't use text below 11px or below `faint`.
- Don't use glyph characters (↓ ↑) in place of icons or words.
