//! Provider error normalization tests (`PROV-004`).

mod common;

use codie_ai::{Compat, HeaderMap, MAX_ERROR_BODY_CHARS, OpenAiProvider};
use codie_core::agent::ChatClient;
use codie_core::content::{StopReason, ThinkingLevel};
use codie_core::message::Message;
use codie_testkit::spec;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
#[spec("PROV-004")]
async fn spec_prov_004_error_normalization() {
    let server = MockServer::start().await;
    let body = "x".repeat(MAX_ERROR_BODY_CHARS + 1000);
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(400).set_body_string(body))
        .mount(&server)
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
        &[Message::user("hi")],
        &common::options(&model, ThinkingLevel::Off),
    ))
    .await;

    let message = common::terminal(&events);
    assert_eq!(message.stop_reason, StopReason::Error);
    let error = message.error_message.as_deref().expect("error message");
    assert!(error.starts_with("400: "));
    assert_eq!(
        error.chars().filter(|character| *character == 'x').count(),
        MAX_ERROR_BODY_CHARS
    );
    assert_eq!(error.chars().count(), "400: ".len() + MAX_ERROR_BODY_CHARS);
}

#[tokio::test]
#[spec("PROV-004")]
async fn spec_prov_004_transport_errors_include_the_cause_chain() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind a local port");
    let address = listener.local_addr().expect("local address");
    drop(listener);

    let model = common::deepseek_model();
    let provider = OpenAiProvider::new(
        "deepseek",
        format!("http://{address}"),
        "test-key",
        vec![model.clone()],
        Compat::deepseek(),
        HeaderMap::new(),
    );

    let events = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        common::drain(provider.stream(
            &[Message::user("hi")],
            &common::options(&model, ThinkingLevel::Off),
        )),
    )
    .await
    .expect("transport failure should terminate");

    let message = common::terminal(&events);
    assert_eq!(message.stop_reason, StopReason::Error);
    let error = message.error_message.as_deref().expect("error message");
    assert!(!error.is_empty());
    assert!(error.contains(':'), "expected a cause chain, got: {error}");
}
