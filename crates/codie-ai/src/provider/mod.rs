//! Static provider registry.
//!
//! This module is the single place that knows which providers exist. Adding a
//! provider means adding a [`ProviderSpec`] module here and listing it in
//! [`PROVIDERS`]; the CLI, TUI, auth commands, model picker, and probe code all
//! discover it through this registry.
//!
//! A provider is not a protocol. Every bundled provider currently speaks the
//! same OpenAI-compatible wire format in [`crate::openai`]; the registry only
//! carries identity, endpoint, credentials, compatibility, headers, models,
//! and catalog/probe behavior.

use crate::model::Model;
use crate::openai::OpenAiProvider;

pub mod alibaba;
pub mod baseten;
pub mod cerebras;
pub mod common;
pub mod deepinfra;
pub mod deepseek;
pub mod fireworks_ai;
pub mod gemini;
pub mod groq;
pub mod lmstudio;
pub mod mistral;
pub mod moonshotai;
pub mod nebius;
pub mod novita_ai;
pub mod nvidia;
pub mod ollama;
pub mod openai;
pub mod opencode;
pub mod opencode_go;
pub mod openrouter;
pub mod requesty;
pub mod sambanova;
pub mod spec;
pub mod togetherai;
pub mod vllm;
pub mod xai;
pub mod zai;

pub use spec::{CatalogKind, ProbeKind, ProviderSpec};

/// Every provider bundled with the binary, in picker order.
pub static PROVIDERS: &[ProviderSpec] = &[
    deepseek::SPEC,
    openrouter::SPEC,
    gemini::SPEC,
    openai::SPEC,
    groq::SPEC,
    cerebras::SPEC,
    mistral::SPEC,
    xai::SPEC,
    togetherai::SPEC,
    deepinfra::SPEC,
    fireworks_ai::SPEC,
    moonshotai::SPEC,
    zai::SPEC,
    nvidia::SPEC,
    nebius::SPEC,
    sambanova::SPEC,
    novita_ai::SPEC,
    alibaba::SPEC,
    baseten::SPEC,
    requesty::SPEC,
    opencode::SPEC,
    opencode_go::SPEC,
    lmstudio::SPEC,
    ollama::SPEC,
    vllm::SPEC,
];

/// Returns every bundled provider in picker order.
#[must_use]
pub fn all() -> &'static [ProviderSpec] {
    PROVIDERS
}

/// Returns the provider with `id`, matching ASCII case-insensitively.
#[must_use]
pub fn find(id: &str) -> Option<&'static ProviderSpec> {
    PROVIDERS
        .iter()
        .find(|provider| provider.id.eq_ignore_ascii_case(id))
}

/// Returns true when the provider requires an API key.
///
/// Unknown providers are treated as requiring one so a typo never silently
/// starts a keyless session.
#[must_use]
pub fn requires_key(id: &str) -> bool {
    find(id).is_none_or(|provider| provider.requires_key)
}

/// Returns every canonical provider identifier.
#[must_use]
pub fn ids() -> Vec<&'static str> {
    PROVIDERS.iter().map(|provider| provider.id).collect()
}

/// Returns the bundled models for one provider.
#[must_use]
pub fn models(id: &str) -> Vec<Model> {
    find(id).map_or_else(Vec::new, |provider| (provider.models)())
}

/// Returns every bundled model across every provider.
#[must_use]
pub fn all_models() -> Vec<Model> {
    PROVIDERS
        .iter()
        .flat_map(|provider| (provider.models)())
        .collect()
}

/// Builds the OpenAI-compatible client for a registered provider.
///
/// Returns `None` for an unknown provider instead of panicking. Callers that
/// have already validated an identifier can treat this as an invariant.
#[must_use]
pub fn build_client(id: &str, api_key: impl Into<String>) -> Option<OpenAiProvider> {
    let provider = find(id)?;
    if provider.api != crate::OPENAI_COMPLETIONS {
        return None;
    }
    Some(OpenAiProvider::new(
        provider.id,
        provider.base_url,
        api_key,
        (provider.models)(),
        (provider.compat)(),
        (provider.headers)(),
    ))
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::model::SUPPORTED_PROVIDERS;

    #[test]
    fn registry_matches_the_public_provider_list() {
        assert_eq!(ids(), SUPPORTED_PROVIDERS);
    }

    #[test]
    fn every_provider_is_complete_and_unique() {
        let mut ids = HashSet::new();

        for provider in all() {
            assert!(ids.insert(provider.id), "duplicate id {}", provider.id);
            assert!(!provider.name.is_empty(), "{} has no name", provider.id);
            assert!(
                provider.base_url.starts_with("https://")
                    || (!provider.requires_key && provider.base_url.starts_with("http://")),
                "{} has an invalid base URL",
                provider.id
            );
            assert_eq!(
                provider.api,
                crate::OPENAI_COMPLETIONS,
                "{} uses an unexpected wire API",
                provider.id
            );
            assert!(
                provider.env_var.ends_with("_API_KEY"),
                "{} has an unexpected env var {}",
                provider.id,
                provider.env_var
            );

            let models = (provider.models)();
            if provider.requires_key {
                assert!(!models.is_empty(), "{} has no curated models", provider.id);
            }
            if !models.is_empty() {
                assert!(
                    models
                        .iter()
                        .any(|model| model.id == provider.default_model),
                    "{} default model {} is missing",
                    provider.id,
                    provider.default_model
                );
            }
            for model in models {
                assert_eq!(
                    model.provider.as_ref(),
                    provider.id,
                    "{} model {} belongs to {}",
                    provider.id,
                    model.id,
                    model.provider
                );
                assert_eq!(
                    model.base_url, provider.base_url,
                    "{} model {} has the wrong base URL",
                    provider.id, model.id
                );
            }
        }
    }

    #[test]
    fn lookup_is_case_insensitive_and_client_construction_uses_the_spec() {
        let provider = find("DeepSeek").expect("case-insensitive lookup");
        assert_eq!(provider.id, "deepseek");

        let client = build_client("openrouter", "sk-test").expect("registered provider");
        assert_eq!(client.provider_id(), "openrouter");
        assert_eq!(client.base_url(), "https://openrouter.ai/api/v1");
        assert_eq!(client.models().len(), 2);
        assert!(build_client("unknown", "sk-test").is_none());
    }

    #[test]
    fn catalog_and_probe_kinds_are_declared_per_provider() {
        assert_eq!(find("openrouter").unwrap().catalog, CatalogKind::OpenRouter);
        assert_eq!(find("openrouter").unwrap().probe, ProbeKind::OpenRouterKey);
        assert_eq!(find("gemini").unwrap().probe, ProbeKind::GeminiNative);
        assert_eq!(find("deepseek").unwrap().probe, ProbeKind::Models);
    }
}
