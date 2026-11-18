//! Mistral provider declaration.

use super::common;
use super::spec::{CatalogKind, ProbeKind, ProviderSpec};
use crate::model::{Compat, HeaderMap, Model, ModelCost, api_id, provider_id};

/// The provider declaration.
pub const SPEC: ProviderSpec = ProviderSpec {
    id: "mistral",
    name: "Mistral",
    env_var: "MISTRAL_API_KEY",
    requires_key: true,
    base_url: "https://api.mistral.ai/v1",
    api: crate::OPENAI_COMPLETIONS,
    default_model: "codestral-latest",
    compat: Compat::openai_compatible,
    headers: common::empty_headers,
    models,
    catalog: CatalogKind::OpenAiModels,
    probe: ProbeKind::Models,
};

fn models() -> Vec<Model> {
    vec![
        Model {
            id: "codestral-latest".to_owned(),
            name: "Codestral (latest)".to_owned(),
            provider: provider_id("mistral"),
            api: api_id(),
            base_url: SPEC.base_url.to_owned(),
            context_window: 256000,
            max_output_tokens: 4096,
            reasoning: false,
            reasoning_mandatory: false,
            thinking_levels: common::thinking_levels(false, &[]),
            cost: ModelCost {
                input: 0.3,
                output: 0.9,
                cache_read: 0.03,
                cache_write: 0.0,
            },
            compat: Compat::openai_compatible(),
            headers: HeaderMap::new(),
        },
        Model {
            id: "codestral-2508".to_owned(),
            name: "Codestral 25.08".to_owned(),
            provider: provider_id("mistral"),
            api: api_id(),
            base_url: SPEC.base_url.to_owned(),
            context_window: 256000,
            max_output_tokens: 8192,
            reasoning: false,
            reasoning_mandatory: false,
            thinking_levels: common::thinking_levels(false, &[]),
            cost: ModelCost {
                input: 0.3,
                output: 0.9,
                cache_read: 0.03,
                cache_write: 0.0,
            },
            compat: Compat::openai_compatible(),
            headers: HeaderMap::new(),
        },
        Model {
            id: "deepseek-v4-1-flash-1017".to_owned(),
            name: "DeepSeek V4.1 Flash 1017".to_owned(),
            provider: provider_id("mistral"),
            api: api_id(),
            base_url: SPEC.base_url.to_owned(),
            context_window: 1048576,
            max_output_tokens: 128000,
            reasoning: true,
            reasoning_mandatory: false,
            thinking_levels: common::thinking_levels(true, &["none", "low", "high", "max"]),
            cost: ModelCost {
                input: 0.3,
                output: 1.2,
                cache_read: 0.007,
                cache_write: 0.0,
            },
            compat: Compat::openai_compatible(),
            headers: HeaderMap::new(),
        },
        Model {
            id: "mistral-large-4-0-8".to_owned(),
            name: "Mistral Large 4.0.8".to_owned(),
            provider: provider_id("mistral"),
            api: api_id(),
            base_url: SPEC.base_url.to_owned(),
            context_window: 1048576,
            max_output_tokens: 262144,
            reasoning: true,
            reasoning_mandatory: false,
            thinking_levels: common::thinking_levels(true, &["low", "medium", "high"]),
            cost: ModelCost {
                input: 0.7,
                output: 2.1,
                cache_read: 0.07,
                cache_write: 0.0,
            },
            compat: Compat::openai_compatible(),
            headers: HeaderMap::new(),
        },
    ]
}
