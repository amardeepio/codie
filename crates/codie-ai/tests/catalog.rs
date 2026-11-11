//! OpenRouter catalog refresh and selection tests (`PROV-020`, `PROV-021`).

use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use codie_ai::{
    CATALOG_FRESHNESS, CatalogCache, OpenRouterCatalog, RefreshOutcome, selectable_models,
};
use codie_testkit::spec;
use serde_json::json;
use wiremock::matchers::{method, path};
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

#[tokio::test]
#[spec("PROV-021")]
async fn spec_prov_021_catalog_refresh_fetches_and_caches_etag() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/models"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("etag", "\"catalog-v1\"")
                .set_body_json(catalog_body()),
        )
        .expect(1)
        .mount(&server)
        .await;

    let home = tempfile::tempdir().expect("temp dir");
    let cache_path = home.path().join("openrouter-models.json");
    let catalog = OpenRouterCatalog::new(&cache_path)
        .with_endpoint(endpoint(&server))
        .with_timeout(Duration::from_secs(1));

    assert_eq!(
        catalog.refresh().await,
        RefreshOutcome::Fetched { models: 3 }
    );
    let cache = catalog.load_cache().expect("cache written");
    assert_eq!(cache.etag.as_deref(), Some("\"catalog-v1\""));
    assert_eq!(cache.models.len(), 3);
    assert!(cache.is_fresh(now()));
}

#[tokio::test]
#[spec("PROV-021")]
async fn spec_prov_021_not_modified_extends_freshness() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/models"))
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
        .expect(1)
        .mount(&server)
        .await;

    let home = tempfile::tempdir().expect("temp dir");
    let cache_path = home.path().join("openrouter-models.json");
    write_cache(
        &cache_path,
        &CatalogCache {
            etag: Some("\"catalog-v1\"".to_owned()),
            fetched_at: now() - CATALOG_FRESHNESS.as_secs() - 10,
            models: Vec::new(),
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
        .expect(1)
        .mount(&server)
        .await;

    let home = tempfile::tempdir().expect("temp dir");
    let cache_path = home.path().join("openrouter-models.json");
    let stale = CatalogCache {
        etag: None,
        fetched_at: now() - CATALOG_FRESHNESS.as_secs() - 10,
        models: vec![codie_ai::CatalogModel {
            id: "cached/model".to_owned(),
            ..codie_ai::CatalogModel::default()
        }],
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
async fn spec_prov_021_background_refresh_does_not_block() {
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
    assert_eq!(ids, ["deepseek/deepseek-v4.1-flash-1017"]);
    assert!(
        !ids.iter().any(|id| id.contains("embedding")),
        "non-chat models must be excluded"
    );
}
