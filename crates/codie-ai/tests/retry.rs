//! Retry policy tests (`PROV-003`).

mod common;

use codie_ai::{Compat, HeaderMap, OpenAiProvider};
use codie_core::agent::ChatClient;
use codie_core::content::{StopReason, ThinkingLevel};
use codie_core::message::Message;
use codie_testkit::spec;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn provider(server: &MockServer) -> (OpenAiProvider, codie_ai::Model) {
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
#[spec("PROV-003")]
async fn spec_prov_003_retry_policy_and_backoff() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("retry-after-ms", "10")
                .set_body_string("rate limited"),
        )
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(500)
                .insert_header("retry-after-ms", "10")
                .set_body_string("upstream failure"),
        )
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(common::script(&[
                    r#"{"id":"resp-1","choices":[{"index":0,"delta":{"content":"recovered"}}]}"#,
                    r#"{"id":"resp-1","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#,
                ])),
        )
        .with_priority(2)
        .mount(&server)
        .await;

    let (provider, model) = provider(&server);
    let events = common::drain(provider.stream(
        &[Message::user("hi")],
        &common::options(&model, ThinkingLevel::Off),
    ))
    .await;

    let message = common::terminal(&events);
    assert_eq!(message.stop_reason, StopReason::Stop);
    assert_eq!(message.text(), "recovered");
    let requests = server
        .received_requests()
        .await
        .expect("requests are recorded");
    assert_eq!(requests.len(), 3, "expected two retries");
}

#[tokio::test]
#[spec("PROV-003")]
async fn spec_prov_003_non_retryable_4xx_fails_immediately() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(400).set_body_string("bad request"))
        .mount(&server)
        .await;

    let (provider, model) = provider(&server);
    let events = common::drain(provider.stream(
        &[Message::user("hi")],
        &common::options(&model, ThinkingLevel::Off),
    ))
    .await;

    let message = common::terminal(&events);
    assert_eq!(message.stop_reason, StopReason::Error);
    assert_eq!(message.error_message.as_deref(), Some("400: bad request"));
    let requests = server
        .received_requests()
        .await
        .expect("requests are recorded");
    assert_eq!(requests.len(), 1);
}

#[tokio::test]
#[spec("PROV-003")]
async fn spec_prov_003_retry_after_above_cap_fails_fast() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("retry-after-ms", "120000")
                .set_body_string("slow down"),
        )
        .mount(&server)
        .await;

    let (provider, model) = provider(&server);
    let events = common::drain(provider.stream(
        &[Message::user("hi")],
        &common::options(&model, ThinkingLevel::Off),
    ))
    .await;

    let message = common::terminal(&events);
    assert_eq!(message.stop_reason, StopReason::Error);
    assert!(
        message
            .error_message
            .as_deref()
            .is_some_and(|message| message.contains("above the 60000ms cap")),
        "error: {:?}",
        message.error_message
    );
    let requests = server
        .received_requests()
        .await
        .expect("requests are recorded");
    assert_eq!(requests.len(), 1);
}
