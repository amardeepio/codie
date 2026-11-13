//! Shared helpers for the `codie-ai` integration tests.

#![allow(dead_code, unreachable_pub)]

use codie_ai::{Compat, HeaderMap, Model, OpenAiProvider, ProviderId, builtin_models, find_model};
use codie_core::agent::{ModelRef, StreamOptions};
use codie_core::content::{Content, ThinkingLevel, ToolDecl};
use codie_core::event::{AssistantDelta, AssistantEventStream};
use codie_core::message::{AssistantMessage, Message};
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Returns the built-in DeepSeek Flash model.
pub fn deepseek_model() -> Model {
    find_model("deepseek", "deepseek-flash").expect("deepseek-flash is built in")
}

/// Returns the built-in OpenRouter Auto model.
pub fn openrouter_model() -> Model {
    find_model("openrouter", "openrouter/auto").expect("openrouter/auto is built in")
}

/// Returns a minimal custom model for conversion tests.
pub fn custom_model(id: &str, provider: &str, compat: Compat) -> Model {
    Model {
        id: id.to_owned(),
        name: id.to_owned(),
        provider: ProviderId::Owned(provider.to_owned()),
        api: ProviderId::Borrowed(codie_ai::OPENAI_COMPLETIONS),
        base_url: String::new(),
        context_window: 128_000,
        max_output_tokens: 8192,
        reasoning: compat.thinking_format != codie_ai::ThinkingFormat::OpenAi,
        reasoning_mandatory: false,
        thinking_levels: deepseek_model().thinking_levels,
        cost: Default::default(),
        compat,
        headers: HeaderMap::new(),
    }
}

/// Builds stream options targeting `model`.
pub fn options(model: &Model, thinking: ThinkingLevel) -> StreamOptions {
    StreamOptions {
        model: ModelRef {
            api: model.api.to_string(),
            provider: model.provider.to_string(),
            model: model.id.clone(),
        },
        thinking,
        max_output_tokens: None,
        temperature: None,
        session_id: Some("test-session".to_owned()),
        cancel: CancellationToken::new(),
    }
}

/// Builds a system message with the given text and tools.
pub fn system(text: &str, tools: Vec<ToolDecl>) -> Message {
    Message::system(text, tools)
}

/// A representative tool declaration.
pub fn read_tool() -> ToolDecl {
    ToolDecl {
        name: "read".to_owned(),
        description: "Read a file".to_owned(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {"path": {"type": "string"}},
            "required": ["path"]
        }),
    }
}

/// Builds a provider targeting a wiremock server.
pub fn provider_for(
    server: &MockServer,
    provider_id: &str,
    model: Model,
    compat: Compat,
    headers: HeaderMap,
) -> OpenAiProvider {
    OpenAiProvider::new(
        provider_id,
        server.uri(),
        "test-key",
        vec![model],
        compat,
        headers,
    )
}

/// Mounts a successful SSE response at `/chat/completions`.
pub async fn mount_sse(server: &MockServer, body: String) {
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(body),
        )
        .mount(server)
        .await;
}

/// Turns JSON payloads into an SSE body terminated by `[DONE]`.
pub fn script(payloads: &[&str]) -> String {
    let mut body = String::new();
    for payload in payloads {
        body.push_str("data: ");
        body.push_str(payload);
        body.push_str("\n\n");
    }
    body.push_str("data: [DONE]\n\n");
    body
}

/// Drains a stream to completion and returns every event.
pub async fn drain(mut stream: AssistantEventStream) -> Vec<AssistantDelta> {
    let mut events = Vec::new();
    while let Some(event) = stream.recv().await {
        events.push(event);
    }
    events
}

/// Returns the terminal assistant message from an event list.
pub fn terminal(events: &[AssistantDelta]) -> &AssistantMessage {
    events
        .iter()
        .find_map(AssistantDelta::terminal_message)
        .expect("the stream should contain a terminal event")
}

/// Returns the concatenated deltas for a content index.
pub fn deltas_for(events: &[AssistantDelta], index: usize) -> String {
    events
        .iter()
        .filter_map(|event| match event {
            AssistantDelta::TextDelta {
                index: event_index,
                delta,
            }
            | AssistantDelta::ToolCallDelta {
                index: event_index,
                delta,
            }
            | AssistantDelta::ThinkingDelta {
                index: event_index,
                delta,
            } if *event_index == index => Some(delta.as_str()),
            _ => None,
        })
        .collect()
}

/// Returns the text of the first text block in an assistant message.
pub fn first_text(message: &AssistantMessage) -> &str {
    message
        .content
        .iter()
        .find_map(|block| match block {
            Content::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .unwrap_or("")
}

/// The canonical mixed stream: reasoning, text, usage, and a normal stop.
pub fn mixed_stream() -> String {
    script(&[
        r#"{"id":"resp-1","model":"deepseek-flash","choices":[{"index":0,"delta":{"role":"assistant","reasoning_content":"think "}}]}"#,
        r#"{"id":"resp-1","choices":[{"index":0,"delta":{"reasoning_content":"more"}}]}"#,
        r#"{"id":"resp-1","choices":[{"index":0,"delta":{"content":"Hello"}}]}"#,
        r#"{"id":"resp-1","choices":[{"index":0,"delta":{"content":" world"}}]}"#,
        r#"{"id":"resp-1","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#,
        r#"{"id":"resp-1","choices":[],"usage":{"prompt_tokens":1200,"completion_tokens":80,"prompt_cache_hit_tokens":900,"completion_tokens_details":{"reasoning_tokens":20}}}"#,
    ])
}

/// Asserts that the built-in catalog still contains the expected default.
pub fn assert_default_catalog() {
    assert!(
        builtin_models()
            .iter()
            .any(|model| model.id == "openrouter/auto")
    );
}
