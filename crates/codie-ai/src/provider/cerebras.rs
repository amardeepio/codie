//! Cerebras provider declaration.

use super::common;
use super::spec::{CatalogKind, ProbeKind, ProviderSpec};
use crate::model::{Compat, HeaderMap, Model, ModelCost, api_id, provider_id};

/// The provider declaration.
pub const SPEC: ProviderSpec = ProviderSpec {
    id: "cerebras",
    name: "Cerebras",
    env_var: "CEREBRAS_API_KEY",
    requires_key: true,
    base_url: "https://api.cerebras.ai/v1",
    api: crate::OPENAI_COMPLETIONS,
    default_model: "qwen-3.8-27b",
    compat: Compat::openai_compatible,
    headers: common::empty_headers,
    models,
    catalog: CatalogKind::OpenAiModels,
    probe: ProbeKind::Models,
};

fn models() -> Vec<Model> {
    vec![
        Model {
            id: "qwen-3.8-27b".to_owned(),
            name: "Qwen3.8 27B".to_owned(),
            provider: provider_id("cerebras"),
            api: api_id(),
            base_url: SPEC.base_url.to_owned(),
            context_window: 131072,
            max_output_tokens: 40960,
            reasoning: true,
            reasoning_mandatory: false,
            thinking_levels: common::thinking_levels(true, &["none", "low", "medium", "high"]),
            cost: ModelCost {
                input: 0.99,
                output: 1.49,
                cache_read: 0.99,
                cache_write: 0.0,
            },
            compat: Compat::openai_compatible(),
            headers: HeaderMap::new(),
        },
        Model {
            id: "gpt-oss-120b".to_owned(),
            name: "GPT OSS 120B".to_owned(),
            provider: provider_id("cerebras"),
            api: api_id(),
            base_url: SPEC.base_url.to_owned(),
            context_window: 131072,
            max_output_tokens: 40960,
            reasoning: true,
            reasoning_mandatory: false,
            thinking_levels: common::thinking_levels(true, &["low", "medium", "high"]),
            cost: ModelCost {
                input: 0.35,
                output: 0.75,
                cache_read: 0.35,
                cache_write: 0.0,
            },
            compat: Compat::openai_compatible(),
            headers: HeaderMap::new(),
        },
    ]
}
