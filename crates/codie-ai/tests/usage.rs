//! Usage normalization and cost accounting tests (`PROV-015`).

mod common;

use codie_ai::model::ModelCost;
use codie_ai::{Compat, HeaderMap, OpenAiProvider, estimate_cost, normalize_usage};
use codie_core::agent::ChatClient;
use codie_core::content::{ThinkingLevel, Usage};
use codie_core::message::Message;
use codie_testkit::spec;
use serde_json::json;

const COST: ModelCost = ModelCost {
    input: 0.30,
    output: 1.20,
    cache_read: 0.006,
    cache_write: 0.0,
};

#[test]
#[spec("PROV-015")]
fn spec_prov_015_usage_mapping() {
    let deepseek = json!({
        "prompt_tokens": 1200,
        "completion_tokens": 80,
        "prompt_cache_hit_tokens": 900,
        "completion_tokens_details": {"reasoning_tokens": 20}
    });
    let usage = normalize_usage("deepseek", &deepseek, &COST);
    assert_eq!(usage.input, 300);
    assert_eq!(usage.cache_read, 900);
    assert_eq!(usage.output, 80);
    assert_eq!(usage.cache_write, 0);
    assert_eq!(usage.reasoning, Some(20));
    assert_eq!(usage.total_tokens, 1280);
    assert!(!usage.cost.total_from_provider);
    assert_eq!(usage.cost.total, 191_400);

    let openrouter = json!({
        "prompt_tokens": 1200,
        "completion_tokens": 80,
        "prompt_tokens_details": {"cached_tokens": 900},
        "completion_tokens_details": {"reasoning_tokens": 20},
        "cost": 0.00042,
        "is_byok": true,
        "cost_details": {"upstream_inference_cost": 0.0003}
    });
    let usage = normalize_usage("openrouter", &openrouter, &COST);
    assert_eq!(usage.input, 300);
    assert_eq!(usage.cache_read, 900);
    assert_eq!(usage.output, 80);
    assert_eq!(usage.reasoning, Some(20));
    assert_eq!(usage.total_tokens, 1280);
    assert_eq!(usage.cost.total, 420_000);
    assert!(usage.cost.total_from_provider);
}

#[test]
#[spec("PROV-015")]
fn spec_prov_015_missing_usage_is_zero() {
    let usage = normalize_usage("deepseek", &json!({}), &COST);
    assert_eq!(usage, Usage::default());
    assert_eq!(usage.total_tokens, 0);
}

#[test]
#[spec("MSG-005")]
fn spec_msg_005_total_never_counts_reasoning_twice() {
    let raw = json!({
        "prompt_tokens": 100,
        "completion_tokens": 50,
        "completion_tokens_details": {"reasoning_tokens": 30}
    });
    let usage = normalize_usage("deepseek", &raw, &COST);
    assert_eq!(usage.reasoning, Some(30));
    assert_eq!(usage.output, 50);
    assert_eq!(
        usage.total_tokens,
        usage.input + usage.output + usage.cache_read + usage.cache_write
    );
    assert_eq!(usage.total_tokens, 150);
}

#[test]
#[spec("MSG-006")]
fn spec_msg_006_cost_is_integer_nano_usd() {
    let usage = Usage::new(1_000_000, 1_000_000, 0, 0);
    let estimated = estimate_cost(&COST, &usage);
    assert_eq!(estimated.input, 300_000_000);
    assert_eq!(estimated.output, 1_200_000_000);
    assert_eq!(estimated.total, 1_500_000_000);
    assert!(!estimated.total_from_provider);

    let provider_cost = normalize_usage(
        "openrouter",
        &json!({"prompt_tokens": 1, "cost": 0.00042}),
        &COST,
    );
    assert_eq!(provider_cost.cost.total, 420_000);
    assert!(provider_cost.cost.total_from_provider);
}

#[tokio::test]
#[spec("PROV-015")]
async fn spec_prov_015_usage_is_attached_at_done() {
    let server = wiremock::MockServer::start().await;
    common::mount_sse(&server, common::mixed_stream()).await;

    let model = common::deepseek_model();
    let provider = OpenAiProvider::new(
        "deepseek",
        server.uri(),
        "test-key",
        vec![model.clone()],
        Compat::deepseek(),
        HeaderMap::new(),
    );
    let events = common::drain(provider.stream(
        &[Message::user("hi")],
        &common::options(&model, ThinkingLevel::Off),
    ))
    .await;

    let message = common::terminal(&events);
    assert_eq!(message.usage.input, 300);
    assert_eq!(message.usage.cache_read, 900);
    assert_eq!(message.usage.output, 80);
    assert_eq!(message.usage.reasoning, Some(20));
    assert_eq!(message.usage.total_tokens, 1280);
    assert!(!message.usage.cost.total_from_provider);
}
