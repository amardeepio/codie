# ADR 0004: In-process subagents with host-owned orchestration

- Status: Proposed
- Date: 2026-11-22
- Deciders: codie maintainers
- Related: `PLAN.MD` §1.2, §12.7 (proposed), `specs/agent/0003-subagent-delegation.md`

## Context

Subagents were an explicit v1.0 non-goal. The loop is now stable: it is generic over `ChatClient` and `Toolbox`,
it owns event ordering, and it already isolates one run's transcript from persisted history. That makes a child
run cheap to define: another `Agent::run` with its own context, token, and toolbox.

The open question is where orchestration lives. The loop has no I/O and must not learn about sessions, model
catalogs, or prompts. Extensions are out-of-process by ADR 0002 and cannot spawn a child with the parent's client
and policy without a new protocol surface.

## Decision

Subagents run in-process, owned by the host (CLI or TUI), behind a narrow port.

- A new crate, `codie-subagents`, implements the `task` tool, the child toolbox intersection, the concurrency
  semaphore, and the budget meter. It depends on `codie-core`, `codie-session`, and `codie-prompt`, never on
  `codie-ai` or `codie-tools`.
- The host injects the chat client and the already-wrapped parent toolbox (policy or permission). The child
  therefore inherits the parent's permission decisions and cannot escalate.
- `task` is a host-registered tool, not a tenth built-in. It is added by a `SubagentToolbox` wrapper so
  `TOOL-001` keeps its exact registry order.
- A child is a fresh transcript: child system prompt plus the caller's prompt. Parent history is not copied.
- Child progress is forwarded through the existing `ToolExecutionUpdate` channel. No new `AgentEvent` variant and
  no protocol break.
- Depth is carried on `AgentConfig` and `ToolContext`; `max_depth = 0` allows the root to delegate but forbids
  grandchildren.
- Child sessions are new session files with `parent_session` set and a `custom` `subagent` entry, so the frozen
  session format is unchanged.
- Extension loop hooks do not run inside children in v1; extension tools remain available through the parent
  toolbox.

## Consequences

Positive:

- No change to loop semantics, event ordering, or protocol schemas.
- Child isolation is the same mechanism as any run, so replay and compaction already work.
- Permission inheritance is structural: the child toolbox wraps the parent toolbox.
- Tests use the existing scripted-client harness; no live provider is needed.

Negative:

- A child shares the process, so a core panic still takes the process down. The delegation boundary catches child
  panics, but there is no memory or CPU isolation.
- Children share the working directory. Concurrent edits rely on the per-path lock (`TOOL-008`).
- Extension hooks are absent in children until a serialization model exists for concurrent hook calls.
- The host must construct a child system prompt, which duplicates prompt inputs across CLI and TUI wiring.

## Alternatives considered

- **Out-of-process child (`codie` subprocess).** Rejected for v1: requires credential handoff, a new IPC surface,
  and a second policy negotiation; sandboxing is a separate post-1.0 epic.
- **Extension-provided subagents.** Rejected: ADR 0002 keeps foreign code out of the core, and an extension cannot
  call back into the parent's `ChatClient` or toolbox without a large protocol addition.
- **A planner call before the parent model runs.** Rejected: doubles latency and cost and changes the meaning of
  the user's prompt. `/subagents force` plus one follow-up nudge is enough.
- **Adding `task` to `Registry::builtin()`.** Rejected: `TOOL-001` freezes the registry order and count.

## References

- `specs/agent/0003-subagent-delegation.md`
- `specs/security/0001-permissions.md` (`SEC-009`)
- `specs/prompt/0001-system-prompt.md` (`PRMPT-013`, `PRMPT-014`)
