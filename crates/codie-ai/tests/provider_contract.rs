//! Provider contract tests (`PROV-001`, `PROV-002`).

mod common;

use std::time::{Duration, Instant};

use codie_ai::{Compat, HeaderMap, OpenAiProvider};
use codie_core::agent::ChatClient;
use codie_core::content::{StopReason, ThinkingLevel};
use codie_core::event::AssistantDelta;
use codie_core::message::Message;
use codie_testkit::spec;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn slow_provider(server: &MockServer) -> (OpenAiProvider, codie_ai::Model) {
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
#[spec("PROV-001")]
async fn spec_prov_001_stream_returns_before_io() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(common::mixed_stream())
                .set_delay(Duration::from_secs(2)),
        )
        .mount(&server)
        .await;

    let (provider, model) = slow_provider(&server);
    let options = common::options(&model, ThinkingLevel::Off);

    let started = Instant::now();
    let mut stream = provider.stream(&[Message::user("hi")], &options);
    assert!(
        started.elapsed() < Duration::from_millis(250),
        "stream() blocked for {:?}",
        started.elapsed()
    );

    let first = tokio::time::timeout(Duration::from_millis(250), stream.recv())
        .await
        .expect("start should arrive without waiting for the server")
        .expect("the stream should yield an event");
    assert!(matches!(first, AssistantDelta::Start { .. }));
}

#[tokio::test]
#[spec("PROV-002")]
async fn spec_prov_002_abort_maps_to_aborted() {
    let server = MockServer::start().await;
    common::mount_sse(&server, common::mixed_stream()).await;

    let (provider, model) = slow_provider(&server);
    let options = common::options(&model, ThinkingLevel::Off);
    options.cancel.cancel();

    let events = common::drain(provider.stream(&[Message::user("hi")], &options)).await;
    let message = common::terminal(&events);
    assert_eq!(message.stop_reason, StopReason::Aborted);
    assert_eq!(message.error_message.as_deref(), Some("request aborted"));
    assert!(matches!(
        events.last(),
        Some(AssistantDelta::Error {
            reason: StopReason::Aborted,
            ..
        })
    ));

    let requests = server
        .received_requests()
        .await
        .expect("requests are recorded");
    assert!(requests.is_empty(), "abort should not send a request");
}

#[tokio::test]
#[spec("PROV-002")]
async fn spec_prov_002_abort_interrupts_an_in_flight_request() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(common::mixed_stream())
                .set_delay(Duration::from_secs(2)),
        )
        .mount(&server)
        .await;

    let (provider, model) = slow_provider(&server);
    let options = common::options(&model, ThinkingLevel::Off);
    let mut stream = provider.stream(&[Message::user("hi")], &options);

    let first = tokio::time::timeout(Duration::from_millis(250), stream.recv())
        .await
        .expect("start should arrive promptly")
        .expect("the stream should yield an event");
    assert!(matches!(first, AssistantDelta::Start { .. }));

    options.cancel.cancel();
    let events = tokio::time::timeout(Duration::from_secs(1), common::drain(stream))
        .await
        .expect("abort should interrupt the in-flight request");

    let message = common::terminal(&events);
    assert_eq!(message.stop_reason, StopReason::Aborted);
}
