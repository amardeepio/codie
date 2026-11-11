---
id: SPEC-TUI-003
title: Command palette and live pickers
status: implemented
version: 1.0.0
owners: ["@codie-maintainers"]
requirements: ["TUI-014", "TUI-015", "TUI-016", "TUI-017"]
---

# Context

Phase 2 required slash commands to be typed and completed with Tab. Phase 3 makes discovery and switching
faster: pressing `/` opens a filterable command palette, and model, theme, and reasoning choices are lists that
apply immediately. The footer reports throughput so users can see streaming speed. This spec covers the
interaction; model/provider behavior remains in `SPEC-PROV-001` and themes in `SPEC-TUI-001`.

# Normative behavior

**TUI-014 (MUST)** Pressing `/` in an empty editor opens a filterable command list. Typing filters by name or
description, `Up`/`Down` and `PageUp`/`PageDown` move the selection with scrolling, `Enter` selects, and `Esc`
closes without running anything. Selecting a command with optional text inserts `/name ` into the editor.
Verification: `crates/codie-tui/src/app.rs::spec_tui_014_slash_opens_a_filterable_command_palette`

**TUI-015 (MUST)** The model picker lists models for the active provider, marks the current model, applies the
selection to the next request, updates the context window, and persists a model-change entry. The theme picker
applies `system`, `dark`, or `light` immediately.
Verification: `crates/codie-tui/src/app.rs::spec_tui_015_model_and_theme_pickers_apply_selection`

**TUI-016 (MUST)** The reasoning picker lists `off`, `minimal`, `low`, `medium`, `high`, `xhigh`, and `max`,
marks the current level, applies the selection, and persists a thinking-level entry.
Verification: `crates/codie-tui/src/app.rs::spec_tui_016_reasoning_picker_sets_and_persists_the_level`

**TUI-017 (MUST)** The footer shows tokens per second using provider usage and duration for completed turns, and
a live estimate while text streams. A missing duration keeps the previous rate instead of showing zero.
Verification: `crates/codie-tui/src/app.rs::spec_tui_017_footer_reports_tokens_per_second`

# State machine / data shapes

| Key | Palette effect |
|---|---|
| printable | Append to the filter |
| `Backspace` | Remove the last filter character |
| `Up` / `Down` | Move one row |
| `PageUp` / `PageDown` | Move five rows |
| `Enter` | Apply the selected row |
| `Esc` | Close the overlay |

The list keeps the selected row visible by computing a scroll offset from the overlay height. The palette is
only opened from an empty editor, so a leading `/` inside a multi-line prompt is still literal text.

# Edge cases

- An empty result set shows "No matches." and `Enter` does nothing.
- `/model <id>` and `/theme <name>` typed manually still work and persist the same state.
- Selecting the current model or theme is allowed and only refreshes metadata.
- The model list falls back to the current model when the catalog is unavailable.
- Tokens/sec is an estimate during streaming and an exact usage/duration ratio after completion.

# Non-goals

Provider switching, live OpenRouter catalog refresh inside the picker, custom theme discovery, and a global
keybinding for the palette are deferred. Token rate is not used for billing.

# Security and performance

The palette is local state and generates no I/O. Selecting a model or theme performs one small session append.
Rendering is bounded by the overlay height and never blocks the agent task.
