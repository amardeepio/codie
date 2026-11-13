//! OpenRouter catalog refresh, coding filter, and conversion tests
//! (`PROV-020`, `PROV-021`).

use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use codie_ai::{
    CATALOG_FRESHNESS, CODING_INDEX_THRESHOLD, CatalogCache, OpenRouterCatalog,
    PROGRAMMING_CATEGORY, RefreshOutcome, coding_models, selectable_models,
};
use codie_testkit::spec;
use serde_json::json;
use wiremock::matchers::{method, path, query_param, query_param_is_missing};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn catalog_body() -> serde_json::Value {
    json!({
        "data": [
            {
                "id": "deepseek/deepseek-v4.1-flash-1017",
                "name": "DeepSeek V4.1 Flash",
                "context_length": 1_000_000,
                "architecture": {
                    "input_modalities": ["text"],
                    "output_modalities": ["text"],
                    "modality": "text->text"
                },
                "supported_parameters": ["tools", "reasoning"],
                "pricing": {
                    "prompt": "0.0000003",
                    "completion": "0.0000012"
                },
                "top_provider": {
                    "context_length": 1_000_000,
                    "max_completion_tokens": 384_000
                },
                "reasoning": {
                    "mandatory": false,
                    "supported_efforts": ["low", "medium", "high"]
                }
            },
            {
                "id": "z-ai/glm-5.3",
                "name": "GLM 5.3",
                "context_length": 1_048_576,
                "architecture": {
                    "input_modalities": ["text"],
                    "output_modalities": ["text"]
                },
                "supported_parameters": ["tools"],
                "benchmarks": {
                    "artificial_analysis": { "coding_index": 74.8 }
                }
            },
            {
                "id": "openai/gpt-6-astra",
                "name": "GPT-6 Astra",
                "context_length": 1_050_000,
                "architecture": {
                    "input_modalities": ["text", "image"],
                    "output_modalities": ["text"]
                },
                "supported_parameters": ["tools", "reasoning"],
                "pricing": {
                    "prompt": "0.00001",
                    "completion": "0.00005",
                    "input_cache_read": "0.000001"
                },
                "top_provider": {
                    "context_length": 1_050_000,
                    "max_completion_tokens": 500_000
                },
                "benchmarks": {
                    "artificial_analysis": { "coding_index": 76.9 }
                },
                "reasoning": {
                    "mandatory": true,
                    "supported_efforts": ["low", "high", "xhigh"]
                }
            },
            {
                "id": "vendor/listed",
                "name": "Listed Coding Model",
                "context_length": 200_000,
                "architecture": {
                    "input_modalities": ["text"],
                    "output_modalities": ["text"]
                },
                "supported_parameters": ["tools"]
            },
            {
                "id": "vendor/below",
                "name": "Below Threshold",
                "architecture": {
                    "input_modalities": ["text"],
                    "output_modalities": ["text"]
                },
                "supported_parameters": ["tools"],
                "benchmarks": {
                    "artificial_analysis": { "coding_index": 60.0 }
                }
            },
            {
                "id": "vendor/no-tools",
                "name": "No Tools",
                "architecture": {
                    "input_modalities": ["text"],
                    "output_modalities": ["text"]
                },
                "benchmarks": {
                    "artificial_analysis": { "coding_index": 90.0 }
                }
            },
            {
                "id": "vendor/batch",
                "name": "Batch Base",
                "architecture": {
                    "input_modalities": ["text"],
                    "output_modalities": ["text"]
                },
                "supported_parameters": ["tools"],
                "benchmarks": {
                    "artificial_analysis": { "coding_index": 80.0 }
                }
            },
            {
                "id": "vendor/batch:batch",
                "name": "Batch Variant",
                "architecture": {
                    "input_modalities": ["text"],
                    "output_modalities": ["text"]
                },
                "supported_parameters": ["tools"],
                "benchmarks": {
                    "artificial_analysis": { "coding_index": 80.0 }
                }
            },
            {
                "id": "deepseek/deepseek-v4.1-flash-1017:free",
                "name": "DeepSeek V4.1 Flash (free)",
                "architecture": {
                    "input_modalities": ["text"],
                    "output_modalities": ["text"]
                },
                "supported_parameters": ["tools"]
            },
            {
                "id": "~stealth/alias",
                "name": "Stealth Alias",
                "architecture": {
                    "input_modalities": ["text"],
                    "output_modalities": ["text"]
                },
                "supported_parameters": ["tools"],
                "benchmarks": {
                    "artificial_analysis": { "coding_index": 90.0 }
                }
            },
            {
                "id": "openai/text-embedding-3-large",
                "name": "Embedding",
                "architecture": {
                    "input_modalities": ["text"],
                    "output_modalities": ["embeddings"],
                    "modality": "text->embeddings"
                }
            },
            {
                "id": "openai/dall-e-3",
                "name": "Image generator",
                "architecture": {
                    "input_modalities": ["text"],
                    "output_modalities": ["image"],
                    "modality": "text->image"
                }
            }
        ]
    })
}

fn programming_body() -> serde_json::Value {
    json!({
        "data": [
            { "id": "deepseek/deepseek-v4.1-flash-1017" },
            { "id": "vendor/listed" }
        ]
    })
}

fn endpoint(server: &MockServer) -> String {
    format!("{}/models", server.uri())
}

fn write_cache(path: &Path, cache: &CatalogCache) {
    std::fs::create_dir_all(path.parent().expect("cache parent")).expect("cache dir");
    std::fs::write(path, serde_json::to_vec(cache).expect("cache json")).expect("write cache");
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

async fn mount_catalog(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/models"))
        .and(query_param("category", PROGRAMMING_CATEGORY))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("etag", "\"programming-v1\"")
                .set_body_json(programming_body()),
        )
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/models"))
        .and(query_param_is_missing("category"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("etag", "\"catalog-v1\"")
                .set_body_json(catalog_body()),
        )
        .mount(server)
        .await;
}

#[tokio::test]
#[spec("PROV-021")]
async fn spec_prov_021_catalog_refresh_fetches_and_caches_etag() {
    let server = MockServer::start().await;
    mount_catalog(&server).await;

    let home = tempfile::tempdir().expect("temp dir");
    let cache_path = home.path().join("openrouter-models.json");
    let catalog = OpenRouterCatalog::new(&cache_path)
        .with_endpoint(endpoint(&server))
        .with_timeout(Duration::from_secs(1));

    assert_eq!(
        catalog.refresh().await,
        RefreshOutcome::Fetched { models: 12 }
    );
    let cache = catalog.load_cache().expect("cache written");
    assert_eq!(cache.etag.as_deref(), Some("\"catalog-v1\""));
    assert_eq!(
        cache.programming_etag.as_deref(),
        Some("\"programming-v1\"")
    );
    assert_eq!(
        cache.programming,
        ["deepseek/deepseek-v4.1-flash-1017", "vendor/listed"]
    );
    assert_eq!(cache.models.len(), 12);
    assert!(cache.is_fresh(now()));
}

#[tokio::test]
#[spec("PROV-021")]
async fn spec_prov_021_not_modified_extends_freshness() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/models"))
        .and(query_param_is_missing("category"))
        .respond_with(|request: &wiremock::Request| {
            let etag = request
                .headers
                .get("if-none-match")
                .and_then(|value| value.to_str().ok());
            if etag == Some("\"catalog-v1\"") {
                ResponseTemplate::new(304)
            } else {
                ResponseTemplate::new(200)
                    .insert_header("etag", "\"catalog-v1\"")
                    .set_body_json(catalog_body())
            }
        })
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/models"))
        .and(query_param("category", PROGRAMMING_CATEGORY))
        .respond_with(|request: &wiremock::Request| {
            let etag = request
                .headers
                .get("if-none-match")
                .and_then(|value| value.to_str().ok());
            if etag == Some("\"programming-v1\"") {
                ResponseTemplate::new(304)
            } else {
                ResponseTemplate::new(200)
                    .insert_header("etag", "\"programming-v1\"")
                    .set_body_json(programming_body())
            }
        })
        .mount(&server)
        .await;

    let home = tempfile::tempdir().expect("temp dir");
    let cache_path = home.path().join("openrouter-models.json");
    write_cache(
        &cache_path,
        &CatalogCache {
            etag: Some("\"catalog-v1\"".to_owned()),
            programming_etag: Some("\"programming-v1\"".to_owned()),
            fetched_at: now() - CATALOG_FRESHNESS.as_secs() - 10,
            models: Vec::new(),
            programming: vec!["vendor/listed".to_owned()],
        },
    );

    let catalog = OpenRouterCatalog::new(&cache_path)
        .with_endpoint(endpoint(&server))
        .with_timeout(Duration::from_secs(1));

    assert_eq!(catalog.refresh().await, RefreshOutcome::NotModified);
    let cache = catalog.load_cache().expect("cache still present");
    assert!(
        cache.is_fresh(now()),
        "a 304 must extend the freshness window"
    );
}

#[tokio::test]
#[spec("PROV-021")]
async fn spec_prov_021_refresh_failure_serves_stale_cache() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/models"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let home = tempfile::tempdir().expect("temp dir");
    let cache_path = home.path().join("openrouter-models.json");
    let stale = CatalogCache {
        etag: None,
        programming_etag: None,
        fetched_at: now() - CATALOG_FRESHNESS.as_secs() - 10,
        models: vec![codie_ai::CatalogModel {
            id: "cached/model".to_owned(),
            ..codie_ai::CatalogModel::default()
        }],
        programming: vec!["cached/model".to_owned()],
    };
    write_cache(&cache_path, &stale);

    let catalog = OpenRouterCatalog::new(&cache_path)
        .with_endpoint(endpoint(&server))
        .with_timeout(Duration::from_secs(1));

    let outcome = catalog.refresh().await;
    assert!(
        matches!(outcome, RefreshOutcome::Stale { .. }),
        "a failure must be reported as stale, not fatal: {outcome:?}"
    );
    assert_eq!(
        catalog.load_cache().expect("stale cache survives"),
        stale,
        "a failed refresh must not delete the stale cache"
    );
}

#[tokio::test]
#[spec("PROV-021")]
async fn spec_prov_021_catalog_refresh_is_non_blocking() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/models"))
        .respond_with(|_request: &wiremock::Request| {
            std::thread::sleep(Duration::from_millis(400));
            ResponseTemplate::new(200).set_body_json(catalog_body())
        })
        .mount(&server)
        .await;

    let home = tempfile::tempdir().expect("temp dir");
    let catalog = std::sync::Arc::new(
        OpenRouterCatalog::new(home.path().join("openrouter-models.json"))
            .with_endpoint(endpoint(&server))
            .with_timeout(Duration::from_secs(5)),
    );

    let started = Instant::now();
    catalog.spawn_refresh();
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_millis(100),
        "spawn_refresh blocked for {elapsed:?}"
    );
}

#[test]
#[spec("PROV-020")]
fn spec_prov_020_model_id_and_filtering() {
    let response: serde_json::Value = catalog_body();
    let cache: CatalogCache =
        serde_json::from_value(json!({ "fetched_at": 0, "models": response["data"] }))
            .expect("catalog parses");

    let selectable = selectable_models(&cache);
    let ids = selectable
        .iter()
        .map(|model| model.id.as_str())
        .collect::<Vec<_>>();
    assert!(ids.contains(&"deepseek/deepseek-v4.1-flash-1017"));
    assert!(
        !ids.iter().any(|id| id.contains("embedding")),
        "non-chat models must be excluded"
    );
    assert!(
        !ids.iter().any(|id| id.contains("dall-e")),
        "image models must be excluded"
    );
}

#[test]
#[spec("PROV-020")]
fn spec_prov_020_coding_models_filter_and_order() {
    let response: serde_json::Value = catalog_body();
    let cache: CatalogCache = serde_json::from_value(json!({
        "fetched_at": 0,
        "models": response["data"],
        "programming": ["deepseek/deepseek-v4.1-flash-1017", "vendor/listed"],
    }))
    .expect("catalog parses");

    let models = coding_models(&cache);
    let ids = models
        .iter()
        .map(|model| model.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        ids,
        [
            "deepseek/deepseek-v4.1-flash-1017",
            "vendor/listed",
            "vendor/batch",
            "openai/gpt-6-astra",
            "z-ai/glm-5.3",
        ],
        "programming order first, then scored models descending"
    );
    assert!(
        !ids.iter().any(|id| id.contains("below")),
        "scores under {CODING_INDEX_THRESHOLD} must be excluded"
    );
    assert!(
        !ids.iter().any(|id| id.contains("no-tools")),
        "models without tool support must be excluded"
    );
    assert!(
        !ids.iter()
            .any(|id| id.contains(":batch") || id.starts_with('~')),
        "variant aliases must be excluded"
    );

    let astra = models
        .iter()
        .find(|model| model.id == "openai/gpt-6-astra")
        .expect("astra is selectable");
    assert_eq!(astra.context_window, 1_050_000);
    assert_eq!(astra.max_output_tokens, 500_000);
    assert_eq!(astra.cost.input, 10.0);
    assert_eq!(astra.cost.output, 50.0);
    assert_eq!(astra.cost.cache_read, 1.0);
    assert!(astra.reasoning_mandatory);
    assert!(!astra.supports_thinking_level(codie_core::ThinkingLevel::Off));

    let flash = models
        .iter()
        .find(|model| model.id == "deepseek/deepseek-v4.1-flash-1017")
        .expect("flash is selectable");
    assert_eq!(flash.cost.input, 0.3);
    assert_eq!(flash.cost.output, 1.2);
    assert!(flash.supports_thinking_level(codie_core::ThinkingLevel::Off));
}

#[test]
fn catalog_parses_without_new_optional_fields() {
    let cache: CatalogCache = serde_json::from_value(json!({
        "fetched_at": 0,
        "models": [{ "id": "legacy/model" }],
    }))
    .expect("older caches stay readable");
    assert_eq!(cache.models.len(), 1);
    assert!(cache.programming.is_empty());
}
