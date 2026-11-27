---
id: SPEC-TUI-007
title: Mouse text selection and clipboard copy
status: implemented
version: 1.0.0
owners: ["@codie-maintainers"]
requirements: ["TUI-029"]
---

# Context

The interactive app captures the mouse for scrolling and click-to-toggle, so the terminal's own selection does
not work with a plain drag. Without an in-app selection users cannot copy a command they are reviewing, a diff
fragment, or part of an answer. This spec covers selecting rendered text with the mouse and copying it to the
system clipboard; `/copy` for the last assistant message is unchanged.

# Normative behavior

**TUI-029 (MUST)** Dragging with the left mouse button over the rendered screen highlights the covered cells in
reverse video and copies their text to the system clipboard when the button is released. Copied rows are trimmed
of trailing padding, empty leading and trailing rows are dropped, and the rows are joined with newlines. The
editor title reports the copy (`copied N line(s)`) or the clipboard failure without appending a transcript entry
or changing the scroll position. A press and release on the same cell is treated as a click, not a selection,
and any key press, wheel scroll, or new press clears the highlight and the notice.
Verification: `crates/codie-tui/src/render.rs::selected_cells_copy_as_trimmed_text`,
`crates/codie-tui/src/render.rs::selection_highlights_the_rendered_screen`,
`crates/codie-tui/src/app.rs::spec_tui_029_drag_selects_text_and_click_still_toggles`

# State machine / data shapes

`Selection { anchor: (row, column), cursor: (row, column) }` stores screen coordinates, which match the rendered
frame buffer and therefore work for the transcript, tool bodies, and overlays with no per-widget bookkeeping.

| Input | Effect |
|---|---|
| `Down(Left)` | `begin_selection`, highlight starts on one cell |
| `Drag(Left)` | `drag_selection`, extends the highlight |
| `Up(Left)` without movement | Treated as a click; the hit-test target toggles |
| `Up(Left)` after movement | `copy_pending`, the next frame copies |
| Key, wheel, or a new press | Clears `selection` and `notice` |

Copying happens in the renderer because it owns the frame buffer. It reads cell symbols rather than the widget
tree, so wide characters and emoji copy as displayed.

# Edge cases

- A selection that covers only padding or empty rows copies nothing and reports nothing.
- Selections are clamped to the frame; a terminal resize during a drag cannot read outside the buffer.
- The clipboard is written through `pbcopy`, `wl-copy`/`xclip`, or `clip`; when every command fails the notice
  names the error and the highlight remains so the selection can be retried.
- Streaming content that moves under a finished selection may shift what the highlight covers until the next
  input clears it.
- Plain mode is unaffected; it has no frame buffer and keeps its line-oriented output.

# Non-goals

Keyboard-driven selection, block/rectangular selection, and OSC 52 clipboard writes for remote sessions are out
of scope. `/copy` continues to target the last assistant message only.

# Security and performance

Selection reads only cells already on screen and never re-reads the session file. `render::draw` performs no I/O:
it captures the selected text into `App::pending_copy`, and the event loop copies it on a blocking task, so a
slow, missing, or hung clipboard command never blocks rendering or input. Clipboard writes run once per release,
not per frame, and every helper is killed after a two-second timeout.
