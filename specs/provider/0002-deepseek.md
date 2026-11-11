---
id: SPEC-PROV-002
title: DeepSeek provider profile
status: accepted
version: 1.0.0
owners: ["@codie-maintainers"]
requirements: []
---

# Context

DeepSeek is served by the canonical OpenAI-compatible adapter (`SPEC-PROV-001`). This profile records the
provider-specific values and quirks without allocating new requirement IDs. Every behavior here is a
specialization of `PROV-010`..`PROV-022`.

# Profile

| Setting | Value |
|---|---|
| Provider id | `deepseek` |
| Base URL | `https://api.deepseek.com` (a trailing `/v1` is normalized away) |
| Default model | `deepseek-flash` |
| Wire API | `openai-completions` |
| Max-output field | `max_tokens` |
| Thinking encoding | `thinking: { type: enabled }` plus `reasoning_effort` when the model names one |
| Streaming usage | `stream_options: { include_usage: true }` |
| Cache usage | `prompt_cache_hit_tokens` |
| Reasoning replay | assistant messages replay `reasoning_content` |
| Stop reason | `finish_reason` is always present on a successful stream |
| Store / developer role | not supported |
| Attribution headers | none |

# Quirks

- Reasoning deltas arrive as `delta.reasoning_content`. They are assembled into `thinking` content and never
  mixed with visible text.
- `finish_reason = tool_calls` maps to `StopReason::ToolUse`; partial calls are never executed.
- `prompt_tokens = input + cached`; codie reports uncached input separately and recomputes the total.
- Pricing in the bundled catalog is USD per million tokens: `deepseek-flash` at $0.30 input, $1.20 output,
  $0.006 cache read.

# Verification

The profile is exercised by the DeepSeek reason and recorded-fixture tests in `crates/codie-ai/tests/`
(`reasoning.rs`, `provider_fixtures.rs`, `usage.rs`).
