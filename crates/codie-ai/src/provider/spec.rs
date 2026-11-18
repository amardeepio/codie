//! Provider metadata shared by the CLI, TUI, auth, and model picker.

use crate::model::{Compat, HeaderMap, Model};

/// How a provider exposes its model catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatalogKind {
    /// Models ship in the binary and change only with a codie release.
    Bundled,
    /// Models come from OpenRouter's live `/models` endpoint.
    OpenRouter,
    /// Models come from a standard OpenAI-compatible `/models` endpoint.
    OpenAiModels,
}

/// The read-only endpoint used to verify an API key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeKind {
    /// `GET {base}/models` with a bearer token.
    Models,
    /// `GET {base}/key` with a bearer token.
    OpenRouterKey,
    /// Gemini's native model list with the key in a query parameter.
    GeminiNative,
}

/// Everything codie needs to know about one provider.
///
/// A provider is a runtime unit: identity, credential variable, endpoint,
/// default model, compatibility quirks, headers, catalog, and key probe. The
/// wire protocol stays in [`crate::openai`]; the agent loop never sees any of
/// this.
#[derive(Debug, Clone, Copy)]
pub struct ProviderSpec {
    /// Canonical lowercase identifier used on the wire and in `auth.json`.
    pub id: &'static str,
    /// Human-readable name.
    pub name: &'static str,
    /// Environment variable that supplies the API key.
    pub env_var: &'static str,
    /// False for keyless local endpoints such as Ollama and LM Studio.
    pub requires_key: bool,
    /// Default API base URL.
    pub base_url: &'static str,
    /// Wire protocol identifier, currently `openai-completions` for every
    /// bundled provider.
    pub api: &'static str,
    /// Model selected after a provider switch when nothing else is configured.
    pub default_model: &'static str,
    /// Compatibility defaults for models not present in the catalog.
    pub compat: fn() -> Compat,
    /// Extra headers sent with every request.
    pub headers: fn() -> HeaderMap,
    /// Bundled models.
    pub models: fn() -> Vec<Model>,
    /// How the model picker refreshes this provider's catalog.
    pub catalog: CatalogKind,
    /// How the connect flow checks a key.
    pub probe: ProbeKind,
}
