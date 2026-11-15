---
id: SPEC-PROV-005
title: Google Gemini provider profile
status: accepted
version: 1.0.0
owners: ["@codie-maintainers"]
requirements: []
---

# Context

Google Gemini is served by the canonical OpenAI-compatible adapter (`SPEC-PROV-001`) through its
OpenAI-compatible endpoint. This profile records the provider-specific values and quirks without allocating new
requirement IDs. `PROV-024` (thought-signature replay) is canonical in `0001-openai-compatible.md`.

# Profile

| Setting | Value |
|---|---|
| Provider id | `gemini` |
| Base URL | `https://generativelanguage.googleapis.com/v1beta/openai` |
| Default model | `gemini-4-hydrogen` |
| Wire API | `openai-completions` |
| Max-output field | `max_tokens` (`max_completion_tokens` is accepted as an alias by the provider) |
| Thinking encoding | top-level `reasoning_effort`: `low`, `medium`, `high`, plus `xhigh` on `gemini-4-hydrogen` |
| Reasoning mandatory | yes for Gemini 3 and Gemini 4; `off` omits the field and never sends `"none"` |
| Streaming usage | `stream_options: { include_usage: true }` |
| Cache usage | `prompt_tokens_details.cached_tokens` |
| Reasoning replay | none; thought summaries are not requested, so no `reasoning_content` is replayed |
| Tool-call replay | `extra_content.google.thought_signature` captured and returned verbatim (`PROV-024`) |
| Stop reason | `finish_reason` is present on successful streams; `tool_calls` and `stop` both produce executable calls |
| Store / developer role | not supported |
| Attribution headers | none |
| Catalog | bundled with the binary; unknown IDs remain usable through `--model` |
| Cost | estimated from bundled USD prices; the provider reports no cost |

# Bundled catalog

Prices are USD per million tokens as published on 2026-11-15. Gemini 4 prices change on 2026-12-01; the
bundled values must be refreshed when that takes effect. Cache writes are billed as storage per hour, so the
per-token write price is zero.

| Model | Context | Max output | Input | Output | Cache read | Reasoning levels |
|---|---|---|---|---|---|---|
| `gemini-4-hydrogen` | 1,048,576 | 65,536 | $1.50 | $7.50 | $0.075 | `low`, `medium`, `high`, `xhigh` |
| `gemini-4-argon` | 1,048,576 | 65,536 | $2.00 | $10.00 | $0.10 | `low`, `medium`, `high` |
| `gemini-4-carbon` | 1,048,576 | 65,536 | $2.00 | $10.00 | $0.10 | `low`, `medium`, `high` |
| `gemini-3.8-flash` | 1,048,576 | 65,536 | $0.75 | $3.75 | $0.075 | `low`, `medium`, `high` |
| `gemini-3.5-flash-lite` | 1,048,576 | 65,536 | $0.30 | $2.50 | $0.03 | `low`, `medium`, `high` |

# Quirks

- `reasoning_effort: "minimal"` is mapped to `low` by Gemini 4; codie sends `low` for the `minimal` level.
  Gemini 3.8 Flash rejects `minimal`, so that level is omitted and the model default applies.
- Reasoning cannot be disabled on Gemini 3 and Gemini 4. The `off` level sends no `reasoning_effort` at all.
- Streaming tool calls may reuse `index: 0` for parallel calls or omit `index`. The adapter treats a non-empty
  `id` as authoritative so parallel calls are never merged (`PROV-013`).
- Gemini 3 and Gemini 4 reject a replayed function call that omits the signature returned with the original
  call. The adapter stores it on the tool-call content block (and therefore in the session transcript) and
  replays it as `extra_content.google.thought_signature` (`PROV-024`).
- Gemini emits `usage` in the final chunk when `include_usage` is requested; cache reads arrive in
  `prompt_tokens_details.cached_tokens`.
- Gemini may report `finish_reason: "stop"` on a tool-calling turn. codie executes the returned calls whenever
  the message contains tool calls, independent of the stop reason.

# Verification

See `crates/codie-ai/tests/gemini.rs`.
