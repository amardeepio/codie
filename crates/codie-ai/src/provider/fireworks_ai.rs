//! Fireworks AI provider declaration.

use super::common;
use super::spec::{CatalogKind, ProbeKind, ProviderSpec};
use crate::model::{Compat, HeaderMap, Model, ModelCost, api_id, provider_id};

/// The provider declaration.
pub const SPEC: ProviderSpec = ProviderSpec {
    id: "fireworks-ai",
    name: "Fireworks AI",
    env_var: "FIREWORKS_API_KEY",
    requires_key: true,
    base_url: "https://api.fireworks.ai/inference/v1",
    api: crate::OPENAI_COMPLETIONS,
    default_model: "accounts/fireworks/models/deepseek-v4p1-flash-1017",
    compat: Compat::openai_compatible,
    headers: common::empty_headers,
    models,
    catalog: CatalogKind::OpenAiModels,
    probe: ProbeKind::Models,
};

fn models() -> Vec<Model> {
    vec![
        Model {
            id: "accounts/fireworks/models/deepseek-v4p1-flash-1017".to_owned(),
            name: "DeepSeek V4.1 Flash 1017".to_owned(),
            provider: provider_id("fireworks-ai"),
            api: api_id(),
            base_url: SPEC.base_url.to_owned(),
            context_window: 1000000,
            max_output_tokens: 384000,
            reasoning: true,
            reasoning_mandatory: false,
            thinking_levels: common::thinking_levels(true, &["low", "high", "max"]),
            cost: ModelCost {
                input: 0.3,
                output: 1.2,
                cache_read: 0.006,
                cache_write: 0.0,
            },
            compat: Compat::openai_compatible(),
            headers: HeaderMap::new(),
        },
        Model {
            id: "accounts/fireworks/models/kimi-k3p1-pro".to_owned(),
            name: "Kimi K3.1 Pro".to_owned(),
            provider: provider_id("fireworks-ai"),
            api: api_id(),
            base_url: SPEC.base_url.to_owned(),
            context_window: 1048576,
            max_output_tokens: 1048576,
            reasoning: true,
            reasoning_mandatory: false,
            thinking_levels: common::thinking_levels(true, &["low", "medium", "high", "max"]),
            cost: ModelCost {
                input: 2.0,
                output: 10.0,
                cache_read: 0.2,
                cache_write: 0.0,
            },
            compat: Compat::openai_compatible(),
            headers: HeaderMap::new(),
        },
        Model {
            id: "accounts/fireworks/models/minimax-m3p1-flash".to_owned(),
            name: "MiniMax-M3.1-Flash".to_owned(),
            provider: provider_id("fireworks-ai"),
            api: api_id(),
            base_url: SPEC.base_url.to_owned(),
            context_window: 1000000,
            max_output_tokens: 512000,
            reasoning: true,
            reasoning_mandatory: false,
            thinking_levels: common::thinking_levels(
                true,
                &["low", "medium", "high", "xhigh", "max"],
            ),
            cost: ModelCost {
                input: 0.15,
                output: 0.6,
                cache_read: 0.015,
                cache_write: 0.0,
            },
            compat: Compat::openai_compatible(),
            headers: HeaderMap::new(),
        },
        Model {
            id: "accounts/fireworks/models/glm-5p3-flash".to_owned(),
            name: "GLM 5.3 Flash".to_owned(),
            provider: provider_id("fireworks-ai"),
            api: api_id(),
            base_url: SPEC.base_url.to_owned(),
            context_window: 1048573,
            max_output_tokens: 131072,
            reasoning: true,
            reasoning_mandatory: false,
            thinking_levels: common::thinking_levels(true, &["low", "high", "max"]),
            cost: ModelCost {
                input: 0.15,
                output: 0.5,
                cache_read: 0.03,
                cache_write: 0.0,
            },
            compat: Compat::openai_compatible(),
            headers: HeaderMap::new(),
        },
    ]
}
