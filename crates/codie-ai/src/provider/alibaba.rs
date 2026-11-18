//! Alibaba DashScope provider declaration.

use super::common;
use super::spec::{CatalogKind, ProbeKind, ProviderSpec};
use crate::model::{Compat, HeaderMap, Model, ModelCost, api_id, provider_id};

/// The provider declaration.
pub const SPEC: ProviderSpec = ProviderSpec {
    id: "alibaba",
    name: "Alibaba DashScope",
    env_var: "DASHSCOPE_API_KEY",
    requires_key: true,
    base_url: "https://dashscope-intl.aliyuncs.com/compatible-mode/v1",
    api: crate::OPENAI_COMPLETIONS,
    default_model: "qwen3-coder-flash",
    compat: Compat::openai_compatible,
    headers: common::empty_headers,
    models,
    catalog: CatalogKind::OpenAiModels,
    probe: ProbeKind::Models,
};

fn models() -> Vec<Model> {
    vec![
        Model {
            id: "qwen3-coder-flash".to_owned(),
            name: "Qwen3 Coder Flash".to_owned(),
            provider: provider_id("alibaba"),
            api: api_id(),
            base_url: SPEC.base_url.to_owned(),
            context_window: 1000000,
            max_output_tokens: 65536,
            reasoning: false,
            reasoning_mandatory: false,
            thinking_levels: common::thinking_levels(false, &[]),
            cost: ModelCost {
                input: 0.3,
                output: 1.5,
                cache_read: 0.06,
                cache_write: 0.0,
            },
            compat: Compat::openai_compatible(),
            headers: HeaderMap::new(),
        },
        Model {
            id: "qwen3.8-max".to_owned(),
            name: "Qwen3.8 Max".to_owned(),
            provider: provider_id("alibaba"),
            api: api_id(),
            base_url: SPEC.base_url.to_owned(),
            context_window: 1000000,
            max_output_tokens: 131072,
            reasoning: true,
            reasoning_mandatory: false,
            thinking_levels: common::thinking_levels(true, &["low", "medium", "xhigh"]),
            cost: ModelCost {
                input: 2.0,
                output: 6.0,
                cache_read: 0.25,
                cache_write: 0.0,
            },
            compat: Compat::openai_compatible(),
            headers: HeaderMap::new(),
        },
        Model {
            id: "qwen3.8-flash".to_owned(),
            name: "Qwen3.8 Flash".to_owned(),
            provider: provider_id("alibaba"),
            api: api_id(),
            base_url: SPEC.base_url.to_owned(),
            context_window: 1000000,
            max_output_tokens: 131072,
            reasoning: true,
            reasoning_mandatory: false,
            thinking_levels: common::thinking_levels(true, &["low", "medium", "xhigh"]),
            cost: ModelCost {
                input: 0.15,
                output: 0.47,
                cache_read: 0.016,
                cache_write: 0.0,
            },
            compat: Compat::openai_compatible(),
            headers: HeaderMap::new(),
        },
        Model {
            id: "qwen3.7-plus".to_owned(),
            name: "Qwen3.7 Plus".to_owned(),
            provider: provider_id("alibaba"),
            api: api_id(),
            base_url: SPEC.base_url.to_owned(),
            context_window: 1000000,
            max_output_tokens: 131072,
            reasoning: true,
            reasoning_mandatory: false,
            thinking_levels: common::thinking_levels(true, &[]),
            cost: ModelCost {
                input: 0.4,
                output: 1.6,
                cache_read: 0.08,
                cache_write: 0.0,
            },
            compat: Compat::openai_compatible(),
            headers: HeaderMap::new(),
        },
    ]
}
