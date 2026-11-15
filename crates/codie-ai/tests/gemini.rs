//! Google Gemini provider profile tests (`SPEC-PROV-005`).
//!
//! Gemini is served through its OpenAI-compatible endpoint, so these tests
//! pin the profile values (base URL, field names, reasoning levels) and the
//! thought-signature round trip that multi-turn tool calling requires.

mod common;

use codie_ai::{
    Compat, HeaderMap, OpenAiProvider, PROVIDER_GEMINI, build_request_body, find_model,
    provider_base_url, provider_compat,
};
use codie_core::agent::ChatClient;
use codie_core::content::{Content, StopReason, ThinkingLevel};
use codie_core::message::Message;
use codie_testkit::spec;
use serde_json::{Value, json};

fn hydrogen() -> codie_ai::Model {
    find_model(PROVIDER_GEMINI, "gemini-4-hydrogen").expect("gemini-4-hydrogen is built in")
}

fn reasoning_effort(model: &codie_ai::Model, level: ThinkingLevel) -> Option<Value> {
    let options = common::options(model, level);
    build_request_body(model, &[Message::user("hi")], &options)
        .get("reasoning_effort")
        .cloned()
}

#[test]
fn gemini_profile_is_registered() {
    assert_eq!(
        provider_base_url(PROVIDER_GEMINI),
        "https://generativelanguage.googleapis.com/v1beta/openai"
    );
    assert_eq!(provider_compat(PROVIDER_GEMINI), Compat::gemini());

    let provider = OpenAiProvider::gemini("test-key");
    assert_eq!(provider.base_url(), provider_base_url(PROVIDER_GEMINI));
    assert!(
        provider
            .models()
            .iter()
            .any(|model| model.id == "gemini-4-hydrogen")
    );

    let model = hydrogen();
    assert_eq!(model.api, "openai-completions");
    assert_eq!(model.context_window, 1_048_576);
    assert_eq!(model.max_output_tokens, 65_536);
    assert!(model.reasoning);
    assert!(model.reasoning_mandatory);
}

#[test]
#[spec("PROV-010")]
fn spec_prov_010_gemini_uses_max_tokens() {
    let model = hydrogen();
    let mut options = common::options(&model, ThinkingLevel::High);
    options.max_output_tokens = Some(4096);

    let body = build_request_body(
        &model,
        &[
            common::system("be helpful", vec![common::read_tool()]),
            Message::user("read math.py"),
        ],
        &options,
    );

    assert_eq!(body["model"], "gemini-4-hydrogen");
    assert_eq!(body["max_tokens"], 4096);
    assert!(body.get("max_completion_tokens").is_none());
    assert_eq!(body["stream_options"]["include_usage"], true);
    assert_eq!(body["reasoning_effort"], "high");
    assert!(body.get("store").is_none());
}

#[test]
#[spec("PROV-011")]
fn spec_prov_011_gemini_reasoning_levels() {
    let hydrogen = hydrogen();
    assert_eq!(
        reasoning_effort(&hydrogen, ThinkingLevel::Minimal),
        Some(json!("low"))
    );
    assert_eq!(
        reasoning_effort(&hydrogen, ThinkingLevel::Low),
        Some(json!("low"))
    );
    assert_eq!(
        reasoning_effort(&hydrogen, ThinkingLevel::XHigh),
        Some(json!("xhigh"))
    );
    // Reasoning cannot be disabled on Gemini 3 and 4, so `off` omits the field
    // instead of sending `"none"`.
    assert_eq!(reasoning_effort(&hydrogen, ThinkingLevel::Off), None);

    let flash = find_model(PROVIDER_GEMINI, "gemini-3.8-flash").unwrap();
    // `minimal` is rejected by Gemini 3.8 Flash, so the field is omitted.
    assert_eq!(reasoning_effort(&flash, ThinkingLevel::Minimal), None);
    assert_eq!(
        reasoning_effort(&flash, ThinkingLevel::High),
        Some(json!("high"))
    );
}

#[tokio::test]
#[spec("PROV-024")]
async fn spec_prov_024_gemini_thought_signature_round_trip() {
    let server = wiremock::MockServer::start().await;
    common::mount_sse(
        &server,
        include_str!("fixtures/gemini_recorded.sse").to_owned(),
    )
    .await;

    let model = hydrogen();
    let provider = common::provider_for(
        &server,
        PROVIDER_GEMINI,
        model.clone(),
        Compat::gemini(),
        HeaderMap::new(),
    );
    let events = common::drain(provider.stream(
        &[Message::user("read math.py")],
        &common::options(&model, ThinkingLevel::Medium),
    ))
    .await;

    let message = common::terminal(&events);
    assert_eq!(message.stop_reason, StopReason::ToolUse);
    assert_eq!(message.text(), "Reading the file.");
    assert_eq!(message.usage.input, 80);
    assert_eq!(message.usage.cache_read, 20);
    assert_eq!(message.usage.output, 30);
    assert_eq!(message.usage.reasoning, Some(10));
    assert_eq!(message.usage.cost.total, 346_500);

    let calls = message.tool_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].id, "function-call-1");
    assert_eq!(calls[0].name, "read");
    assert_eq!(calls[0].arguments["path"], "math.py");

    let signature = message.content.iter().find_map(|block| match block {
        Content::ToolCall {
            thought_signature, ..
        } => thought_signature.clone(),
        _ => None,
    });
    assert_eq!(signature.as_deref(), Some("sig-abc"));

    // Replaying the assistant turn must return the signature to Gemini.
    let assistant = Message::assistant(
        codie_ai::OPENAI_COMPLETIONS,
        PROVIDER_GEMINI,
        "gemini-4-hydrogen",
        message.content.clone(),
        StopReason::ToolUse,
    );
    let body = build_request_body(
        &model,
        &[assistant],
        &common::options(&model, ThinkingLevel::Medium),
    );
    assert_eq!(
        body["messages"][0]["tool_calls"][0]["extra_content"]["google"]["thought_signature"],
        "sig-abc"
    );

    // Calls without a signature keep the plain OpenAI shape.
    let unsigned = Message::assistant(
        codie_ai::OPENAI_COMPLETIONS,
        PROVIDER_GEMINI,
        "gemini-4-hydrogen",
        vec![Content::ToolCall {
            id: "call_2".to_owned(),
            name: "read".to_owned(),
            arguments: json!({"path": "lib.rs"}),
            thought_signature: None,
        }],
        StopReason::ToolUse,
    );
    let body = build_request_body(
        &model,
        &[unsigned],
        &common::options(&model, ThinkingLevel::Medium),
    );
    assert!(
        body["messages"][0]["tool_calls"][0]
            .get("extra_content")
            .is_none()
    );
}

#[tokio::test]
#[spec("PROV-013")]
async fn spec_prov_013_gemini_parallel_calls_share_an_index() {
    let server = wiremock::MockServer::start().await;
    let body = common::script(&[
        r#"{"id":"g1","model":"gemini-4-hydrogen","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_a","type":"function","function":{"name":"read","arguments":"{\"path\":\"a.txt\"}"}}]},"finish_reason":null}]}"#,
        r#"{"id":"g1","model":"gemini-4-hydrogen","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_b","type":"function","function":{"name":"read","arguments":"{\"path\":\"b.txt\"}"}}]},"finish_reason":null}]}"#,
        r#"{"id":"g1","model":"gemini-4-hydrogen","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}"#,
    ]);
    common::mount_sse(&server, body).await;

    let model = hydrogen();
    let provider = common::provider_for(
        &server,
        PROVIDER_GEMINI,
        model.clone(),
        Compat::gemini(),
        HeaderMap::new(),
    );
    let events = common::drain(provider.stream(
        &[Message::user("read both")],
        &common::options(&model, ThinkingLevel::Off),
    ))
    .await;

    let message = common::terminal(&events);
    let calls = message.tool_calls();
    assert_eq!(calls.len(), 2, "a reused index must not merge calls");
    assert_eq!(calls[0].id, "call_a");
    assert_eq!(calls[0].arguments["path"], "a.txt");
    assert_eq!(calls[1].id, "call_b");
    assert_eq!(calls[1].arguments["path"], "b.txt");
}
