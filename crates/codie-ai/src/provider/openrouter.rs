//! OpenRouter provider declaration.

use std::collections::BTreeMap;

use codie_core::ThinkingLevel;

use super::spec::{CatalogKind, ProbeKind, ProviderSpec};
use crate::model::{
    Compat, DEFAULT_HTTP_REFERER, DEFAULT_X_TITLE, HeaderMap, Model, ModelCost,
    PROVIDER_OPENROUTER, api_id, provider_id,
};

/// The OpenRouter provider.
pub const SPEC: ProviderSpec = ProviderSpec {
    id: PROVIDER_OPENROUTER,
    name: "OpenRouter",
    env_var: "OPENROUTER_API_KEY",
    requires_key: true,
    base_url: "https://openrouter.ai/api/v1",
    api: crate::OPENAI_COMPLETIONS,
    default_model: "openrouter/auto",
    compat: Compat::openrouter,
    headers: attribution_headers,
    models,
    catalog: CatalogKind::OpenRouter,
    probe: ProbeKind::OpenRouterKey,
};

fn attribution_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("HTTP-Referer".to_owned(), DEFAULT_HTTP_REFERER.to_owned());
    headers.insert("X-Title".to_owned(), DEFAULT_X_TITLE.to_owned());
    headers
}

fn thinking_levels() -> BTreeMap<ThinkingLevel, Option<String>> {
    BTreeMap::from([
        (ThinkingLevel::Off, Some("none".to_owned())),
        (ThinkingLevel::Minimal, Some("minimal".to_owned())),
        (ThinkingLevel::Low, Some("low".to_owned())),
        (ThinkingLevel::Medium, Some("medium".to_owned())),
        (ThinkingLevel::High, Some("high".to_owned())),
    ])
}

fn models() -> Vec<Model> {
    vec![
        Model {
            id: "openrouter/auto".to_owned(),
            name: "OpenRouter Auto".to_owned(),
            provider: provider_id(PROVIDER_OPENROUTER),
            api: api_id(),
            base_url: SPEC.base_url.to_owned(),
            context_window: 128_000,
            max_output_tokens: 8192,
            reasoning: false,
            reasoning_mandatory: false,
            thinking_levels: thinking_levels(),
            cost: ModelCost::default(),
            compat: Compat::openrouter(),
            headers: HeaderMap::new(),
        },
        Model {
            id: "deepseek/deepseek-v4.1-flash-1017".to_owned(),
            name: "DeepSeek V4.1 Flash (OpenRouter)".to_owned(),
            provider: provider_id(PROVIDER_OPENROUTER),
            api: api_id(),
            base_url: SPEC.base_url.to_owned(),
            context_window: 1_000_000,
            max_output_tokens: 384_000,
            reasoning: true,
            reasoning_mandatory: false,
            thinking_levels: thinking_levels(),
            // OpenRouter reports the billed cost per request, so the catalog
            // price is only an estimate for offline planning.
            cost: ModelCost::default(),
            compat: Compat::openrouter(),
            headers: HeaderMap::new(),
        },
    ]
}
