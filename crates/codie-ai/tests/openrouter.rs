//! OpenRouter-specific request tests (`PROV-018`).

mod common;

use codie_ai::{Compat, DEFAULT_HTTP_REFERER, DEFAULT_X_TITLE, HeaderMap, OpenAiProvider};
use codie_core::agent::ChatClient;
use codie_core::content::ThinkingLevel;
use codie_core::message::Message;
use codie_testkit::spec;

#[tokio::test]
#[spec("PROV-018")]
async fn spec_prov_018_attribution_headers() {
    let server = wiremock::MockServer::start().await;
    common::mount_sse(
        &server,
        common::script(&[
            r#"{"id":"resp-1","choices":[{"index":0,"delta":{"content":"ok"}}]}"#,
            r#"{"id":"resp-1","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#,
        ]),
    )
    .await;

    let model = common::openrouter_model();
    let mut headers = HeaderMap::new();
    headers.insert("HTTP-Referer".to_owned(), DEFAULT_HTTP_REFERER.to_owned());
    headers.insert("X-Title".to_owned(), DEFAULT_X_TITLE.to_owned());
    let provider = OpenAiProvider::new(
        "openrouter",
        server.uri(),
        "test-key",
        vec![model.clone()],
        Compat::openrouter(),
        headers,
    );

    let mut options = common::options(&model, ThinkingLevel::Off);
    options.session_id = Some("session-alpha".to_owned());
    let events = common::drain(provider.stream(&[Message::user("hi")], &options)).await;
    assert!(
        events
            .iter()
            .any(|event| event.terminal_message().is_some())
    );

    let requests = server
        .received_requests()
        .await
        .expect("requests are recorded");
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.url.path(), "/chat/completions");
    assert_eq!(
        request
            .headers
            .get("http-referer")
            .and_then(|value| value.to_str().ok()),
        Some(DEFAULT_HTTP_REFERER)
    );
    assert_eq!(
        request
            .headers
            .get("x-title")
            .and_then(|value| value.to_str().ok()),
        Some(DEFAULT_X_TITLE)
    );
    assert_eq!(
        request
            .headers
            .get("x-session-id")
            .and_then(|value| value.to_str().ok()),
        Some("session-alpha")
    );
    assert_eq!(
        request
            .headers
            .get("authorization")
            .and_then(|value| value.to_str().ok()),
        Some("Bearer test-key")
    );
    assert_eq!(
        request
            .headers
            .get("content-type")
            .and_then(|value| value.to_str().ok()),
        Some("application/json")
    );
    assert_eq!(
        request
            .headers
            .get("accept")
            .and_then(|value| value.to_str().ok()),
        Some("text/event-stream")
    );
    assert!(
        request
            .headers
            .get("user-agent")
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.starts_with("codie/"))
    );
}

#[test]
fn openrouter_model_ids_are_sent_verbatim() {
    let model = common::custom_model(
        "anthropic/claude-sonnet-4",
        "openrouter",
        Compat::openrouter(),
    );
    let provider = OpenAiProvider::new(
        "openrouter",
        "https://openrouter.ai/api/v1",
        "test-key",
        vec![model.clone()],
        Compat::openrouter(),
        HeaderMap::new(),
    );
    let body = provider.request_body(
        &[Message::user("hi")],
        &common::options(&model, ThinkingLevel::Off),
    );
    assert_eq!(
        body["model"],
        serde_json::json!("anthropic/claude-sonnet-4")
    );
}

#[tokio::test]
#[spec("PROV-018")]
async fn spec_prov_018_deepseek_sends_no_attribution() {
    let server = wiremock::MockServer::start().await;
    common::mount_sse(
        &server,
        common::script(&[
            r#"{"id":"resp-1","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#,
        ]),
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

    let mut options = common::options(&model, ThinkingLevel::Off);
    options.session_id = Some("session-alpha".to_owned());
    common::drain(provider.stream(&[Message::user("hi")], &options)).await;

    let requests = server
        .received_requests()
        .await
        .expect("requests are recorded");
    assert_eq!(requests.len(), 1);
    let headers = &requests[0].headers;
    assert!(headers.get("http-referer").is_none());
    assert!(headers.get("x-title").is_none());
    assert!(headers.get("x-session-id").is_none());
}
