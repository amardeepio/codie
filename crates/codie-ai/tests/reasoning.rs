//! Reasoning replay tests (`PROV-016`).

mod common;

use codie_ai::{Compat, HeaderMap, OpenAiProvider};
use codie_core::content::{Content, StopReason, ThinkingLevel};
use codie_core::message::Message;
use codie_testkit::spec;
use serde_json::json;

#[test]
#[spec("PROV-016")]
fn spec_prov_016_reasoning_replay() {
    let deepseek = common::deepseek_model();
    let client = OpenAiProvider::new(
        "deepseek",
        "https://example.invalid",
        "test-key",
        vec![deepseek.clone()],
        Compat::deepseek(),
        HeaderMap::new(),
    );
    let assistant = Message::assistant(
        codie_ai::OPENAI_COMPLETIONS,
        "deepseek",
        "deepseek-flash",
        vec![Content::thinking("private plan"), Content::text("visible")],
        StopReason::Stop,
    );
    let body = client.request_body(
        &[assistant],
        &common::options(&deepseek, ThinkingLevel::Off),
    );
    assert_eq!(
        body["messages"][0]["reasoning_content"],
        json!("private plan")
    );
    assert_eq!(body["messages"][0]["content"], json!("visible"));

    let assistant = Message::assistant(
        codie_ai::OPENAI_COMPLETIONS,
        "deepseek",
        "deepseek-flash",
        vec![Content::text("visible")],
        StopReason::Stop,
    );
    let body = client.request_body(
        &[assistant],
        &common::options(&deepseek, ThinkingLevel::Off),
    );
    assert_eq!(body["messages"][0]["reasoning_content"], json!(""));

    let mut openrouter = common::openrouter_model();
    openrouter.reasoning = true;
    let client = OpenAiProvider::new(
        "openrouter",
        "https://example.invalid",
        "test-key",
        vec![openrouter.clone()],
        Compat::openrouter(),
        HeaderMap::new(),
    );
    let assistant = Message::assistant(
        codie_ai::OPENAI_COMPLETIONS,
        "openrouter",
        "openrouter/auto",
        vec![Content::thinking("private plan"), Content::text("visible")],
        StopReason::Stop,
    );
    let body = client.request_body(
        &[assistant],
        &common::options(&openrouter, ThinkingLevel::Off),
    );
    assert!(body["messages"][0].get("reasoning_content").is_none());
    assert_eq!(body["messages"][0]["content"], json!("visible"));
}
