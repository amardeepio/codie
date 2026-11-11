---
id: SPEC-TUI-002
title: Session controls and picker
status: implemented
version: 1.0.0
owners: ["@codie-maintainers"]
requirements: ["TUI-011", "TUI-012", "TUI-013"]
---

# Context

Phase 2 shipped the transcript, editor, and slash-command shell. Phase 3 makes sessions first-class in the
interactive app: inspect the tree, branch, clone, name, export, resume from a picker, and compact the context
without leaving the terminal. The file-format behavior lives in `SPEC-SESS-001`, `SPEC-SESS-002`, and
`SPEC-CTX-001`; this spec covers only the interaction.

# Normative behavior

**TUI-011 (MUST)** `/tree` shows every entry with a branch marker, depth, identifier, kind, and description;
`/fork <entry>` moves the leaf and refreshes history; `/clone` copies the session and switches to the copy;
`/name [text]` sets or clears the name; `/export [path]` writes JSONL. All commands report failure instead of
panicking.
Verification: `crates/codie-tui/src/app.rs::spec_tui_011_tree_fork_and_export_commands`

**TUI-012 (MUST)** `--resume` with no identifier opens a picker that lists sessions for the current directory,
filters by typed text, toggles recency/name sorting, renames with `r`, and deletes with `d` followed by `y`.
Enter opens the selected session and replaces the in-memory history with its projection.
Verification: `crates/codie-tui/src/app.rs::spec_tui_012_picker_filters_renames_and_deletes`

**TUI-013 (MUST)** `/compact [instructions]` records a compaction request with the optional instructions and
does not submit them as a prompt.
Verification: `crates/codie-tui/src/app.rs::spec_tui_013_compact_command_requests_a_summary`

# State machine / data shapes

Keys inside the picker:

| Key | Effect |
|---|---|
| `Up` / `k` | Move selection up |
| `Down` / `j` | Move selection down |
| printable | Append to the search query |
| `Backspace` | Remove the last search character |
| `s` | Toggle recency/name sort |
| `r` | Rename the selected session |
| `d`, then `y` | Delete the selected session |
| `Enter` | Resume the selected session |
| `Esc` / `q` | Close the picker |

The picker is an overlay; it never blocks the agent task, and it is only shown when the terminal is interactive.

# Edge cases

- An empty session directory shows "No sessions match."
- A rename that clears the text clears the session name.
- Deleting the last session leaves an empty picker rather than closing the app.
- `/fork` without an identifier shows the tree and a hint.
- `/clone`, `/name`, `/export`, and `/compact` report "No session is being persisted" for `--no-session`.
- A failed export or rename leaves the session unchanged and shows an error entry.

# Non-goals

RPC-driven session control is Phase 5. HTML export, transcript search, and the model picker are separate
features. This spec does not change keybindings outside the picker.

# Security and performance

The picker reads session headers and entries but never writes during listing. Delete removes only the selected
file and is confirmed. Rendering is line-based and bounded by the overlay height; loading a session is linear in
file size and happens on the UI thread only after the user selects it.
