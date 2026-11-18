# Providers

codie ships one OpenAI-compatible adapter and a static registry of remote and local providers. Remote providers
use API-key authentication and stream responses as Server-Sent Events. Ollama, LM Studio, and vLLM are keyless local
endpoints.

## Support matrix

| Provider | Provider ID | Default base URL | Credential |
|---|---|---|---|
| DeepSeek | `deepseek` | `https://api.deepseek.com` | `DEEPSEEK_API_KEY` |
| OpenRouter | `openrouter` | `https://openrouter.ai/api/v1` | `OPENROUTER_API_KEY` |
| Google Gemini | `gemini` | `https://generativelanguage.googleapis.com/v1beta/openai` | `GEMINI_API_KEY` |
| OpenAI | `openai` | `https://api.openai.com/v1` | `OPENAI_API_KEY` |
| Groq | `groq` | `https://api.groq.com/openai/v1` | `GROQ_API_KEY` |
| Cerebras | `cerebras` | `https://api.cerebras.ai/v1` | `CEREBRAS_API_KEY` |
| Mistral | `mistral` | `https://api.mistral.ai/v1` | `MISTRAL_API_KEY` |
| xAI | `xai` | `https://api.x.ai/v1` | `XAI_API_KEY` |
| Together AI | `togetherai` | `https://api.together.xyz/v1` | `TOGETHER_API_KEY` |
| DeepInfra | `deepinfra` | `https://api.deepinfra.com/v1/openai` | `DEEPINFRA_API_KEY` |
| Fireworks AI | `fireworks-ai` | `https://api.fireworks.ai/inference/v1` | `FIREWORKS_API_KEY` |
| Moonshot AI | `moonshotai` | `https://api.moonshot.ai/v1` | `MOONSHOT_API_KEY` |
| Z.AI | `zai` | `https://api.z.ai/api/paas/v4` | `ZHIPU_API_KEY` |
| NVIDIA NIM | `nvidia` | `https://integrate.api.nvidia.com/v1` | `NVIDIA_API_KEY` |
| Nebius Token Factory | `nebius` | `https://api.tokenfactory.nebius.com/v1` | `NEBIUS_API_KEY` |
| SambaNova | `sambanova` | `https://api.sambanova.ai/v1` | `SAMBANOVA_API_KEY` |
| Novita AI | `novita-ai` | `https://api.novita.ai/openai` | `NOVITA_API_KEY` |
| Alibaba DashScope | `alibaba` | `https://dashscope-intl.aliyuncs.com/compatible-mode/v1` | `DASHSCOPE_API_KEY` |
| Baseten | `baseten` | `https://inference.baseten.co/v1` | `BASETEN_API_KEY` |
| Requesty | `requesty` | `https://router.requesty.ai/v1` | `REQUESTY_API_KEY` |
| OpenCode | `opencode` | `https://opencode.ai/inference/openai/v1` | `OPENCODE_API_KEY` |
| OpenCode Go | `opencode-go` | `https://opencode.ai/inference/go/openai/v1` | `OPENCODE_API_KEY` |
| Ollama | `ollama` | `http://127.0.0.1:11434/v1` | none |
| LM Studio | `lmstudio` | `http://127.0.0.1:1234/v1` | none |
| vLLM | `vllm` | `http://127.0.0.1:8000/v1` | none |

Every provider speaks the `openai-completions` API. A provider is a runtime unit (id, base URL, auth, catalog,
filters); an API is a wire protocol; a model carries the API, provider, limits, cost, and compatibility flags.
Keeping these separate is what lets one adapter serve every provider without provider-specific branches in the
agent loop.

## Adding a provider

Provider knowledge lives in one static registry: `crates/codie-ai/src/provider/`. Each provider is a small module
that declares its identity, environment variable, base URL, default model, compatibility set, headers, bundled
models, catalog kind, and key-probe kind:

```rust
pub const SPEC: ProviderSpec = ProviderSpec {
    id: "groq",
    name: "Groq",
    env_var: "GROQ_API_KEY",
    base_url: "https://api.groq.com/openai/v1",
    api: OPENAI_COMPLETIONS,
    default_model: "llama-3.3-70b-versatile",
    compat: Compat::openai,
    headers: empty_headers,
    models,
    catalog: CatalogKind::Bundled,
    probe: ProbeKind::Models,
};
```

Set `catalog` to `CatalogKind::OpenAiModels` when the provider exposes a standard `GET /models` endpoint; the
generic catalog then refreshes and caches it automatically. Keep `CatalogKind::Bundled` when the provider needs
curated metadata or does not publish a usable model list.

Add the module to `provider/mod.rs` and list the spec in `PROVIDERS`. The CLI provider picker, `/provider`,
`/connect`, `codie auth`, `codie models`, model lookup, and key probes all read from the registry, so no other
provider branch needs to change. The registry conformance test requires unique IDs and environment variables, a
valid HTTPS base URL, a default model that exists, and models whose provider and base URL match the spec.

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
written to disk.

`codie auth login` checks the key with the provider before storing it, so a rejected key never replaces a working
one. Use `--no-verify` for a proxy or an offline endpoint, or `codie auth verify <provider>` to check the stored
or environment key later. The check is a read-only request: OpenRouter uses `GET /key` (its `/models` endpoint is
public), Gemini uses its native model list, and the other providers use `GET /models`. A timeout is six seconds,
and rate limiting counts as a usable key with a warning. `auth status` stays offline unless `--verify` is passed;
it exits `2` when no provider is configured.

In the full-screen interface, `/connect` or `/login` opens the provider picker. Selecting a provider without a key
opens a masked key field; Enter checks the key, Ctrl+V pastes, Ctrl+R reveals, and Ctrl+S saves without checking
from the error screen. A valid key is stored and the provider switches immediately. `/models` is an alias for
`/model`. `/auth` shows credential status, `/logout` removes a stored key, and `/provider` marks connected
providers and offers a connect row.
On a first run with no credentials, the TUI opens directly on the connect overlay instead of exiting.

Ollama, LM Studio, and vLLM are keyless: they show `no key required`, switch without a prompt, and are not counted
as configured credentials by `codie auth status`. If a local deployment uses a reverse proxy with a bearer token,
pass `--api-key` for the run or store an optional key with `codie auth login <provider> --api-key`.

A provider `401`/`403` during a run opens the connect overlay for the active provider. Provider switching is
refused while a run is streaming; finish or cancel the run first. The last provider and model chosen in the TUI are
saved in `state.json` and reused when neither `config.toml` nor the environment sets a default.

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

Providers declared with `CatalogKind::OpenAiModels` use the generic catalog instead. It calls
`GET {base_url}/models` with the provider key, caches the result as `<provider>-models.json`, and accepts the
standard OpenAI response shape plus optional `context_length`, `max_model_len`, `max_completion_tokens`,
`supported_parameters`, and `reasoning` metadata. Embedding, reranking, moderation, audio, speech, image, video,
and guard/safety models are filtered out. Curated entries are merged first and win on duplicate IDs. The refresh
is best-effort, has a four-second timeout, preserves the previous cache on failure, and runs in the background when
a key is available.

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
