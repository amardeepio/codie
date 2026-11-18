//! xAI provider declaration.

use super::common;
use super::spec::{CatalogKind, ProbeKind, ProviderSpec};
use crate::model::{Compat, HeaderMap, Model, ModelCost, api_id, provider_id};

/// The provider declaration.
pub const SPEC: ProviderSpec = ProviderSpec {
    id: "xai",
    name: "xAI",
    env_var: "XAI_API_KEY",
    requires_key: true,
    base_url: "https://api.x.ai/v1",
    api: crate::OPENAI_COMPLETIONS,
    default_model: "grok-4.7",
    compat: Compat::openai_compatible,
    headers: common::empty_headers,
    models,
    catalog: CatalogKind::OpenAiModels,
    probe: ProbeKind::Models,
};

fn models() -> Vec<Model> {
    vec![
        Model {
            id: "grok-4.7".to_owned(),
            name: "Grok 4.7".to_owned(),
            provider: provider_id("xai"),
            api: api_id(),
            base_url: SPEC.base_url.to_owned(),
            context_window: 500000,
            max_output_tokens: 500000,
            reasoning: true,
            reasoning_mandatory: false,
            thinking_levels: common::thinking_levels(true, &["low", "medium", "high", "xhigh"]),
            cost: ModelCost {
                input: 2.0,
                output: 6.0,
                cache_read: 0.5,
                cache_write: 0.0,
            },
            compat: Compat::openai_compatible(),
            headers: HeaderMap::new(),
        },
        Model {
            id: "grok-4.6".to_owned(),
            name: "Grok 4.6".to_owned(),
            provider: provider_id("xai"),
            api: api_id(),
            base_url: SPEC.base_url.to_owned(),
            context_window: 500000,
            max_output_tokens: 500000,
            reasoning: true,
            reasoning_mandatory: false,
            thinking_levels: common::thinking_levels(true, &["low", "medium", "high", "xhigh"]),
            cost: ModelCost {
                input: 2.0,
                output: 6.0,
                cache_read: 0.5,
                cache_write: 0.0,
            },
            compat: Compat::openai_compatible(),
            headers: HeaderMap::new(),
        },
        Model {
            id: "grok-4.5".to_owned(),
            name: "Grok 4.5".to_owned(),
            provider: provider_id("xai"),
            api: api_id(),
            base_url: SPEC.base_url.to_owned(),
            context_window: 500000,
            max_output_tokens: 500000,
            reasoning: true,
            reasoning_mandatory: false,
            thinking_levels: common::thinking_levels(true, &["low", "medium", "high", "xhigh"]),
            cost: ModelCost {
                input: 2.0,
                output: 6.0,
                cache_read: 0.3,
                cache_write: 0.0,
            },
            compat: Compat::openai_compatible(),
            headers: HeaderMap::new(),
        },
        Model {
            id: "grok-4.3".to_owned(),
            name: "Grok 4.3".to_owned(),
            provider: provider_id("xai"),
            api: api_id(),
            base_url: SPEC.base_url.to_owned(),
            context_window: 1000000,
            max_output_tokens: 30000,
            reasoning: true,
            reasoning_mandatory: false,
            thinking_levels: common::thinking_levels(
                true,
                &["none", "low", "medium", "high", "xhigh"],
            ),
            cost: ModelCost {
                input: 1.25,
                output: 2.5,
                cache_read: 0.2,
                cache_write: 0.0,
            },
            compat: Compat::openai_compatible(),
            headers: HeaderMap::new(),
        },
    ]
}
