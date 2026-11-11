# Glossary

Terms are defined here once and used with these meanings throughout `specs/`, `docs/`, and `PLAN.MD`. Where a
term has a common meaning that is close but wrong, the "Not" line records the distinction.

## message

The atomic unit of conversation state. After lowering, a message is exactly one of `system`, `user`,
`assistant`, or `toolResult`, and contains an ordered list of content blocks. Messages are what the provider
adapter receives and what the session persists.

Not: a UI row. One message may render as many rows, and host-only entries (branch summaries, custom messages)
are lowered before they become messages.

See `MSG-001`, `MSG-002`, and `specs/message/0001-message-model.md`.

## turn

One assistant request and the work that follows it: the assistant stream, at most one tool batch, and the
resulting tool-result messages. A turn begins with `TurnStart` and ends with `TurnEnd`. `max_turns` bounds the
number of turns in a run.

Not: a user message. A user message may be delivered at the start of a turn, and one turn may produce no user
message at all.

See `AGENT-009` and `specs/agent/0001-turn-loop.md`.

## run

One invocation of the agent loop, from `AgentStart` to `AgentEnd`, initiated by a prompt. A run contains one or
more turns and ends for exactly one reason: completion, error, abort, or turn limit. The protocol layer's
`agent_settled` follows the run, after retries and compaction have also settled.

Not: a process lifetime. One process may execute many runs.

See `AGENT-001` and `PROTO-004`.

## steering

User input delivered to the current run before the next assistant request. Steering is polled before the first
request and after each completed turn. It does not cancel an in-flight stream; it changes what the next request
sees.

Not: cancellation. Use abort (`Esc`) to stop the current stream.

See `AGENT-007`.

## follow-up

User input queued during a run and consumed only when the loop would otherwise exit. A follow-up keeps the run
alive for at least one more turn, so the user can add context without racing the model.

Not: steering. Steering is delivered at the next turn boundary; a follow-up is delivered only at the exit
boundary.

See `AGENT-007`.

## compaction

Host-side reduction of the model-visible context when the transcript approaches the context window. Compaction
appends a summary entry with `first_kept_entry_id`; it never deletes or rewrites prior session entries. The
summary has a fixed structure and omits thinking blocks.

Not: truncation of the session file. The file keeps every entry; only the projection changes.

See `CTX-001`, `CTX-004`, `CTX-005`.

## projection

The derived, model-visible transcript computed from the session tree: choose a branch, apply the latest
compaction, apply context edits, and lower host-only entries. A projection is a pure function of stored
entries. It is never persisted as a replacement for history.

Not: the session file. The file is the source of truth; the projection is a view.

See `SESS-005` and `PLAN.MD` §5.3.

## provider

A runtime unit identified by `ProviderId`, with a base URL, auth source, model catalog, and model filters. A
provider answers "who do we talk to and with what credentials", not "what wire format do we speak".

Not: a model vendor. OpenRouter is one provider that fronts many vendors.

See `PROV-001` and `PLAN.MD` §6.2.1.

## api

A wire-protocol identifier such as `openai-completions` or `anthropic-messages`. The API determines request
and response shapes, streaming grammar, and compatibility flags. Both launch providers use
`openai-completions` in v1.0.

Not: a provider. Provider and API are orthogonal; one API can serve many providers, and one provider may expose
several APIs in the future.

See `PROV-010` and `PROV-022`.

## tool call

An assistant content block requesting a tool invocation. It carries a provider-assigned opaque `id`, a tool
`name`, and parsed JSON `arguments`. A tool call is a proposal; it is executed only after the assistant message
is final.

Not: a tool result. A call with no result is an incomplete turn, not a completed one.

See `MSG-002`, `MSG-009`, `AGENT-002`.

## tool result

A `toolResult` message linked to a tool call by `tool_call_id`, carrying output content and an `is_error` flag.
Tool failures are results, not loop failures; the model can recover from them.

Not: a run failure. Only a loop, hook, or transport failure ends the run.

See `AGENT-003`, `AGENT-005`.

## session

An append-only JSONL file containing a header and a tree of entries. It is the durable source of truth for
messages, model changes, thinking-level changes, compactions, branches, labels, and usage. Sessions are created
lazily and are safe under concurrent processes via an advisory lock.

Not: a transcript. The transcript is one projection of a session branch.

See `SESS-001`..`SESS-009`.

## branch

A path from the session root to a leaf entry. Branching creates a new child without modifying siblings; forking
copies a branch into a new session file with `parent_session` set. A session may contain many branches, but one
run projects exactly one.

Not: a git branch. codie branches are internal session-tree paths and are unrelated to version control.

See `SESS-005`, `SESS-006`.

## Additional terms

- **assistant event stream**: the provider-layer sequence `start -> text/thinking/toolcall deltas -> done|error`.
  It is an event stream, not a promise, and carries a live partial message (`MSG-040`..`MSG-044`).
- **transcript**: an ordered `Vec<Message>` produced by projection and consumed by the agent loop (`PLAN.MD`
  §5.3).
- **tool batch**: the set of tool calls from one finalized assistant message, executed in parallel or
  sequentially with results appended in source order (`AGENT-003`, `AGENT-004`).
- **usage**: normalized token counts and integer nano-USD cost attached to a finalized assistant message
  (`MSG-005`, `MSG-006`).
- **stop reason**: the terminal classification of an assistant message: `stop`, `length`, `tool_use`, `error`,
  or `aborted`. `pending` exists only on a partial message (`MSG-004`).
