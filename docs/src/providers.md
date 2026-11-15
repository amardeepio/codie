# Providers

codie ships one OpenAI-compatible adapter and three provider presets: OpenRouter, DeepSeek, and Google Gemini.
All use HTTP with API-key authentication and stream responses as Server-Sent Events. All three are available in
the binary.

## Support matrix

| Provider | Provider ID | Default base URL | Credential |
|---|---|---|---|
| DeepSeek | `deepseek` | `https://api.deepseek.com` | `DEEPSEEK_API_KEY` |
| OpenRouter | `openrouter` | `https://openrouter.ai/api/v1` | `OPENROUTER_API_KEY` |
| Google Gemini | `gemini` | `https://generativelanguage.googleapis.com/v1beta/openai` | `GEMINI_API_KEY` |

All three speak the `openai-completions` API. A provider is a runtime unit (id, base URL, auth, catalog, filters); an
API is a wire protocol; a model carries the API, provider, limits, cost, and compatibility flags. Keeping these
separate is what lets one adapter serve every provider without provider-specific branches in the agent loop.

## Configuration

```toml
# ~/.config/codie/config.toml
default_provider = "deepseek"
default_model = "deepseek-flash"
thinking = "medium"
max_turns = 200
base_url = "" # optional proxy or test endpoint

[permissions]
allow = ["edit", "bash"]
```

Precedence is CLI flag, environment variable, `config.toml`, then built-in default. `codie config get` prints
the effective values.

## Authentication

Resolution order is explicit `--api-key`, stored credential, provider environment variable, then an error.
Stored credentials live in `auth.json` with mode `0600`, are written atomically and fsynced, and are never
logged or placed in child-process argv. `--api-key` is ephemeral, requires an explicit `--model`, and is never
written to disk. Use `codie auth login`, `codie auth logout`, and `codie auth status [--json]` to manage stored
keys. `auth status` exits `2` when no provider is configured.

The normative auth requirements are `PROV-030`..`PROV-034` in
[`specs/provider/0004-auth.md`](../../specs/provider/0004-auth.md).

## Differences the adapter handles

| Concern | DeepSeek | OpenRouter | Gemini |
|---|---|---|---|
| Max output field | `max_tokens` | `max_completion_tokens` | `max_tokens` |
| Reasoning parameter | `thinking` plus optional `reasoning_effort` | `reasoning.effort` | top-level `reasoning_effort` |
| Reasoning replay | Required on assistant messages | Dropped unless the model requires it | Dropped; tool-call thought signatures are replayed |
| Reasoning off | `thinking: {type: "disabled"}` | `reasoning: {effort: "none"}` | Not supported on Gemini 3/4; the field is omitted |
| Cost | Estimated from model prices | Provider-reported `usage.cost`, authoritative | Estimated from model prices |
| Routing | Fixed base URL | Optional `provider.order` / `only` / `allow_fallbacks` | Fixed base URL |
| Attribution | None | `HTTP-Referer` and `X-Title` | None |
| Catalog | Bundled in the binary, updated with codie releases | `GET /models`, ETag, 4-hour freshness, stale-on-failure | Bundled in the binary; other IDs usable with `--model` |
| Session affinity | None | `x-session-id` | None |

## Streaming and tool calls

Responses are parsed as SSE, including frames split across arbitrary chunk boundaries, comment and keepalive
lines, CRLF, and a terminal `[DONE]`. Tool-call deltas accumulate by index and ID; argument fragments are
concatenated as text and parsed once at the end. Invalid JSON becomes an error tool result that the model can
recover from rather than a loop failure. Stop reasons and usage are normalized before the agent loop sees them.

Retries cover network errors, 408, 409, 429, 5xx, and `x-should-retry: true`, honoring `retry-after-ms` and
`retry-after`. Authentication failures, other 4xx responses, and context overflow are never retried. A retry
never re-emits content already delivered to the consumer.

## Model selection

Models are selected by `provider/model`, for example `deepseek/deepseek-flash` or
`gemini/gemini-4-hydrogen`. OpenRouter model IDs are used verbatim inside that path; codie does not rewrite
vendor prefixes. A model is selectable only when it is a chat model whose input modalities include text. The
DeepSeek and Gemini catalogs ship as bundled data, so newly released models require a codie release, though any
model ID can still be passed to `--model`.

OpenRouter models come from the live `/models` catalog, cached under the data directory. The picker narrows the
catalog to coding models: models that support tool calling and either appear in OpenRouter's `programming`
category or score at least 70 on the Artificial Analysis coding index. `:batch` aliases and tilde aliases are
dropped, and models sharing a base ID are deduplicated. Curated entries (`openrouter/auto` and the DeepSeek
OpenRouter default) are merged first, so the picker stays usable when the cache is empty or stale. The catalog
refresh runs in the background in every mode; a completed refresh appears the next time `/model` opens.

`codie models [--provider <p>] [--json]` lists the merged catalog from the local cache without a network call.

In the TUI, `/provider` switches between DeepSeek, OpenRouter, and Gemini in place. The agent keeps one stable
client handle, so an in-flight session can change provider without restarting; the switch selects the new
provider's default model and persists a model-change entry.

## Testing and limitations

Provider behavior is tested with recorded fixtures and a scripted fake provider; CI never calls a live API and
never records credentials. Text is the only supported input in v1.0. Anthropic-native passthrough, OAuth login,
and additional vendors beyond these three are out of scope. See
[`specs/provider/0001-openai-compatible.md`](../../specs/provider/0001-openai-compatible.md)
for the normative wire contract.
