//! OpenCode provider declaration.

use super::common;
use super::spec::{CatalogKind, ProbeKind, ProviderSpec};
use crate::model::{Compat, HeaderMap, Model, ModelCost, api_id, provider_id};

/// The provider declaration.
pub const SPEC: ProviderSpec = ProviderSpec {
    id: "opencode",
    name: "OpenCode",
    env_var: "OPENCODE_API_KEY",
    requires_key: true,
    base_url: "https://opencode.ai/inference/openai/v1",
    api: crate::OPENAI_COMPLETIONS,
    default_model: "gpt-5.3-codex",
    compat: Compat::openai_compatible,
    headers: common::empty_headers,
    models,
    catalog: CatalogKind::OpenAiModels,
    probe: ProbeKind::Models,
};

fn models() -> Vec<Model> {
    vec![
        Model {
            id: "gpt-5.3-codex".to_owned(),
            name: "GPT-5.3 Codex".to_owned(),
            provider: provider_id("opencode"),
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
            compat: Compat::openai_compatible(),
            headers: HeaderMap::new(),
        },
        Model {
            id: "kimi-k2.7-code".to_owned(),
            name: "Kimi K2.7 Code".to_owned(),
            provider: provider_id("opencode"),
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
            id: "gpt-5.3-codex-spark".to_owned(),
            name: "GPT-5.3 Codex Spark".to_owned(),
            provider: provider_id("opencode"),
            api: api_id(),
            base_url: SPEC.base_url.to_owned(),
            context_window: 128000,
            max_output_tokens: 128000,
            reasoning: true,
            reasoning_mandatory: false,
            thinking_levels: common::thinking_levels(true, &["low", "medium", "high", "xhigh"]),
            cost: ModelCost {
                input: 1.75,
                output: 14.0,
                cache_read: 0.175,
                cache_write: 0.0,
            },
            compat: Compat::openai_compatible(),
            headers: HeaderMap::new(),
        },
        Model {
            id: "gpt-5.1-codex-max".to_owned(),
            name: "GPT-5.1 Codex Max".to_owned(),
            provider: provider_id("opencode"),
            api: api_id(),
            base_url: SPEC.base_url.to_owned(),
            context_window: 400000,
            max_output_tokens: 128000,
            reasoning: true,
            reasoning_mandatory: false,
            thinking_levels: common::thinking_levels(true, &["low", "medium", "high", "xhigh"]),
            cost: ModelCost {
                input: 1.25,
                output: 10.0,
                cache_read: 0.125,
                cache_write: 0.0,
            },
            compat: Compat::openai_compatible(),
            headers: HeaderMap::new(),
        },
    ]
}
