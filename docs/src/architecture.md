# Architecture

codie separates the wire, the conversation, the loop, and the interfaces. The separation is enforced by crate
boundaries, not convention: the loop has no HTTP client, the provider adapter knows nothing about tool
implementations, and only the CLI reads environment variables or decides process exit codes.

## Crate graph

```text
                    ┌──────────────┐
                    │  codie-cli   │  binary, arg parsing, mode dispatch
                    └──────┬───────┘
       ┌───────────────────┼───────────────────┐
       ▼                   ▼                   ▼
┌────────────┐      ┌─────────────┐     ┌──────────────┐
│ codie-tui  │      │codie-proto  │     │ codie-prompt │
└─────┬──────┘      └──────┬──────┘     └──────┬───────┘
      │                    │                   │
      └────────────────────┼───────────────────┘
                           ▼
                    ┌─────────────┐
                    │ codie-core  │  loop, messages, events
                    └──┬───────┬──┘
                       │       │
              ┌────────▼──┐  ┌─▼────────────┐
              │ codie-ai  │  │ codie-tools  │
              └────────┬──┘  └──────┬───────┘
                       │            │
              ┌────────▼────────────▼──────┐
              │      codie-context         │  compaction, tokens
              └────────────────────────────┘

codie-session: append-only JSONL store and projection
codie-testkit (dev only): fake provider, fixtures, #[spec] macro
```

The target graph is deliberately wider than the Phase 0 workspace. codie starts with `core`, `ai`, `tools`,
`session`, and `cli`; `context`, `prompt`, `tui`, and `protocol` are split out when a second consumer appears.
Premature crates are as harmful as monolithic modules.

Current repository crates are `codie-core`, `codie-ai`, `codie-tools`, `codie-session`, `codie-cli`,
`codie-testkit`, and the `codie-spec-macro` helper used by the test attribute. `xtask` is repository automation,
not a shipped library.

## Boundaries

- `codie-core` depends only on `codie-ai` types and `codie-context` traits. It performs no I/O, terminal work,
  or HTTP.
- `codie-ai` owns HTTP, SSE, provider quirks, retries, and auth resolution. It never knows how a tool is
  implemented.
- `codie-tools` depends on `codie-core` for the tool trait and on `codie-context` for truncation, never the
  reverse.
- `codie-session` owns the append-only store and projection. It does not call providers.
- `codie-cli` is the only crate that reads environment variables or maps outcomes to process exit codes.
- Public APIs are `pub(crate)` by default; crates expose narrow, documented surfaces.

## Runtime model

- One `tokio` multi-thread runtime with `worker_threads = min(4, cores)`.
- One agent task owns the loop and serializes state transitions.
- Events flow over a bounded `mpsc` channel (capacity 256) to the UI or protocol writer, so backpressure is
  real and memory is bounded.
- Tool execution uses a `JoinSet` plus a `Semaphore` with `max_parallel_tools = 8`.
- Cancellation uses `tokio_util::sync::CancellationToken` at every I/O boundary.
- Blocking filesystem work uses `spawn_blocking`; the reactor is never blocked.
- Child processes run in their own process group and are killed as a group on abort or timeout.

## State layering

```text
SessionStore (append-only JSONL, source of truth)
        │  projection (branch + compaction + context edits)
        ▼
Transcript (Vec<Message>)  ──►  ProviderContext (normalized for wire)
        │
        ▼
AgentState (derived: messages, streaming partial, pending tool calls)
```

The loop works on a working copy. `AgentState` is reduced from events. Persistence happens only on a finalized
`MessageEnd`, never on partials. This is what makes replay, fork, and compaction safe.

## Event flow

A provider stream is `start -> text/thinking/toolcall deltas -> done|error`, carrying a live partial message.
The agent loop wraps it in run and turn events, then executes the tool batch after the assistant message is
final. Tool results are appended in source order even when execution completes out of order. The full ordering
is normative in [`specs/agent/0001-turn-loop.md`](../../specs/agent/0001-turn-loop.md).

## Where behavior is specified

- Message model and assistant events: [`specs/message/0001-message-model.md`](../../specs/message/0001-message-model.md).
- Turn loop, steering, abort, hooks: [`specs/agent/0001-turn-loop.md`](../../specs/agent/0001-turn-loop.md).
- Provider wire behavior: [`specs/provider/0001-openai-compatible.md`](../../specs/provider/0001-openai-compatible.md).
- Scope, phases, and crate policy: [`PLAN.MD`](../../PLAN.MD) §5.
