---
id: SPEC-AGENT-003
title: Subagent delegation
status: draft
version: 1.0.0
owners: ["@codie-maintainers"]
requirements: ["AGENT-014", "AGENT-015", "AGENT-016", "AGENT-017", "AGENT-018", "AGENT-019", "AGENT-020", "AGENT-021", "AGENT-022", "AGENT-023", "AGENT-024", "AGENT-025", "AGENT-026", "AGENT-027", "AGENT-028", "AGENT-029", "AGENT-030", "AGENT-031", "AGENT-032", "AGENT-033", "AGENT-034", "AGENT-035", "AGENT-036", "AGENT-037", "AGENT-038", "AGENT-039", "AGENT-048"]
---

# Context

A single agent is good at holding one thread of work. Some tasks contain independent threads — search these
three modules, review these two packages, summarize these logs — and running them one after another wastes wall
clock time. Subagent delegation lets the model hand a self-contained task to a child run and receive one summary
back.

The child is a full agent run, not a prompt expansion: it has its own transcript, model, turn budget, tool
subset, cancellation token, and (optionally) session file. It never sees the parent conversation unless the
parent includes the needed context in the `prompt` argument, and its messages never enter the parent transcript.
The parent transcript changes by exactly one `task` tool result.

This spec defines the delegation mechanics, the isolation and policy guarantees, the budgets, and the two user
activation paths. It intentionally does not define a scheduler, durable background agents, or inter-agent
messaging; those are separate post-1.0 epics.

Related: `SPEC-AGENT-001` (turn loop), `SPEC-TOOL-001` (tool contract), `SPEC-SEC-001` (permissions),
`SPEC-PRMPT-001` (system prompt), `SPEC-SESS-001` (session format).

# Normative behavior

**AGENT-014 (MUST)** When subagents are enabled the host toolbox exposes a `task` tool whose object schema requires `description` and `prompt` and optionally accepts `agent` (`general` or `explore`) and `model`; invalid arguments or an unknown agent type produce an error result and start no run.
Verification: `crates/codie-subagents/tests/subagents.rs::spec_agent_014_task_tool_contract`

**AGENT-015 (MUST)** A child run starts from a fresh transcript containing only the child system prompt and the caller's `prompt`; parent messages are never copied into it, and child messages are never appended to the parent transcript, which changes only by the single `task` result.
Verification: `crates/codie-subagents/tests/subagents.rs::spec_agent_015_child_context_isolated`

**AGENT-016 (MUST)** Each child run uses its own `AgentConfig`, `max_turns` (default 50), cancellation token, and session id; child turns do not consume the parent's `max_turns` budget.
Verification: `crates/codie-subagents/tests/subagents.rs::spec_agent_016_child_turn_budget_independent`

**AGENT-017 (MUST)** At most `max_parallel` (default 4) child runs execute concurrently across the process; additional calls wait, and parent tool results remain in assistant source order per `AGENT-003`.
Verification: `crates/codie-subagents/tests/subagents.rs::spec_agent_017_parallel_children_bounded`

**AGENT-018 (MUST)** Cancelling the parent run cancels every running child through a child token; each child settles with an aborted result and the parent continues or ends per `AGENT-008`. A child exceeding `timeout_secs` is aborted the same way.
Verification: `crates/codie-subagents/tests/subagents.rs::spec_agent_018_cancellation_propagates`

**AGENT-019 (MUST)** Delegation depth is bounded by `max_depth` (default 0); a child at the limit does not receive `task` in its toolbox, and an attempted call resolves to an unknown-tool error result.
Verification: `crates/codie-subagents/tests/subagents.rs::spec_agent_019_depth_limit_blocks_recursion`

**AGENT-020 (MUST)** A `task` result contains the child's final assistant text, run status, child session id, turns used, and aggregated usage; text over `result_max_bytes` (default 20000) is elided in the middle on UTF-8 boundaries so both the beginning and the conclusion survive with an explicit notice; the summary is labelled as untrusted data for the parent model, and a non-completed child yields `is_error = true`.
Verification: `crates/codie-subagents/tests/subagents.rs::spec_agent_020_result_contract_and_truncation`

**AGENT-021 (MUST)** When `persist_sessions` is true a child session is created in the configured session directory with `parent_session` set to the parent id and a `custom` entry of type `subagent` recording call id, agent type, depth, model, and prompt; the parent session gains no other entries.
Verification: `crates/codie-subagents/tests/subagents.rs::spec_agent_021_child_session_linked`

**AGENT-022 (MUST)** A child's toolbox is the intersection of the parent's permitted tools, the agent type's `tools` list, and the built-in registry; a child can never execute a tool the parent policy denies, including shell and mutation tools.
Verification: `crates/codie-subagents/tests/subagents.rs::spec_agent_022_tool_subset_no_escalation`

**AGENT-023 (MUST)** Aggregate child usage is bounded by `max_total_tokens` and `max_total_cost_usd`; when a cap would be exceeded the remaining batch is aborted and every affected result is an error naming the exhausted budget.
Verification: `crates/codie-subagents/tests/subagents.rs::spec_agent_023_aggregate_budget_enforced`

**AGENT-024 (MUST)** Child progress reaches the parent only as `ToolExecutionUpdate` events on the `task` call, carrying the child session id, turn count, and a bounded activity label; an unchanged label is sent at most once per second, while a changed turn or tool is sent immediately. No child `MessageStart`, `MessageEnd`, `TurnStart`, or `TurnEnd` appears on the parent stream.
Verification: `crates/codie-subagents/tests/subagents.rs::spec_agent_024_progress_without_parent_pollution`

**AGENT-025 (MUST)** A `model` argument or per-type configured model is resolved against the active catalog before the child starts; an unknown provider/model pair produces an error result and starts no run.
Verification: `crates/codie-subagents/tests/subagents.rs::spec_agent_025_model_selection_validated`

**AGENT-026 (MUST)** Extension loop hooks do not run for child runs; extension tools remain available subject to `AGENT-022`, while built-in loop behavior, compaction, and persistence still apply to children.
Verification: `crates/codie-subagents/tests/subagents.rs::extension_hooks_do_not_run_inside_children`

**AGENT-027 (MUST)** A child run that ends in error, turn limit, panic, or join failure is converted to an error `task` result and never fails the parent run; only process-fatal invariant failures propagate.
Verification: `crates/codie-subagents/tests/subagents.rs::spec_agent_027_child_failure_is_parent_result`

**AGENT-028 (MUST)** Delegation mode is one of `off`, `auto`, or `force` per session with an optional one-shot override; when the `task` tool is declared the effective mode for a run is resolved as prompt suppression, then prompt request, then session off, then session force or one-shot, then auto, and a host that does not declare the tool reports `off`.
Verification: `crates/codie-subagents/tests/intent.rs::spec_agent_028_delegation_mode_precedence`

**AGENT-029 (MUST)** Prompt intent detection is deterministic and inspects only the first 500 characters after removing fenced code blocks; negation phrases suppress delegation, the documented affirmative phrases request it, and `make it faster` without an agent or delegation noun does not request it.
Verification: `crates/codie-subagents/tests/intent.rs::spec_agent_029_prompt_intent_detection`

**AGENT-030 (SHOULD)** When a run is forced and its first turn ends without any `task` call, the host queues exactly one follow-up asking the model to delegate or justify not delegating; a second turn is not forced.
Verification: `crates/codie-subagents/tests/subagents.rs::spec_agent_030_forced_nudge_once`

**AGENT-031 (MUST)** Agent definitions load from `<config_dir>/agents/<name>.md` and, for a trusted project, `<project>/.codie/agents/<name>.md`. Each file has optional frontmatter (`name`, `description`, `model`, `tools`, `max-turns`, `timeout-secs`) and a non-empty body used as the persona instruction. A definition extends the type registry, overrides a built-in or configured type by name, and keeps the existing field value when a field is omitted. The resulting names appear in the `task` schema enum and in the model-visible description with their descriptions. Unknown files, invalid frontmatter, empty bodies, and invalid numbers are skipped with a warning rather than failing startup.
Verification: `crates/codie-prompt/tests/agents.rs::discovers_user_and_project_agents_with_project_precedence`, `crates/codie-subagents/tests/subagents.rs::agent_definitions_extend_the_task_surface`

**AGENT-032 (MUST)** A type's `prompt` is appended to the child system prompt after the delegated-run preamble, and a type's `max_turns` or `timeout-secs` overrides the global value for that child only. Omitting them inherits the global configuration.
Verification: `crates/codie-subagents/tests/subagents.rs::persona_prompt_reaches_the_child_system_message`, `crates/codie-subagents/tests/subagents.rs::per_type_turn_budget_applies`

**AGENT-033 (MUST)** A tool's progress is forwarded to the host as `ToolExecutionUpdate` on the same call, in order before `ToolExecutionEnd`. A child's periodic update carries a bounded label naming the current tool and, when present, its path, command, pattern, query, or URL. Progress never enters the parent transcript as a message. Updates are best-effort: a tool that clones the sender into a detached task must not delay or hang the run.
Verification: `crates/codie-core/tests/agent_loop.rs::tool_progress_is_forwarded_before_the_end`

**AGENT-034 (MUST)** The agent type that owns a call is available as `AgentConfig.agent` and `ToolContext.agent`, and is `None` for the root run. An interactive permission request names the asking agent and its depth, and an "always" approval applies only to that agent type; it does not grant the same tool to another persona or to the root run.
Verification: `crates/codie-tui/src/permissions.rs::always_is_scoped_to_the_asking_agent`, `crates/codie-cli/src/permissions.rs::always_is_scoped_to_the_asking_agent`, `crates/codie-cli/tests/rpc.rs::spec_proto_013_child_permissions_are_attributed_and_scoped`

**AGENT-035 (SHOULD)** A host may expose a `question` tool to the parent and its children. The tool asks one question, returns the answer as text, and returns an explicit dismissal notice when the user declines. A child never blocks the whole batch: the tool is scheduled sequentially.
Verification: `crates/codie-tui/src/questions.rs::answers_round_trip_through_the_broker`, `crates/codie-tui/src/questions.rs::tool_reports_the_answer_and_dismissal`

**AGENT-036 (MUST)** The runtime's inherited model and known-model list can be replaced at runtime. A host calls the setter when the parent model or provider changes, and children started afterwards use the new model; children never keep routing to the startup model.
Verification: `crates/codie-subagents/tests/subagents.rs::changing_the_parent_model_routes_children_to_it`, `crates/codie-tui/src/app.rs::model_changes_reach_the_subagent_runtime`

**AGENT-037 (MUST)** A child waiting for a concurrency slot is registered in `queued` state, visible to the host with its agent, label, and depth, and cancellable before it starts. Cancelling a queued child returns an error result without consuming a permit or starting a run.
Verification: `crates/codie-subagents/tests/subagents.rs::queued_children_are_visible_and_cancellable`

**AGENT-038 (MUST)** When a host installs a guarded nested `task` tool, a child's `task` call resolves to that tool instead of constructing an unguarded one, so a grandchild passes the same permission gate as the root call.
Verification: `crates/codie-subagents/tests/subagents.rs::nested_task_uses_the_installed_gate`

**AGENT-039 (MUST)** A provider or terminal error reported by the child is preserved in the `task` result: the final text names the error when the child produced no summary, and the structured details carry an `error` field.
Verification: `crates/codie-subagents/tests/subagents.rs::child_provider_errors_are_reported`

**AGENT-048 (MUST)** A child that exceeds its wall-clock limit returns a timed-out result promptly, carrying the partial summary and usage collected so far and an `error` naming the timeout, instead of waiting for the provider or discarding the transcript.
Verification: `crates/codie-subagents/tests/subagents.rs::timeout_reports_a_timed_out_result_without_waiting`

# Delegation model

```text
parent run (depth 0)
  └─ task(description, prompt, agent, model?)
       ├─ resolve effective mode: off | auto | force
       ├─ validate args, agent type, model, depth, budget
       ├─ acquire one of max_parallel permits
       ├─ build child toolbox  = parent tools ∩ type tools; task hidden at depth limit
       ├─ build child prompt   = child system prompt + caller prompt
       ├─ create child session = parent_session link + custom "subagent" entry
       ├─ Agent::run(child_prompt, [], AgentConfig { delegation_depth: depth + 1 })
       │    └─ drain events → throttled ToolProgress on the parent call
       └─ summarize → one ToolResult in the parent transcript
```

Depth is stored on `AgentConfig` and copied to `ToolContext` by the loop. The root run has depth 0. `task` is
advertised while `depth <= max_depth`; a child run has `depth + 1`. `max_depth = 0` therefore allows the root to
delegate but forbids grandchildren.

Delegation mode resolution for one run:

```text
if task is not declared                -> off   (headless configuration disabled)
else if prompt intent == suppressed    -> off
else if prompt intent == requested     -> force
else if session mode == off            -> off
else if session mode == force or once  -> force
else                                   -> auto
```

The interactive host always declares `task` and uses the runtime mode as the switch; `[subagents] enabled = false`
only makes the initial mode `off`. A headless host does not declare `task` unless delegation is explicitly
enabled, so an unconfigured CI run cannot delegate by accident.

A `force` run is advisory: the host does not fabricate tool calls. It adds the persisted mode directive, and
`AGENT-030` permits one follow-up when the first turn ignores the request.

# Data shapes

```rust
pub struct TaskArgs {
    pub description: String,
    pub prompt: String,
    pub agent: String,           // configured type name; general and explore are built in
    pub model: Option<String>,
}

pub struct TaskOutcome {
    pub status: RunStatus,
    pub summary: String,
    pub session_id: Option<String>,
    pub agent: String,
    pub depth: usize,
    pub turns: u32,
    pub usage: Usage,
    pub persisted: bool,
}

pub struct SubagentConfig {
    pub enabled: bool,
    pub max_parallel: usize,     // default 4
    pub max_depth: usize,        // default 0
    pub max_turns: usize,        // default 50
    pub timeout_secs: u64,       // default 600
    pub persist_sessions: bool,  // default true
    pub result_max_bytes: usize, // default 20_000
    pub max_total_tokens: Option<u64>,
    pub max_total_cost_usd: Option<f64>,
    pub types: BTreeMap<String, AgentTypeConfig>,
}

pub struct AgentTypeConfig {
    pub model: Option<String>,
    pub tools: ToolSelection,        // Inherit | Only(Vec<String>)
    pub description: Option<String>, // shown in the task description
    pub prompt: Option<String>,      // appended to the child system prompt
    pub max_turns: Option<usize>,    // overrides the global budget
    pub timeout: Option<Duration>,   // overrides the global limit
}
```

A definition file uses the same field names:

```markdown
---
description: Reviews rendered interfaces and accessibility
model: provider/model        # optional
tools: read, grep, bash      # omit or "inherit" to keep the type's selection
max-turns: 12                # optional
timeout-secs: 120            # optional
---

You review interfaces for contrast, focus order, empty states, and narrow viewports.
Report file:line evidence; do not edit files.
```

Tool arguments (`task`) on the wire:

```json
{
  "description": "Audit provider retries",
  "prompt": "Read crates/codie-ai/src/openai.rs and list every retry path with file:line. Do not edit files.",
  "agent": "explore"
}
```

Model-visible result:

```text
<subagent agent="explore" status="completed" session="ses_01..." turns="3" cost_usd="0.0040">
Found three retry paths: ...
</subagent>
```

Host-only `details`:

```json
{
  "session_id": "ses_01...",
  "agent": "explore",
  "depth": 1,
  "status": "completed",
  "turns": 3,
  "usage": { "total_tokens": 1540, "cost_usd": 0.004 },
  "persisted": true
}
```

The child session stores the generic `custom` entry:

```json
{
  "type": "custom",
  "custom_type": "subagent",
  "data": {
    "call_id": "call_...",
    "parent_session": "ses_parent",
    "agent": "explore",
    "depth": 1,
    "model": "deepseek-chat",
    "prompt_sha256": "…"
  }
}
```

The prompt digest keeps the entry small and avoids duplicating potentially large prompts. The full task prompt is
the first user message of the child session.

# Activation

- `/subagents` reports enabled, effective mode, parallelism, depth, budgets, and agent types.
- `/subagents on|off|force|once` changes the session mode; `parallel N` and `types` are also accepted.
- `--subagents off|auto|force` and `--subagents-parallel N` provide the headless equivalent.
- Prompt intent detection applies to the user's text in every frontend. Negation wins over any affirmative phrase.
- Mode changes are persisted as system messages so replay sees the same instructions the model saw.

# Edge cases

- Empty or whitespace-only `prompt`: schema or validation error; no child starts.
- Unknown `agent` or `model`: error result; no child starts.
- `max_parallel` reached while sibling `task` calls wait: the loop's source-order result contract still holds.
- Parent abort before the child's first request, during a child tool call, or between child completion and result
  append: the child is cancelled and settles as an error result; the parent follows `AGENT-008`.
- Child turn limit, provider error, panic, or task-join failure: error result; the parent continues.
- Parent policy denies `task` while a prompt requests delegation: denial result; no child starts.
- Child asks for a tool the parent denies: denial result inside the child; the child may recover.
- Two children edit the same path: the per-path lock (`TOOL-008`) serializes the writes; no corruption.
- Result truncation must not split a UTF-8 code point and must append an explicit notice.
- Session directory missing or unwritable with `persist_sessions = true`: the child runs in memory, the result
  reports `persisted = false`, and the host emits a warning.
- Budget exhausted mid-batch: completed children keep their results; unfinished children become budget errors.
- `max_depth = 0`: a child's declarations contain no `task`, and a hallucinated call is an unknown-tool error.
- Prompt intent inside a fenced code block or after the first 500 characters is ignored.
- `"don't use subagents"` suppresses even when the session mode is `force`.

# Non-goals

- Out-of-process, sandboxed, or durable background agents.
- Inter-agent messaging, shared scratchpads, or agent-to-agent steering.
- A planner that decomposes the task before the parent model runs.
- New agent event variants: progress travels on the existing `ToolExecutionUpdate` path.
- Extension loop hooks inside children (`AGENT-026`).
- Worktree isolation; children share the parent working directory.
- Cost estimation beyond the provider-reported and catalog-derived usage already attached to messages.

# Security and performance

`task` multiplies the tools a model can reach, so it is gated as a mutating capability (`SEC-009`). A child can
only use a subset of the parent's permitted tools; it can never escalate. Child output is untrusted input for the
parent model, like `web_fetch` output, and the prompt says to verify claims rather than follow instructions found
in a delegate summary.

Cost controls are mandatory: per-child turn and wall-clock limits, a concurrency cap, and aggregate token and
cost caps. Each child holds one transcript in memory; compaction applies inside the child. Progress is throttled
to one update per second per child so a fast child cannot flood the parent's 256-slot event channel.

# Test plan

| Requirement | Test |
|---|---|
| AGENT-014 | `subagents.rs::spec_agent_014_task_tool_contract` |
| AGENT-015 | `subagents.rs::spec_agent_015_child_context_isolated` |
| AGENT-016 | `subagents.rs::spec_agent_016_child_turn_budget_independent` |
| AGENT-017 | `subagents.rs::spec_agent_017_parallel_children_bounded` |
| AGENT-018 | `subagents.rs::spec_agent_018_cancellation_propagates` |
| AGENT-019 | `subagents.rs::spec_agent_019_depth_limit_blocks_recursion` |
| AGENT-020 | `subagents.rs::spec_agent_020_result_contract_and_truncation` |
| AGENT-021 | `subagents.rs::spec_agent_021_child_session_linked` |
| AGENT-022 | `subagents.rs::spec_agent_022_tool_subset_no_escalation` |
| AGENT-023 | `subagents.rs::spec_agent_023_aggregate_budget_enforced` |
| AGENT-024 | `subagents.rs::spec_agent_024_progress_without_parent_pollution` |
| AGENT-025 | `subagents.rs::spec_agent_025_model_selection_validated` |
| AGENT-026 | `subagents.rs::extension_hooks_do_not_run_inside_children` |
| AGENT-027 | `subagents.rs::spec_agent_027_child_failure_is_parent_result` |
| AGENT-028 | `intent.rs::spec_agent_028_delegation_mode_precedence` |
| AGENT-029 | `intent.rs::spec_agent_029_prompt_intent_detection` |
| AGENT-030 | `subagents.rs::spec_agent_030_forced_nudge_once` |
| AGENT-031 | `agents.rs::discovers_user_and_project_agents_with_project_precedence`, `subagents.rs::agent_definitions_extend_the_task_surface` |
| AGENT-032 | `subagents.rs::persona_prompt_reaches_the_child_system_message`, `subagents.rs::per_type_turn_budget_applies` |
| AGENT-033 | `agent_loop.rs::tool_progress_is_forwarded_before_the_end` |
| AGENT-034 | `permissions.rs::always_is_scoped_to_the_asking_agent` |
| AGENT-035 | `questions.rs::answers_round_trip_through_the_broker`, `questions.rs::tool_reports_the_answer_and_dismissal` |
| AGENT-036 | `subagents.rs::changing_the_parent_model_routes_children_to_it`, `app.rs::model_changes_reach_the_subagent_runtime` |
| AGENT-037 | `subagents.rs::queued_children_are_visible_and_cancellable` |
| AGENT-038 | `subagents.rs::nested_task_uses_the_installed_gate` |
| AGENT-039 | `subagents.rs::child_provider_errors_are_reported` |
| AGENT-048 | `subagents.rs::timeout_reports_a_timed_out_result_without_waiting` |

Tests use a scripted `ChatClient`; no live provider is contacted.

# Reference implementation

The design borrows the "one tool call, one child context, one summary" delegation shape used by OpenCode and
Claude Code, adapted to codie's event-driven loop and append-only session model. No code is copied.
