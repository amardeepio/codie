//! NVIDIA NIM provider declaration.

use super::common;
use super::spec::{CatalogKind, ProbeKind, ProviderSpec};
use crate::model::{Compat, HeaderMap, Model, ModelCost, api_id, provider_id};

/// The provider declaration.
pub const SPEC: ProviderSpec = ProviderSpec {
    id: "nvidia",
    name: "NVIDIA NIM",
    env_var: "NVIDIA_API_KEY",
    requires_key: true,
    base_url: "https://integrate.api.nvidia.com/v1",
    api: crate::OPENAI_COMPLETIONS,
    default_model: "deepseek-ai/deepseek-v4.1-flash",
    compat: Compat::openai_compatible,
    headers: common::empty_headers,
    models,
    catalog: CatalogKind::OpenAiModels,
    probe: ProbeKind::Models,
};

fn models() -> Vec<Model> {
    vec![
        Model {
            id: "deepseek-ai/deepseek-v4.1-flash".to_owned(),
            name: "DeepSeek V4.1 Flash".to_owned(),
            provider: provider_id("nvidia"),
            api: api_id(),
            base_url: SPEC.base_url.to_owned(),
            context_window: 1000000,
            max_output_tokens: 384000,
            reasoning: true,
            reasoning_mandatory: false,
            thinking_levels: common::thinking_levels(true, &["none", "low", "high", "max"]),
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
            id: "minimaxai/minimax-m3.1-flash".to_owned(),
            name: "MiniMax-M3.1-Flash".to_owned(),
            provider: provider_id("nvidia"),
            api: api_id(),
            base_url: SPEC.base_url.to_owned(),
            context_window: 262144,
            max_output_tokens: 262144,
            reasoning: true,
            reasoning_mandatory: false,
            thinking_levels: common::thinking_levels(
                true,
                &["low", "medium", "high", "xhigh", "max"],
            ),
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
            id: "nvidia/nemotron-3.5-super-vl-120b-a12b".to_owned(),
            name: "Nemotron 3.5 Super VL 120B A12B".to_owned(),
            provider: provider_id("nvidia"),
            api: api_id(),
            base_url: SPEC.base_url.to_owned(),
            context_window: 1048576,
            max_output_tokens: 8192,
            reasoning: true,
            reasoning_mandatory: false,
            thinking_levels: common::thinking_levels(true, &[]),
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
            id: "z-ai/glm-5.3-flash".to_owned(),
            name: "GLM-5.3-Flash".to_owned(),
            provider: provider_id("nvidia"),
            api: api_id(),
            base_url: SPEC.base_url.to_owned(),
            context_window: 1000000,
            max_output_tokens: 131072,
            reasoning: true,
            reasoning_mandatory: false,
            thinking_levels: common::thinking_levels(true, &["low", "high", "max"]),
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
