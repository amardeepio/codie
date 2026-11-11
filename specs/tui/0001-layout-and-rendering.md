---
id: SPEC-TUI-001
title: Interactive terminal interface
status: accepted
version: 1.0.0
owners: ["@codie-maintainers"]
requirements: ["TUI-001", "TUI-002", "TUI-003", "TUI-004", "TUI-005", "TUI-006", "TUI-007", "TUI-008", "TUI-009", "TUI-010"]
---

# Context

Phase 2 adds the primary human experience: an alternate-screen transcript with a fixed editor dock, streaming
assistant output, tool blocks, a footer, themes, slash commands, steering, keybindings, and permission prompts.

# Requirements

**TUI-001 (MUST)** `full` mode uses the alternate screen with a scrolling transcript, a fixed editor dock, and a one-line footer; `inline` mode renders the same layout without entering the alternate screen.
Verification: `crates/codie-tui/src/render.rs::spec_tui_001_fixed_editor_and_footer`

**TUI-002 (MUST)** Assistant streaming replaces the cumulative message text in place rather than appending duplicates, and rendering is coalesced by a tick so the agent task is never blocked by the UI.
Verification: `crates/codie-tui/src/app.rs::spec_tui_002_streaming_updates_replace_text`

**TUI-003 (MUST)** The editor supports multiline input, grapheme-correct movement, history, undo/redo, kill/yank, bracketed paste with large-paste markers, and slash-command completion.
Verification: `crates/codie-tui/src/editor.rs::movement_is_grapheme_correct`

**TUI-004 (MUST)** Markdown rendering supports headings, emphasis, lists, blockquotes, tables, fenced code blocks with keyword highlighting, links, and horizontal rules.
Verification: `crates/codie-tui/src/markdown.rs::headings_lists_and_code_render`

**TUI-005 (MUST)** Tool calls render as stateful blocks with running/success/error states, a compact summary, and expandable output (`Ctrl+O`); edit output colors added and removed lines.
Verification: `crates/codie-tui/src/transcript.rs::tool_blocks_transition_and_expand`

**TUI-006 (MUST)** The footer shows the working directory, git branch when present, model, thinking level, tokens, cost, context estimate, and session identifier.
Verification: `crates/codie-tui/src/render.rs::empty_state_and_footer_render`

**TUI-007 (MUST)** Keybindings are configurable in `keybindings.toml`; unbinding is supported with an empty key list, and unknown actions or keys are rejected with an error.
Verification: `crates/codie-tui/src/keybindings.rs::spec_tui_007_default_bindings_and_unbind`

**TUI-008 (MUST)** Themes are declarative TOML files with semantic tokens; `system`, `dark`, and `light` are built in, the body text must meet a 4.5:1 contrast ratio, and a theme file reloads when it changes.
Verification: `crates/codie-tui/src/theme.rs::spec_tui_008_theme_hot_reload`

**TUI-009 (MUST)** A panic, SIGINT, SIGTERM, or SIGHUP restores raw mode, leaves the alternate screen, and shows the cursor before the process exits.
Verification: `crates/codie-cli/tests/pty.rs::spec_tui_009_terminal_restored_on_signals`

**TUI-010 (MUST)** `--tui-mode plain` emits append-only text with no alternate screen and no cursor movement.
Verification: `crates/codie-cli/tests/pty.rs::spec_tui_010_plain_mode_is_append_only`

# Steering and permissions

- Enter while streaming enqueues a steering message; Alt+Enter queues a follow-up.
- Mutating tools prompt with allow-once, always, and deny; read-only tools never prompt.
- Permission prompts queue instead of overwriting each other when tools run in parallel.

# Failure modes

- Terminal initialization failure falls back to an error and restores the terminal.
- A closed permission channel denies the call.
- A model or run error is shown as an error entry; the app stays open.
- SIGTERM/SIGHUP restore the terminal and exit 143/129; Ctrl+C cancels a run and a second press quits.

# Security and performance

The TUI never prints an API key. Tool output is bounded per block. Rendering happens at most once per tick
(about 30 frames per second) and uses ratatui's cell diffing.

# Performance

`cargo bench -p codie-tui --bench streaming` simulates 5,000 tokens over 60 frames. The release build must stay
inside one second total and 16.7 ms per frame; the recorded baseline is 24–35 ms total (0.4–0.6 ms per frame,
over 140k tokens/s).

# Out of scope

- OSC 8 hyperlinks and full language grammars; code fences use lightweight keyword highlighting.
- A terminal-emulator matrix; restore is verified through PTY integration tests.
