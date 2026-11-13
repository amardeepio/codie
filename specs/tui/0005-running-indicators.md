---
id: SPEC-TUI-005
title: Running indicators and transcript scrolling
status: implemented
version: 1.0.0
owners: ["@codie-maintainers"]
requirements: ["TUI-020", "TUI-021", "TUI-022", "TUI-023"]
---

# Context

A long tool call and a stuck model stream look identical when the only status text is `streaming`. A command that
holds a pipe open, a slow build, or a timed-out server can leave the user unsure whether anything is happening.
This spec makes tool execution visibly distinct from model streaming.

# Normative behavior

**TUI-020 (MUST)** While a tool call is executing, its transcript block shows an animated marker and the elapsed
time, and the editor border replaces the generic `streaming` title with the running tool's name and elapsed time.
The indicator stops when the tool finishes or fails.
Verification: `crates/codie-tui/src/render.rs::running_tool_replaces_the_streaming_title`

**TUI-021 (MUST)** The transcript scrolls with PgUp/PgDn, Shift+Up/Down, Alt+Up/Down, and the mouse wheel.
Scrolling up leaves follow mode and the editor border shows how to return to the newest output; when no run is
active, Esc or scrolling back to the bottom resumes following.
Verification: `crates/codie-tui/src/app.rs::scroll_keys_leave_follow_mode_and_esc_returns_to_bottom`

**TUI-022 (MUST)** Reasoning is collapsed to an animated one-line indicator by default. Clicking the indicator
expands the live reasoning stream, and Esc collapses it again. While reasoning is the only live output, the
editor border title reads `thinking`.
Verification: `crates/codie-tui/src/render.rs::thinking_starts_collapsed_and_toggles_with_a_click`

**TUI-023 (MUST)** Esc or `/stop` stops an active model request or tool call. Esc closes open reasoning before
stopping the run and, when idle, returns the transcript to the bottom. Cancelling a tool kills the bash process
group, drops queued steering and follow-ups, and reports when nothing is running. An aborted run returns the
editor to idle and reports `Stopped.`
Verification: `crates/codie-tui/src/app.rs::escape_stops_a_running_turn`,
`crates/codie-tui/src/app.rs::slash_stop_cancels_the_active_run`,
`crates/codie-tui/src/app.rs::aborted_runs_return_to_idle`

# Edge cases

- Several tools run in parallel: the most recent running block titles the editor, and every running block
  animates.
- The elapsed label switches from seconds to minutes and hours without overflowing the border.
- A tool that finishes while the assistant continues streaming hands the title back to `streaming`.
- Plain mode remains append-only and does not animate.

# Non-goals

No progress bars, percentage estimates, or per-tool ETA. Tool output stays in the collapsible block.
