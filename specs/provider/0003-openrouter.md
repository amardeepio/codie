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
| Coding picker | `programming` category or coding index ≥ 70, with tool calling |
| Switching | `/provider` in the TUI swaps the client and selects the provider default |
| Wire API | `openai-completions` only |
| Max-output field | `max_completion_tokens` unless the model overrides it |
| Thinking encoding | `reasoning: { effort }`, with `off` mapped to `none` |
| Streaming usage | `stream_options: { include_usage: true }` |
| Cache usage | `prompt_tokens_details.cached_tokens` |
| Cost | authoritative USD `cost` field; converted to integer nano-USD |
| Attribution | `HTTP-Referer: https://github.com/amardeepio/codie`, `X-Title: codie` |
| Session affinity | `x-session-id` header when a session id exists |

# Catalog refresh

- Endpoints: `/models` under the configured base URL, plus `/models?category=programming` for the curated
  coding list. The two responses are cached together with separate ETags.
- Freshness: four hours; a fresh cache makes no request.
- Conditional requests use the stored `ETag`; `304 Not Modified` extends freshness.
- Timeout: four seconds.
- Failures (transport, timeout, non-2xx, malformed JSON) are logged and the stale cache is served. A refresh
  never fails a run and never blocks startup; it is spawned in interactive, print, JSON, and RPC modes.
- Only text-in/text-out chat models are selectable; embedding and image-output models are excluded.
- The picker narrows that set to coding models: tool calling plus either membership in the `programming`
  category or a coding index of at least 70. `:batch` and `~` aliases are dropped and shared base IDs are
  deduplicated. Unscored models in the category are still offered, so newly released models appear before
  benchmarks exist.
- Model metadata from the catalog carries the real context window, output limit, price, and reasoning efforts
  into the adapter. A model that advertises mandatory reasoning never sends `reasoning: { effort: "none" }`.
- Curated built-ins are always merged first, so `openrouter/auto` stays the default and an empty cache still
  leaves a working picker.

# Verification

See `crates/codie-ai/tests/openrouter.rs` and `crates/codie-ai/tests/catalog.rs`.
