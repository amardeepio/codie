# Providers

codie ships one OpenAI-compatible adapter and two provider presets: OpenRouter and DeepSeek. Both use HTTP with
API-key authentication and stream responses as Server-Sent Events. Provider support is planned for Phase 1;
nothing in this chapter is available in the Phase 0 binary.

## Support matrix

| Provider | Provider ID | Default base URL | Credential |
|---|---|---|---|
| DeepSeek | `deepseek` | `https://api.deepseek.com` | `DEEPSEEK_API_KEY` |
| OpenRouter | `openrouter` | `https://openrouter.ai/api/v1` | `OPENROUTER_API_KEY` |

Both speak the `openai-completions` API. A provider is a runtime unit (id, base URL, auth, catalog, filters); an
API is a wire protocol; a model carries the API, provider, limits, cost, and compatibility flags. Keeping these
separate is what lets one adapter serve both providers without provider-specific branches in the agent loop.

## Configuration

```toml
# ~/.config/codie/config.toml
default_model = "deepseek/deepseek-flash"
thinking = "medium"

[providers.deepseek]
api_key_env = "DEEPSEEK_API_KEY"
base_url = "https://api.deepseek.com"

[providers.openrouter]
api_key_env = "OPENROUTER_API_KEY"
base_url = "https://openrouter.ai/api/v1"
referer = "https://github.com/amardeepio/codie"
title = "codie"
```

Precedence is CLI flag, environment variable, project config, user config, then built-in default.

## Authentication

Resolution order is explicit `--api-key`, stored credential, provider environment variable, then an error.
Stored credentials live in `auth.json` with mode `0600`, are written atomically and fsynced, and are never
logged or placed in child-process argv. `--api-key` is ephemeral and is never written to disk. Planned commands
are `codie auth login`, `codie auth logout`, and `codie auth status [--json]`.

The normative auth requirements are `PROV-030`..`PROV-034`, reserved for a future auth spec. See
[`specs/provider/0001-openai-compatible.md`](../../specs/provider/0001-openai-compatible.md) for the current
summary.

## Differences the adapter handles

| Concern | DeepSeek | OpenRouter |
|---|---|---|
| Max output field | `max_tokens` | `max_completion_tokens` |
| Reasoning parameter | `thinking` plus optional `reasoning_effort` | `reasoning.effort` |
| Reasoning replay | Required on assistant messages | Dropped unless the model requires it |
| Cost | Estimated from model prices | Provider-reported `usage.cost`, authoritative |
| Routing | Fixed base URL | Optional `provider.order` / `only` / `allow_fallbacks` |
| Attribution | None | `HTTP-Referer` and `X-Title` |
| Catalog | Bundled TOML, refreshed only in codie releases | `GET /models`, ETag, 4-hour freshness |
| Session affinity | None | `x-session-id` |

## Streaming and tool calls

Responses are parsed as SSE, including frames split across arbitrary chunk boundaries, comment and keepalive
lines, CRLF, and a terminal `[DONE]`. Tool-call deltas accumulate by index and ID; argument fragments are
concatenated as text and parsed once at the end. Invalid JSON becomes an error tool result that the model can
recover from rather than a loop failure. Stop reasons and usage are normalized before the agent loop sees them.

Retries cover network errors, 408, 409, 429, 5xx, and `x-should-retry: true`, honoring `retry-after-ms` and
`retry-after`. Authentication failures, other 4xx responses, and context overflow are never retried. A retry
never re-emits content already delivered to the consumer.

## Model selection

Models are selected by `provider/model`, for example `deepseek/deepseek-flash`. OpenRouter model IDs are used
verbatim inside that path; codie does not rewrite vendor prefixes. A model is selectable only when it is a chat
model whose input modalities include text. `codie models [--provider <p>] [--json]` is planned for Phase 1; the
DeepSeek catalog ships as bundled data, so newly released DeepSeek models require a codie release.

## Testing and limitations

Provider behavior is tested with recorded fixtures and a scripted fake provider; CI never calls a live API and
never records credentials. Text is the only supported input in v1.0. Anthropic-native passthrough, OAuth login,
and additional vendors are out of scope. See [`specs/provider/0001-openai-compatible.md`](../../specs/provider/0001-openai-compatible.md)
for the normative wire contract.
