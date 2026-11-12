//! Provider abstractions and wire adapters for codie.
//!
//! Phase 1 provides one OpenAI-compatible adapter that serves DeepSeek and
//! OpenRouter, plus the model catalog, SSE framing, and credential storage the
//! adapter needs. The adapter implements [`codie_core::agent::ChatClient`] and
//! emits [`codie_core::event::AssistantDelta`] events; it is the only place in
//! codie that knows a provider's wire format.
//!
//! - [`openai::OpenAiProvider`] — HTTP, retries, SSE, tool calls, usage.
//! - [`model::Model`] — model identity, pricing, and compatibility matrix.
//! - [`sse::SseParser`] — byte-oriented Server-Sent Events framing.
//! - [`auth::AuthStore`] — atomic `auth.json` credentials and key resolution.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

use std::borrow::Cow;

pub mod auth;
pub mod catalog;
pub mod model;
pub mod openai;
pub mod record;
pub mod redact;
pub mod sse;
pub mod update;

pub use auth::{AuthError, AuthStore, resolve_key};
pub use catalog::{
    Architecture, CATALOG_FRESHNESS, CATALOG_TIMEOUT, CatalogCache, CatalogModel,
    OPENROUTER_MODELS_URL, OpenRouterCatalog, RefreshOutcome, is_selectable, selectable_models,
};
pub use model::{
    Compat, DEFAULT_HTTP_REFERER, DEFAULT_X_TITLE, HeaderMap, MaxTokensField, Model, ModelCost,
    PROVIDER_DEEPSEEK, PROVIDER_OPENROUTER, ThinkingFormat, builtin_models, fallback_model,
    find_model, provider_base_url, provider_compat,
};
pub use openai::{
    MAX_ATTEMPTS, MAX_ERROR_BODY_CHARS, MAX_RETRY_DELAY, OpenAiProvider, backoff_delay,
    build_request_body, estimate_cost, map_stop_reason, normalize_transport_error, normalize_usage,
};
pub use redact::{is_secret_header, redact_header, redact_text, redacted_headers};
pub use sse::{SseEvent, SseParser};
pub use update::{
    CHECK_TIMEOUT, RELEASES_URL, TAGS_URL, UpdateStatus, check, compare_versions,
    latest_from_payload,
};

/// A provider identifier such as `deepseek` or `openrouter`.
pub type ProviderId = Cow<'static, str>;

/// A wire-protocol identifier such as `openai-completions`.
pub type ApiId = Cow<'static, str>;

/// The OpenAI-compatible chat-completions API used by every v1.0 provider.
pub const OPENAI_COMPLETIONS: &str = "openai-completions";

/// Returns the wire API used by every provider supported in v1.0.
#[must_use]
pub const fn default_api() -> &'static str {
    OPENAI_COMPLETIONS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v1_uses_openai_completions() {
        assert_eq!(default_api(), OPENAI_COMPLETIONS);
        assert_eq!(default_api(), "openai-completions");
    }
}
