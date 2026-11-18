//! OpenAI provider declaration.

use super::common;
use super::spec::{CatalogKind, ProbeKind, ProviderSpec};
use crate::model::{Compat, HeaderMap, Model, ModelCost, api_id, provider_id};

/// The provider declaration.
pub const SPEC: ProviderSpec = ProviderSpec {
    id: "openai",
    name: "OpenAI",
    env_var: "OPENAI_API_KEY",
    requires_key: true,
    base_url: "https://api.openai.com/v1",
    api: crate::OPENAI_COMPLETIONS,
    default_model: "gpt-6.2-sol",
    compat: Compat::openai,
    headers: common::empty_headers,
    models,
    catalog: CatalogKind::OpenAiModels,
    probe: ProbeKind::Models,
};

fn models() -> Vec<Model> {
    vec![
        Model {
            id: "gpt-6.2-sol".to_owned(),
            name: "GPT-6.2 Sol".to_owned(),
            provider: provider_id("openai"),
            api: api_id(),
            base_url: SPEC.base_url.to_owned(),
            context_window: 1050000,
            max_output_tokens: 128000,
            reasoning: true,
            reasoning_mandatory: false,
            thinking_levels: common::thinking_levels(
                true,
                &["low", "medium", "high", "xhigh", "max"],
            ),
            cost: ModelCost {
                input: 2.0,
                output: 10.0,
                cache_read: 0.1,
                cache_write: 0.0,
            },
            compat: Compat::openai(),
            headers: HeaderMap::new(),
        },
        Model {
            id: "gpt-5.3-codex-spark".to_owned(),
            name: "GPT-5.3 Codex Spark".to_owned(),
            provider: provider_id("openai"),
            api: api_id(),
            base_url: SPEC.base_url.to_owned(),
            context_window: 128000,
            max_output_tokens: 32000,
            reasoning: true,
            reasoning_mandatory: false,
            thinking_levels: common::thinking_levels(
                true,
                &["none", "low", "medium", "high", "xhigh"],
            ),
            cost: ModelCost {
                input: 1.75,
                output: 14.0,
                cache_read: 0.175,
                cache_write: 0.0,
            },
            compat: Compat::openai(),
            headers: HeaderMap::new(),
        },
        Model {
            id: "gpt-5.3-codex".to_owned(),
            name: "GPT-5.3 Codex".to_owned(),
            provider: provider_id("openai"),
            api: api_id(),
            base_url: SPEC.base_url.to_owned(),
            context_window: 400000,
            max_output_tokens: 128000,
            reasoning: true,
            reasoning_mandatory: false,
            thinking_levels: common::thinking_levels(
                true,
                &["none", "low", "medium", "high", "xhigh"],
            ),
            cost: ModelCost {
                input: 1.75,
                output: 14.0,
                cache_read: 0.175,
                cache_write: 0.0,
            },
            compat: Compat::openai(),
            headers: HeaderMap::new(),
        },
        Model {
            id: "gpt-6.2-astra".to_owned(),
            name: "GPT-6.2 Astra".to_owned(),
            provider: provider_id("openai"),
            api: api_id(),
            base_url: SPEC.base_url.to_owned(),
            context_window: 1050000,
            max_output_tokens: 128000,
            reasoning: true,
            reasoning_mandatory: false,
            thinking_levels: common::thinking_levels(
                true,
                &["low", "medium", "high", "xhigh", "max"],
            ),
            cost: ModelCost {
                input: 8.0,
                output: 40.0,
                cache_read: 0.4,
                cache_write: 0.0,
            },
            compat: Compat::openai(),
            headers: HeaderMap::new(),
        },
    ]
}
