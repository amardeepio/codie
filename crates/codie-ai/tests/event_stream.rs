//! Assistant event-stream ordering tests (`MSG-040`..`MSG-042`).

mod common;

use codie_ai::{Compat, HeaderMap, OpenAiProvider};
use codie_core::agent::ChatClient;
use codie_core::content::{Content, StopReason, ThinkingLevel};
use codie_core::event::AssistantDelta;
use codie_core::message::Message;
use codie_testkit::spec;

async fn run_mixed() -> Vec<AssistantDelta> {
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
    common::drain(provider.stream(
        &[Message::user("hi")],
        &common::options(&model, ThinkingLevel::Off),
    ))
    .await
}

#[tokio::test]
#[spec("MSG-040")]
async fn spec_msg_040_start_precedes_content() {
    let events = run_mixed().await;
    let first = events.first().expect("the stream should not be empty");
    match first {
        AssistantDelta::Start { partial } => {
            assert!(partial.content.is_empty());
            assert_eq!(partial.stop_reason, StopReason::Pending);
            assert_eq!(partial.provider, "deepseek");
            assert_eq!(partial.model, "deepseek-flash");
        }
        other => panic!("expected start first, got {other:?}"),
    }

    let starts = events
        .iter()
        .filter(|event| matches!(event, AssistantDelta::Start { .. }))
        .count();
    assert_eq!(starts, 1);
}

#[tokio::test]
#[spec("MSG-041")]
async fn spec_msg_041_partial_is_live() {
    let events = run_mixed().await;

    let thinking_end = events
        .iter()
        .find_map(|event| match event {
            AssistantDelta::ThinkingEnd { index, content } => Some((*index, content)),
            _ => None,
        })
        .expect("thinking should end");
    assert_eq!(thinking_end.0, 0);
    assert_eq!(thinking_end.1.as_str(), common::deltas_for(&events, 0));

    let text_end = events
        .iter()
        .find_map(|event| match event {
            AssistantDelta::TextEnd { index, content } => Some((*index, content)),
            _ => None,
        })
        .expect("text should end");
    assert_eq!(text_end.0, 1);
    assert_eq!(text_end.1.as_str(), common::deltas_for(&events, 1));

    let message = common::terminal(&events);
    match &message.content[0] {
        Content::Thinking { thinking, .. } => assert_eq!(thinking, thinking_end.1),
        other => panic!("expected a thinking block, got {other:?}"),
    }
    match &message.content[1] {
        Content::Text { text, .. } => assert_eq!(text, text_end.1),
        other => panic!("expected a text block, got {other:?}"),
    }
}

#[tokio::test]
#[spec("MSG-042")]
async fn spec_msg_042_exactly_one_terminal_event() {
    let events = run_mixed().await;

    let terminals = events
        .iter()
        .filter(|event| event.terminal_message().is_some())
        .count();
    assert_eq!(terminals, 1);
    assert!(
        events
            .last()
            .is_some_and(|event| event.terminal_message().is_some())
    );

    let message = common::terminal(&events);
    assert_eq!(message.stop_reason, StopReason::Stop);
    assert!(message.error_message.is_none());
}
