//! Conformance tests for the core message model (MSG-001..MSG-010).

use codie_core::{
    AssistantEventStream, AssistantMessage, Content, Cost, Message, StopReason, ToolCall, ToolDecl,
    Usage, collapse_system_messages, current_tools, normalize_context, now_ms,
};
use codie_testkit::spec;
use serde_json::json;

const API: &str = "test-api";
const PROVIDER: &str = "test-provider";
const MODEL: &str = "test-model";

fn tool_decl(name: &str) -> ToolDecl {
    ToolDecl {
        name: name.to_owned(),
        description: format!("{name} tool"),
        parameters: json!({"type": "object"}),
    }
}

fn text_of(message: &Message) -> Option<String> {
    match message {
        Message::User(user) => Some(
            user.content
                .iter()
                .filter_map(Content::as_text)
                .collect::<Vec<_>>()
                .join(""),
        ),
        Message::System(system) => Some(system.content.clone()),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// MSG-001: every wire message is one of four kinds
// ---------------------------------------------------------------------------

#[test]
#[spec("MSG-001")]
fn spec_msg_001_message_kinds_lower_before_wire() {
    let messages = vec![
        Message::system("system prompt", vec![tool_decl("read")]),
        Message::user("hello"),
        Message::assistant(
            API,
            PROVIDER,
            MODEL,
            vec![Content::text("answer")],
            StopReason::Stop,
        ),
        Message::tool_result("call_1", "read", vec![Content::text("contents")], false),
    ];

    for message in &messages {
        assert!(matches!(
            message.role(),
            "system" | "user" | "assistant" | "tool_result"
        ));
        let value = serde_json::to_value(message).expect("serialize message");
        let role = value["role"].as_str().expect("role field");
        assert!(
            matches!(role, "system" | "user" | "assistant" | "tool_result"),
            "unexpected wire role: {role}"
        );
    }

    // Lowering to a single leading system message preserves textual content and
    // the relative order of the remaining messages.
    let lowered = normalize_context(
        "system prompt",
        vec![tool_decl("read")],
        vec![
            Message::user("first"),
            Message::assistant(
                API,
                PROVIDER,
                MODEL,
                vec![Content::text("answer")],
                StopReason::Stop,
            ),
            Message::tool_result("call_1", "read", vec![Content::text("contents")], false),
        ],
    );
    assert!(matches!(lowered[0], Message::System(_)));
    let user_texts: Vec<String> = lowered.iter().filter_map(text_of).collect();
    assert_eq!(user_texts, ["system prompt", "first"]);
    assert_eq!(lowered[1].role(), "user");
    assert_eq!(lowered[2].role(), "assistant");
    assert_eq!(lowered[3].role(), "tool_result");
}

// ---------------------------------------------------------------------------
// MSG-002: content order is preserved, blocks are never merged or reordered
// ---------------------------------------------------------------------------

#[test]
#[spec("MSG-002")]
fn spec_msg_002_content_order_preserved() {
    let blocks = vec![
        Content::thinking("plan"),
        Content::text("answer"),
        Content::ToolCall {
            id: "call_1".to_owned(),
            name: "read".to_owned(),
            arguments: json!({"path": "a.txt"}),
            thought_signature: None,
        },
        Content::Image {
            data: "aGk=".to_owned(),
            mime_type: "image/png".to_owned(),
        },
    ];

    let message = Message::assistant(API, PROVIDER, MODEL, blocks.clone(), StopReason::ToolUse);
    let assistant = message.as_assistant().expect("assistant message");
    assert_eq!(assistant.content, blocks);
    assert!(matches!(assistant.content[0], Content::Thinking { .. }));
    assert!(matches!(assistant.content[1], Content::Text { .. }));
    assert!(matches!(assistant.content[2], Content::ToolCall { .. }));
    assert!(matches!(assistant.content[3], Content::Image { .. }));

    // Adjacent text blocks are not merged.
    let two_text = vec![Content::text("a"), Content::text("b")];
    let split = Message::assistant(API, PROVIDER, MODEL, two_text, StopReason::Stop);
    assert_eq!(
        split.as_assistant().expect("assistant").content.len(),
        2,
        "adjacent text blocks must not be merged"
    );

    // Serialization round-trips the exact order.
    let wire = serde_json::to_value(&message).expect("serialize");
    let back: Message = serde_json::from_value(wire).expect("deserialize");
    assert_eq!(back, message);

    // An assistant message may be empty with a terminal stop reason.
    let empty = Message::assistant(API, PROVIDER, MODEL, Vec::new(), StopReason::Stop);
    assert!(empty.as_assistant().expect("assistant").content.is_empty());
}

// ---------------------------------------------------------------------------
// MSG-003: assistant metadata and optional-field discipline
// ---------------------------------------------------------------------------

#[test]
#[spec("MSG-003")]
fn spec_msg_003_assistant_metadata_roundtrip() {
    let mut message = AssistantMessage::partial(API, PROVIDER, MODEL);
    message.content = vec![Content::text("hello")];
    message.response_model = Some("served-model".to_owned());
    message.response_id = Some("resp_123".to_owned());
    message.thinking_level = Some("medium".to_owned());
    message.provider_thinking_level = Some("provider-medium".to_owned());
    message.usage = Usage::new(3, 4, 5, 6);
    message.stop_reason = StopReason::Stop;
    message.raw_stop_reason = Some("end_turn".to_owned());
    message.duration_ms = Some(42);
    message.timestamp = 1_700_000_000_000;

    let wire = serde_json::to_value(Message::Assistant(message.clone())).expect("serialize");
    let back: Message = serde_json::from_value(wire).expect("deserialize");
    assert_eq!(back, Message::Assistant(message));

    // Optional provider fields are absent when the provider did not supply them,
    // while `usage` is always present (zero-valued when unreported).
    let bare = AssistantMessage::partial(API, PROVIDER, MODEL);
    let bare_wire = serde_json::to_value(Message::Assistant(bare)).expect("serialize");
    assert!(bare_wire.get("response_model").is_none());
    assert!(bare_wire.get("response_id").is_none());
    assert!(bare_wire.get("error_message").is_none());
    assert!(bare_wire.get("usage").is_some());

    // error_message is present exactly for the error stop reason.
    let error = AssistantMessage::error(API, PROVIDER, MODEL, StopReason::Error, "boom");
    assert_eq!(error.stop_reason, StopReason::Error);
    assert_eq!(error.error_message.as_deref(), Some("boom"));
    let stopped = AssistantMessage::partial(API, PROVIDER, MODEL);
    assert!(stopped.error_message.is_none());
}

// ---------------------------------------------------------------------------
// MSG-004: stop reasons, pending is never finalized
// ---------------------------------------------------------------------------

#[test]
#[spec("MSG-004")]
fn spec_msg_004_pending_is_not_persistable() {
    let reasons = [
        (StopReason::Pending, "pending"),
        (StopReason::Stop, "stop"),
        (StopReason::Length, "length"),
        (StopReason::ToolUse, "tool_use"),
        (StopReason::Error, "error"),
        (StopReason::Aborted, "aborted"),
        (StopReason::Deferred, "deferred"),
    ];
    for (reason, wire) in reasons {
        assert_eq!(
            serde_json::to_value(reason).expect("serialize"),
            json!(wire)
        );
    }

    // A finalized message carries a non-pending reason; every terminal reason is
    // representable on a message.
    for reason in [
        StopReason::Stop,
        StopReason::Length,
        StopReason::ToolUse,
        StopReason::Error,
        StopReason::Aborted,
        StopReason::Deferred,
    ] {
        let message = Message::assistant(API, PROVIDER, MODEL, Vec::new(), reason);
        assert_eq!(
            message.as_assistant().expect("assistant").stop_reason,
            reason
        );
        assert_ne!(
            message.as_assistant().expect("assistant").stop_reason,
            StopReason::Pending
        );
    }
}

#[tokio::test]
#[spec("MSG-004")]
async fn spec_msg_004_stream_without_stop_reason_is_error() {
    let (stream, sender) = AssistantEventStream::channel(4);
    drop(sender);

    let message = stream.result().await;
    assert_eq!(
        message.stop_reason,
        StopReason::Error,
        "a stream without a terminal stop reason must not stay pending"
    );
}

// ---------------------------------------------------------------------------
// MSG-005: normalized usage
// ---------------------------------------------------------------------------

#[test]
#[spec("MSG-005")]
fn spec_msg_005_usage_normalization() {
    let usage = Usage::new(10, 20, 30, 40);
    assert_eq!(usage.input, 10);
    assert_eq!(usage.output, 20);
    assert_eq!(usage.cache_read, 30);
    assert_eq!(usage.cache_write, 40);
    assert_eq!(usage.total_tokens, 100);
    assert_eq!(usage.reasoning, None);

    // Reasoning is a subset of output and is not counted twice.
    let mut with_reasoning = usage.clone();
    with_reasoning.reasoning = Some(15);
    with_reasoning.recompute_total();
    assert_eq!(with_reasoning.total_tokens, 100);

    // Absent provider counters are zero, not inferred.
    let zero = Usage::default();
    assert_eq!(zero.total_tokens, 0);
    assert_eq!(zero.input, 0);
    assert_eq!(zero.output, 0);
    assert_eq!(zero.cache_read, 0);
    assert_eq!(zero.cache_write, 0);
    assert_eq!(zero.reasoning, None);
    assert!(zero.is_zero());

    let wire = serde_json::to_value(&with_reasoning).expect("serialize");
    let back: Usage = serde_json::from_value(wire).expect("deserialize");
    assert_eq!(back, with_reasoning);
}

// ---------------------------------------------------------------------------
// MSG-006: integral nano-USD costs
// ---------------------------------------------------------------------------

#[test]
#[spec("MSG-006")]
fn spec_msg_006_cost_is_integer_nano_usd() {
    let nanos: u64 = Cost::nanos_from_usd(0.00042);
    assert_eq!(nanos, 420_000);
    assert_eq!(
        Cost::nanos_from_usd(0.000_000_001),
        1,
        "sub-nano amounts round to the nearest nano-USD"
    );
    assert_eq!(Cost::nanos_from_usd(0.000_000_001_6), 2);
    assert_eq!(Cost::nanos_from_usd(0.000_000_000_4), 0);
    assert_eq!(Cost::nanos_from_usd(-1.0), 0);
    assert_eq!(Cost::nanos_from_usd(f64::NAN), 0);
    assert_eq!(Cost::nanos_from_usd(f64::INFINITY), 0);

    let cost = Cost::new(100, 200, 300, 400);
    assert_eq!(cost.total, 1_000);
    assert!(
        !cost.total_from_provider,
        "estimates are not provider costs"
    );
    assert!((cost.usd() - 0.000_001).abs() < f64::EPSILON);

    let wire = serde_json::to_value(cost).expect("serialize");
    assert!(
        wire["total"].is_u64(),
        "cost must serialize as an integer, got {wire:?}"
    );
}

// ---------------------------------------------------------------------------
// MSG-007: system prompt and tools are leading messages
// ---------------------------------------------------------------------------

#[test]
#[spec("MSG-007")]
fn spec_msg_007_system_and_tools_are_leading_messages() {
    let tools = vec![tool_decl("read"), tool_decl("write")];
    let context = normalize_context("system prompt", tools.clone(), vec![Message::user("hi")]);

    match &context[0] {
        Message::System(system) => {
            assert_eq!(system.content, "system prompt");
            assert_eq!(system.tools, tools);
        }
        other => panic!("expected a leading system message, got {other:?}"),
    }
    assert_eq!(current_tools(&context), tools);
    assert!(
        !context[1..]
            .iter()
            .any(|message| matches!(message, Message::System(_))),
        "tool declarations must not appear as a parallel field or trailing system turn"
    );

    // An existing leading system message is authoritative and is not duplicated.
    let existing = vec![
        Message::system("existing", tools.clone()),
        Message::user("hi"),
    ];
    assert_eq!(
        normalize_context("ignored", vec![tool_decl("other")], existing.clone()),
        existing
    );

    let wire = serde_json::to_value(&context).expect("serialize");
    assert!(wire[0].get("tools").is_some());
    assert!(
        wire[1].get("tools").is_none(),
        "only the leading system message carries tool declarations"
    );
}

// ---------------------------------------------------------------------------
// MSG-008: normalize and collapse are order-preserving and idempotent
// ---------------------------------------------------------------------------

#[test]
#[spec("MSG-008")]
fn spec_msg_008_normalize_and_collapse_system_messages() {
    let once = normalize_context("prompt", Vec::new(), vec![Message::user("hi")]);
    assert_eq!(once.len(), 2);
    assert!(matches!(once[0], Message::System(_)));
    assert_eq!(normalize_context("prompt", Vec::new(), once.clone()), once);

    let messages = vec![
        Message::system("one", vec![tool_decl("a")]),
        Message::user("hi"),
        Message::system("two", vec![tool_decl("b")]),
        Message::assistant(
            API,
            PROVIDER,
            MODEL,
            vec![Content::text("ok")],
            StopReason::Stop,
        ),
    ];
    let collapsed = collapse_system_messages(messages);

    assert_eq!(collapsed.len(), 3);
    match &collapsed[0] {
        Message::System(system) => {
            assert_eq!(system.content, "one\n\ntwo");
            assert_eq!(system.tools, vec![tool_decl("b")], "last tools win");
        }
        other => panic!("expected collapsed system message, got {other:?}"),
    }
    assert_eq!(collapsed[1].role(), "user");
    assert_eq!(collapsed[2].role(), "assistant");

    assert_eq!(
        collapse_system_messages(collapsed.clone()),
        collapsed,
        "collapsing must be idempotent"
    );
}

// ---------------------------------------------------------------------------
// MSG-009: provider-assigned tool-call IDs are opaque and preserved
// ---------------------------------------------------------------------------

#[test]
#[spec("MSG-009")]
fn spec_msg_009_tool_call_ids_are_preserved() {
    let original = "call_abc/with:odd.chars-Δ";
    let block = Content::ToolCall {
        id: original.to_owned(),
        name: "read".to_owned(),
        arguments: json!({"path": "a.txt"}),
        thought_signature: None,
    };

    let call: ToolCall = block.as_tool_call().expect("tool call");
    assert_eq!(
        call.id, original,
        "IDs must not be regenerated or reformatted"
    );

    let wire = serde_json::to_value(&block).expect("serialize");
    let back: Content = serde_json::from_value(wire).expect("deserialize");
    assert_eq!(back, block);

    let result = Message::tool_result(original, "read", vec![Content::text("contents")], false);
    match &result {
        Message::ToolResult(result) => {
            assert_eq!(result.tool_call_id, original);
            assert_eq!(result.tool_name, "read");
        }
        other => panic!("expected tool result, got {other:?}"),
    }

    // Distinct IDs stay distinct.
    let first = Content::ToolCall {
        id: "id-1".to_owned(),
        name: "read".to_owned(),
        arguments: json!({}),
        thought_signature: None,
    };
    let second = Content::ToolCall {
        id: "id-2".to_owned(),
        name: "read".to_owned(),
        arguments: json!({}),
        thought_signature: None,
    };
    assert_ne!(
        first.as_tool_call().expect("first").id,
        second.as_tool_call().expect("second").id
    );
}

// ---------------------------------------------------------------------------
// MSG-010: timestamps are Unix milliseconds captured at creation
// ---------------------------------------------------------------------------

#[test]
#[spec("MSG-010")]
fn spec_msg_010_timestamp_units() {
    let before = now_ms();
    let user = Message::user("hi");
    let after = now_ms();

    let timestamp = user.timestamp();
    assert!(
        timestamp >= before && timestamp <= after,
        "user timestamp {timestamp} outside [{before}, {after}]"
    );
    assert!(
        timestamp > 1_000_000_000_000,
        "timestamps must be Unix milliseconds"
    );

    // The timestamp survives serialization and replay unchanged.
    let wire = serde_json::to_value(&user).expect("serialize");
    let back: Message = serde_json::from_value(wire).expect("deserialize");
    assert_eq!(back.timestamp(), timestamp);

    let assistant = Message::assistant(
        API,
        PROVIDER,
        MODEL,
        vec![Content::text("hi")],
        StopReason::Stop,
    );
    assert!(assistant.timestamp() > 1_000_000_000_000);

    let result = Message::tool_result("call_1", "read", vec![Content::text("ok")], false);
    assert!(result.timestamp() > 1_000_000_000_000);

    // System messages are constructed without a creation time; the session layer
    // stamps entries as RFC 3339 strings.
    assert_eq!(Message::system("prompt", Vec::new()).timestamp(), 0);
}
