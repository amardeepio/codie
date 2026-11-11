---
id: SPEC-AGENT-002
title: Transcript continuation without a new prompt
status: implemented
version: 1.0.0
owners: ["@codie-maintainers"]
requirements: ["AGENT-013"]
---

# Context

A failed request sometimes needs to be retried with the same transcript, most importantly after a context
overflow is compacted (`CTX-006`). Reusing `run` would append the user prompt a second time. This spec adds an
explicit continuation entry point so recovery cannot duplicate user input.

# Normative behavior

**AGENT-013 (MUST)** `Agent::resume` runs the supplied history without appending a prompt or emitting prompt
message events; the first provider request sees exactly the supplied transcript.
Verification: `crates/codie-core/tests/agent_loop.rs::spec_agent_013_resume_continues_without_adding_a_prompt`

# State machine / data shapes

`run(prompt, history, config)` is equivalent to `resume(history + [prompt], config)` plus the prompt
`MessageStart`/`MessageEnd` events. Both share the same loop, hooks, queues, turn budget, cancellation, and
terminal `AgentEnd` ordering.

# Edge cases

- An empty history with `resume` sends an empty context to the provider; callers must supply at least a system or
  user message.
- Steering and follow-up queues behave exactly as in `run`.
- `resume` never appends the prompt to the persisted session; persistence remains event-driven.

# Non-goals

`resume` does not implement retries, compaction, or provider fallback. Those live in the caller.

# Security and performance

No new I/O or trust boundary. The method reuses the existing stream and event channel, so cancellation and
backpressure behavior is unchanged.
