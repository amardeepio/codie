//! DeepSeek provider declaration.

use std::collections::BTreeMap;

use codie_core::ThinkingLevel;

use super::spec::{CatalogKind, ProbeKind, ProviderSpec};
use crate::model::{Compat, HeaderMap, Model, ModelCost, PROVIDER_DEEPSEEK, api_id, provider_id};

/// The DeepSeek provider.
pub const SPEC: ProviderSpec = ProviderSpec {
    id: PROVIDER_DEEPSEEK,
    name: "DeepSeek",
    env_var: "DEEPSEEK_API_KEY",
    requires_key: true,
    base_url: "https://api.deepseek.com",
    api: crate::OPENAI_COMPLETIONS,
    default_model: "deepseek-flash",
    compat: Compat::deepseek,
    headers: empty_headers,
    models,
    catalog: CatalogKind::Bundled,
    probe: ProbeKind::Models,
};

fn empty_headers() -> HeaderMap {
    HeaderMap::new()
}

fn thinking_levels() -> BTreeMap<ThinkingLevel, Option<String>> {
    BTreeMap::from([
        (ThinkingLevel::Off, None),
        (ThinkingLevel::Minimal, Some("minimal".to_owned())),
        (ThinkingLevel::Low, Some("low".to_owned())),
        (ThinkingLevel::Medium, None),
        (ThinkingLevel::High, Some("high".to_owned())),
    ])
}

fn models() -> Vec<Model> {
    vec![
        Model {
            id: "deepseek-flash".to_owned(),
            name: "DeepSeek Flash".to_owned(),
            provider: provider_id(PROVIDER_DEEPSEEK),
            api: api_id(),
            base_url: SPEC.base_url.to_owned(),
            context_window: 1_000_000,
            max_output_tokens: 384_000,
            reasoning: true,
            reasoning_mandatory: false,
            thinking_levels: thinking_levels(),
            cost: ModelCost {
                input: 0.30,
                output: 1.20,
                cache_read: 0.006,
                cache_write: 0.0,
            },
            compat: Compat::deepseek(),
            headers: HeaderMap::new(),
        },
        Model {
            id: "deepseek-v4-pro".to_owned(),
            name: "DeepSeek V4 Pro".to_owned(),
            provider: provider_id(PROVIDER_DEEPSEEK),
            api: api_id(),
            base_url: SPEC.base_url.to_owned(),
            context_window: 1_000_000,
            max_output_tokens: 384_000,
            reasoning: true,
            reasoning_mandatory: false,
            thinking_levels: thinking_levels(),
            cost: ModelCost {
                input: 1.32,
                output: 3.96,
                cache_read: 0.0,
                cache_write: 0.0,
            },
            compat: Compat::deepseek(),
            headers: HeaderMap::new(),
        },
    ]
}
