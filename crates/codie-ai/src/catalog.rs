//! OpenRouter model-catalog refresh, coding-model selection, and conversion.
//!
//! OpenRouter publishes a live model catalog with modality, pricing, benchmark,
//! and reasoning metadata. codie keeps a small on-disk cache so startup never
//! waits for the network: a refresh is best-effort, bounded by a four-second
//! timeout, and serves the previous cache when it fails (`PROV-021`). Models
//! that are not text-in/text-out chat models are excluded from selection rather
//! than coerced (`PROV-020`).
//!
//! The interactive picker only offers coding models: a model is selectable
//! when it supports tool calling and either appears in OpenRouter's
//! `programming` use-case category or reports an Artificial Analysis coding
//! index of at least [`CODING_INDEX_THRESHOLD`].

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use codie_core::ThinkingLevel;
use serde::{Deserialize, Serialize};

use crate::OPENAI_COMPLETIONS;
use crate::model::{Compat, HeaderMap, Model, ModelCost, PROVIDER_OPENROUTER, provider_base_url};

/// Maximum time a catalog refresh may take before it is abandoned.
pub const CATALOG_TIMEOUT: Duration = Duration::from_secs(4);

/// How long a cached catalog is considered fresh.
pub const CATALOG_FRESHNESS: Duration = Duration::from_secs(4 * 60 * 60);

/// Default OpenRouter catalog endpoint.
pub const OPENROUTER_MODELS_URL: &str = "https://openrouter.ai/api/v1/models";

/// The OpenRouter use-case category used for coding models.
pub const PROGRAMMING_CATEGORY: &str = "programming";

/// Minimum Artificial Analysis coding index for a model that is not listed in
/// the `programming` category.
pub const CODING_INDEX_THRESHOLD: f64 = 70.0;

/// Fallback context window when the catalog omits one.
pub const FALLBACK_CONTEXT_WINDOW: u32 = 128_000;

/// Fallback maximum output tokens when the catalog omits one.
pub const FALLBACK_MAX_OUTPUT_TOKENS: u32 = 8_192;

/// One model entry as published by OpenRouter.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CatalogModel {
    /// Provider-scoped model identifier, used verbatim.
    pub id: String,
    /// Human-readable name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Canonical slug shared by variants such as `:batch`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canonical_slug: Option<String>,
    /// Context window in tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_length: Option<u32>,
    /// Modality metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub architecture: Option<Architecture>,
    /// Raw pricing object; values are strings in the provider response.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pricing: Option<serde_json::Value>,
    /// The primary provider's limits for this model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_provider: Option<TopProvider>,
    /// Request parameters the model accepts.
    #[serde(default)]
    pub supported_parameters: Vec<String>,
    /// Reasoning metadata, present only for reasoning models.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<Reasoning>,
    /// Third-party benchmark rankings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub benchmarks: Option<Benchmarks>,
    /// Concrete model targeted by a tilde alias.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias_target: Option<AliasTarget>,
}

/// Modality metadata for a catalog model.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Architecture {
    /// Accepted input modalities such as `text` and `image`.
    #[serde(default)]
    pub input_modalities: Vec<String>,
    /// Produced output modalities such as `text` and `embeddings`.
    #[serde(default)]
    pub output_modalities: Vec<String>,
    /// Provider's combined modality string, for example `text->text`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modality: Option<String>,
}

/// Limits reported by a model's primary serving provider.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TopProvider {
    /// Context window offered by the provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_length: Option<u32>,
    /// Maximum completion tokens offered by the provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_completion_tokens: Option<u32>,
}

/// Reasoning capabilities published for a model.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reasoning {
    /// True when reasoning cannot be disabled.
    #[serde(default)]
    pub mandatory: bool,
    /// True when reasoning is enabled when the request omits the parameter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_enabled: Option<bool>,
    /// Provider-native effort names such as `low`, `medium`, or `xhigh`.
    #[serde(default)]
    pub supported_efforts: Vec<String>,
}

/// Third-party benchmark data.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Benchmarks {
    /// Artificial Analysis indices.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artificial_analysis: Option<ArtificialAnalysis>,
}

/// Artificial Analysis index values.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ArtificialAnalysis {
    /// Coding index, or `None` when the model is unscored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coding_index: Option<f64>,
    /// Agentic index, or `None` when the model is unscored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agentic_index: Option<f64>,
    /// Intelligence index, or `None` when the model is unscored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intelligence_index: Option<f64>,
}

/// The concrete model targeted by a tilde alias.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AliasTarget {
    /// Routable model ID.
    #[serde(default)]
    pub slug: String,
    /// Human-readable name.
    #[serde(default)]
    pub name: String,
}

/// The cached catalog file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CatalogCache {
    /// ETag returned by the provider, used for conditional requests.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub etag: Option<String>,
    /// ETag for the `programming` category response.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub programming_etag: Option<String>,
    /// Unix seconds when the catalog was last confirmed fresh.
    pub fetched_at: u64,
    /// Every model returned by the provider.
    #[serde(default)]
    pub models: Vec<CatalogModel>,
    /// IDs returned by the `programming` category, in provider order.
    #[serde(default)]
    pub programming: Vec<String>,
}

impl CatalogCache {
    /// Returns true when the cache was confirmed within the freshness window.
    #[must_use]
    pub fn is_fresh(&self, now: u64) -> bool {
        now.saturating_sub(self.fetched_at) < CATALOG_FRESHNESS.as_secs()
    }
}

/// The result of one refresh attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefreshOutcome {
    /// The cache was still fresh; no request was made.
    Fresh {
        /// Number of cached models.
        models: usize,
    },
    /// A new catalog was fetched and stored.
    Fetched {
        /// Number of fetched models.
        models: usize,
    },
    /// The provider answered `304 Not Modified`; freshness was extended.
    NotModified,
    /// The refresh failed; callers keep serving the stale cache, if any.
    Stale {
        /// Human-readable failure reason.
        error: String,
    },
}

/// A refreshable, cache-backed OpenRouter model catalog.
#[derive(Debug, Clone)]
pub struct OpenRouterCatalog {
    cache_path: PathBuf,
    endpoint: String,
    timeout: Duration,
    client: reqwest::Client,
}

impl OpenRouterCatalog {
    /// Creates a catalog backed by `cache_path` and the public endpoint.
    #[must_use]
    pub fn new(cache_path: impl Into<PathBuf>) -> Self {
        Self {
            cache_path: cache_path.into(),
            endpoint: OPENROUTER_MODELS_URL.to_owned(),
            timeout: CATALOG_TIMEOUT,
            client: reqwest::Client::new(),
        }
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
                    path = %self.cache_path.display(),
                    %error,
                    "ignoring invalid OpenRouter catalog cache"
                );
                None
            }
        }
    }

    /// Returns the selectable coding models from the cached catalog.
    ///
    /// Never touches the network: callers get the last successful refresh, or
    /// an empty list before the first refresh.
    #[must_use]
    pub fn cached_coding_models(&self) -> Vec<Model> {
        self.load_cache()
            .map(|cache| coding_models(&cache))
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
    /// freshness. Any network, timeout, or parse failure returns
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

        let previous = cache.clone().unwrap_or_default();
        let mut next = previous.clone();
        let mut changed = false;
        let mut not_modified = false;

        match self.fetch_models(previous.etag.as_deref()).await {
            Ok(FetchedCatalog::Models { etag, models }) => {
                next.etag = etag;
                next.models = models;
                changed = true;
            }
            Ok(FetchedCatalog::NotModified) => not_modified = true,
            Err(error) => {
                tracing::warn!(%error, "OpenRouter catalog refresh failed; serving cached data");
                return RefreshOutcome::Stale {
                    error: error.to_string(),
                };
            }
        }

        if changed || next.programming.is_empty() {
            match self
                .fetch_programming(previous.programming_etag.as_deref())
                .await
            {
                Ok(FetchedProgramming::Ids { etag, ids }) => {
                    next.programming_etag = etag;
                    next.programming = ids;
                    changed = true;
                }
                Ok(FetchedProgramming::NotModified) => {}
                Err(error) => {
                    tracing::warn!(
                        %error,
                        "OpenRouter programming category refresh failed; serving cached data"
                    );
                }
            }
        }

        if changed {
            next.fetched_at = now;
            if let Err(error) = self.save_cache(&next) {
                tracing::warn!(
                    path = %self.cache_path.display(),
                    %error,
                    "cannot persist the OpenRouter catalog"
                );
            }
            RefreshOutcome::Fetched {
                models: next.models.len(),
            }
        } else if not_modified {
            next.fetched_at = now;
            if let Err(error) = self.save_cache(&next) {
                tracing::warn!(
                    path = %self.cache_path.display(),
                    %error,
                    "cannot persist the OpenRouter catalog"
                );
            }
            RefreshOutcome::NotModified
        } else {
            RefreshOutcome::Stale {
                error: "catalog refresh made no progress".to_owned(),
            }
        }
    }

    /// Starts a refresh in the background and returns immediately.
    ///
    /// This is the startup path: it never blocks the caller, and a failure is
    /// logged rather than propagated (`PROV-021`).
    pub fn spawn_refresh(self: Arc<Self>) {
        tokio::spawn(async move {
            match self.refresh().await {
                RefreshOutcome::Stale { error } => {
                    tracing::warn!(%error, "background catalog refresh failed");
                }
                outcome => {
                    tracing::debug!(?outcome, "background catalog refresh finished");
                }
            }
        });
    }

    async fn fetch_models(&self, etag: Option<&str>) -> Result<FetchedCatalog, CatalogError> {
        let mut request = self.client.get(&self.endpoint);
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
            models: body.data,
        })
    }

    async fn fetch_programming(
        &self,
        etag: Option<&str>,
    ) -> Result<FetchedProgramming, CatalogError> {
        let mut request = self
            .client
            .get(&self.endpoint)
            .query(&[("category", PROGRAMMING_CATEGORY)]);
        if let Some(etag) = etag {
            request = request.header(reqwest::header::IF_NONE_MATCH, etag);
        }

        let response = tokio::time::timeout(self.timeout, request.send())
            .await
            .map_err(|_| CatalogError::Timeout(self.timeout))?
            .map_err(|error| CatalogError::Transport(error.to_string()))?;

        if response.status() == reqwest::StatusCode::NOT_MODIFIED {
            return Ok(FetchedProgramming::NotModified);
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

        Ok(FetchedProgramming::Ids {
            etag,
            ids: body.data.into_iter().map(|model| model.id).collect(),
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

/// Returns the catalog entries that are selectable chat models.
///
/// A model must accept `text` input and produce text output. Entries without
/// architecture metadata are excluded; codie never guesses (`PROV-020`).
#[must_use]
pub fn selectable_models(cache: &CatalogCache) -> Vec<&CatalogModel> {
    cache
        .models
        .iter()
        .filter(|model| is_selectable(model))
        .collect()
}

/// Returns true when `model` is a text-in, text-out chat model.
#[must_use]
pub fn is_selectable(model: &CatalogModel) -> bool {
    let Some(architecture) = &model.architecture else {
        return false;
    };

    let input_text = architecture
        .input_modalities
        .iter()
        .any(|modality| modality.eq_ignore_ascii_case("text"));

    let output_text = if architecture.output_modalities.is_empty() {
        architecture
            .modality
            .as_deref()
            .and_then(|modality| modality.split_once("->"))
            .is_some_and(|(_, output)| {
                output
                    .split('+')
                    .any(|modality| modality.eq_ignore_ascii_case("text"))
            })
    } else {
        architecture
            .output_modalities
            .iter()
            .any(|modality| modality.eq_ignore_ascii_case("text"))
    };

    input_text && output_text
}

/// Returns the coding models to offer in the picker, converted for the adapter.
///
/// The list is ordered as follows: models in OpenRouter's `programming`
/// category in provider order, then scored models above
/// [`CODING_INDEX_THRESHOLD`] ordered by coding index. Variants share their
/// base model ID, so `:batch` and `:free` entries never duplicate a model that
/// is already listed.
#[must_use]
pub fn coding_models(cache: &CatalogCache) -> Vec<Model> {
    let programming = cache
        .programming
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let mut seen = BTreeSet::new();
    let mut models = Vec::new();

    for id in &cache.programming {
        let Some(candidate) = cache.models.iter().find(|model| &model.id == id) else {
            continue;
        };
        if !is_coding_selectable(candidate, &programming) {
            continue;
        }
        if let Some(model) = add_model(candidate, &mut seen) {
            models.push(model);
        }
    }

    let mut scored = cache
        .models
        .iter()
        .filter(|model| {
            !programming.contains(model.id.as_str())
                && is_coding_selectable(model, &programming)
                && !seen.contains(base_id(&model.id))
        })
        .collect::<Vec<_>>();
    scored.sort_by(|left, right| {
        coding_index(right)
            .partial_cmp(&coding_index(left))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    for candidate in scored {
        if let Some(model) = add_model(candidate, &mut seen) {
            models.push(model);
        }
    }

    models
}

/// Returns true when `model` is an offerable coding model.
///
/// A model must be a text chat model that supports tool calling, and must
/// either appear in `programming` or meet [`CODING_INDEX_THRESHOLD`].
#[must_use]
pub fn is_coding_selectable(model: &CatalogModel, programming: &BTreeSet<&str>) -> bool {
    is_selectable(model)
        && supports_tools(model)
        && (programming.contains(model.id.as_str())
            || coding_index(model) >= CODING_INDEX_THRESHOLD)
}

/// Converts one catalog entry into an adapter [`Model`].
#[must_use]
pub fn catalog_model(model: &CatalogModel) -> Model {
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
        provider: PROVIDER_OPENROUTER.into(),
        api: OPENAI_COMPLETIONS.into(),
        base_url: provider_base_url(PROVIDER_OPENROUTER).to_owned(),
        context_window,
        max_output_tokens,
        reasoning: model.reasoning.is_some() || supports_parameter(model, "reasoning"),
        reasoning_mandatory: model
            .reasoning
            .as_ref()
            .is_some_and(|value| value.mandatory),
        thinking_levels: thinking_levels(model),
        cost: catalog_cost(model),
        compat: Compat::openrouter(),
        headers: HeaderMap::new(),
    }
}

/// Merges curated built-ins with catalog models, keeping curated entries first.
///
/// Both lists are deduplicated by model ID, so a curated model always wins over
/// the same model from the catalog.
#[must_use]
pub fn merge_models(curated: Vec<Model>, cached: Vec<Model>) -> Vec<Model> {
    let mut ids = curated
        .iter()
        .map(|model| model.id.clone())
        .collect::<BTreeSet<_>>();
    let mut merged = curated;
    for model in cached {
        if ids.insert(model.id.clone()) {
            merged.push(model);
        }
    }
    merged
}

fn add_model(model: &CatalogModel, seen: &mut BTreeSet<String>) -> Option<Model> {
    if is_variant_alias(&model.id) || !seen.insert(base_id(&model.id).to_owned()) {
        return None;
    }
    Some(catalog_model(model))
}

fn is_variant_alias(id: &str) -> bool {
    id.starts_with('~') || id.ends_with(":batch") || id.contains(":batch:")
}

fn base_id(id: &str) -> &str {
    id.split(':').next().unwrap_or(id)
}

fn supports_tools(model: &CatalogModel) -> bool {
    supports_parameter(model, "tools")
}

fn supports_parameter(model: &CatalogModel, parameter: &str) -> bool {
    model
        .supported_parameters
        .iter()
        .any(|value| value.eq_ignore_ascii_case(parameter))
}

fn coding_index(model: &CatalogModel) -> f64 {
    model
        .benchmarks
        .as_ref()
        .and_then(|benchmarks| benchmarks.artificial_analysis.as_ref())
        .and_then(|index| index.coding_index)
        .filter(|value| value.is_finite())
        .unwrap_or(0.0)
}

fn thinking_levels(model: &CatalogModel) -> BTreeMap<ThinkingLevel, Option<String>> {
    let Some(reasoning) = &model.reasoning else {
        return BTreeMap::new();
    };

    let mut levels = BTreeMap::new();
    if !reasoning.mandatory {
        levels.insert(ThinkingLevel::Off, Some("none".to_owned()));
    }
    for effort in &reasoning.supported_efforts {
        if let Some(level) = ThinkingLevel::parse(effort) {
            levels.insert(level, Some(effort.clone()));
        }
    }
    if reasoning.supported_efforts.is_empty() {
        levels.extend([
            (ThinkingLevel::Minimal, Some("minimal".to_owned())),
            (ThinkingLevel::Low, Some("low".to_owned())),
            (ThinkingLevel::Medium, Some("medium".to_owned())),
            (ThinkingLevel::High, Some("high".to_owned())),
        ]);
    }
    levels
}

fn catalog_cost(model: &CatalogModel) -> ModelCost {
    ModelCost {
        input: price_per_million(model, "prompt"),
        output: price_per_million(model, "completion"),
        cache_read: price_per_million(model, "input_cache_read"),
        cache_write: price_per_million(model, "input_cache_write"),
    }
}

fn price_per_million(model: &CatalogModel, key: &str) -> f64 {
    let Some(value) = model.pricing.as_ref().and_then(|pricing| pricing.get(key)) else {
        return 0.0;
    };
    let per_token = match value {
        serde_json::Value::String(text) => text.parse::<f64>().unwrap_or(0.0),
        serde_json::Value::Number(number) => number.as_f64().unwrap_or(0.0),
        _ => 0.0,
    };
    if per_token.is_finite() && per_token > 0.0 {
        // Round to micro-USD per million tokens so the conversion does not
        // leak binary floating-point noise into displayed prices.
        (per_token * 1_000_000.0 * 1_000_000.0).round() / 1_000_000.0
    } else {
        0.0
    }
}

#[derive(Debug, Deserialize)]
struct ModelsResponse {
    #[serde(default)]
    data: Vec<CatalogModel>,
}

enum FetchedCatalog {
    Models {
        etag: Option<String>,
        models: Vec<CatalogModel>,
    },
    NotModified,
}

enum FetchedProgramming {
    Ids {
        etag: Option<String>,
        ids: Vec<String>,
    },
    NotModified,
}

#[derive(Debug, thiserror::Error)]
enum CatalogError {
    #[error("catalog request timed out after {0:?}")]
    Timeout(Duration),
    #[error("catalog transport error: {0}")]
    Transport(String),
    #[error("catalog endpoint returned status {0}")]
    Status(u16),
    #[error("catalog response is not valid JSON: {0}")]
    Parse(String),
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(id: &str, input: &[&str], output: &[&str]) -> CatalogModel {
        CatalogModel {
            id: id.to_owned(),
            architecture: Some(Architecture {
                input_modalities: input.iter().map(|value| (*value).to_owned()).collect(),
                output_modalities: output.iter().map(|value| (*value).to_owned()).collect(),
                modality: None,
            }),
            ..CatalogModel::default()
        }
    }

    fn coding(id: &str, index: f64) -> CatalogModel {
        let mut model = model(id, &["text"], &["text"]);
        model.supported_parameters = vec!["tools".to_owned()];
        model.benchmarks = Some(Benchmarks {
            artificial_analysis: Some(ArtificialAnalysis {
                coding_index: Some(index),
                ..ArtificialAnalysis::default()
            }),
        });
        model
    }

    #[test]
    fn text_chat_models_are_selectable() {
        assert!(is_selectable(&model("text", &["text"], &["text"])));
        assert!(is_selectable(&model(
            "multimodal",
            &["text", "image"],
            &["text"]
        )));
    }

    #[test]
    fn non_text_models_are_excluded() {
        assert!(!is_selectable(&model(
            "embeddings",
            &["text"],
            &["embeddings"]
        )));
        assert!(!is_selectable(&model("image", &["text"], &["image"])));
        assert!(!is_selectable(&model("audio", &["audio"], &["text"])));
    }

    #[test]
    fn missing_architecture_is_excluded() {
        let model = CatalogModel {
            id: "mystery".to_owned(),
            ..CatalogModel::default()
        };
        assert!(!is_selectable(&model));
    }

    #[test]
    fn modality_string_is_a_fallback() {
        let mut model = model("legacy", &["text"], &[]);
        model.architecture.as_mut().unwrap().modality = Some("text+image->text".to_owned());
        assert!(is_selectable(&model));

        model.architecture.as_mut().unwrap().modality = Some("text->image".to_owned());
        assert!(!is_selectable(&model));
    }

    #[test]
    fn freshness_window_is_four_hours() {
        let now = 10_000_000;
        let fresh = CatalogCache {
            fetched_at: now - 60,
            ..CatalogCache::default()
        };
        let stale = CatalogCache {
            fetched_at: now - CATALOG_FRESHNESS.as_secs() - 1,
            ..CatalogCache::default()
        };
        assert!(fresh.is_fresh(now));
        assert!(!stale.is_fresh(now));
    }

    #[test]
    fn coding_selection_uses_category_or_score() {
        let programming_model = model("vendor/listed", &["text"], &["text"]);
        let mut listed = programming_model.clone();
        listed.supported_parameters = vec!["tools".to_owned()];

        let mut scored = coding("vendor/scored", 70.0);
        scored.name = Some("Scored".to_owned());

        let listed_and_scored = coding("vendor/both", 71.0);

        let programming = BTreeSet::from(["vendor/listed", "vendor/both"]);
        assert!(is_coding_selectable(&listed, &programming));
        assert!(is_coding_selectable(&scored, &programming));
        assert!(is_coding_selectable(&listed_and_scored, &programming));

        let below = coding("vendor/below", 69.9);
        assert!(!is_coding_selectable(&below, &programming));

        let no_tools = model("vendor/listed-no-tools", &["text"], &["text"]);
        let programming = BTreeSet::from(["vendor/listed-no-tools"]);
        assert!(!is_coding_selectable(&no_tools, &programming));
    }

    #[test]
    fn coding_models_order_and_dedupe_variants() {
        let auto = CatalogModel {
            supported_parameters: vec!["tools".to_owned()],
            ..model("openrouter/auto", &["text"], &["text"])
        };

        let mut listed = coding("vendor/listed", 99.0);
        listed.supported_parameters = vec!["tools".to_owned()];
        let mut listed_free = listed.clone();
        listed_free.id = "vendor/listed:free".to_owned();
        let mut batch = coding("vendor/batchy", 80.0);
        batch.id = "vendor/batchy:batch".to_owned();
        let scored = coding("vendor/scored", 75.0);

        let cache = CatalogCache {
            models: vec![
                auto.clone(),
                listed.clone(),
                listed_free.clone(),
                batch.clone(),
                scored.clone(),
            ],
            programming: vec![listed.id.clone(), listed_free.id.clone()],
            ..CatalogCache::default()
        };

        let models = coding_models(&cache);
        let ids = models
            .iter()
            .map(|model| model.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(ids, ["vendor/listed", "vendor/scored"]);
    }

    #[test]
    fn catalog_model_converts_metadata() {
        let mut entry = coding("vendor/model", 81.0);
        entry.name = Some("Vendor Model".to_owned());
        entry.context_length = Some(1_000_000);
        entry.top_provider = Some(TopProvider {
            context_length: Some(1_000_000),
            max_completion_tokens: Some(200_000),
        });
        entry.pricing = Some(serde_json::json!({
            "prompt": "0.0000005",
            "completion": "0.0000015",
            "input_cache_read": "0.00000005",
            "input_cache_write": "0.0000001",
        }));
        entry.reasoning = Some(Reasoning {
            mandatory: false,
            default_enabled: Some(true),
            supported_efforts: vec!["low".to_owned(), "high".to_owned(), "xhigh".to_owned()],
        });

        let model = catalog_model(&entry);
        assert_eq!(model.name, "Vendor Model");
        assert_eq!(model.context_window, 1_000_000);
        assert_eq!(model.max_output_tokens, 200_000);
        assert!(model.reasoning);
        assert!(!model.reasoning_mandatory);
        assert_eq!(model.cost.input, 0.5);
        assert_eq!(model.cost.output, 1.5);
        assert_eq!(model.cost.cache_read, 0.05);
        assert_eq!(model.cost.cache_write, 0.1);
        assert_eq!(
            model.thinking_effort(ThinkingLevel::High),
            Some("high".to_owned())
        );
        assert_eq!(model.thinking_effort(ThinkingLevel::Off), None);
        assert!(model.supports_thinking_level(ThinkingLevel::Off));
    }

    #[test]
    fn negative_prices_and_missing_limits_fall_back() {
        let mut entry = coding("openrouter/auto", 0.0);
        entry.pricing = Some(serde_json::json!({
            "prompt": "-1",
            "completion": "-1",
        }));
        let model = catalog_model(&entry);
        assert_eq!(model.cost, ModelCost::default());
        assert_eq!(model.context_window, FALLBACK_CONTEXT_WINDOW);
        assert_eq!(model.max_output_tokens, FALLBACK_MAX_OUTPUT_TOKENS);
    }

    #[test]
    fn mandatory_reasoning_cannot_be_disabled() {
        let mut entry = coding("vendor/reasoner", 0.0);
        entry.reasoning = Some(Reasoning {
            mandatory: true,
            default_enabled: Some(true),
            supported_efforts: vec!["low".to_owned(), "high".to_owned()],
        });
        let model = catalog_model(&entry);
        assert!(model.reasoning_mandatory);
        assert!(!model.supports_thinking_level(ThinkingLevel::Off));
        assert!(!model.thinking_levels.contains_key(&ThinkingLevel::Off));
    }

    #[test]
    fn merge_keeps_curated_first_and_dedupes() {
        let curated = crate::model::builtin_models()
            .into_iter()
            .filter(|model| model.provider.as_ref() == PROVIDER_OPENROUTER)
            .collect::<Vec<_>>();
        let mut duplicate = curated[1].clone();
        duplicate.name = "From catalog".to_owned();
        let mut extra = curated[1].clone();
        extra.id = "vendor/extra".to_owned();

        let merged = merge_models(curated.clone(), vec![duplicate, extra]);
        assert_eq!(merged[0].id, "openrouter/auto");
        assert_eq!(merged[1].name, curated[1].name);
        assert!(merged.iter().any(|model| model.id == "vendor/extra"));
        assert_eq!(merged.len(), curated.len() + 1);
    }
}
