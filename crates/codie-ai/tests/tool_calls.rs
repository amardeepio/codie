//! Tool-call assembly tests (`PROV-013`).

mod common;

use codie_ai::{Compat, HeaderMap, OpenAiProvider};
use codie_core::agent::ChatClient;
use codie_core::content::{StopReason, ThinkingLevel};
use codie_core::event::AssistantDelta;
use codie_core::message::Message;
use codie_testkit::spec;
use serde_json::json;

fn provider(server: &wiremock::MockServer) -> (OpenAiProvider, codie_ai::Model) {
    let model = common::deepseek_model();
    let provider = OpenAiProvider::new(
        "deepseek",
        server.uri(),
        "test-key",
        vec![model.clone()],
        Compat::deepseek(),
        HeaderMap::new(),
    );
    (provider, model)
}

#[tokio::test]
#[spec("PROV-013")]
async fn spec_prov_013_tool_call_accumulation() {
    let server = wiremock::MockServer::start().await;
    let body = common::script(&[
        r#"{"id":"resp-1","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"type":"function","function":{"name":"read","arguments":"{\"path\":"}}]}}]}"#,
        r#"{"id":"resp-1","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"arguments":"\"src/lib.rs\"}"}}]}}]}"#,
        r#"{"id":"resp-1","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}"#,
        r#"{"id":"resp-1","choices":[],"usage":{"prompt_tokens":10,"completion_tokens":5}}"#,
    ]);
    common::mount_sse(&server, body).await;

    let (provider, model) = provider(&server);
    let events = common::drain(provider.stream(
        &[Message::user("read the file")],
        &common::options(&model, ThinkingLevel::Off),
    ))
    .await;

    assert!(matches!(events.first(), Some(AssistantDelta::Start { .. })));
    assert!(
        events
            .iter()
            .any(|event| matches!(event, AssistantDelta::ToolCallStart { index: 0 }))
    );
    assert_eq!(common::deltas_for(&events, 0), "{\"path\":\"src/lib.rs\"}");

    let tool_end = events
        .iter()
        .find_map(|event| match event {
            AssistantDelta::ToolCallEnd { index, call } => Some((*index, call)),
            _ => None,
        })
        .expect("the tool call should end");
    assert_eq!(tool_end.0, 0);
    assert_eq!(tool_end.1.id, "call_1");
    assert_eq!(tool_end.1.name, "read");
    assert_eq!(tool_end.1.arguments, json!({"path": "src/lib.rs"}));

    let message = common::terminal(&events);
    assert_eq!(message.stop_reason, StopReason::ToolUse);
    assert_eq!(message.tool_calls().len(), 1);
    assert_eq!(message.tool_calls()[0].id, "call_1");
}

#[tokio::test]
#[spec("PROV-013")]
async fn spec_prov_013_invalid_arguments_are_recoverable() {
    let server = wiremock::MockServer::start().await;
    let body = common::script(&[
        r#"{"id":"resp-1","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_bad","type":"function","function":{"name":"read","arguments":"{not json"}}]}}]}"#,
        r#"{"id":"resp-1","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}"#,
    ]);
    common::mount_sse(&server, body).await;

    let (provider, model) = provider(&server);
    let events = common::drain(provider.stream(
        &[Message::user("read the file")],
        &common::options(&model, ThinkingLevel::Off),
    ))
    .await;

    let tool_end = events
        .iter()
        .find_map(|event| match event {
            AssistantDelta::ToolCallEnd { call, .. } => Some(call.clone()),
            _ => None,
        })
        .expect("the tool call should end");
    assert_eq!(tool_end.arguments, json!({}));

    let message = common::terminal(&events);
    let note = common::first_text(message);
    assert!(
        note.contains("malformed tool-call arguments"),
        "note: {note}"
    );
    assert_eq!(message.tool_calls().len(), 1);
}
