//! Stop-reason mapping tests (`PROV-014`).

mod common;

use codie_ai::{Compat, HeaderMap, OpenAiProvider, map_stop_reason};
use codie_core::agent::ChatClient;
use codie_core::content::{StopReason, ThinkingLevel};
use codie_core::message::Message;
use codie_testkit::spec;

#[test]
#[spec("PROV-014")]
fn spec_prov_014_stop_reason_mapping() {
    assert_eq!(map_stop_reason(None), Ok(StopReason::Stop));
    assert_eq!(map_stop_reason(Some("stop")), Ok(StopReason::Stop));
    assert_eq!(map_stop_reason(Some("end_turn")), Ok(StopReason::Stop));
    assert_eq!(map_stop_reason(Some("length")), Ok(StopReason::Length));
    assert_eq!(map_stop_reason(Some("max_tokens")), Ok(StopReason::Length));
    assert_eq!(map_stop_reason(Some("tool_calls")), Ok(StopReason::ToolUse));
    assert_eq!(
        map_stop_reason(Some("function_call")),
        Ok(StopReason::ToolUse)
    );

    let content_filter = map_stop_reason(Some("content_filter")).unwrap_err();
    assert!(content_filter.contains("content_filter"));
    let unknown = map_stop_reason(Some("something_new")).unwrap_err();
    assert!(unknown.contains("something_new"));
}

#[tokio::test]
#[spec("PROV-014")]
async fn spec_prov_014_unknown_reason_is_an_error_with_raw_value() {
    let server = wiremock::MockServer::start().await;
    let body = common::script(&[
        r#"{"id":"resp-1","choices":[{"index":0,"delta":{},"finish_reason":"something_new"}]}"#,
    ]);
    common::mount_sse(&server, body).await;

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
    assert_eq!(message.stop_reason, StopReason::Error);
    assert_eq!(message.raw_stop_reason.as_deref(), Some("something_new"));
    assert!(
        message
            .error_message
            .as_deref()
            .is_some_and(|message| message.contains("something_new"))
    );
}

#[tokio::test]
#[spec("PROV-014")]
async fn spec_prov_014_content_filter_finishes_as_error() {
    let server = wiremock::MockServer::start().await;
    let body = common::script(&[
        r#"{"id":"resp-1","choices":[{"index":0,"delta":{},"finish_reason":"content_filter"}]}"#,
    ]);
    common::mount_sse(&server, body).await;

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
    assert_eq!(message.stop_reason, StopReason::Error);
    assert_eq!(message.raw_stop_reason.as_deref(), Some("content_filter"));
}
