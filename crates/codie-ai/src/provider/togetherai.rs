//! Together AI provider declaration.

use super::common;
use super::spec::{CatalogKind, ProbeKind, ProviderSpec};
use crate::model::{Compat, HeaderMap, Model, ModelCost, api_id, provider_id};

/// The provider declaration.
pub const SPEC: ProviderSpec = ProviderSpec {
    id: "togetherai",
    name: "Together AI",
    env_var: "TOGETHER_API_KEY",
    requires_key: true,
    base_url: "https://api.together.xyz/v1",
    api: crate::OPENAI_COMPLETIONS,
    default_model: "moonshotai/Kimi-K3.1-Pro",
    compat: Compat::openai_compatible,
    headers: common::empty_headers,
    models,
    catalog: CatalogKind::OpenAiModels,
    probe: ProbeKind::Models,
};

fn models() -> Vec<Model> {
    vec![
        Model {
            id: "moonshotai/Kimi-K3.1-Pro".to_owned(),
            name: "Kimi K3.1 Pro".to_owned(),
            provider: provider_id("togetherai"),
            api: api_id(),
            base_url: SPEC.base_url.to_owned(),
            context_window: 1048576,
            max_output_tokens: 1048576,
            reasoning: true,
            reasoning_mandatory: false,
            thinking_levels: common::thinking_levels(
                true,
                &["none", "low", "medium", "high", "max"],
            ),
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
            id: "deepseek-ai/DeepSeek-V4.1-Flash".to_owned(),
            name: "DeepSeek V4.1 Flash".to_owned(),
            provider: provider_id("togetherai"),
            api: api_id(),
            base_url: SPEC.base_url.to_owned(),
            context_window: 1048576,
            max_output_tokens: 384000,
            reasoning: true,
            reasoning_mandatory: false,
            thinking_levels: common::thinking_levels(true, &["none", "low", "high", "max"]),
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
            id: "zai-org/GLM-5.3".to_owned(),
            name: "GLM-5.3".to_owned(),
            provider: provider_id("togetherai"),
            api: api_id(),
            base_url: SPEC.base_url.to_owned(),
            context_window: 1048575,
            max_output_tokens: 262144,
            reasoning: true,
            reasoning_mandatory: false,
            thinking_levels: common::thinking_levels(true, &["low", "high", "max"]),
            cost: ModelCost {
                input: 1.4,
                output: 4.4,
                cache_read: 0.26,
                cache_write: 0.0,
            },
            compat: Compat::openai_compatible(),
            headers: HeaderMap::new(),
        },
        Model {
            id: "zai-org/GLM-5.3-Flash".to_owned(),
            name: "GLM-5.3-Flash".to_owned(),
            provider: provider_id("togetherai"),
            api: api_id(),
            base_url: SPEC.base_url.to_owned(),
            context_window: 1048575,
            max_output_tokens: 400000,
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
