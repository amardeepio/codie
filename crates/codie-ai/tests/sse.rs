//! SSE framing tests (`PROV-012`).

mod common;

use codie_ai::sse::SseParser;
use codie_ai::{Compat, HeaderMap, SseEvent};
use codie_core::agent::ChatClient;
use codie_core::content::ThinkingLevel;
use codie_testkit::spec;

fn parse_chunks(body: &[u8], chunk_size: usize) -> Vec<SseEvent> {
    let mut parser = SseParser::new();
    let mut events = Vec::new();
    for chunk in body.chunks(chunk_size) {
        events.extend(parser.push(chunk));
    }
    events.extend(parser.finish());
    events
}

#[test]
#[spec("PROV-012")]
fn spec_prov_012_sse_frame_reassembly() {
    let body = include_str!("fixtures/plain.sse");
    let baseline = parse_chunks(body.as_bytes(), body.len());
    assert_eq!(baseline.len(), 3);
    assert_eq!(baseline[0].data, r#"{"kind":"one"}"#);
    assert_eq!(baseline[1].data, r#"{"kind":"two"}"#);
    assert!(baseline[2].is_done());

    // Split at every possible chunk size, including one byte per chunk, which
    // cuts multi-byte sequences and CRLF pairs apart.
    for chunk_size in 1..=body.len() {
        let events = parse_chunks(body.as_bytes(), chunk_size);
        assert_eq!(events, baseline, "failed with chunk size {chunk_size}");
    }
}

#[test]
#[spec("PROV-012")]
fn spec_prov_012_tolerates_crlf_and_comments() {
    let body = include_str!("fixtures/plain.sse").replace('\n', "\r\n");
    let events = parse_chunks(body.as_bytes(), 3);
    assert_eq!(events.len(), 3);
    assert_eq!(events[0].data, r#"{"kind":"one"}"#);
    assert!(events[2].is_done());
}

#[test]
#[spec("PROV-012")]
fn spec_prov_012_multiline_data_is_joined() {
    let body = include_str!("fixtures/multiline.sse");
    let events = parse_chunks(body.as_bytes(), 5);
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].data, "{\"joined\":\n\"value\"}");
    assert!(events[1].is_done());
}

#[test]
#[spec("PROV-012")]
fn spec_prov_012_tool_call_and_usage_frames() {
    let tool_events = parse_chunks(include_bytes!("fixtures/tool_calls.sse"), 7);
    assert_eq!(tool_events.len(), 4);
    assert!(tool_events[0].data.contains("\"tool_calls\""));
    assert!(
        tool_events[2]
            .data
            .contains("\"finish_reason\":\"tool_calls\"")
    );
    assert!(tool_events[3].is_done());

    let usage_events = parse_chunks(include_bytes!("fixtures/usage.sse"), 11);
    assert_eq!(usage_events.len(), 4);
    assert!(usage_events[2].data.contains("\"usage\""));
    assert!(usage_events[3].is_done());
}

#[tokio::test]
#[spec("PROV-012")]
async fn spec_prov_012_eof_without_finish_reason_is_an_error() {
    let server = wiremock::MockServer::start().await;
    let body = common::script(&[
        r#"{"id":"resp-1","choices":[{"index":0,"delta":{"content":"partial"}}]}"#,
    ]);
    common::mount_sse(&server, body).await;

    let provider = common::provider_for(
        &server,
        "deepseek",
        common::deepseek_model(),
        Compat::deepseek(),
        HeaderMap::new(),
    );
    let options = common::options(&common::deepseek_model(), ThinkingLevel::Off);
    let events = common::drain(provider.stream(&[codie_core::Message::user("hi")], &options)).await;
    let message = common::terminal(&events);
    assert_eq!(message.stop_reason, codie_core::StopReason::Error);
    assert_eq!(
        message.error_message.as_deref(),
        Some("stream ended without finish_reason")
    );
}

#[tokio::test]
#[spec("PROV-012")]
async fn spec_prov_012_missing_finish_reason_allowed_by_compat() {
    let server = wiremock::MockServer::start().await;
    let body = common::script(&[
        r#"{"id":"resp-1","choices":[{"index":0,"delta":{"content":"partial"}}]}"#,
    ]);
    common::mount_sse(&server, body).await;

    let mut model = common::deepseek_model();
    model.compat.supports_finish_reason = false;
    let provider = common::provider_for(
        &server,
        "deepseek",
        model.clone(),
        model.compat,
        HeaderMap::new(),
    );
    let events = common::drain(provider.stream(
        &[codie_core::Message::user("hi")],
        &common::options(&model, ThinkingLevel::Off),
    ))
    .await;

    let message = common::terminal(&events);
    assert_eq!(message.stop_reason, codie_core::StopReason::Stop);
    assert_eq!(message.text(), "partial");
}
