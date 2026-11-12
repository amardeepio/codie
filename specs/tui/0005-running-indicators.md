---
id: SPEC-TUI-005
title: Running indicators and transcript scrolling
status: implemented
version: 1.0.0
owners: ["@codie-maintainers"]
requirements: ["TUI-020", "TUI-021"]
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
Scrolling up leaves follow mode and the editor border shows how to return to the newest output; Esc, or scrolling
back to the bottom, resumes following.
Verification: `crates/codie-tui/src/app.rs::scroll_keys_leave_follow_mode_and_esc_returns_to_bottom`

# Edge cases

- Several tools run in parallel: the most recent running block titles the editor, and every running block
  animates.
- The elapsed label switches from seconds to minutes and hours without overflowing the border.
- A tool that finishes while the assistant continues streaming hands the title back to `streaming`.
- Plain mode remains append-only and does not animate.

# Non-goals

No progress bars, percentage estimates, or per-tool ETA. Tool output stays in the collapsible block.
