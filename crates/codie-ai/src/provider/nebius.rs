//! Nebius Token Factory provider declaration.

use super::common;
use super::spec::{CatalogKind, ProbeKind, ProviderSpec};
use crate::model::{Compat, HeaderMap, Model, ModelCost, api_id, provider_id};

/// The provider declaration.
pub const SPEC: ProviderSpec = ProviderSpec {
    id: "nebius",
    name: "Nebius Token Factory",
    env_var: "NEBIUS_API_KEY",
    requires_key: true,
    base_url: "https://api.tokenfactory.nebius.com/v1",
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
            provider: provider_id("nebius"),
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
            id: "thinkingmachines/Inkling-2-Small".to_owned(),
            name: "Inkling 2 Small".to_owned(),
            provider: provider_id("nebius"),
            api: api_id(),
            base_url: SPEC.base_url.to_owned(),
            context_window: 1048576,
            max_output_tokens: 1048576,
            reasoning: true,
            reasoning_mandatory: false,
            thinking_levels: common::thinking_levels(
                true,
                &["none", "minimal", "low", "medium", "high", "xhigh", "max"],
            ),
            cost: ModelCost {
                input: 0.3,
                output: 1.0,
                cache_read: 0.06,
                cache_write: 0.0,
            },
            compat: Compat::openai_compatible(),
            headers: HeaderMap::new(),
        },
        Model {
            id: "MiniMaxAI/MiniMax-M3.1-Flash".to_owned(),
            name: "MiniMax-M3.1-Flash".to_owned(),
            provider: provider_id("nebius"),
            api: api_id(),
            base_url: SPEC.base_url.to_owned(),
            context_window: 1048576,
            max_output_tokens: 1048576,
            reasoning: true,
            reasoning_mandatory: false,
            thinking_levels: common::thinking_levels(
                true,
                &["none", "low", "medium", "high", "xhigh", "max"],
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
            id: "deepseek-ai/DeepSeek-V4.1-Flash-1017".to_owned(),
            name: "DeepSeek V4.1 Flash 1017".to_owned(),
            provider: provider_id("nebius"),
            api: api_id(),
            base_url: SPEC.base_url.to_owned(),
            context_window: 1048576,
            max_output_tokens: 1048576,
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
    ]
}
