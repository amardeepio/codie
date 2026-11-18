//! Novita AI provider declaration.

use super::common;
use super::spec::{CatalogKind, ProbeKind, ProviderSpec};
use crate::model::{Compat, HeaderMap, Model, ModelCost, api_id, provider_id};

/// The provider declaration.
pub const SPEC: ProviderSpec = ProviderSpec {
    id: "novita-ai",
    name: "Novita AI",
    env_var: "NOVITA_API_KEY",
    requires_key: true,
    base_url: "https://api.novita.ai/openai",
    api: crate::OPENAI_COMPLETIONS,
    default_model: "moonshotai/kimi-k2.7-code",
    compat: Compat::openai_compatible,
    headers: common::empty_headers,
    models,
    catalog: CatalogKind::OpenAiModels,
    probe: ProbeKind::Models,
};

fn models() -> Vec<Model> {
    vec![
        Model {
            id: "moonshotai/kimi-k2.7-code".to_owned(),
            name: "Kimi K2.7 Code".to_owned(),
            provider: provider_id("novita-ai"),
            api: api_id(),
            base_url: SPEC.base_url.to_owned(),
            context_window: 262144,
            max_output_tokens: 262144,
            reasoning: true,
            reasoning_mandatory: false,
            thinking_levels: common::thinking_levels(true, &[]),
            cost: ModelCost {
                input: 0.95,
                output: 4.0,
                cache_read: 0.19,
                cache_write: 0.0,
            },
            compat: Compat::openai_compatible(),
            headers: HeaderMap::new(),
        },
        Model {
            id: "deepseek/deepseek-v4.1-flash-1017".to_owned(),
            name: "DeepSeek V4.1 Flash 1017".to_owned(),
            provider: provider_id("novita-ai"),
            api: api_id(),
            base_url: SPEC.base_url.to_owned(),
            context_window: 1048576,
            max_output_tokens: 393216,
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
            id: "qwen/qwen4-max".to_owned(),
            name: "Qwen4 Max".to_owned(),
            provider: provider_id("novita-ai"),
            api: api_id(),
            base_url: SPEC.base_url.to_owned(),
            context_window: 1000000,
            max_output_tokens: 131000,
            reasoning: true,
            reasoning_mandatory: false,
            thinking_levels: common::thinking_levels(true, &["none", "low", "high", "xhigh"]),
            cost: ModelCost {
                input: 1.413,
                output: 4.24,
                cache_read: 0.071,
                cache_write: 0.0,
            },
            compat: Compat::openai_compatible(),
            headers: HeaderMap::new(),
        },
        Model {
            id: "minimax/minimax-m3.1-flash".to_owned(),
            name: "MiniMax-M3.1-Flash".to_owned(),
            provider: provider_id("novita-ai"),
            api: api_id(),
            base_url: SPEC.base_url.to_owned(),
            context_window: 1000000,
            max_output_tokens: 524288,
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
    ]
}
