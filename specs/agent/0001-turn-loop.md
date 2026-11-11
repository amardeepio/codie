---
id: SPEC-AGENT-001
title: Agent turn loop
status: accepted
version: 1.0.0
owners: ["@codie-maintainers"]
requirements: ["AGENT-001", "AGENT-002", "AGENT-003", "AGENT-004", "AGENT-005", "AGENT-006", "AGENT-007", "AGENT-008", "AGENT-009", "AGENT-010", "AGENT-011", "AGENT-012"]
---

# Context

The agent loop is the only component that serializes state transitions between the model, tools, steering
queues, hooks, and persistence. If event ordering, result ordering, or failure classification is wrong, the
transcript diverges from what the model saw and every later turn is built on corrupt history. This spec fixes
the run lifecycle, the exact ordering guarantees, steering and follow-up semantics, abort and turn-budget
behavior, the tool batch contract, the hook contract, and failure synthesis.

The loop owns mechanics, not policy. Compaction, permissions, and rendering are host concerns; the loop exposes
hooks so the host can intervene without reaching into loop state.

# Normative behavior

**AGENT-001 (MUST)** `AgentEnd` is the last event of every run, including runs that end by error, abort, or turn limit. No agent event may follow it. The protocol layer's `agent_settled` is emitted later, after retries and compaction have settled, and is not an agent-loop event.
Verification: `crates/codie-core/tests/agent_loop.rs::spec_agent_001_agent_end_is_last`

**AGENT-002 (MUST)** The assistant message is finalized and its `MessageEnd` has been observed before any tool starts. A tool MUST NOT execute against a partial assistant message, and a tool result MUST NOT be appended before the assistant message it answers.
Verification: `crates/codie-core/tests/agent_loop.rs::spec_agent_002_assistant_finalized_before_tools`

**AGENT-003 (MUST)** Tool results are appended in assistant source order, exactly one per finalized call, with `tool_call_id` and `tool_name` copied verbatim. A missing result, an extra result, or a reordered result is a loop invariant violation and MUST fail the run with an internal error rather than persisting a malformed turn.
Verification: `crates/codie-core/tests/agent_loop.rs::spec_agent_003_results_in_source_order`

**AGENT-004 (MUST)** In parallel mode, `ToolExecutionEnd` is completion-ordered, while transcript order remains source order. In sequential mode, execution and transcript order both follow source order. The configured `tool_execution` mode MUST NOT change the persisted order.
Verification: `crates/codie-core/tests/agent_loop.rs::spec_agent_004_parallel_completion_order`

**AGENT-005 (MUST)** A tool failure (unknown tool, schema validation error, policy block, ordinary error, abort, or caught panic) becomes a `toolResult` with `is_error = true` and a model-readable message. It MUST NOT terminate the run. The one exception is cancellation of the whole run, which ends with an aborted assistant message per `AGENT-008`.
Verification: `crates/codie-core/tests/agent_loop.rs::spec_agent_005_tool_failures_become_results`

**AGENT-006 (MUST)** When `stop_reason == length` and the assistant message contains tool calls, the loop MUST NOT execute them. It synthesizes one error result per call telling the model that the response was truncated and that the call must be re-issued, appends those results in source order, and continues the run.
Verification: `crates/codie-core/tests/agent_loop.rs::spec_agent_006_length_never_executes_tools`

**AGENT-007 (MUST)** Steering is polled before the first request and after each completed turn. Follow-ups are polled only when the loop would otherwise exit. `one-at-a-time` and `all` modes are configurable for both queues. Delivered input is appended as user messages in queue order and is visible to the next request; steering MUST NOT cancel an in-flight stream.
Verification: `crates/codie-core/tests/agent_loop.rs::spec_agent_007_steering_and_follow_up_polling`

**AGENT-008 (MUST)** Abort is cooperative. The cancellation token propagates to the provider stream, hooks, and tools. If cancellation occurs while the assistant stream is open, the partial is discarded and the run ends with an assistant message whose stop reason is `aborted`. If it occurs after the assistant message is finalized, that message is retained and unfinished tool calls become aborted error results; no second assistant message is synthesized. Either way the run ends with `AgentEnd`, and abort MUST NOT be reported as a generic provider error.
Verification: `crates/codie-core/tests/agent_loop.rs::spec_agent_008_abort_is_cooperative`

**AGENT-009 (MUST)** `max_turns` (default 200) bounds the number of assistant requests in one run. Before starting a request that would exceed the budget, the loop ends the run with `status = turn_limit`, emits a diagnostic, and queues nothing. The limit applies to every run regardless of mode.
Verification: `crates/codie-core/tests/agent_loop.rs::spec_agent_009_max_turns_enforced`

**AGENT-010 (MUST)** Hooks (`prepare_request`, `prepare_next_turn`, `finish_turn`, `before_tool`, `after_tool`, `transform_context`, `convert_to_llm`) are fallible and may replace the context or model. A hook error fails the run with a synthesized assistant error message and `AgentEnd`; a hook MUST NOT panic the loop. `before_tool` may block a call, and a block becomes an error tool result rather than a run failure.
Verification: `crates/codie-core/tests/agent_loop.rs::spec_agent_010_hook_errors_fail_run`

**AGENT-011 (MUST)** The loop never mutates persisted state directly. Persistence is event-driven and occurs only after a finalized `MessageEnd`; partial messages, in-flight tool updates, and hook scratch state are never written to the session.
Verification: `crates/codie-core/tests/agent_loop.rs::spec_agent_011_persistence_is_event_driven`

**AGENT-012 (MUST)** A panic in a tool is caught with `catch_unwind` and converted to an error tool result. A panic in the loop itself is fatal: the process writes a crash log, restores the terminal if attached, and exits non-zero. The loop MUST NOT catch and continue after its own invariant failure.
Verification: `crates/codie-core/tests/agent_loop.rs::spec_agent_012_tool_panic_is_caught`

# Loop

```text
run(prompt):
  emit AgentStart
  transcript = project(session) + [prompt]
  turns = 0
  loop:
    emit TurnStart
    deliver prepared messages, then steering messages
    if turns >= max_turns:
      emit diagnostic("turn limit reached")
      status = TurnLimit
      break
    turns += 1
    assistant = stream_llm(transcript)
    append assistant
    if assistant.stop_reason in {error, aborted}:
      status = assistant.stop_reason
      break
    calls = tool_calls(assistant)
    if calls and assistant.stop_reason == length:
      results = synthetic_truncation_errors(calls)
    else if calls:
      results = execute_batch(calls)
    else:
      results = []
    append results in source order
    emit TurnEnd
    if finish_turn says end:
      status = Completed
      break
    if no calls and no steering and no follow-up:
      status = Completed
      break
    if nothing changed this iteration:
      emit diagnostic("no progress")
      status = Completed
      break
  emit AgentEnd(status)
```

`turns` counts assistant requests, not tool rounds. A turn with several tool calls still consumes one unit.
The no-progress guard exists to prevent a hook or empty stream from spinning without new messages.

# Event ordering

The agent layer emits these events around the provider deltas:

1. `AgentStart`.
2. `TurnStart`.
3. `MessageStart(user)` and `MessageEnd(user)` for each delivered steering or follow-up message.
4. `MessageStart(assistant)`, provider deltas, `MessageEnd(assistant)`.
5. `ToolExecutionStart` for every call, then zero or more `ToolExecutionUpdate`, then `ToolExecutionEnd` as
   each call finishes.
6. `MessageStart(toolResult)` and `MessageEnd(toolResult)` for every result, in source order.
7. `TurnEnd`.
8. Repeat from step 2 while the loop continues.
9. `AgentEnd`.

Provider deltas are nested inside the assistant message span. `ToolExecutionEnd` may be completion-ordered, but
steps 6 and 7 are not emitted until every call in the batch has settled.

# Steering and follow-ups

- Steering is checked at the top of each turn, before the request. Input received while a stream is running is
  therefore visible to the next request, not the current one.
- Follow-ups are checked only at the exit boundary. If a follow-up exists, the loop starts another turn instead
  of ending the run.
- `one-at-a-time` delivers the oldest queued item per boundary; `all` drains the queue in FIFO order.
- Delivered input is persisted as user messages through the normal `MessageEnd` path, so replay reconstructs
  exactly what the model saw.
- Undelivered steering at abort remains the host's responsibility; the loop reports the queue depth in the
  `AgentEnd` diagnostic but does not silently persist it.

# Tool batch contract

1. The batch is the ordered list of tool calls from one finalized assistant message.
2. Source order is the order of `ToolCall` blocks in that message.
3. Execution mode is `parallel` or `sequential`; parallel execution is bounded by `max_parallel_tools` (8).
4. Every call produces exactly one result, including unknown tools, schema errors, policy blocks, and panics.
5. Tool `updates` are advisory and may be dropped under backpressure; they never affect the final result.
6. A tool that returns `terminate = true` asks the host to stop after the batch; it does not skip results for
   the remaining calls.

# Hook contract

| Hook | When | May change | Failure behavior |
|---|---|---|---|
| `prepare_request` | Before every provider request | Context, model | Synthesized assistant error, run ends |
| `prepare_next_turn` | After results, before next turn | Context, model | Synthesized assistant error, run ends |
| `finish_turn` | After `TurnEnd`, before exit decision | Exit decision | Synthesized assistant error, run ends |
| `before_tool` | Before each tool starts | Block or rewrite call | Block becomes an error result |
| `after_tool` | After each tool settles | Result content | Synthesized assistant error, run ends |
| `transform_context` | During projection | Context | Synthesized assistant error, run ends |
| `convert_to_llm` | Before request serialization | Wire messages | Synthesized assistant error, run ends |

Hooks are ordered deterministically, run on the agent task, and receive a cancellation token. A hook error is
never converted into a tool result unless the hook is `before_tool`, where blocking is a normal outcome.

# Failure synthesis

- Provider setup or stream failure: assistant message with `stop_reason = error`, `error_message` set, run
  status `error`.
- Abort: assistant message with `stop_reason = aborted`, run status `aborted`.
- Turn limit: no synthetic assistant message; `AgentEnd(status = turn_limit)` plus diagnostic.
- Hook failure: assistant message with `stop_reason = error`, `error_message` naming the hook, run status
  `error`.
- Tool failure or panic: error tool result; run continues.
- Loop invariant failure: fatal internal error and crash log; no attempt to synthesize a recoverable turn.

# Edge cases

- A run whose first request fails before `start` still emits `AgentStart`, `TurnStart`, the synthesized error
  message, `TurnEnd`, and `AgentEnd`.
- An assistant message with no content and no tool calls ends the run as completed after `TurnEnd`.
- `stop_reason = length` with no tool calls ends the run as completed; no synthetic results are created.
- A `before_tool` block for one call does not cancel sibling calls in the same batch.
- Abort during tool execution cancels the remaining tools and produces aborted error results for calls that had
  already been announced, then ends the run.
- A follow-up delivered at the exit boundary starts a new turn and resets the no-progress guard, but not the
  turn budget.
- Zero `max_turns` is invalid configuration; the loop rejects it before `AgentStart`.

# Non-goals

- Compaction thresholds and summaries: `CTX-001`..`CTX-007`.
- Permission decisions: `SEC-001`..`SEC-003`; the loop consumes policy outcomes as tool results.
- Provider retry and SSE parsing: `PROV-003`, `PROV-012`.
- Protocol framing and `agent_settled`: `PROTO-002`, `PROTO-004`.
- Multi-agent orchestration and sub-agents: explicitly out of scope for v1.0.

# Reference implementation

pi: `packages/agent/src/agent-loop.ts`, `packages/agent/src/agent.ts`, and `packages/agent/src/types.ts`
(behavioral reference only; no code copied).
