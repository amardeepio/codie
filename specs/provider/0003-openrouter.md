---
id: SPEC-PROV-003
title: OpenRouter provider profile
status: accepted
version: 1.0.0
owners: ["@codie-maintainers"]
requirements: []
---

# Context

OpenRouter is served by the canonical OpenAI-compatible adapter (`SPEC-PROV-001`). This profile records the
provider-specific values, attribution, cost reporting, and catalog refresh behavior. It allocates no new
requirement IDs; `PROV-018` (attribution), `PROV-020` (model selection), `PROV-021` (catalog refresh), and
`PROV-022` (OpenAI-compatible only) are canonical.

# Profile

| Setting | Value |
|---|---|
| Provider id | `openrouter` |
| Base URL | `https://openrouter.ai/api/v1` |
| Default model | `openrouter/auto` |
| Full model IDs | namespaced IDs such as `deepseek/deepseek-v4.1-flash-1017`, used verbatim |
| Wire API | `openai-completions` only |
| Max-output field | `max_completion_tokens` unless the model overrides it |
| Thinking encoding | `reasoning: { effort }`, with `off` mapped to `none` |
| Streaming usage | `stream_options: { include_usage: true }` |
| Cache usage | `prompt_tokens_details.cached_tokens` |
| Cost | authoritative USD `cost` field; converted to integer nano-USD |
| Attribution | `HTTP-Referer: https://github.com/amardeepio/codie`, `X-Title: codie` |
| Session affinity | `x-session-id` header when a session id exists |

# Catalog refresh

- Endpoint: `/models` under the configured base URL.
- Freshness: four hours; a fresh cache makes no request.
- Conditional requests use the stored `ETag`; `304 Not Modified` extends freshness.
- Timeout: four seconds.
- Failures (transport, timeout, non-2xx, malformed JSON) are logged and the stale cache is served. A refresh
  never fails a run and never blocks startup.
- Only text-in/text-out chat models are selectable; embedding and image-output models are excluded.

# Verification

See `crates/codie-ai/tests/openrouter.rs` and `crates/codie-ai/tests/catalog.rs`.
