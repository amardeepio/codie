//! Groq provider declaration.

use super::common;
use super::spec::{CatalogKind, ProbeKind, ProviderSpec};
use crate::model::{Compat, HeaderMap, Model, ModelCost, api_id, provider_id};

/// The provider declaration.
pub const SPEC: ProviderSpec = ProviderSpec {
    id: "groq",
    name: "Groq",
    env_var: "GROQ_API_KEY",
    requires_key: true,
    base_url: "https://api.groq.com/openai/v1",
    api: crate::OPENAI_COMPLETIONS,
    default_model: "qwen/qwen3.8-27b",
    compat: Compat::openai_compatible_max_completion,
    headers: common::empty_headers,
    models,
    catalog: CatalogKind::OpenAiModels,
    probe: ProbeKind::Models,
};

fn models() -> Vec<Model> {
    vec![
        Model {
            id: "qwen/qwen3.8-27b".to_owned(),
            name: "Qwen3.8 27B".to_owned(),
            provider: provider_id("groq"),
            api: api_id(),
            base_url: SPEC.base_url.to_owned(),
            context_window: 131072,
            max_output_tokens: 16384,
            reasoning: true,
            reasoning_mandatory: false,
            thinking_levels: common::thinking_levels(
                true,
                &["none", "default", "low", "medium", "high"],
            ),
            cost: ModelCost {
                input: 0.8,
                output: 4.0,
                cache_read: 0.0,
                cache_write: 0.0,
            },
            compat: Compat::openai_compatible_max_completion(),
            headers: HeaderMap::new(),
        },
        Model {
            id: "openai/gpt-oss-120b".to_owned(),
            name: "GPT OSS 120B".to_owned(),
            provider: provider_id("groq"),
            api: api_id(),
            base_url: SPEC.base_url.to_owned(),
            context_window: 131072,
            max_output_tokens: 65536,
            reasoning: true,
            reasoning_mandatory: false,
            thinking_levels: common::thinking_levels(true, &["low", "medium", "high"]),
            cost: ModelCost {
                input: 0.15,
                output: 0.6,
                cache_read: 0.075,
                cache_write: 0.0,
            },
            compat: Compat::openai_compatible_max_completion(),
            headers: HeaderMap::new(),
        },
        Model {
            id: "openai/gpt-oss-20b".to_owned(),
            name: "GPT OSS 20B".to_owned(),
            provider: provider_id("groq"),
            api: api_id(),
            base_url: SPEC.base_url.to_owned(),
            context_window: 131072,
            max_output_tokens: 65536,
            reasoning: true,
            reasoning_mandatory: false,
            thinking_levels: common::thinking_levels(true, &["low", "medium", "high"]),
            cost: ModelCost {
                input: 0.075,
                output: 0.3,
                cache_read: 0.0375,
                cache_write: 0.0,
            },
            compat: Compat::openai_compatible_max_completion(),
            headers: HeaderMap::new(),
        },
    ]
}
