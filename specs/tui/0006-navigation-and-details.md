---
id: SPEC-TUI-006
title: Context, permissions, tool navigation, turns, and session preview
status: implemented
version: 1.0.0
owners: ["@codie-maintainers"]
requirements: ["TUI-024", "TUI-025", "TUI-026", "TUI-027", "TUI-028"]
---

# Context

Long sessions need quick access to context usage, safe review of mutating tools, navigation across tool
blocks, folding of old turns, and enough session metadata to choose the right session.

# Requirements

**TUI-024 (MUST)** The footer shows a responsive context meter with a colour that warns as the compaction
threshold approaches. `/usage` and a click on the footer open a breakdown of context estimates, session token
components, cost, throughput, and session id.
Verification: `crates/codie-tui/src/render.rs::footer_shows_a_context_meter_and_usage_overlay`

**TUI-025 (MUST)** Permission prompts preview the pending operation: `edit` and overwriting `write` calls show a
diff, new files show numbered code, and `bash` shows the command. Decisions remain allow-once, always-allow,
deny, enter-to-allow, and full access.
Verification: `crates/codie-tui/src/render.rs::permission_overlay_renders_an_edit_diff`

**TUI-026 (MUST)** Clicking a tool block header toggles that block. The tool picker supports keyboard selection,
and a dedicated action expands or collapses every tool block.
Verification: `crates/codie-tui/src/render.rs::tool_headers_toggle_on_click_and_all_at_once`

**TUI-027 (MUST)** Starting a new user turn folds older turns to one header line with a summary of tools, edits,
errors, and hidden replies. Clicking a turn header or using the turn toggle expands or collapses turns.
Verification: `crates/codie-tui/src/render.rs::older_turns_collapse_and_expand_on_click`,
`crates/codie-tui/src/transcript.rs::turns_fold_and_unfold`

**TUI-028 (MUST)** The session picker shows a preview of the selected session with its name, model, message count,
tokens, cost, working directory, age, last prompt, and last reply.
Verification: `crates/codie-tui/src/render.rs::session_picker_shows_a_preview`

# Edge cases

- The context meter hides lower-priority footer parts before truncating the model and context information.
- Permission previews are capped and fall back to pretty JSON for unknown tools.
- Tool and turn toggles keep the transcript cache coherent by bumping its revision.
- A steering message does not start a new turn.

# Non-goals

No mouse text selection, no session preview thumbnails, and no cross-session search.
