//! Request conversion and compatibility gating tests.

mod common;

use codie_ai::{Compat, HeaderMap, MaxTokensField, Model, OpenAiProvider, ThinkingFormat};
use codie_core::content::{Content, StopReason, ThinkingLevel};
use codie_core::message::Message;
use codie_testkit::spec;
use serde_json::json;

fn provider(model: Model, compat: Compat) -> OpenAiProvider {
    OpenAiProvider::new(
        model.provider.to_string(),
        "https://example.invalid",
        "test-key",
        vec![model],
        compat,
        HeaderMap::new(),
    )
}

#[test]
#[spec("PROV-010")]
fn spec_prov_010_max_tokens_field() {
    let model = common::deepseek_model();
    let client = provider(model.clone(), Compat::deepseek());
    let mut options = common::options(&model, ThinkingLevel::Off);

    let body = client.request_body(&[Message::user("hi")], &options);
    assert_eq!(body["max_tokens"], json!(384_000));
    assert!(body.get("max_completion_tokens").is_none());

    options.max_output_tokens = Some(1234);
    let body = client.request_body(&[Message::user("hi")], &options);
    assert_eq!(body["max_tokens"], json!(1234));

    let mut unbounded = model.clone();
    unbounded.max_output_tokens = 0;
    let client = provider(unbounded.clone(), Compat::deepseek());
    options.max_output_tokens = None;
    let body = client.request_body(&[Message::user("hi")], &options);
    assert!(body.get("max_tokens").is_none());

    let mut openrouter = common::openrouter_model();
    openrouter.reasoning = true;
    let options = common::options(&openrouter, ThinkingLevel::Off);
    let client = provider(openrouter, Compat::openrouter());
    let body = client.request_body(&[Message::user("hi")], &options);
    assert_eq!(body["max_completion_tokens"], json!(8192));
    assert!(body.get("max_tokens").is_none());
}

#[test]
#[spec("PROV-011")]
fn spec_prov_011_reasoning_parameter_selection() {
    let deepseek = common::deepseek_model();
    let client = provider(deepseek.clone(), Compat::deepseek());

    let body = client.request_body(
        &[Message::user("hi")],
        &common::options(&deepseek, ThinkingLevel::High),
    );
    assert_eq!(body["thinking"], json!({"type": "enabled"}));
    assert_eq!(body["reasoning_effort"], json!("high"));

    let body = client.request_body(
        &[Message::user("hi")],
        &common::options(&deepseek, ThinkingLevel::Off),
    );
    assert_eq!(body["thinking"], json!({"type": "disabled"}));
    assert!(body.get("reasoning_effort").is_none());

    let body = client.request_body(
        &[Message::user("hi")],
        &common::options(&deepseek, ThinkingLevel::XHigh),
    );
    assert_eq!(body["reasoning_effort"], json!("high"));

    let mut openrouter = common::openrouter_model();
    openrouter.reasoning = true;
    let client = provider(openrouter.clone(), Compat::openrouter());

    let body = client.request_body(
        &[Message::user("hi")],
        &common::options(&openrouter, ThinkingLevel::High),
    );
    assert_eq!(body["reasoning"], json!({"effort": "high"}));
    assert!(body.get("thinking").is_none());

    let body = client.request_body(
        &[Message::user("hi")],
        &common::options(&openrouter, ThinkingLevel::Off),
    );
    assert_eq!(body["reasoning"], json!({"effort": "none"}));

    let body = client.request_body(
        &[Message::user("hi")],
        &common::options(&openrouter, ThinkingLevel::Max),
    );
    assert_eq!(body["reasoning"], json!({"effort": "high"}));

    let mut openai = common::custom_model("gpt-test", "openai", Compat::openai());
    openai.reasoning = true;
    let client = provider(openai.clone(), Compat::openai());
    let body = client.request_body(
        &[Message::user("hi")],
        &common::options(&openai, ThinkingLevel::High),
    );
    assert_eq!(body["reasoning_effort"], json!("high"));
    assert!(body.get("thinking").is_none());
    assert!(body.get("reasoning").is_none());
}

#[test]
#[spec("PROV-019")]
fn spec_prov_019_request_field_gating() {
    let mut compat = Compat::deepseek();
    compat.supports_usage_in_streaming = false;
    compat.max_tokens_field = MaxTokensField::MaxTokens;
    let mut model = common::deepseek_model();
    model.reasoning = false;
    model.max_output_tokens = 0;
    model.compat = compat;
    let client = provider(model.clone(), compat);

    let mut options = common::options(&model, ThinkingLevel::High);
    options.max_output_tokens = None;
    options.temperature = None;
    let body = client.request_body(&[Message::user("hi")], &options);

    assert!(body.get("stream_options").is_none());
    assert!(body.get("thinking").is_none());
    assert!(body.get("reasoning_effort").is_none());
    assert!(body.get("reasoning").is_none());
    assert!(body.get("max_tokens").is_none());
    assert!(body.get("store").is_none());
    assert!(body.get("temperature").is_none());
    assert_eq!(body["stream"], json!(true));

    options.temperature = Some(0.7);
    let body = client.request_body(&[Message::user("hi")], &options);
    assert_eq!(body["temperature"], json!(0.7_f32));
}

#[test]
#[spec("PROV-017")]
fn spec_prov_017_tool_result_wire_shape() {
    let model = common::deepseek_model();
    let client = provider(model.clone(), Compat::deepseek());
    let context = vec![
        common::system("system", vec![]),
        Message::tool_result(
            "call_1",
            "read",
            vec![Content::text("file contents")],
            false,
        ),
    ];

    let body = client.request_body(&context, &common::options(&model, ThinkingLevel::Off));
    assert_eq!(
        body["messages"][1],
        json!({
            "role": "tool",
            "tool_call_id": "call_1",
            "content": "file contents"
        })
    );
}

#[test]
#[spec("PROV-023")]
fn spec_prov_023_user_images_use_data_urls() {
    let model = common::deepseek_model();
    let client = provider(model.clone(), Compat::deepseek());
    let context = vec![Message::user_with_content(vec![
        Content::text("look"),
        Content::Image {
            data: "aGVsbG8=".to_owned(),
            mime_type: "image/png".to_owned(),
        },
    ])];

    let body = client.request_body(&context, &common::options(&model, ThinkingLevel::Off));
    assert_eq!(body["messages"][0]["content"][0]["type"], "text");
    assert_eq!(body["messages"][0]["content"][1]["type"], "image_url");
    assert_eq!(
        body["messages"][0]["content"][1]["image_url"]["url"],
        "data:image/png;base64,aGVsbG8="
    );
}

#[test]
#[spec("PROV-017")]
fn spec_prov_017_image_tool_results_are_downgraded() {
    let model = common::deepseek_model();
    let client = provider(model.clone(), Compat::deepseek());
    let context = vec![Message::tool_result(
        "call_1",
        "screenshot",
        vec![
            Content::text("see attached"),
            Content::Image {
                data: "aGVsbG8=".to_owned(),
                mime_type: "image/png".to_owned(),
            },
        ],
        false,
    )];

    let body = client.request_body(&context, &common::options(&model, ThinkingLevel::Off));
    assert_eq!(
        body["messages"][0]["content"],
        json!("see attached\n[image omitted: image/png]")
    );
}

#[test]
#[spec("MSG-007")]
fn spec_msg_007_system_and_tools_are_projected_from_messages() {
    let model = common::deepseek_model();
    let client = provider(model.clone(), Compat::deepseek());
    let context = vec![
        common::system("system prompt", vec![common::read_tool()]),
        Message::user("Fix the test"),
    ];

    let body = client.request_body(&context, &common::options(&model, ThinkingLevel::Off));
    assert_eq!(
        body["messages"][0],
        json!({"role": "system", "content": "system prompt"})
    );
    assert_eq!(
        body["messages"][1],
        json!({"role": "user", "content": "Fix the test"})
    );
    assert_eq!(
        body["tools"],
        json!([{
            "type": "function",
            "function": {
                "name": "read",
                "description": "Read a file",
                "parameters": {
                    "type": "object",
                    "properties": {"path": {"type": "string"}},
                    "required": ["path"]
                }
            }
        }])
    );
}

#[test]
fn assistant_conversion_preserves_text_and_call_order() {
    let model = common::deepseek_model();
    let client = provider(model.clone(), Compat::deepseek());
    let assistant = Message::assistant(
        codie_ai::OPENAI_COMPLETIONS,
        "deepseek",
        "deepseek-flash",
        vec![
            Content::thinking("plan"),
            Content::text("reading "),
            Content::text("the file"),
            Content::ToolCall {
                id: "call_1".to_owned(),
                name: "read".to_owned(),
                arguments: json!({"path": "a.txt"}),
                thought_signature: None,
            },
        ],
        StopReason::ToolUse,
    );

    let body = client.request_body(
        &[assistant, Message::user("next")],
        &common::options(&model, ThinkingLevel::Off),
    );

    assert_eq!(
        body["messages"][0],
        json!({
            "role": "assistant",
            "content": "reading the file",
            "reasoning_content": "plan",
            "tool_calls": [{
                "id": "call_1",
                "type": "function",
                "function": {
                    "name": "read",
                    "arguments": "{\"path\":\"a.txt\"}"
                }
            }]
        })
    );
}

#[test]
fn empty_assistant_content_is_null() {
    let model = common::deepseek_model();
    let client = provider(model.clone(), Compat::deepseek());
    let assistant = Message::assistant(
        codie_ai::OPENAI_COMPLETIONS,
        "deepseek",
        "deepseek-flash",
        Vec::new(),
        StopReason::Stop,
    );

    let body = client.request_body(&[assistant], &common::options(&model, ThinkingLevel::Off));
    assert_eq!(body["messages"][0]["content"], json!(null));
    assert_eq!(body["messages"][0]["reasoning_content"], json!(""));
}

#[test]
fn deepseek_v1_base_url_is_normalized() {
    let provider = OpenAiProvider::deepseek("test-key");
    assert_eq!(provider.base_url(), "https://api.deepseek.com");

    let provider = OpenAiProvider::new(
        "deepseek",
        "https://api.deepseek.com/v1/",
        "test-key",
        vec![common::deepseek_model()],
        Compat::deepseek(),
        HeaderMap::new(),
    );
    assert_eq!(provider.base_url(), "https://api.deepseek.com");
}

#[test]
fn thinking_format_variants_are_exposed() {
    assert_ne!(ThinkingFormat::OpenAi, ThinkingFormat::OpenRouter);
    assert_ne!(ThinkingFormat::OpenRouter, ThinkingFormat::DeepSeek);
}
