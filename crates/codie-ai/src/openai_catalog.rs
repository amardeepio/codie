//! Generic OpenAI-compatible model catalog.
//!
//! Providers that expose `GET /models` use this cache. The request is
//! best-effort, bounded by the shared four-second timeout, and always leaves the
//! previous cache in place on failure (`PROV-021`). The endpoint accepts the
//! common OpenAI response shape, plus the extra metadata some providers add
//! (`context_length`, `max_model_len`, `supported_parameters`, reasoning).
//!
//! The picker only shows chat models. Embedding, reranking, moderation, audio,
//! speech, image, video, and guard/safety entries are filtered out rather than
//! guessed into the chat list (`PROV-020`).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use codie_core::ThinkingLevel;
use serde::Deserialize;

use crate::catalog::{
    CATALOG_TIMEOUT, CatalogCache, CatalogModel, FALLBACK_CONTEXT_WINDOW,
    FALLBACK_MAX_OUTPUT_TOKENS, Reasoning, RefreshOutcome, TopProvider,
};
use crate::model::{HeaderMap, Model, ModelCost};
use crate::provider::ProviderSpec;

/// A refreshable, cache-backed OpenAI-compatible `/models` catalog.
#[derive(Clone)]
pub struct OpenAiCatalog {
    spec: ProviderSpec,
    cache_path: PathBuf,
    endpoint: String,
    api_key: Option<String>,
    timeout: Duration,
    client: reqwest::Client,
}

impl std::fmt::Debug for OpenAiCatalog {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OpenAiCatalog")
            .field("provider", &self.spec.id)
            .field("cache_path", &self.cache_path)
            .field("endpoint", &self.endpoint)
            .field("api_key", &self.api_key.as_ref().map(|_| "<redacted>"))
            .field("timeout", &self.timeout)
            .finish_non_exhaustive()
    }
}

impl OpenAiCatalog {
    /// Creates a catalog for `spec`, backed by `cache_path`.
    #[must_use]
    pub fn new(spec: ProviderSpec, cache_path: impl Into<PathBuf>) -> Self {
        Self {
            cache_path: cache_path.into(),
            endpoint: format!("{}/models", spec.base_url.trim_end_matches('/')),
            spec,
            api_key: None,
            timeout: CATALOG_TIMEOUT,
            client: reqwest::Client::new(),
        }
    }

    /// Sends `Authorization: Bearer <key>` with catalog requests.
    #[must_use]
    pub fn with_api_key(mut self, key: impl Into<String>) -> Self {
        let key = key.into();
        let key = key.trim();
        if !key.is_empty() {
            self.api_key = Some(key.to_owned());
        }
        self
    }

    /// Overrides the catalog endpoint (proxy or test endpoint).
    #[must_use]
    pub fn with_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = endpoint.into();
        self
    }

    /// Overrides the four-second refresh timeout.
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Returns the cache path.
    #[must_use]
    pub fn cache_path(&self) -> &Path {
        &self.cache_path
    }

    /// Loads the cached catalog, returning `None` when it is missing or invalid.
    #[must_use]
    pub fn load_cache(&self) -> Option<CatalogCache> {
        let data = fs::read(&self.cache_path).ok()?;
        match serde_json::from_slice(&data) {
            Ok(cache) => Some(cache),
            Err(error) => {
                tracing::debug!(
                    provider = self.spec.id,
                    path = %self.cache_path.display(),
                    %error,
                    "ignoring invalid provider model catalog cache"
                );
                None
            }
        }
    }

    /// Returns selectable chat models from the cached catalog.
    ///
    /// Never touches the network: callers get the last successful refresh, or
    /// an empty list before the first refresh.
    #[must_use]
    pub fn cached_models(&self) -> Vec<Model> {
        self.load_cache()
            .map(|cache| {
                let mut seen = BTreeSet::new();
                cache
                    .models
                    .iter()
                    .filter(|model| is_openai_chat_model(&model.id))
                    .filter(|model| seen.insert(model.id.clone()))
                    .map(|model| catalog_model(&self.spec, model))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Returns true when the cache exists and is inside the freshness window.
    #[must_use]
    pub fn is_cache_fresh(&self) -> bool {
        self.load_cache()
            .is_some_and(|cache| cache.is_fresh(unix_now()))
    }

    /// Returns true when a cache exists but is outside the freshness window.
    #[must_use]
    pub fn has_stale_cache(&self) -> bool {
        self.load_cache()
            .is_some_and(|cache| !cache.is_fresh(unix_now()))
    }

    /// Refreshes the cache without ever failing the caller.
    ///
    /// A fresh cache short-circuits the request. `304 Not Modified` extends
    /// freshness. Any network, timeout, HTTP, or parse failure returns
    /// [`RefreshOutcome::Stale`] and leaves the old cache in place.
    pub async fn refresh(&self) -> RefreshOutcome {
        let cache = self.load_cache();
        let now = unix_now();

        if let Some(cache) = &cache
            && cache.is_fresh(now)
        {
            return RefreshOutcome::Fresh {
                models: cache.models.len(),
            };
        }

        let previous = cache.unwrap_or_default();
        let etag = previous.etag.clone();
        match self.fetch_models(etag.as_deref()).await {
            Ok(FetchedCatalog::Models { etag, models }) => {
                if models.is_empty() && !previous.models.is_empty() {
                    return RefreshOutcome::Stale {
                        error: "provider returned an empty model list".to_owned(),
                    };
                }
                let next = CatalogCache {
                    etag,
                    programming_etag: None,
                    fetched_at: now,
                    models,
                    programming: Vec::new(),
                };
                if let Err(error) = self.save_cache(&next) {
                    tracing::warn!(
                        provider = self.spec.id,
                        path = %self.cache_path.display(),
                        %error,
                        "cannot persist the provider model catalog"
                    );
                }
                RefreshOutcome::Fetched {
                    models: next.models.len(),
                }
            }
            Ok(FetchedCatalog::NotModified) => {
                let mut next = previous;
                next.fetched_at = now;
                if let Err(error) = self.save_cache(&next) {
                    tracing::warn!(
                        provider = self.spec.id,
                        path = %self.cache_path.display(),
                        %error,
                        "cannot persist the provider model catalog"
                    );
                }
                RefreshOutcome::NotModified
            }
            Err(error) => {
                tracing::warn!(
                    provider = self.spec.id,
                    %error,
                    "provider catalog refresh failed; serving cached data"
                );
                RefreshOutcome::Stale {
                    error: error.to_string(),
                }
            }
        }
    }

    /// Starts a refresh in the background and returns immediately.
    pub fn spawn_refresh(self: std::sync::Arc<Self>) {
        tokio::spawn(async move {
            match self.refresh().await {
                RefreshOutcome::Stale { error } => {
                    tracing::warn!(
                        provider = self.spec.id,
                        %error,
                        "background provider catalog refresh failed"
                    );
                }
                outcome => {
                    tracing::debug!(
                        provider = self.spec.id,
                        ?outcome,
                        "background provider catalog refresh finished"
                    );
                }
            }
        });
    }

    async fn fetch_models(&self, etag: Option<&str>) -> Result<FetchedCatalog, CatalogError> {
        let mut request = self.client.get(&self.endpoint);
        if let Some(api_key) = &self.api_key {
            request = request.bearer_auth(api_key);
        }
        if let Some(etag) = etag {
            request = request.header(reqwest::header::IF_NONE_MATCH, etag);
        }

        let response = tokio::time::timeout(self.timeout, request.send())
            .await
            .map_err(|_| CatalogError::Timeout(self.timeout))?
            .map_err(|error| CatalogError::Transport(error.to_string()))?;

        if response.status() == reqwest::StatusCode::NOT_MODIFIED {
            return Ok(FetchedCatalog::NotModified);
        }
        if !response.status().is_success() {
            return Err(CatalogError::Status(response.status().as_u16()));
        }

        let etag = response
            .headers()
            .get(reqwest::header::ETAG)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);

        let body: ModelsResponse = response
            .json()
            .await
            .map_err(|error| CatalogError::Parse(error.to_string()))?;

        Ok(FetchedCatalog::Models {
            etag,
            models: body
                .data
                .into_iter()
                .filter(|model| !model.id.trim().is_empty())
                .map(entry)
                .collect(),
        })
    }

    fn save_cache(&self, cache: &CatalogCache) -> std::io::Result<()> {
        if let Some(parent) = self.cache_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let temp = self.cache_path.with_extension("json.tmp");
        let data = serde_json::to_vec_pretty(cache).map_err(std::io::Error::other)?;
        fs::write(&temp, data)?;
        fs::rename(&temp, &self.cache_path)
    }
}

/// Converts a cached OpenAI catalog entry into an adapter [`Model`].
#[must_use]
pub fn catalog_model(spec: &ProviderSpec, model: &CatalogModel) -> Model {
    let context_window = model
        .context_length
        .filter(|value| *value > 0)
        .unwrap_or(FALLBACK_CONTEXT_WINDOW);
    let max_output_tokens = model
        .top_provider
        .as_ref()
        .and_then(|provider| provider.max_completion_tokens)
        .filter(|value| *value > 0)
        .unwrap_or(FALLBACK_MAX_OUTPUT_TOKENS)
        .min(context_window);

    Model {
        id: model.id.clone(),
        name: model.name.clone().unwrap_or_else(|| model.id.clone()),
        provider: spec.id.into(),
        api: spec.api.into(),
        base_url: spec.base_url.to_owned(),
        context_window,
        max_output_tokens,
        reasoning: model.reasoning.is_some() || supports_parameter(model, "reasoning"),
        reasoning_mandatory: model
            .reasoning
            .as_ref()
            .is_some_and(|reasoning| reasoning.mandatory),
        thinking_levels: thinking_levels(model),
        cost: ModelCost::default(),
        compat: (spec.compat)(),
        headers: HeaderMap::new(),
    }
}

/// Returns true when `id` looks like a chat model rather than an embedding,
/// reranker, speech, image, video, moderation, or guard model.
#[must_use]
pub fn is_openai_chat_model(id: &str) -> bool {
    let id = id.trim().to_ascii_lowercase();
    if id.is_empty() {
        return false;
    }
    const EXCLUDED: &[&str] = &[
        "embed",
        "rerank",
        "moderation",
        "whisper",
        "tts",
        "speech",
        "transcribe",
        "audio",
        "realtime",
        "dall-e",
        "image",
        "video",
        "sora",
        "guard",
        "safety",
        "bge-",
        "gte-",
        "e5-",
        "clip",
        "stable-diffusion",
        "flux",
        "music",
        "voice",
    ];
    !EXCLUDED.iter().any(|fragment| id.contains(fragment))
}

fn entry(raw: OpenAiModel) -> CatalogModel {
    let context_length = raw
        .context_length
        .or(raw.context_window)
        .or(raw.max_model_len)
        .or_else(|| {
            raw.top_provider
                .as_ref()
                .and_then(|provider| provider.context_length)
        })
        .filter(|value| *value > 0);
    let max_output_tokens = raw
        .max_completion_tokens
        .or_else(|| {
            raw.top_provider
                .as_ref()
                .and_then(|provider| provider.max_completion_tokens)
        })
        .filter(|value| *value > 0);
    let top_provider =
        (context_length.is_some() || max_output_tokens.is_some()).then_some(TopProvider {
            context_length,
            max_completion_tokens: max_output_tokens,
        });

    CatalogModel {
        id: raw.id,
        name: raw.name,
        context_length,
        top_provider,
        pricing: raw.pricing,
        supported_parameters: raw.supported_parameters,
        reasoning: raw.reasoning,
        ..CatalogModel::default()
    }
}

fn supports_parameter(model: &CatalogModel, parameter: &str) -> bool {
    model
        .supported_parameters
        .iter()
        .any(|value| value.eq_ignore_ascii_case(parameter))
}

fn thinking_levels(model: &CatalogModel) -> BTreeMap<ThinkingLevel, Option<String>> {
    if model.reasoning.is_none() && !supports_parameter(model, "reasoning") {
        return BTreeMap::new();
    }

    let mut levels = BTreeMap::from([
        (ThinkingLevel::Off, Some("none".to_owned())),
        (ThinkingLevel::Minimal, Some("minimal".to_owned())),
        (ThinkingLevel::Low, Some("low".to_owned())),
        (ThinkingLevel::Medium, Some("medium".to_owned())),
        (ThinkingLevel::High, Some("high".to_owned())),
    ]);
    if let Some(reasoning) = &model.reasoning {
        if reasoning.mandatory {
            levels.remove(&ThinkingLevel::Off);
        }
        for effort in &reasoning.supported_efforts {
            if let Some(level) = ThinkingLevel::parse(effort) {
                levels.insert(level, Some(effort.clone()));
            }
        }
    }
    levels
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

#[derive(Debug, Deserialize)]
struct ModelsResponse {
    #[serde(default)]
    data: Vec<OpenAiModel>,
}

#[derive(Debug, Deserialize)]
struct OpenAiModel {
    id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    context_length: Option<u32>,
    #[serde(default)]
    context_window: Option<u32>,
    #[serde(default)]
    max_model_len: Option<u32>,
    #[serde(default)]
    max_completion_tokens: Option<u32>,
    #[serde(default)]
    top_provider: Option<TopProvider>,
    #[serde(default)]
    pricing: Option<serde_json::Value>,
    #[serde(default)]
    supported_parameters: Vec<String>,
    #[serde(default)]
    reasoning: Option<Reasoning>,
}

enum FetchedCatalog {
    Models {
        etag: Option<String>,
        models: Vec<CatalogModel>,
    },
    NotModified,
}

#[derive(Debug)]
enum CatalogError {
    Timeout(Duration),
    Transport(String),
    Status(u16),
    Parse(String),
}

impl std::fmt::Display for CatalogError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Timeout(timeout) => {
                write!(formatter, "catalog request timed out after {timeout:?}")
            }
            Self::Transport(error) => write!(formatter, "catalog transport error: {error}"),
            Self::Status(status) => write!(formatter, "catalog returned HTTP {status}"),
            Self::Parse(error) => write!(formatter, "catalog response was not valid JSON: {error}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::merge_models;
    use crate::model::Compat;
    use crate::provider::{CatalogKind, ProbeKind};
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn empty_headers() -> HeaderMap {
        HeaderMap::new()
    }

    fn no_models() -> Vec<Model> {
        Vec::new()
    }

    fn spec(server: &MockServer) -> ProviderSpec {
        ProviderSpec {
            id: "test",
            name: "Test",
            env_var: "TEST_API_KEY",
            requires_key: true,
            base_url: Box::leak(server.uri().into_boxed_str()),
            api: crate::OPENAI_COMPLETIONS,
            default_model: "test-chat",
            compat: Compat::openai,
            headers: empty_headers,
            models: no_models,
            catalog: CatalogKind::OpenAiModels,
            probe: ProbeKind::Models,
        }
    }

    #[test]
    fn filters_non_chat_ids() {
        assert!(is_openai_chat_model("llama-3.3-70b-versatile"));
        assert!(is_openai_chat_model("qwen3-coder"));
        assert!(!is_openai_chat_model("text-embedding-3-large"));
        assert!(!is_openai_chat_model("whisper-large-v3"));
        assert!(!is_openai_chat_model("llama-guard-3-8b"));
        assert!(!is_openai_chat_model("dall-e-3"));
        assert!(!is_openai_chat_model("gpt-4o-realtime-preview"));
        assert!(!is_openai_chat_model(""));
    }

    fn catalog_cache(models: Vec<CatalogModel>) -> CatalogCache {
        CatalogCache {
            fetched_at: unix_now(),
            models,
            ..CatalogCache::default()
        }
    }

    fn catalog_model_entry(id: &str, context: u32) -> CatalogModel {
        CatalogModel {
            id: id.to_owned(),
            name: Some(id.to_owned()),
            context_length: Some(context),
            ..CatalogModel::default()
        }
    }

    #[tokio::test]
    async fn cached_models_use_the_provider_spec() {
        let server = MockServer::start().await;
        let spec = spec(&server);
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("test-models.json");
        fs::write(
            &path,
            serde_json::to_vec(&catalog_cache(vec![
                catalog_model_entry("test-chat", 256_000),
                catalog_model_entry("text-embedding-3-small", 8_000),
            ]))
            .unwrap(),
        )
        .unwrap();

        let catalog = OpenAiCatalog::new(spec, path);
        let models = catalog.cached_models();
        assert_eq!(models.len(), 1);
        let model = &models[0];
        assert_eq!(model.id, "test-chat");
        assert_eq!(model.provider, "test");
        assert_eq!(model.base_url, server.uri());
        assert_eq!(model.context_window, 256_000);
        assert_eq!(model.max_output_tokens, FALLBACK_MAX_OUTPUT_TOKENS);
    }

    #[tokio::test]
    async fn refresh_sends_the_key_and_filters_the_catalog() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/models"))
            .and(header("authorization", "Bearer sk-test"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [
                    {"id": "test-chat", "context_length": 128000},
                    {"id": "text-embedding-3-large"},
                    {"id": "whisper-large-v3"},
                    {"id": "test-reasoner", "supported_parameters": ["reasoning"]}
                ]
            })))
            .mount(&server)
            .await;

        let spec = spec(&server);
        let directory = tempfile::tempdir().unwrap();
        let catalog = OpenAiCatalog::new(spec, directory.path().join("test-models.json"))
            .with_api_key("sk-test");
        let outcome = catalog.refresh().await;
        assert_eq!(outcome, RefreshOutcome::Fetched { models: 4 });

        let models = catalog.cached_models();
        assert_eq!(
            models
                .iter()
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>(),
            vec!["test-chat", "test-reasoner"]
        );
        assert!(models[1].reasoning);
        assert_eq!(models[0].context_window, 128_000);
    }

    #[tokio::test]
    async fn conditional_refresh_extends_the_cache() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/models"))
            .and(header("if-none-match", "\"abc\""))
            .respond_with(ResponseTemplate::new(304))
            .mount(&server)
            .await;

        let spec = spec(&server);
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("test-models.json");
        let mut cache = catalog_cache(vec![catalog_model_entry("test-chat", 64_000)]);
        cache.fetched_at = 0;
        cache.etag = Some("\"abc\"".to_owned());
        fs::write(&path, serde_json::to_vec(&cache).unwrap()).unwrap();

        let catalog = OpenAiCatalog::new(spec, &path);
        assert_eq!(catalog.refresh().await, RefreshOutcome::NotModified);
        assert_eq!(catalog.cached_models().len(), 1);
        assert!(catalog.is_cache_fresh());
    }

    #[tokio::test]
    async fn refresh_failure_keeps_the_previous_cache() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/models"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let spec = spec(&server);
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("test-models.json");
        let mut cache = catalog_cache(vec![catalog_model_entry("test-chat", 32_000)]);
        cache.fetched_at = 0;
        fs::write(&path, serde_json::to_vec(&cache).unwrap()).unwrap();

        let catalog = OpenAiCatalog::new(spec, &path);
        assert!(matches!(
            catalog.refresh().await,
            RefreshOutcome::Stale { .. }
        ));
        assert_eq!(catalog.cached_models().len(), 1);
    }

    #[tokio::test]
    async fn merge_keeps_curated_entries_first_and_deduplicates() {
        let server = MockServer::start().await;
        let spec = spec(&server);
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("test-models.json");
        fs::write(
            &path,
            serde_json::to_vec(&catalog_cache(vec![
                catalog_model_entry("test-chat", 128_000),
                catalog_model_entry("test-extra", 64_000),
            ]))
            .unwrap(),
        )
        .unwrap();

        let catalog = OpenAiCatalog::new(spec, path);
        let curated = vec![catalog_model(
            &spec,
            &catalog_model_entry("test-chat", 256_000),
        )];
        let merged = merge_models(curated, catalog.cached_models());
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].id, "test-chat");
        assert_eq!(merged[0].context_window, 256_000);
        assert_eq!(merged[1].id, "test-extra");
    }

    #[tokio::test]
    async fn debug_never_includes_the_key() {
        let server = MockServer::start().await;
        let catalog = OpenAiCatalog::new(spec(&server), "test.json").with_api_key("sk-secret");
        let debug = format!("{catalog:?}");
        assert!(!debug.contains("sk-secret"), "{debug}");
        assert!(debug.contains("redacted"), "{debug}");
    }
}
