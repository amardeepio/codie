//! Conformance tests for the assistant event stream (MSG-040..MSG-044).

use codie_core::{
    AssistantDelta, AssistantEventStream, AssistantMessage, Content, StopReason, Usage,
};
use codie_testkit::spec;

const API: &str = "test-api";
const PROVIDER: &str = "test-provider";
const MODEL: &str = "test-model";

fn partial() -> AssistantMessage {
    AssistantMessage::partial(API, PROVIDER, MODEL)
}

fn final_text(text: &str, usage: Usage) -> AssistantMessage {
    let mut message = partial();
    message.content = vec![Content::text(text)];
    message.usage = usage;
    message.stop_reason = StopReason::Stop;
    message
}

/// Builds a provider stream with two text deltas that concatenate to `Hello`.
fn text_deltas(content: &str, usage: Usage) -> Vec<AssistantDelta> {
    let mut deltas = vec![
        AssistantDelta::Start { partial: partial() },
        AssistantDelta::TextStart { index: 0 },
    ];
    for chunk in content.as_bytes().chunks(2) {
        deltas.push(AssistantDelta::TextDelta {
            index: 0,
            delta: String::from_utf8(chunk.to_vec()).expect("ascii chunks"),
        });
    }
    deltas.push(AssistantDelta::TextEnd {
        index: 0,
        content: content.to_owned(),
    });
    deltas.push(AssistantDelta::Done {
        reason: StopReason::Stop,
        message: final_text(content, usage),
    });
    deltas
}

fn producer(deltas: Vec<AssistantDelta>) -> AssistantEventStream {
    let (stream, sender) = AssistantEventStream::channel(32);
    tokio::spawn(async move {
        for delta in deltas {
            sender.send(delta).await;
        }
    });
    stream
}

async fn drain(mut stream: AssistantEventStream) -> Vec<AssistantDelta> {
    let mut events = Vec::new();
    while let Some(event) = stream.recv().await {
        events.push(event);
    }
    events
}

fn terminal_count(events: &[AssistantDelta]) -> usize {
    events
        .iter()
        .filter(|event| event.terminal_message().is_some())
        .count()
}

// ---------------------------------------------------------------------------
// MSG-040: start precedes all content
// ---------------------------------------------------------------------------

#[tokio::test]
#[spec("MSG-040")]
async fn spec_msg_040_start_precedes_content() {
    let events = drain(producer(text_deltas("Hello", Usage::new(1, 2, 0, 0)))).await;

    assert!(
        matches!(events.first(), Some(AssistantDelta::Start { .. })),
        "start must be the first event"
    );
    let starts = events
        .iter()
        .filter(|event| matches!(event, AssistantDelta::Start { .. }))
        .count();
    assert_eq!(starts, 1, "at most one start per stream");
}

#[tokio::test]
#[spec("MSG-040")]
async fn spec_msg_040_setup_failure_may_skip_start() {
    let message = AssistantMessage::error(API, PROVIDER, MODEL, StopReason::Error, "setup failed");
    let events = drain(producer(vec![AssistantDelta::Error {
        reason: StopReason::Error,
        message,
    }]))
    .await;

    assert!(matches!(events.first(), Some(AssistantDelta::Error { .. })));
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, AssistantDelta::Start { .. }))
            .count(),
        0
    );
}

// ---------------------------------------------------------------------------
// MSG-041: deltas are append-only and match the authoritative block content
// ---------------------------------------------------------------------------

#[tokio::test]
#[spec("MSG-041")]
async fn spec_msg_041_partial_is_live() {
    let events = drain(producer(text_deltas("Hello", Usage::new(1, 2, 0, 0)))).await;

    let concatenated: String = events
        .iter()
        .filter_map(|event| match event {
            AssistantDelta::TextDelta { delta, .. } => Some(delta.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(concatenated, "Hello");

    let authoritative = events.iter().find_map(|event| match event {
        AssistantDelta::TextEnd { content, .. } => Some(content.clone()),
        _ => None,
    });
    assert_eq!(authoritative.as_deref(), Some("Hello"));
    assert_eq!(concatenated, authoritative.expect("text end"));

    // The partial carried by `start` is empty; content only arrives via deltas.
    let start_content = events.iter().find_map(|event| match event {
        AssistantDelta::Start { partial } => Some(partial.content.clone()),
        _ => None,
    });
    assert_eq!(start_content, Some(Vec::new()));
}

// ---------------------------------------------------------------------------
// MSG-042: exactly one terminal event
// ---------------------------------------------------------------------------

#[tokio::test]
#[spec("MSG-042")]
async fn spec_msg_042_exactly_one_terminal_event() {
    let events = drain(producer(text_deltas("Hello", Usage::new(1, 2, 0, 0)))).await;

    assert_eq!(terminal_count(&events), 1);
    assert!(
        events
            .last()
            .and_then(AssistantDelta::terminal_message)
            .is_some(),
        "the terminal event must be last"
    );
}

#[tokio::test]
#[spec("MSG-042")]
async fn spec_msg_042_stream_without_terminal_is_error() {
    let (stream, sender) = AssistantEventStream::channel(4);
    sender
        .send(AssistantDelta::Start { partial: partial() })
        .await;
    drop(sender);

    let message = stream.result().await;
    assert_eq!(message.stop_reason, StopReason::Error);
    assert!(
        message
            .error_message
            .as_deref()
            .is_some_and(|error| error.contains("without a terminal")),
        "unexpected error: {:?}",
        message.error_message
    );
}

// ---------------------------------------------------------------------------
// MSG-043: redacted thinking may be complete at start and emit no deltas
// ---------------------------------------------------------------------------

#[tokio::test]
#[spec("MSG-043")]
async fn spec_msg_043_redacted_thinking_complete_at_start() {
    let redacted = Content::Thinking {
        thinking: "hidden reasoning".to_owned(),
        signature: Some("sig".to_owned()),
        redacted: true,
    };

    let mut started = partial();
    started.content = vec![redacted.clone()];
    let mut done = partial();
    done.content = vec![redacted.clone()];
    done.stop_reason = StopReason::Stop;

    let deltas = vec![
        AssistantDelta::Start { partial: started },
        AssistantDelta::Done {
            reason: StopReason::Stop,
            message: done.clone(),
        },
    ];

    let events = drain(producer(deltas.clone())).await;
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(
                event,
                AssistantDelta::ThinkingStart { .. }
                    | AssistantDelta::ThinkingDelta { .. }
                    | AssistantDelta::ThinkingEnd { .. }
            ))
            .count(),
        0,
        "redacted thinking does not require deltas"
    );

    let message = producer(deltas).result().await;
    assert_eq!(message.content.len(), 1);
    match &message.content[0] {
        Content::Thinking {
            thinking,
            redacted: is_redacted,
            ..
        } => {
            assert_eq!(thinking, "hidden reasoning");
            assert!(is_redacted);
        }
        other => panic!("expected redacted thinking, got {other:?}"),
    }
    assert!(
        message.content.iter().all(|content| !content.is_text()),
        "redacted thinking must not be summarized into visible text"
    );
}

// ---------------------------------------------------------------------------
// MSG-044: recv() and result() observe the same terminal message
// ---------------------------------------------------------------------------

#[tokio::test]
#[spec("MSG-044")]
async fn spec_msg_044_iterate_and_await_do_not_race() {
    let usage = Usage::new(11, 22, 33, 44);
    let deltas = text_deltas("Hello", usage.clone());

    let mut stream = producer(deltas.clone());
    let mut terminal = None;
    while let Some(event) = stream.recv().await {
        if let Some(message) = event.terminal_message() {
            terminal = Some(message.clone());
        }
    }
    let awaited = producer(deltas).result().await;

    let terminal = terminal.expect("terminal event");
    assert_eq!(
        terminal, awaited,
        "the awaited message must equal the terminal event message"
    );
    assert_eq!(awaited.stop_reason, StopReason::Stop);
    assert_eq!(awaited.usage, usage);
}
