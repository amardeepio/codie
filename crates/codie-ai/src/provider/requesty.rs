//! Requesty provider declaration.

use super::common;
use super::spec::{CatalogKind, ProbeKind, ProviderSpec};
use crate::model::{Compat, HeaderMap, Model, ModelCost, api_id, provider_id};

/// The provider declaration.
pub const SPEC: ProviderSpec = ProviderSpec {
    id: "requesty",
    name: "Requesty",
    env_var: "REQUESTY_API_KEY",
    requires_key: true,
    base_url: "https://router.requesty.ai/v1",
    api: crate::OPENAI_COMPLETIONS,
    default_model: "devstral-latest",
    compat: Compat::openai_compatible,
    headers: common::empty_headers,
    models,
    catalog: CatalogKind::OpenAiModels,
    probe: ProbeKind::Models,
};

fn models() -> Vec<Model> {
    vec![
        Model {
            id: "devstral-latest".to_owned(),
            name: "devstral-latest".to_owned(),
            provider: provider_id("requesty"),
            api: api_id(),
            base_url: SPEC.base_url.to_owned(),
            context_window: 256000,
            max_output_tokens: 256000,
            reasoning: true,
            reasoning_mandatory: false,
            thinking_levels: common::thinking_levels(
                true,
                &["none", "low", "medium", "high", "max"],
            ),
            cost: ModelCost {
                input: 0.44,
                output: 2.2,
                cache_read: 0.44,
                cache_write: 0.0,
            },
            compat: Compat::openai_compatible(),
            headers: HeaderMap::new(),
        },
        Model {
            id: "gpt-5.3-codex".to_owned(),
            name: "GPT-5.3 Codex".to_owned(),
            provider: provider_id("requesty"),
            api: api_id(),
            base_url: SPEC.base_url.to_owned(),
            context_window: 400000,
            max_output_tokens: 128000,
            reasoning: true,
            reasoning_mandatory: false,
            thinking_levels: common::thinking_levels(
                true,
                &["none", "low", "medium", "high", "max"],
            ),
            cost: ModelCost {
                input: 1.75,
                output: 14.0,
                cache_read: 0.175,
                cache_write: 0.0,
            },
            compat: Compat::openai_compatible(),
            headers: HeaderMap::new(),
        },
        Model {
            id: "kimi-k2.7-code".to_owned(),
            name: "Kimi K2.7 Code".to_owned(),
            provider: provider_id("requesty"),
            api: api_id(),
            base_url: SPEC.base_url.to_owned(),
            context_window: 262144,
            max_output_tokens: 262144,
            reasoning: true,
            reasoning_mandatory: false,
            thinking_levels: common::thinking_levels(
                true,
                &["none", "low", "medium", "high", "max"],
            ),
            cost: ModelCost {
                input: 0.95,
                output: 4.0,
                cache_read: 0.19,
                cache_write: 0.0,
            },
            compat: Compat::openai_compatible(),
            headers: HeaderMap::new(),
        },
        Model {
            id: "gemini-4-hydrogen".to_owned(),
            name: "Gemini 4 Hydrogen".to_owned(),
            provider: provider_id("requesty"),
            api: api_id(),
            base_url: SPEC.base_url.to_owned(),
            context_window: 1048576,
            max_output_tokens: 65536,
            reasoning: true,
            reasoning_mandatory: false,
            thinking_levels: common::thinking_levels(
                true,
                &["none", "low", "medium", "high", "max"],
            ),
            cost: ModelCost {
                input: 1.5,
                output: 7.5,
                cache_read: 0.075,
                cache_write: 0.0,
            },
            compat: Compat::openai_compatible(),
            headers: HeaderMap::new(),
        },
    ]
}
