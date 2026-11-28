# Product

<!-- impeccable:product-schema 1 -->

## Platform

macos

Native macOS menu bar app (not web, iOS or Android). The panel is drawn by Rune, a Rust UI toolkit, not AppKit or SwiftUI controls.

## Users

A developer who works all day with Claude Code or the Codex CLI. From the menu bar they glance at how much of the plan is left, when it resets, which model and which project spend the most, and what the Mac's CPU and memory are doing.

## Product Purpose

Show an AI plan's limits and usage, and this Mac's load, in the menu bar and in one panel under it. Success: the answer is readable in one glance, without opening a browser, a dashboard or Activity Monitor.

## Positioning

Limits are the exact numbers each provider reports; tokens are counted from Claude Code's and the Codex CLI's own logs, each message once. Native Rust (Rune), about 46 MB of memory, no account and no telemetry. Agents and the Mac's state live in one panel.

## Operating Context

- Opened from a menu bar item, read for a few seconds, closed by a click elsewhere or Esc.
- Providers: Claude, Codex, DeepSeek. Only providers the user has signed in to are shown; more can be added.
- Mac measures: CPU, Memory, Disk, Network, Battery, GPU; each can be a menu bar item and a tab.
- Every agent can be shown or hidden in the menu bar and in the panel, and has its own options; one that shows nowhere is not read.
- Sleep: one sentence for what the Mac does now, then keep the Mac awake for a while or until turned off (a cup in the menu bar while it runs), the idle timers, the lid, and what keeps it awake. Each control says what it does and whether it is temporary or permanent. Changes macOS reserves for an administrator ask with its own password dialog.
- Claude Code logs live under `<config>/projects/<project>/…jsonl`, so usage can be grouped by project.

## Capabilities and Constraints

- Light and dark appearance must both be fully supported.
- Must fit a 13" MacBook screen: panel height no more than about 640pt.
- Keyboard: ⌘1…⌘9 switch tabs, ⌘R refreshes, ⌘, opens Settings, ⌘Q quits, Esc closes.
- The panel keeps one fixed size across tabs; content never makes it grow or shrink.
- A translucent system material behind the panel is not required.

## Brand Commitments

- Name: Nerda Bar. App icon: `assets/AppIcon.svg`.
- Provider marks are the providers' own (`assets/providers/*.path`).
- Copy is plain, short, sentence case ("Resets in 2d 4h", "Lasts until reset").

## Evidence on Hand

- Current panel screenshots: `docs/images/*.png`.
- Sample data for renders: `src/demo.rs`. Per-project usage figures in the redesign are illustrative until computed from logs.

## Product Principles

1. One glance answers it: the number that matters is the largest thing in its region.
2. Exact or absent: show provider-reported or log-counted values, never estimates dressed as facts; label estimates.
3. Light by construction: nothing in the design may need polling or work the app does not already do.
4. Stable surface: fixed panel size, fixed slots, values change in place.
