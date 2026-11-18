//! LM Studio provider declaration.

use super::common;
use super::spec::{CatalogKind, ProbeKind, ProviderSpec};
use crate::model::{Compat, HeaderMap, Model, ModelCost, api_id, provider_id};

/// The provider declaration.
pub const SPEC: ProviderSpec = ProviderSpec {
    id: "lmstudio",
    name: "LM Studio",
    env_var: "LMSTUDIO_API_KEY",
    requires_key: false,
    base_url: "http://127.0.0.1:1234/v1",
    api: crate::OPENAI_COMPLETIONS,
    default_model: "qwen/qwen3-coder-30b",
    compat: Compat::openai_compatible,
    headers: common::empty_headers,
    models,
    catalog: CatalogKind::OpenAiModels,
    probe: ProbeKind::Models,
};

fn models() -> Vec<Model> {
    vec![
        Model {
            id: "qwen/qwen3-coder-30b".to_owned(),
            name: "Qwen3 Coder 30B".to_owned(),
            provider: provider_id("lmstudio"),
            api: api_id(),
            base_url: SPEC.base_url.to_owned(),
            context_window: 262144,
            max_output_tokens: 65536,
            reasoning: false,
            reasoning_mandatory: false,
            thinking_levels: common::thinking_levels(false, &[]),
            cost: ModelCost {
                input: 0.0,
                output: 0.0,
                cache_read: 0.0,
                cache_write: 0.0,
            },
            compat: Compat::openai_compatible(),
            headers: HeaderMap::new(),
        },
        Model {
            id: "openai/gpt-oss-20b".to_owned(),
            name: "GPT OSS 20B".to_owned(),
            provider: provider_id("lmstudio"),
            api: api_id(),
            base_url: SPEC.base_url.to_owned(),
            context_window: 131072,
            max_output_tokens: 131072,
            reasoning: true,
            reasoning_mandatory: false,
            thinking_levels: common::thinking_levels(true, &["low", "medium", "high"]),
            cost: ModelCost {
                input: 0.0,
                output: 0.0,
                cache_read: 0.0,
                cache_write: 0.0,
            },
            compat: Compat::openai_compatible(),
            headers: HeaderMap::new(),
        },
        Model {
            id: "qwen/qwen3-30b-a3b-2507".to_owned(),
            name: "Qwen3 30B A3B 2507".to_owned(),
            provider: provider_id("lmstudio"),
            api: api_id(),
            base_url: SPEC.base_url.to_owned(),
            context_window: 262144,
            max_output_tokens: 16384,
            reasoning: false,
            reasoning_mandatory: false,
            thinking_levels: common::thinking_levels(false, &[]),
            cost: ModelCost {
                input: 0.0,
                output: 0.0,
                cache_read: 0.0,
                cache_write: 0.0,
            },
            compat: Compat::openai_compatible(),
            headers: HeaderMap::new(),
        },
    ]
}
