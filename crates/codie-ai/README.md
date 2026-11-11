# codie-ai

Provider abstractions for codie. Phase 1 ships the OpenAI-compatible wire
adapter used by DeepSeek and OpenRouter, the built-in model catalog, SSE
framing, and `auth.json` credential storage.

- `openai::OpenAiProvider` implements `codie_core::agent::ChatClient` and
  streams `AssistantDelta` events.
- `model::Model` carries identity, pricing, and the per-model compatibility
  matrix that gates request fields.
- `sse::SseParser` reassembles byte-oriented Server-Sent Events across
  arbitrary chunk boundaries.
- `auth::AuthStore` reads and atomically writes credentials and resolves keys
  in explicit > stored > environment order.

- Plan: [../../PLAN.MD](../../PLAN.MD)
- Specs: [../../specs/](../../specs/)

See `PLAN.MD` §6.2 and `specs/provider/0001-openai-compatible.md` for the
normative provider contract.
