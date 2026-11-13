---
id: SPEC-PROV-001
title: OpenAI-compatible provider adapter
status: accepted
version: 1.0.0
owners: ["@codie-maintainers"]
requirements: ["PROV-001", "PROV-002", "PROV-003", "PROV-004", "PROV-005", "PROV-010", "PROV-011", "PROV-012", "PROV-013", "PROV-014", "PROV-015", "PROV-016", "PROV-017", "PROV-018", "PROV-019", "PROV-020", "PROV-021", "PROV-022", "PROV-023"]
---

# Context

codie ships one OpenAI-compatible wire adapter for two providers, DeepSeek and OpenRouter. The adapter owns HTTP,
SSE framing, retries, error normalization, tool-call assembly, stop-reason mapping, usage normalization, and the
small compatibility matrix that lets one implementation serve both providers without provider-specific code in
the agent loop. Correctness here is observable: a wrong counter, a misplaced tool argument, or a retried
partial stream corrupts the transcript and the user's cost accounting.

`PROV-019` is allocated here to the compat-gated request-field rule that `PLAN.MD` §6.2.2 states without an ID.
Auth requirements `PROV-030`..`PROV-034` are reserved for `specs/provider/0004-auth.md`; the resolution rules are
summarized informatively below.

# Data shapes

```rust
pub struct Model {
    pub id: String,
    pub provider: ProviderId,
    pub api: ApiId,                 // "openai-completions"
    pub base_url: String,
    pub context_window: u32,
    pub max_output_tokens: u32,
    pub input: InputModalities,     // text | text+image
    pub reasoning: bool,
    pub thinking_levels: ThinkingLevelMap,
    pub cost: ModelCost,            // nano-USD per token
    pub compat: Compat,
    pub headers: HeaderMap,
}

pub struct Compat {
    pub supports_store: bool,
    pub supports_developer_role: bool,
    pub max_tokens_field: MaxTokensField, // MaxTokens | MaxCompletionTokens
    pub requires_reasoning_content_on_assistant_messages: bool,
    pub thinking_format: ThinkingFormat,  // DeepSeek | OpenRouter | OpenAi
    pub supports_strict_mode: bool,
}
```

`ThinkingLevel` is exactly `off | minimal | low | medium | high | xhigh | max`. The adapter is synchronous in
shape and fallible only through events.

# Normative behavior

**PROV-001 (MUST)** `stream` returns an `AssistantEventStream` synchronously. Auth, model-resolution, and connection setup failures are emitted as `error` events, optionally preceded by `start`; once the stream value is returned, the adapter MUST NOT panic or return a synchronous error.
Verification: `crates/codie-ai/tests/provider_contract.rs::spec_prov_001_stream_returns_before_io`

**PROV-002 (MUST)** `stream` honors the `CancellationToken`. Cancellation closes the HTTP stream and emits a terminal event with `stop_reason = aborted`, never a generic provider error. Cancellation after a terminal event is a no-op.
Verification: `crates/codie-ai/tests/provider_contract.rs::spec_prov_002_abort_maps_to_aborted`

**PROV-003 (MUST)** Every request passes through one shared retry layer. Retry on network errors, 408, 409, 429, 5xx, and a truthy `x-should-retry` header. Honor `retry-after-ms`, then `retry-after` in seconds or HTTP date form; otherwise use `min(0.5 * 2^n, 8s) * jitter(0.75..=1.0)`. Cap total delay at `max_retry_delay_ms` (default 60s); a server-requested delay above the cap fails fast with a clear message. Never retry other 4xx, auth failures, or context overflow. A retry MUST NOT re-emit content already delivered to the consumer.
Verification: `crates/codie-ai/tests/retry.rs::spec_prov_003_retry_policy_and_backoff`

**PROV-004 (MUST)** Provider errors normalize to `"<status>: <body>"` with the body truncated to 4000 characters. Transport errors without an HTTP status append the deepest `cause` chain. The normalized string is what enters `AssistantMessage.error_message`; raw bodies stay in trace logs only.
Verification: `crates/codie-ai/tests/errors.rs::spec_prov_004_error_normalization`

**PROV-005 (MUST)** Request and response bodies are logged at `trace` level with `Authorization` and API keys redacted. Fixtures are captured only when the `--record` test feature and its environment opt-in are both present; CI never records and never sends live credentials.
Verification: `crates/codie-ai/tests/redaction.rs::spec_prov_005_trace_redaction_and_recording`

**PROV-010 (MUST)** The maximum-output field is selected per model: DeepSeek uses `max_tokens`; OpenRouter uses `max_completion_tokens` unless the underlying model's `compat.max_tokens_field` says otherwise. The value is never sent when the model declares no output limit or the caller requests none.
Verification: `crates/codie-ai/tests/openai_compat.rs::spec_prov_010_max_tokens_field`

**PROV-011 (MUST)** Reasoning parameters are selected by `thinking_format`: DeepSeek sends `thinking: {type: "enabled"|"disabled"}` plus optional `reasoning_effort`; OpenRouter sends `reasoning: {effort: "<mapped>"}` with `off` mapped to `{effort: "none"}`; the future OpenAI format sends `reasoning_effort`. A level absent from a model's map is clamped down to the nearest supported level. `xhigh` and `max` are unsupported unless explicitly mapped.
Verification: `crates/codie-ai/tests/openai_compat.rs::spec_prov_011_reasoning_parameter_selection`

**PROV-012 (MUST)** SSE parsing accepts `data:` frames split across arbitrary chunk boundaries, ignores comment and keepalive lines, tolerates CRLF and LF, and terminates on `data: [DONE]`. An EOF without a terminal chunk is an error ("stream ended without finish_reason") unless the provider is explicitly marked as omitting finish reasons in `compat`.
Verification: `crates/codie-ai/tests/sse.rs::spec_prov_012_sse_frame_reassembly`

**PROV-013 (MUST)** Tool-call deltas accumulate by `index` (primary) and `id` (secondary). An `id` arriving after arguments is attached to the pending block with the same index. Argument fragments are concatenated as text and parsed once at the end; invalid JSON sets `arguments_error` and produces an error tool result the model can recover from. Arguments are never parsed incrementally.
Verification: `crates/codie-ai/tests/tool_calls.rs::spec_prov_013_tool_call_accumulation`

**PROV-014 (MUST)** Stop reasons map exactly: `null`, `stop`, and `end_turn` to `stop`; `length` and `max_tokens` to `length`; `tool_calls` and `function_call` to `tool_use`; `content_filter` to `error` with a diagnostic message; any unknown value to `error` with the raw value preserved in `raw_stop_reason`.
Verification: `crates/codie-ai/tests/stop_reason.rs::spec_prov_014_stop_reason_mapping`

**PROV-015 (MUST)** Usage maps as follows. DeepSeek: `input = prompt_tokens - prompt_cache_hit_tokens`, `cache_read = prompt_cache_hit_tokens`, `output = completion_tokens`, and `reasoning` from `completion_tokens_details.reasoning_tokens` when present. OpenRouter uses the same shape with `prompt_tokens_details.cached_tokens` as the cache read, converts `usage.cost` from USD to nano-USD and sets `cost_from_provider = true`, and preserves `is_byok` and `cost_details` in diagnostics. Both providers use the `Usage::total_tokens` formula from `MSG-005`.
Verification: `crates/codie-ai/tests/usage.rs::spec_prov_015_usage_mapping`

**PROV-016 (MUST)** Assistant reasoning is replayed verbatim as `reasoning_content` when the model requires it (DeepSeek) and dropped otherwise. When reasoning was not preserved and `compat` requires replay, the adapter sends `reasoning_content: ""`. Thinking blocks are never summarized into visible text.
Verification: `crates/codie-ai/tests/reasoning.rs::spec_prov_016_reasoning_replay`

**PROV-017 (MUST)** Tool results are sent as `{"role":"tool","tool_call_id":"...","content":"..."}` with the matching ID verbatim. Image-bearing tool results are downgraded to a text note because tool results are text on this wire.
Verification: `crates/codie-ai/tests/openai_compat.rs::spec_prov_017_tool_result_wire_shape`

**PROV-018 (MUST)** OpenRouter attribution headers are configurable and default to `HTTP-Referer: https://github.com/amardeepio/codie` and `X-Title: codie`. DeepSeek requests do not send them.
Verification: `crates/codie-ai/tests/openrouter.rs::spec_prov_018_attribution_headers`

**PROV-019 (MUST)** Request fields are emitted only when the selected model's `compat` declarations support them. Unsupported fields MUST be omitted rather than sent with `null` or a default value, and capability resolution MUST be per model, not per provider.
Verification: `crates/codie-ai/tests/openai_compat.rs::spec_prov_019_request_field_gating`

**PROV-020 (MUST)** OpenRouter model IDs are used verbatim (`anthropic/claude-...`, `deepseek/deepseek-...`). A model is selectable only when `architecture.input_modalities` includes `text` and the model is a chat model. Non-chat and non-text models are excluded from selection, not silently coerced. The OpenRouter picker offers coding models only: tool-capable models that either appear in the `programming` use-case category or report an Artificial Analysis coding index of at least 70. Variant aliases (`:batch` and `~` prefixes) are excluded, and models sharing a base ID are deduplicated. Catalog prices are per-token USD strings, converted to USD per million tokens; negative (variable) prices become zero.
Verification: `crates/codie-ai/tests/catalog.rs::spec_prov_020_model_id_and_filtering`, `crates/codie-ai/tests/catalog.rs::spec_prov_020_coding_models_filter_and_order`

**PROV-021 (MUST)** Catalog refresh is best-effort: a 4-second timeout, ETag support, a 4-hour freshness window, and stale cache served on failure. Refresh never blocks startup and runs in the background in every mode, including the interactive TUI. A refresh failure is logged and does not fail the run. The full model list and the `programming` category are refreshed together but cached independently.
Verification: `crates/codie-ai/tests/catalog.rs::spec_prov_021_catalog_refresh_is_non_blocking`

**PROV-022 (MUST)** OpenRouter is used only through its OpenAI-compatible endpoint in v1. Anthropic-native passthrough, provider-specific SDKs, and direct vendor adapters are out of scope and MUST NOT be selected by the router.
Verification: `crates/codie-ai/tests/openrouter.rs::spec_prov_022_openai_compatible_only`

**PROV-023 (MUST)** Image blocks in a user message are sent as `{"type":"image_url","image_url":{"url":"data:<mime>;base64,<data>"}}` alongside text parts. A text-only user message stays a plain string.
Verification: `crates/codie-ai/tests/openai_compat.rs::spec_prov_023_user_images_use_data_urls`

# Request and response shapes

Both providers receive `POST {base_url}/chat/completions` with `Authorization: Bearer <key>`,
`Content-Type: application/json`, and `Accept: text/event-stream`. Fields are emitted only when supported by
`compat`.

```json
{
  "model": "deepseek-flash",
  "messages": [{"role": "system", "content": "You are a coding assistant..."}, {"role": "user", "content": "Fix the failing test in src/lib.rs"}],
  "stream": true,
  "stream_options": {"include_usage": true},
  "max_tokens": 8192,
  "tools": [{"type": "function", "function": {"name": "read", "description": "...", "parameters": {}}}],
  "thinking": {"type": "enabled"},
  "reasoning_effort": "high"
}
```

OpenRouter uses `max_completion_tokens`, `reasoning: {"effort": "high"}`, and an optional
`provider: {"order": [...]}` routing object. A successful response is an SSE stream of OpenAI-shaped chunks:

```text
data: {"id":"...","choices":[{"index":0,"delta":{"role":"assistant","reasoning_content":"We need to "}}]}
data: {"id":"...","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"read","arguments":"{\"path\":"}}]}}]}
data: {"id":"...","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"src/lib.rs\"}"}}]}}]}
data: {"id":"...","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}
data: {"id":"...","choices":[],"usage":{"prompt_tokens":1200,"completion_tokens":80,"prompt_cache_hit_tokens":900}}
data: [DONE]
```

# SSE parsing

The parser is a byte-oriented state machine over `bytes_stream`. It buffers until a blank line completes an
event, joins continuation chunks, strips one trailing `\r` from each line, and treats only `data:` fields as
payload. `[DONE]` is terminal and is not JSON. Comment lines beginning with `:` and empty keepalive frames are
ignored. A JSON decode failure is a terminal error with the raw frame preserved in diagnostics, capped at 4000
characters.

# Tool-call accumulation

1. A delta with a new `index` creates a pending call; the first non-empty `id` and `function.name` are kept.
2. Argument fragments are appended to a string buffer in arrival order.
3. An `id` arriving after arguments attaches to the pending call at the same index.
4. At `finish_reason`, every pending call is parsed once. Valid JSON becomes `arguments`; invalid JSON produces
   `arguments_error` and the raw text is retained for the error result.
5. Calls are emitted in ascending index order, independent of the order in which fragments arrived.

Only choice index 0 is consumed. Additional choices, if a provider returns them despite `n = 1`, are ignored
and recorded in diagnostics rather than merged.

# Retry and mapping tables

| Condition | Action |
|---|---|
| Network error, 408, 409, 429, 5xx, `x-should-retry: true` | Retry with the policy in `PROV-003` |
| `retry-after-ms`, `retry-after` seconds/date | Use the server delay, capped at `max_retry_delay_ms` |
| Other 4xx, 401, 403, context overflow | Fail immediately, normalized by `PROV-004` |
| Abort signal | Terminal `aborted`, no retry |

| Provider | Input | Cache read | Output | Reasoning | Cost |
|---|---|---|---|---|---|
| DeepSeek | `prompt_tokens - prompt_cache_hit_tokens` | `prompt_cache_hit_tokens` | `completion_tokens` | `completion_tokens_details.reasoning_tokens` | model estimate |
| OpenRouter | `prompt_tokens - prompt_tokens_details.cached_tokens` | `prompt_tokens_details.cached_tokens` | `completion_tokens` | details when present | `usage.cost` → nano-USD |

# Provider differences

| Item | DeepSeek | OpenRouter |
|---|---|---|
| Base URL | `https://api.deepseek.com` (accept and normalize `/v1`) | `https://openrouter.ai/api/v1` |
| Auth | `DEEPSEEK_API_KEY` | `OPENROUTER_API_KEY` |
| Max output field | `max_tokens` | `max_completion_tokens` |
| Reasoning | `thinking` plus optional `reasoning_effort` | `reasoning.effort` |
| Reasoning replay | Required on assistant messages | Dropped unless the model requires it |
| Attribution | None | `HTTP-Referer`, `X-Title` |
| Catalog | Bundled `catalog/deepseek.toml` | `GET /models`, ETag cache |

# Auth resolution (informative)

The normative auth requirements are `PROV-030`..`PROV-034` in `specs/provider/0004-auth.md`. Until that spec is
accepted, implementations MUST follow this order: explicit `--api-key`, then the stored credential, then the
provider's environment variable, then an error. Stored credentials live in `auth.json` with mode `0600`, are
written atomically (temp file, fsync, rename), and are never logged, placed in child-process argv, or included
in diagnostics. `--api-key` is ephemeral and never written to disk.

# Edge cases

- `[DONE]` split across chunks, or a chunk boundary in the middle of a UTF-8 escape sequence, must reassemble.
- A usage-only chunk (empty `choices`) after `finish_reason` is expected and must not create an assistant block.
- A tool call whose arguments are `""` is invalid JSON and must produce a recoverable result, not a panic.
- A provider that omits `usage` yields zero counters and a model-estimate cost, never a missing usage record.
- An unknown stop reason is preserved in `raw_stop_reason` and classified as `error`, never guessed as `stop`.

# Non-goals

- Anthropic-native and Google-native adapters, Bedrock, and Azure (`PROV-022`, `PLAN.MD` §1.2).
- OAuth or subscription login; API keys only.
- Tool-result images are downgraded per `PROV-017`; user-message images are supported per `PROV-023`.

# Reference implementation

pi: `packages/ai/src/api/openai-completions.ts`, `src/providers/{openrouter,deepseek}.ts`,
`src/providers/compat-schema.ts`, and `catalog/providers/deepseek.ts` (behavioral reference only; no code
copied).
