//! DeepInfra provider declaration.

use super::common;
use super::spec::{CatalogKind, ProbeKind, ProviderSpec};
use crate::model::{Compat, HeaderMap, Model, ModelCost, api_id, provider_id};

/// The provider declaration.
pub const SPEC: ProviderSpec = ProviderSpec {
    id: "deepinfra",
    name: "DeepInfra",
    env_var: "DEEPINFRA_API_KEY",
    requires_key: true,
    base_url: "https://api.deepinfra.com/v1/openai",
    api: crate::OPENAI_COMPLETIONS,
    default_model: "Qwen/Qwen3-Coder-480B-A35B-Instruct-Turbo",
    compat: Compat::openai_compatible,
    headers: common::empty_headers,
    models,
    catalog: CatalogKind::OpenAiModels,
    probe: ProbeKind::Models,
};

fn models() -> Vec<Model> {
    vec![
        Model {
            id: "Qwen/Qwen3-Coder-480B-A35B-Instruct-Turbo".to_owned(),
            name: "Qwen3 Coder 480B A35B Instruct Turbo".to_owned(),
            provider: provider_id("deepinfra"),
            api: api_id(),
            base_url: SPEC.base_url.to_owned(),
            context_window: 262144,
            max_output_tokens: 66536,
            reasoning: false,
            reasoning_mandatory: false,
            thinking_levels: common::thinking_levels(false, &[]),
            cost: ModelCost {
                input: 0.3,
                output: 1.0,
                cache_read: 0.1,
                cache_write: 0.0,
            },
            compat: Compat::openai_compatible(),
            headers: HeaderMap::new(),
        },
        Model {
            id: "ByteDance/Seed-2.0-code".to_owned(),
            name: "Seed 2.0 Code".to_owned(),
            provider: provider_id("deepinfra"),
            api: api_id(),
            base_url: SPEC.base_url.to_owned(),
            context_window: 256000,
            max_output_tokens: 131072,
            reasoning: true,
            reasoning_mandatory: false,
            thinking_levels: common::thinking_levels(true, &["none", "low", "medium", "high"]),
            cost: ModelCost {
                input: 0.5,
                output: 3.0,
                cache_read: 0.1,
                cache_write: 0.0,
            },
            compat: Compat::openai_compatible(),
            headers: HeaderMap::new(),
        },
        Model {
            id: "deepseek-ai/DeepSeek-V4.1-Flash-1017".to_owned(),
            name: "DeepSeek V4.1 Flash 1017".to_owned(),
            provider: provider_id("deepinfra"),
            api: api_id(),
            base_url: SPEC.base_url.to_owned(),
            context_window: 1048576,
            max_output_tokens: 384000,
            reasoning: true,
            reasoning_mandatory: false,
            thinking_levels: common::thinking_levels(true, &["none", "low", "high", "max"]),
            cost: ModelCost {
                input: 0.2,
                output: 0.6,
                cache_read: 0.006,
                cache_write: 0.0,
            },
            compat: Compat::openai_compatible(),
            headers: HeaderMap::new(),
        },
        Model {
            id: "moonshotai/Kimi-K2.7-Code".to_owned(),
            name: "Kimi K2.7 Code".to_owned(),
            provider: provider_id("deepinfra"),
            api: api_id(),
            base_url: SPEC.base_url.to_owned(),
            context_window: 262144,
            max_output_tokens: 262144,
            reasoning: true,
            reasoning_mandatory: false,
            thinking_levels: common::thinking_levels(true, &[]),
            cost: ModelCost {
                input: 0.68,
                output: 3.4,
                cache_read: 0.136,
                cache_write: 0.0,
            },
            compat: Compat::openai_compatible(),
            headers: HeaderMap::new(),
        },
    ]
}
