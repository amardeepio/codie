//! Moonshot AI provider declaration.

use super::common;
use super::spec::{CatalogKind, ProbeKind, ProviderSpec};
use crate::model::{Compat, HeaderMap, Model, ModelCost, api_id, provider_id};

/// The provider declaration.
pub const SPEC: ProviderSpec = ProviderSpec {
    id: "moonshotai",
    name: "Moonshot AI",
    env_var: "MOONSHOT_API_KEY",
    requires_key: true,
    base_url: "https://api.moonshot.ai/v1",
    api: crate::OPENAI_COMPLETIONS,
    default_model: "kimi-k2.7-code",
    compat: Compat::openai_compatible,
    headers: common::empty_headers,
    models,
    catalog: CatalogKind::OpenAiModels,
    probe: ProbeKind::Models,
};

fn models() -> Vec<Model> {
    vec![
        Model {
            id: "kimi-k2.7-code".to_owned(),
            name: "Kimi K2.7 Code".to_owned(),
            provider: provider_id("moonshotai"),
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
            id: "kimi-k2.7-code-highspeed".to_owned(),
            name: "Kimi K2.7 Code HighSpeed".to_owned(),
            provider: provider_id("moonshotai"),
            api: api_id(),
            base_url: SPEC.base_url.to_owned(),
            context_window: 262144,
            max_output_tokens: 262144,
            reasoning: true,
            reasoning_mandatory: false,
            thinking_levels: common::thinking_levels(true, &[]),
            cost: ModelCost {
                input: 1.9,
                output: 8.0,
                cache_read: 0.38,
                cache_write: 0.0,
            },
            compat: Compat::openai_compatible(),
            headers: HeaderMap::new(),
        },
        Model {
            id: "kimi-k3.1-pro-highspeed".to_owned(),
            name: "Kimi K3.1 Pro Highspeed".to_owned(),
            provider: provider_id("moonshotai"),
            api: api_id(),
            base_url: SPEC.base_url.to_owned(),
            context_window: 1048576,
            max_output_tokens: 1048576,
            reasoning: true,
            reasoning_mandatory: false,
            thinking_levels: common::thinking_levels(true, &["low", "high", "max"]),
            cost: ModelCost {
                input: 4.0,
                output: 20.0,
                cache_read: 0.2,
                cache_write: 0.0,
            },
            compat: Compat::openai_compatible(),
            headers: HeaderMap::new(),
        },
        Model {
            id: "kimi-k3.1-pro".to_owned(),
            name: "Kimi K3.1 Pro".to_owned(),
            provider: provider_id("moonshotai"),
            api: api_id(),
            base_url: SPEC.base_url.to_owned(),
            context_window: 1048576,
            max_output_tokens: 1048576,
            reasoning: true,
            reasoning_mandatory: false,
            thinking_levels: common::thinking_levels(true, &["low", "high", "max"]),
            cost: ModelCost {
                input: 2.0,
                output: 10.0,
                cache_read: 0.2,
                cache_write: 0.0,
            },
            compat: Compat::openai_compatible(),
            headers: HeaderMap::new(),
        },
    ]
}
