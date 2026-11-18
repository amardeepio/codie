//! SambaNova provider declaration.

use super::common;
use super::spec::{CatalogKind, ProbeKind, ProviderSpec};
use crate::model::{Compat, HeaderMap, Model, ModelCost, api_id, provider_id};

/// The provider declaration.
pub const SPEC: ProviderSpec = ProviderSpec {
    id: "sambanova",
    name: "SambaNova",
    env_var: "SAMBANOVA_API_KEY",
    requires_key: true,
    base_url: "https://api.sambanova.ai/v1",
    api: crate::OPENAI_COMPLETIONS,
    default_model: "GLM-5.3",
    compat: Compat::openai_compatible,
    headers: common::empty_headers,
    models,
    catalog: CatalogKind::OpenAiModels,
    probe: ProbeKind::Models,
};

fn models() -> Vec<Model> {
    vec![
        Model {
            id: "GLM-5.3".to_owned(),
            name: "GLM-5.3".to_owned(),
            provider: provider_id("sambanova"),
            api: api_id(),
            base_url: SPEC.base_url.to_owned(),
            context_window: 1048576,
            max_output_tokens: 1048576,
            reasoning: true,
            reasoning_mandatory: false,
            thinking_levels: common::thinking_levels(true, &["low", "high", "max"]),
            cost: ModelCost {
                input: 2.8,
                output: 8.8,
                cache_read: 0.14,
                cache_write: 0.0,
            },
            compat: Compat::openai_compatible(),
            headers: HeaderMap::new(),
        },
        Model {
            id: "MiniMax-M3.1-Flash".to_owned(),
            name: "MiniMax-M3.1-Flash".to_owned(),
            provider: provider_id("sambanova"),
            api: api_id(),
            base_url: SPEC.base_url.to_owned(),
            context_window: 1048576,
            max_output_tokens: 1048576,
            reasoning: true,
            reasoning_mandatory: false,
            thinking_levels: common::thinking_levels(
                true,
                &["low", "medium", "high", "xhigh", "max"],
            ),
            cost: ModelCost {
                input: 0.3,
                output: 1.2,
                cache_read: 0.03,
                cache_write: 0.0,
            },
            compat: Compat::openai_compatible(),
            headers: HeaderMap::new(),
        },
        Model {
            id: "gemma-4-31B-it".to_owned(),
            name: "Gemma 4 31B IT".to_owned(),
            provider: provider_id("sambanova"),
            api: api_id(),
            base_url: SPEC.base_url.to_owned(),
            context_window: 131072,
            max_output_tokens: 131072,
            reasoning: true,
            reasoning_mandatory: false,
            thinking_levels: common::thinking_levels(true, &[]),
            cost: ModelCost {
                input: 0.38,
                output: 1.15,
                cache_read: 0.0,
                cache_write: 0.0,
            },
            compat: Compat::openai_compatible(),
            headers: HeaderMap::new(),
        },
        Model {
            id: "gpt-oss-120b".to_owned(),
            name: "GPT OSS 120B".to_owned(),
            provider: provider_id("sambanova"),
            api: api_id(),
            base_url: SPEC.base_url.to_owned(),
            context_window: 131072,
            max_output_tokens: 131072,
            reasoning: true,
            reasoning_mandatory: false,
            thinking_levels: common::thinking_levels(true, &["low", "medium", "high"]),
            cost: ModelCost {
                input: 0.22,
                output: 0.59,
                cache_read: 0.0,
                cache_write: 0.0,
            },
            compat: Compat::openai_compatible(),
            headers: HeaderMap::new(),
        },
    ]
}
