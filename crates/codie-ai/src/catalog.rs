//! OpenRouter model-catalog refresh and selection filtering.
//!
//! OpenRouter publishes a live model catalog with modality and pricing
//! metadata. codie keeps a small on-disk cache so startup never waits for the
//! network: a refresh is best-effort, bounded by a four-second timeout, and
//! serves the previous cache when it fails (`PROV-021`). Models that are not
//! text-in/text-out chat models are excluded from selection rather than
//! coerced (`PROV-020`).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// Maximum time a catalog refresh may take before it is abandoned.
pub const CATALOG_TIMEOUT: Duration = Duration::from_secs(4);

/// How long a cached catalog is considered fresh.
pub const CATALOG_FRESHNESS: Duration = Duration::from_secs(4 * 60 * 60);

/// Default OpenRouter catalog endpoint.
pub const OPENROUTER_MODELS_URL: &str = "https://openrouter.ai/api/v1/models";

/// One model entry as published by OpenRouter.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CatalogModel {
    /// Provider-scoped model identifier, used verbatim.
    pub id: String,
    /// Human-readable name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Context window in tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_length: Option<u32>,
    /// Modality metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub architecture: Option<Architecture>,
    /// Raw pricing object; values are strings in the provider response.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pricing: Option<serde_json::Value>,
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

/// The cached catalog file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CatalogCache {
    /// ETag returned by the provider, used for conditional requests.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub etag: Option<String>,
    /// Unix seconds when the catalog was last confirmed fresh.
    pub fetched_at: u64,
    /// Every model returned by the provider.
    #[serde(default)]
    pub models: Vec<CatalogModel>,
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

        match self.fetch(cache.as_ref()).await {
            Ok(FetchedCatalog::Models(cache)) => {
                let count = cache.models.len();
                if let Err(error) = self.save_cache(&cache) {
                    tracing::warn!(
                        path = %self.cache_path.display(),
                        %error,
                        "cannot persist the OpenRouter catalog"
                    );
                }
                RefreshOutcome::Fetched { models: count }
            }
            Ok(FetchedCatalog::NotModified) => {
                if let Some(mut cache) = cache {
                    cache.fetched_at = now;
                    if let Err(error) = self.save_cache(&cache) {
                        tracing::warn!(
                            path = %self.cache_path.display(),
                            %error,
                            "cannot persist the OpenRouter catalog"
                        );
                    }
                }
                RefreshOutcome::NotModified
            }
            Err(error) => {
                tracing::warn!(%error, "OpenRouter catalog refresh failed; serving cached data");
                RefreshOutcome::Stale {
                    error: error.to_string(),
                }
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

    async fn fetch(&self, cache: Option<&CatalogCache>) -> Result<FetchedCatalog, CatalogError> {
        let mut request = self.client.get(&self.endpoint);
        if let Some(etag) = cache.and_then(|cache| cache.etag.as_deref()) {
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

        Ok(FetchedCatalog::Models(CatalogCache {
            etag,
            fetched_at: unix_now(),
            models: body.data,
        }))
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

#[derive(Debug, Deserialize)]
struct ModelsResponse {
    #[serde(default)]
    data: Vec<CatalogModel>,
}

enum FetchedCatalog {
    Models(CatalogCache),
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
}
