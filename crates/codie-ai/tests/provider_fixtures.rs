//! Conformance against recorded provider streams (`P1-T16`).
//!
//! The fixtures mirror the shape of a real DeepSeek and OpenRouter stream:
//! reasoning content, incremental tool-call arguments, usage, and an
//! OpenRouter-reported cost. They are replayed through the real adapter over
//! HTTP, so CI stays offline while the wire path is exercised end to end.

mod common;

use codie_ai::{Compat, HeaderMap, OpenAiProvider};
use codie_core::agent::ChatClient;
use codie_core::content::{StopReason, ThinkingLevel};
use codie_core::message::Message;
use codie_testkit::spec;

#[tokio::test]
#[spec("PROV-013")]
async fn spec_prov_013_deepseek_recorded_fixture_replays_reasoning_and_tool_call() {
    let server = wiremock::MockServer::start().await;
    common::mount_sse(
        &server,
        include_str!("fixtures/deepseek_recorded.sse").to_owned(),
    )
    .await;

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
        &[Message::user("read math.py")],
        &common::options(&model, ThinkingLevel::Off),
    ))
    .await;

    let message = common::terminal(&events);
    assert_eq!(message.stop_reason, StopReason::ToolUse);
    assert_eq!(message.text(), "Reading the file.");
    assert!(
        message
            .content
            .iter()
            .any(|content| matches!(content, codie_core::Content::Thinking { thinking, .. } if thinking.contains("read the file")))
    );
    let calls = message.tool_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "read");
    assert_eq!(calls[0].arguments["path"], "math.py");
    assert_eq!(message.usage.input, 100);
    assert_eq!(message.usage.cache_read, 20);
    assert_eq!(message.usage.output, 30);
    assert_eq!(message.usage.reasoning, Some(10));
}

#[tokio::test]
#[spec("PROV-015")]
async fn spec_prov_015_openrouter_recorded_fixture_replays_cost_and_usage() {
    let server = wiremock::MockServer::start().await;
    common::mount_sse(
        &server,
        include_str!("fixtures/openrouter_recorded.sse").to_owned(),
    )
    .await;

    let model = common::openrouter_model();
    let provider = OpenAiProvider::new(
        "openrouter",
        server.uri(),
        "test-key",
        vec![model.clone()],
        Compat::openrouter(),
        HeaderMap::new(),
    );
    let events = common::drain(provider.stream(
        &[Message::user("hello")],
        &common::options(&model, ThinkingLevel::Off),
    ))
    .await;

    let message = common::terminal(&events);
    assert_eq!(message.stop_reason, StopReason::Stop);
    assert_eq!(message.text(), "The test passes.");
    assert_eq!(
        message.response_id.as_deref(),
        Some("or-recorded-1"),
        "recorded response id is preserved"
    );
    assert_eq!(message.usage.input, 50);
    assert_eq!(message.usage.output, 8);
    assert_eq!(message.usage.total_tokens, 58);
    assert_eq!(message.usage.cost.total, 123_000);
    assert!(message.usage.cost.total_from_provider);
}
