---
id: SPEC-MSG-001
title: Message model
status: accepted
version: 1.0.0
owners: ["@codie-maintainers"]
requirements: ["MSG-001", "MSG-002", "MSG-003", "MSG-004", "MSG-005", "MSG-006", "MSG-007", "MSG-008", "MSG-009", "MSG-010", "MSG-040", "MSG-041", "MSG-042", "MSG-043", "MSG-044"]
---

# Context

Every subsystem in codie agrees on one message model: the provider adapter encodes it, the agent loop appends
it, the session store persists it, and the TUI renders it. A loose model leaks provider quirks into the loop and
makes replay, compaction, and cost accounting unreliable. This spec fixes the message kinds, content ordering,
assistant metadata, normalized usage and cost, the system-message-only prompt boundary, tool-call identity,
timestamp units, and the provider event stream that produces an assistant message.

The model is deliberately narrower than any provider's wire format. Provider-specific fields stay in
diagnostics or compatibility flags; they never become loop state.

# Normative behavior

**MSG-001 (MUST)** Every message is exactly one of `system`, `user`, `assistant`, or `toolResult` at the provider boundary. Host-only variants (`bashExecution`, `custom`, `branchSummary`, `compactionSummary`) MAY exist in host state, but they MUST be lowered to one of the four wire kinds before projection into a provider request, and lowering MUST preserve textual content and source order.
Verification: `crates/codie-core/tests/message_model.rs::spec_msg_001_message_kinds_lower_before_wire`

**MSG-002 (MUST)** Content blocks preserve source order and are exactly one of `Text`, `Thinking`, `ToolCall`, or `Image`. No adapter may reorder, merge, or silently drop blocks; the only permitted removals are the provider-specific projection rules in `PROV-016` and `PROV-017`. An empty `user` or `toolResult` message is invalid; an assistant message MAY be empty only when its stop reason is `stop`, `length`, `error`, or `aborted`.
Verification: `crates/codie-core/tests/message_model.rs::spec_msg_002_content_order_preserved`

**MSG-003 (MUST)** An `AssistantMessage` carries `api`, `provider`, `model`, `usage`, and `stop_reason`, plus optional `response_model`, `response_id`, `thinking_level`, `provider_thinking_level`, `error_message`, `raw_stop_reason`, `duration_ms`, and `timestamp`. Optional fields MUST be absent when the provider did not supply them; `usage` MUST always be present and zero-valued when the provider reported none. `error_message` MUST be present when and only when `stop_reason` is `error`.
Verification: `crates/codie-core/tests/message_model.rs::spec_msg_003_assistant_metadata_roundtrip`

**MSG-004 (MUST)** `StopReason` is exactly `pending | stop | length | tool_use | error | aborted | deferred`. `pending` exists only on a partial streaming message and MUST NOT be persisted or passed to a tool executor. Every finalized message MUST carry a non-`pending` stop reason; a stream that ends without one is an error.
Verification: `crates/codie-core/tests/message_model.rs::spec_msg_004_pending_is_not_persistable`

**MSG-005 (MUST)** `Usage` is normalized to `input` (uncached input tokens), `output` (including reasoning tokens), `cache_read`, `cache_write`, optional `reasoning`, `total_tokens`, and `cost`. `total_tokens` MUST equal `input + output + cache_read + cache_write`; `reasoning` is a subset of `output` and MUST NOT be added again. Token counters are unsigned 64-bit values; absent provider counters are zero, not inferred.
Verification: `crates/codie-core/tests/message_model.rs::spec_msg_005_usage_normalization`

**MSG-006 (MUST)** Costs are stored as integer nano-USD (`1 USD = 1_000_000_000`), never as `f64`. Provider-reported USD cost is converted by rounding half away from zero to the nearest nano-USD and sets `cost_from_provider = true`. Estimates use the model's per-token nano-USD prices and set it to `false`. Rendering uses exactly six decimal places of USD; internal arithmetic stays integral.
Verification: `crates/codie-core/tests/message_model.rs::spec_msg_006_cost_is_integer_nano_usd`

**MSG-007 (MUST)** The provider-facing context type is `TranscriptContext`. The system prompt and tool declarations MUST be carried only as leading system messages; no parallel `system_prompt` or `tools` field may cross the adapter boundary. An adapter MAY project those entries into its wire-native `tools` field, but it MUST derive that projection from the messages and MUST NOT maintain a second prompt representation.
Verification: `crates/codie-core/tests/message_model.rs::spec_msg_007_system_and_tools_are_leading_messages`

**MSG-008 (MUST)** `normalize_context` folds a legacy `system_prompt` plus tool declarations into one leading system message. `collapse_system_messages` replays all system messages into a single system message for providers that reject mid-conversation system turns. Collapsing MUST preserve the textual content and relative order of every collapsed message and MUST be idempotent.
Verification: `crates/codie-core/tests/message_model.rs::spec_msg_008_normalize_and_collapse_system_messages`

**MSG-009 (MUST)** Tool-call IDs are provider-assigned opaque strings. codie MUST NOT regenerate, reformat, or reuse them. If a provider emits characters that are invalid on a later hop, sanitization MUST be deterministic, MUST preserve uniqueness within the conversation, and MUST record the original-to-sanitized mapping in diagnostics; the matching tool result MUST use the same sanitized ID.
Verification: `crates/codie-core/tests/message_model.rs::spec_msg_009_tool_call_ids_are_preserved`

**MSG-010 (MUST)** Message timestamps are Unix milliseconds. Session entry timestamps are RFC 3339 strings. A message timestamp MUST be captured when the message is created, not when it is persisted, and MUST survive session replay unchanged.
Verification: `crates/codie-core/tests/message_model.rs::spec_msg_010_timestamp_units`

# Assistant event stream

The provider layer emits the following deltas. The agent layer may wrap them in its own events, but the
provider contract is this sequence.

```rust
pub enum AssistantDelta {
    Start { partial: AssistantMessage },
    TextStart { index: usize },
    TextDelta { index: usize, delta: String },
    TextEnd { index: usize, content: String },
    ThinkingStart { index: usize },
    ThinkingDelta { index: usize, delta: String },
    ThinkingEnd { index: usize, content: String },
    ToolCallStart { index: usize },
    ToolCallDelta { index: usize, delta: String },
    ToolCallEnd { index: usize, call: ToolCall },
    Done { reason: StopReason, message: AssistantMessage },
    Error { reason: StopReason, message: AssistantMessage },
}
```

**MSG-040 (MUST)** `start` precedes all content events. A setup failure MAY emit `error` without a preceding `start`. At most one `start` is emitted per stream, and once emitted it is the first event.
Verification: `crates/codie-ai/tests/event_stream.rs::spec_msg_040_start_precedes_content`

**MSG-041 (MUST)** `partial` is a live value, not a snapshot. Text and thinking blocks are empty at their `*_start` event and authoritative at their `*_end` event. Deltas are append-only, and the `*_end` content MUST equal the concatenation of the deltas for that block.
Verification: `crates/codie-ai/tests/event_stream.rs::spec_msg_041_partial_is_live`

**MSG-042 (MUST)** Exactly one terminal `done` or `error` is emitted, after which the stream ends. No content event may follow a terminal event. A stream that ends without a terminal event is a protocol error surfaced by the adapter as `error`.
Verification: `crates/codie-ai/tests/event_stream.rs::spec_msg_042_exactly_one_terminal_event`

**MSG-043 (MUST)** A redacted thinking block MAY be complete at `start` and emit no `thinking_*` deltas. Its content is marked `redacted = true`; consumers MUST NOT require deltas for it and MUST NOT summarize it into visible text.
Verification: `crates/codie-ai/tests/event_stream.rs::spec_msg_043_redacted_thinking_complete_at_start`

**MSG-044 (MUST)** A consumer can either iterate events (UI) or await the final message (headless) without racing. The stream is backed by a broadcast channel plus a oneshot; the oneshot message MUST be identical to the `message` carried by the terminal event, including usage and stop reason.
Verification: `crates/codie-ai/tests/event_stream.rs::spec_msg_044_iterate_and_await_do_not_race`

# State machine / data shapes

```rust
pub enum Message {
    System(SystemMessage),
    User(UserMessage),
    Assistant(AssistantMessage),
    ToolResult(ToolResultMessage),
}

pub enum Content {
    Text { text: String, signature: Option<String> },
    Thinking { thinking: String, signature: Option<String>, redacted: bool },
    ToolCall { id: String, name: String, arguments: serde_json::Value },
    Image { data: String, mime_type: String }, // Phase 5
}

pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
    pub arguments_error: Option<String>,
}

pub struct Usage {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub reasoning: Option<u64>,
    pub total_tokens: u64,
    pub cost: Cost,
}

pub struct Cost {
    pub nano_usd: u64,
    pub from_provider: bool,
}
```

The assistant stream state machine is `Idle -> Started -> Streaming -> Finalized`. `Started` is entered only by
`Start`; `Streaming` covers text, thinking, and tool-call deltas; `Finalized` is entered by `Done` or `Error` and
has no outgoing transitions. A setup failure takes `Idle -> Finalized` without `Started`.

Invariants:

1. A finalized assistant message is immutable.
2. Within one assistant message, tool-call IDs are unique.
3. A `ToolCallEnd` carries the fully concatenated and parsed arguments for its index.
4. `Done.message` and the oneshot value are byte-identical after serialization.
5. `total_tokens` never double-counts reasoning tokens.
6. A partial message is never persisted; only a `MessageEnd` for a finalized message may be persisted.

# Edge cases

- Empty assistant output with `stop_reason = stop` is valid and persists as a message with an empty content
  vector; a user message with no blocks is rejected at construction.
- A provider that returns usage before `finish_reason` must not finalize early; usage is attached at `Done`.
- A stream aborted after `start` but before any delta emits `error` with `reason = aborted` and a valid partial
  message carrying the aborted stop reason.
- Duplicate tool-call indices are a protocol error; duplicate IDs across separate assistant messages are
  allowed only if the provider actually reuses them, and the session keeps them verbatim.
- An unknown content variant from a future version is rejected on read with a session error, never silently
  dropped.
- Cost overflow beyond `u64::MAX` nano-USD is an internal error rather than a wrapped value.
- Unicode text is stored as UTF-8; block indices count blocks, not bytes or grapheme clusters.
- A zero-token usage record is valid and must render as zero, not as "unknown".

# Non-goals

- Provider wire encoding and SSE parsing: `specs/provider/0001-openai-compatible.md`.
- Session entry layout and migration: `specs/session/0001-jsonl-format.md`.
- Rendering, diffs, and markdown: `specs/tui/0001-layout-and-rendering.md`.
- Image input behavior beyond the Phase 5 placeholder variant; v1 downgrades images per `PROV-017`.

# Reference implementation

pi: `packages/ai/src/types.ts`, `packages/ai/src/utils/transcript.ts`, and
`packages/ai/src/utils/event-stream.ts` (behavioral reference only; no code copied).
