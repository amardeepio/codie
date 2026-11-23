---
id: SPEC-TUI-010
title: Subagent runs and delegation controls
status: draft
version: 1.1.0
owners: ["@codie-maintainers"]
requirements: ["TUI-040", "TUI-041", "TUI-042", "TUI-043", "TUI-044", "TUI-045", "TUI-046", "TUI-047", "TUI-048", "TUI-049"]
---

# Context

Subagents run in the background of a parent turn, so the interface has two jobs: let the user control delegation
without editing configuration, and show that work is happening somewhere else without pretending it is parent
output. The transcript stays a faithful record of the parent run: child progress is rendered as activity on the
`task` row, and the full child transcript lives in its own session.

Delegation mechanics are canonical in `SPEC-AGENT-003`; this spec covers commands and rendering only.

# Requirements

**TUI-040 (MUST)** A running `task` call renders the agent type, description, elapsed time, and the latest activity
label (turn and current tool) as one row; the row updates as progress arrives and, on completion, shows the
status, turns used, cost, and child session id. A missing label, agent, or description falls back to a generic
label instead of an empty row.
Verification: `crates/codie-tui/src/render.rs::task_rows_show_live_activity_and_result_details`

**TUI-041 (SHOULD)** `/subagents open N` opens a read-only, scrollable transcript of the selected child run when
its session was persisted, and reports why it cannot open when the run is unknown or not persisted.
Verification: `crates/codie-tui/src/app.rs::open_child_run_reports_missing_transcripts`

**TUI-042 (MUST)** `/subagents` reports status and accepts `on`, `off`, `force`, `once`, `parallel N`, `types`,
`status`, `open N`, and `stop N`; unknown arguments produce an error message without changing the mode, and each
change is persisted to the session.
Verification: `crates/codie-tui/src/commands.rs::spec_tui_042_subagent_command`

**TUI-043 (MUST)** The footer shows the effective delegation mode, and a prompt that triggers or suppresses
delegation produces one informational line naming the detected phrase.
Verification: `crates/codie-tui/src/render.rs::footer_shows_the_delegation_mode`

**TUI-044 (MUST)** `/subagents stop N` cancels exactly one running child, leaving the parent and other children
running; the footer shows how many children are running while the badge is visible.
Verification: `crates/codie-tui/src/subagents.rs::status_lists_live_activity_and_open_hint`

**TUI-045 (SHOULD)** A `question` request from the parent or a child opens a one-line prompt naming the question;
Enter returns the text, Esc dismisses and the model continues without an answer. While a question or permission
prompt is open, further prompts queue instead of replacing it.
Verification: `crates/codie-tui/src/questions.rs::tool_reports_the_answer_and_dismissal`

**TUI-046 (MUST)** `/reload` re-reads agent definitions and applies them to the active session without a restart:
new personas become selectable, removed files stop being advertised, and live runs keep the configuration they
started with.
Verification: `crates/codie-tui/src/app.rs::reload_applies_project_agent_definitions`

**TUI-047 (SHOULD)** An open child transcript refreshes at most once per second while the child runs, preserving
the scroll position and the current view when the file has not changed. Permission and question overlays name the
asking run's label as well as its agent type and depth.
Verification: `crates/codie-tui/src/app.rs::open_child_overlay_refreshes_while_it_grows`

**TUI-048 (SHOULD)** `/subagents parallel N` applies to the live session when no child is queued or running, and
otherwise reports the current limit without changing it.
Verification: `crates/codie-subagents/tests/subagents.rs::parallelism_changes_only_while_idle`

**TUI-049 (MUST)** A completed `task` result adds its reported tokens and cost to the footer and appends a
`subagent` usage entry to the parent session, so `/usage` and session cost include delegated work.
Verification: `crates/codie-tui/src/app.rs::task_result_usage_reaches_the_footer_and_session`

# Command surface

| Input | Effect |
|---|---|
| `/subagents` | Show enabled, mode, parallelism, depth, budgets, types, and runs |
| `/subagents on` | Session mode `auto` |
| `/subagents off` | Session mode `off`; later calls are refused with an error result |
| `/subagents force` | Session mode `force` until changed |
| `/subagents once` | Force delegation for the next prompt only |
| `/subagents parallel N` | Session override of `max_parallel` (`1..=8`) |
| `/subagents types` | List every type with its description, model, tools, and per-type budgets |
| `/subagents status` | Status block with live and finished runs |
| `/subagents open N` | Read-only transcript of run `N` |
| `/subagents stop N` | Cancel run `N` |

The interactive app always declares `task`, so `/subagents on` and `/subagents off` change the runtime mode
without rewriting the system prompt or restarting. `[subagents] enabled = false` only makes the initial mode
`off`; it does not remove the declaration.

# Interaction

- The footer badge is `sub:off`, `sub:auto`, or `sub:force`, with `▶ N` while children run.
- When prompt detection fires, the transcript shows one line such as
  `Subagents requested for this run (detected "use subagents").`
- While children run, the `task` row shows `Task · general · Label  turn 3 · grep foo  1m 51s`.
- On completion, the row shows `completed · 3 turns · $0.0123 · ses_01...`.
- `/subagents status` numbers runs oldest first; `open` and `stop` use those numbers.
- `/subagents open` scrolls with arrows, `j`/`k`, PgUp/PgDn; Esc or `q` returns.
- `/stop` cancels the parent run; children receive the cancellation token and settle as aborted.

# Non-goals

- A separate dashboard for child runs. The session picker and `/tree` already expose child sessions.
- Streaming child assistant text into the parent transcript. Child text is summarized by the `task` result
  (`AGENT-024`).
- Editing delegation budgets from the TUI; `codie config set` owns configuration.

# Security and performance

Rendering is bounded: one row per child plus the existing tool block. Child progress is throttled by the
producer, not the renderer, so a chatty child cannot force a redraw storm. Child session identifiers are not
secrets, but they are displayed only after completion. The read-only transcript renders projected messages and
never re-enters the parent transcript.
