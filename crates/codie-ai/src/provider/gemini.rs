//! Google Gemini provider declaration.

use std::collections::BTreeMap;

use codie_core::ThinkingLevel;

use super::spec::{CatalogKind, ProbeKind, ProviderSpec};
use crate::model::{Compat, HeaderMap, Model, ModelCost, PROVIDER_GEMINI, api_id, provider_id};

/// The Google Gemini provider, served through its OpenAI-compatible endpoint.
pub const SPEC: ProviderSpec = ProviderSpec {
    id: PROVIDER_GEMINI,
    name: "Gemini",
    env_var: "GEMINI_API_KEY",
    requires_key: true,
    base_url: "https://generativelanguage.googleapis.com/v1beta/openai",
    api: crate::OPENAI_COMPLETIONS,
    default_model: "gemini-4-hydrogen",
    compat: Compat::gemini,
    headers: empty_headers,
    models,
    catalog: CatalogKind::Bundled,
    probe: ProbeKind::GeminiNative,
};

fn empty_headers() -> HeaderMap {
    HeaderMap::new()
}

/// Gemini 4 reasoning levels. `minimal` is mapped to `low` by the provider.
fn gemini4_thinking_levels() -> BTreeMap<ThinkingLevel, Option<String>> {
    BTreeMap::from([
        (ThinkingLevel::Minimal, Some("low".to_owned())),
        (ThinkingLevel::Low, Some("low".to_owned())),
        (ThinkingLevel::Medium, Some("medium".to_owned())),
        (ThinkingLevel::High, Some("high".to_owned())),
    ])
}

/// Gemini 4 Hydrogen additionally accepts `xhigh`.
fn gemini4_hydrogen_thinking_levels() -> BTreeMap<ThinkingLevel, Option<String>> {
    let mut levels = gemini4_thinking_levels();
    levels.insert(ThinkingLevel::XHigh, Some("xhigh".to_owned()));
    levels
}

/// Gemini 3 reasoning levels. `minimal` is rejected by these models.
fn gemini3_thinking_levels() -> BTreeMap<ThinkingLevel, Option<String>> {
    BTreeMap::from([
        (ThinkingLevel::Low, Some("low".to_owned())),
        (ThinkingLevel::Medium, Some("medium".to_owned())),
        (ThinkingLevel::High, Some("high".to_owned())),
    ])
}

fn models() -> Vec<Model> {
    // Gemini pricing is USD per million tokens as published on 2026-11-15.
    // Gemini 4 prices double on 2026-12-01; update the catalog when the new
    // prices take effect. Cache writes are billed as storage per hour, so the
    // per-token write price stays zero here.
    vec![
        Model {
            id: "gemini-4-hydrogen".to_owned(),
            name: "Gemini 4 Hydrogen".to_owned(),
            provider: provider_id(PROVIDER_GEMINI),
            api: api_id(),
            base_url: SPEC.base_url.to_owned(),
            context_window: 1_048_576,
            max_output_tokens: 65_536,
            reasoning: true,
            reasoning_mandatory: true,
            thinking_levels: gemini4_hydrogen_thinking_levels(),
            cost: ModelCost {
                input: 1.50,
                output: 7.50,
                cache_read: 0.075,
                cache_write: 0.0,
            },
            compat: Compat::gemini(),
            headers: HeaderMap::new(),
        },
        Model {
            id: "gemini-4-argon".to_owned(),
            name: "Gemini 4 Argon".to_owned(),
            provider: provider_id(PROVIDER_GEMINI),
            api: api_id(),
            base_url: SPEC.base_url.to_owned(),
            context_window: 1_048_576,
            max_output_tokens: 65_536,
            reasoning: true,
            reasoning_mandatory: true,
            thinking_levels: gemini4_thinking_levels(),
            cost: ModelCost {
                input: 2.00,
                output: 10.00,
                cache_read: 0.10,
                cache_write: 0.0,
            },
            compat: Compat::gemini(),
            headers: HeaderMap::new(),
        },
        Model {
            id: "gemini-4-carbon".to_owned(),
            name: "Gemini 4 Carbon".to_owned(),
            provider: provider_id(PROVIDER_GEMINI),
            api: api_id(),
            base_url: SPEC.base_url.to_owned(),
            context_window: 1_048_576,
            max_output_tokens: 65_536,
            reasoning: true,
            reasoning_mandatory: true,
            thinking_levels: gemini4_thinking_levels(),
            cost: ModelCost {
                input: 2.00,
                output: 10.00,
                cache_read: 0.10,
                cache_write: 0.0,
            },
            compat: Compat::gemini(),
            headers: HeaderMap::new(),
        },
        Model {
            id: "gemini-3.8-flash".to_owned(),
            name: "Gemini 3.8 Flash".to_owned(),
            provider: provider_id(PROVIDER_GEMINI),
            api: api_id(),
            base_url: SPEC.base_url.to_owned(),
            context_window: 1_048_576,
            max_output_tokens: 65_536,
            reasoning: true,
            reasoning_mandatory: true,
            thinking_levels: gemini3_thinking_levels(),
            cost: ModelCost {
                input: 0.75,
                output: 3.75,
                cache_read: 0.075,
                cache_write: 0.0,
            },
            compat: Compat::gemini(),
            headers: HeaderMap::new(),
        },
        Model {
            id: "gemini-3.5-flash-lite".to_owned(),
            name: "Gemini 3.5 Flash-Lite".to_owned(),
            provider: provider_id(PROVIDER_GEMINI),
            api: api_id(),
            base_url: SPEC.base_url.to_owned(),
            context_window: 1_048_576,
            max_output_tokens: 65_536,
            reasoning: true,
            reasoning_mandatory: true,
            thinking_levels: gemini3_thinking_levels(),
            cost: ModelCost {
                input: 0.30,
                output: 2.50,
                cache_read: 0.03,
                cache_write: 0.0,
            },
            compat: Compat::gemini(),
            headers: HeaderMap::new(),
        },
    ]
}
