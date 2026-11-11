//! Context compaction for the interactive app (`CTX-001`..`CTX-007`).
//!
//! Summaries are produced by the active chat client and persisted as a
//! `compaction` entry. The session file is never rewritten: the projection hides
//! the compacted prefix and inserts the summary after the system message.

use std::sync::Arc;

use codie_core::{ChatClient, Content, Message, ModelRef, StreamOptions, ThinkingLevel, Usage};
use codie_session::Session;
use tokio_util::sync::CancellationToken;

/// Tokens reserved for the model's response and the next turn (`CTX-001`).
pub const DEFAULT_RESERVE_TOKENS: u64 = 16_384;

/// Tokens of recent history retained after compaction (`CTX-001`).
pub const DEFAULT_KEEP_TOKENS: u64 = 20_000;

/// Tool output is truncated before it enters a summary (`CTX-004`).
const TOOL_OUTPUT_LIMIT: usize = 2_000;

/// One message is capped before it enters a summary prompt.
const MESSAGE_LIMIT: usize = 20_000;

/// Patterns that identify a provider context-overflow error (`CTX-006`).
const OVERFLOW_PATTERNS: &[&str] = &[
    "context length",
    "context_length",
    "context window",
    "maximum context",
    "too many tokens",
    "prompt is too long",
    "reduce the length",
    "maximum number of tokens",
    "input is too long",
    "token limit",
];

const SUMMARY_INSTRUCTIONS: &str = "\
You are compacting a coding-agent conversation so work can continue in a smaller context window.
Preserve only what is needed to keep working. Return Markdown with exactly these sections:

## Goal
## Constraints
## Progress
## Key Decisions
## Next Steps
## Critical Context

Then list the files touched:

<read-files>
</read-files>
<modified-files>
</modified-files>

Omit thinking blocks. Truncate tool output. Never invent work that did not happen.";

/// The model's summary and the usage it consumed.
#[derive(Debug, Clone)]
pub struct CompactionOutcome {
    /// Model-written summary.
    pub summary: String,
    /// Usage for the summarization request.
    pub usage: Usage,
}

/// Asks the model to summarize `messages`.
///
/// The transcript is rendered as plain text with thinking blocks omitted and
/// tool output truncated, matching `CTX-004`. Errors are user-facing strings.
pub async fn summarize(
    client: Arc<dyn ChatClient>,
    model: ModelRef,
    thinking: ThinkingLevel,
    messages: &[Message],
    instructions: Option<&str>,
    session_id: Option<String>,
    input_limit_tokens: u64,
) -> Result<CompactionOutcome, String> {
    let mut prompt = String::from("Conversation to summarize:\n\n<transcript>\n");
    let budget_chars = input_limit_tokens.saturating_mul(4).max(4_000) as usize;
    let mut rendered = Vec::new();
    let mut used = 0_usize;
    for message in messages.iter().rev() {
        let mut text = render_message(message);
        if text.is_empty() {
            continue;
        }
        let remaining = budget_chars.saturating_sub(used);
        if remaining == 0 {
            break;
        }
        if text.chars().count() > remaining {
            text = text.chars().take(remaining).collect();
            text.push_str("\n[truncated]");
        }
        used = used.saturating_add(text.chars().count());
        rendered.push(text);
        if used >= budget_chars {
            break;
        }
    }
    if rendered.len() < messages.len() {
        prompt.push_str("[older messages omitted to fit the summary request]\n\n");
    }
    for text in rendered.into_iter().rev() {
        prompt.push_str(&text);
        prompt.push_str("\n\n");
    }
    prompt.push_str("</transcript>");
    if let Some(instructions) = instructions
        && !instructions.trim().is_empty()
    {
        prompt.push_str("\n\nAdditional instructions: ");
        prompt.push_str(instructions.trim());
    }

    let context = vec![
        Message::system(SUMMARY_INSTRUCTIONS, Vec::new()),
        Message::user(prompt),
    ];
    let options = StreamOptions {
        model,
        thinking,
        max_output_tokens: None,
        temperature: Some(0.2),
        session_id,
        cancel: CancellationToken::new(),
    };
    let message = client.stream(&context, &options).result().await;
    if message.stop_reason.is_terminal_failure() {
        return Err(message
            .error_message
            .unwrap_or_else(|| "the summarization request failed".to_owned()));
    }
    let summary = message.text();
    if summary.trim().is_empty() {
        return Err("the model returned an empty summary".to_owned());
    }
    Ok(CompactionOutcome {
        summary,
        usage: message.usage.clone(),
    })
}

/// Estimates tokens for a transcript with the documented `CTX-002` rule.
#[must_use]
pub fn estimate_messages_tokens(messages: &[Message]) -> u64 {
    messages.iter().map(estimate_tokens).sum()
}

/// Returns true when a request would exceed the window minus the reserve.
///
/// The prompt is counted before it is appended to history (`CTX-001`).
#[must_use]
pub fn needs_compaction(messages: &[Message], prompt: &Message, context_window: u64) -> bool {
    if context_window == 0 {
        return false;
    }
    let budget = context_window.saturating_sub(DEFAULT_RESERVE_TOKENS);
    let mut tokens = estimate_messages_tokens(messages);
    tokens = tokens.saturating_add(estimate_messages_tokens(std::slice::from_ref(prompt)));
    tokens > budget
}

/// Returns true when a failed assistant message reports a context overflow.
///
/// Providers disagree on wording, so the check is a case-insensitive search for
/// the documented overflow phrases in the error text (`CTX-006`).
#[must_use]
pub fn is_context_overflow(message: &Message) -> bool {
    let Message::Assistant(assistant) = message else {
        return false;
    };
    if assistant.stop_reason != codie_core::StopReason::Error {
        return false;
    }
    let mut text = assistant.error_message.clone().unwrap_or_default();
    if let Some(raw) = assistant.raw_stop_reason.as_ref() {
        text.push(' ');
        text.push_str(raw);
    }
    let text = text.to_lowercase();
    OVERFLOW_PATTERNS
        .iter()
        .any(|pattern| text.contains(pattern))
}

/// Appends a compaction entry and returns the new projection.
///
/// The retained tail is bounded by [`DEFAULT_KEEP_TOKENS`] and never starts at a
/// tool result (`CTX-003`). The system message is always retained.
pub fn apply(
    session: &mut Session,
    outcome: &CompactionOutcome,
    instructions: Option<&str>,
) -> Result<Vec<Message>, String> {
    let branch = session.branch();
    let messages: Vec<&codie_session::Entry> = branch
        .iter()
        .copied()
        .filter(|entry| entry.message().is_some())
        .collect();
    let Some(first_kept) = cut_point(&messages) else {
        return Err("there is nothing to compact".to_owned());
    };
    let tokens_before = session.estimate_tokens();
    let details = instructions
        .map(|instructions| serde_json::json!({"instructions": instructions}))
        .filter(|_| instructions.is_some_and(|value| !value.trim().is_empty()));

    session
        .append_compaction(
            outcome.summary.trim(),
            Some(&first_kept),
            tokens_before,
            details,
            Some(outcome.usage.clone()),
        )
        .map_err(|error| error.to_string())?;
    Ok(session.projection())
}

/// Chooses the first retained message entry identifier.
fn cut_point(messages: &[&codie_session::Entry]) -> Option<String> {
    let first = messages.first()?;
    let system_kept = usize::from(matches!(first.message(), Some(Message::System(_))));
    if messages.len() <= system_kept {
        return None;
    }

    let mut total = 0_u64;
    let mut index = messages.len();
    for position in (system_kept..messages.len()).rev() {
        let Some(message) = messages[position].message() else {
            continue;
        };
        total = total.saturating_add(estimate_tokens(message));
        if total > DEFAULT_KEEP_TOKENS {
            index = position + 1;
            break;
        }
    }
    if index == messages.len() {
        // The whole transcript fits the retained budget. A manual compaction
        // still summarizes all but the last few messages rather than doing
        // nothing, while automatic compaction will skip this case.
        index = messages.len().saturating_sub(4).max(system_kept);
    }

    // Keep at least the final message, and never start the retained tail at a
    // tool result (`CTX-003`).
    index = index.min(messages.len() - 1).max(system_kept);
    while index > system_kept && matches!(messages[index].message(), Some(Message::ToolResult(_))) {
        index -= 1;
    }
    Some(messages[index].id().to_owned())
}

/// Renders one message for the summarization prompt.
fn render_message(message: &Message) -> String {
    match message {
        Message::System(_) => String::new(),
        Message::User(user) => cap_message(format!(
            "[user]\n{}",
            user.content
                .iter()
                .filter_map(Content::as_text)
                .collect::<Vec<_>>()
                .join("")
        )),
        Message::Assistant(assistant) => {
            let mut text = assistant.text();
            for call in assistant.tool_calls() {
                text.push_str(&format!("\n[called {} {}]", call.name, call.arguments));
            }
            cap_message(format!("[assistant]\n{text}"))
        }
        Message::ToolResult(result) => {
            let mut output = result
                .content
                .iter()
                .filter_map(Content::as_text)
                .collect::<Vec<_>>()
                .join("\n");
            if output.chars().count() > TOOL_OUTPUT_LIMIT {
                output = output.chars().take(TOOL_OUTPUT_LIMIT).collect();
                output.push_str("\n[truncated]");
            }
            cap_message(format!("[tool result: {}]\n{output}", result.tool_name))
        }
    }
}

/// Caps one rendered message so a single turn cannot dominate the prompt.
fn cap_message(text: String) -> String {
    if text.chars().count() <= MESSAGE_LIMIT {
        return text;
    }
    let mut capped: String = text.chars().take(MESSAGE_LIMIT).collect();
    capped.push_str("\n[truncated]");
    capped
}

/// The `chars / 4` estimate used by the cut point (`CTX-002`).
fn estimate_tokens(message: &Message) -> u64 {
    let chars = match message {
        Message::System(system) => system.content.chars().count() as u64,
        Message::User(user) => content_chars(&user.content),
        Message::Assistant(assistant) => content_chars(&assistant.content),
        Message::ToolResult(result) => content_chars(&result.content),
    };
    chars.div_ceil(4)
}

fn content_chars(content: &[Content]) -> u64 {
    content
        .iter()
        .map(|content| match content {
            Content::Text { text, .. } => text.chars().count() as u64,
            Content::Thinking { thinking, .. } => thinking.chars().count() as u64,
            Content::ToolCall { arguments, .. } => arguments.to_string().chars().count() as u64,
            Content::Image { .. } => 4_800,
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use codie_core::{AssistantDelta, AssistantEventStream, StopReason};
    use codie_testkit::spec;

    struct ScriptedClient {
        text: String,
        fail: bool,
    }

    impl ChatClient for ScriptedClient {
        fn stream(&self, _context: &[Message], _options: &StreamOptions) -> AssistantEventStream {
            let (stream, sender) = AssistantEventStream::channel(2);
            let text = self.text.clone();
            let fail = self.fail;
            tokio::spawn(async move {
                let mut message =
                    codie_core::AssistantMessage::partial("openai-completions", "fake", "fake");
                message.content = vec![Content::text(text.clone())];
                message.usage = Usage::new(10, 5, 0, 0);
                message.stop_reason = if fail {
                    StopReason::Error
                } else {
                    StopReason::Stop
                };
                if fail {
                    message.error_message = Some("boom".to_owned());
                }
                let _ = sender
                    .send(AssistantDelta::Done {
                        reason: message.stop_reason,
                        message,
                    })
                    .await;
            });
            stream
        }
    }

    fn model() -> ModelRef {
        ModelRef {
            api: "openai-completions".to_owned(),
            provider: "fake".to_owned(),
            model: "fake".to_owned(),
        }
    }

    #[tokio::test]
    async fn summarize_returns_text_and_usage() {
        let client = Arc::new(ScriptedClient {
            text: "## Goal\nfinish".to_owned(),
            fail: false,
        });
        let outcome = summarize(
            client,
            model(),
            ThinkingLevel::Low,
            &[Message::user("hello"), Message::user("world")],
            Some("keep tests"),
            None,
            50_000,
        )
        .await
        .expect("summary");
        assert_eq!(outcome.summary, "## Goal\nfinish");
        assert_eq!(outcome.usage.total_tokens, 15);
    }

    #[tokio::test]
    async fn summarize_surfaces_provider_errors() {
        let client = Arc::new(ScriptedClient {
            text: String::new(),
            fail: true,
        });
        let error = summarize(
            client,
            model(),
            ThinkingLevel::Low,
            &[Message::user("hello")],
            None,
            None,
            50_000,
        )
        .await
        .expect_err("error");
        assert_eq!(error, "boom");
    }

    #[test]
    fn render_omits_thinking_and_truncates_tool_output() {
        let mut assistant = codie_core::AssistantMessage::partial("api", "provider", "model");
        assistant.content = vec![Content::text("answer"), Content::thinking("hidden")];
        let rendered = render_message(&Message::Assistant(assistant));
        assert!(rendered.contains("answer"));
        assert!(!rendered.contains("hidden"));

        let long = "x".repeat(TOOL_OUTPUT_LIMIT + 10);
        let rendered = render_message(&Message::tool_result(
            "call",
            "read",
            vec![Content::text(long)],
            false,
        ));
        assert!(rendered.contains("[truncated]"));
    }

    fn outcome() -> CompactionOutcome {
        CompactionOutcome {
            summary: "## Goal\ncontinue".to_owned(),
            usage: Usage::new(100, 20, 0, 0),
        }
    }

    #[test]
    fn apply_keeps_the_system_message_before_the_summary() {
        let temp = tempfile::tempdir().unwrap();
        let mut session = Session::create(temp.path(), temp.path().join("sessions")).unwrap();
        session
            .append_message(&Message::system("system prompt", Vec::new()))
            .unwrap();
        session
            .append_message(&Message::user("old request"))
            .unwrap();
        session
            .append_message(&Message::assistant(
                "api",
                "provider",
                "model",
                vec![Content::text("recent answer")],
                StopReason::Stop,
            ))
            .unwrap();

        let projected = apply(&mut session, &outcome(), Some("keep tests")).unwrap();
        assert!(matches!(projected[0], Message::System(_)));
        let summary = match &projected[1] {
            Message::User(user) => user
                .content
                .iter()
                .filter_map(Content::as_text)
                .collect::<Vec<_>>()
                .join(""),
            other => panic!("expected a summary message, got {other:?}"),
        };
        assert!(summary.contains("Summary of the conversation"));
        assert_eq!(projected.len(), 4, "system, summary, and the retained tail");
        let compaction = session
            .entries()
            .iter()
            .rev()
            .find_map(|entry| entry.compaction())
            .expect("compaction entry");
        assert!(compaction.2 > 0, "tokens_before must be recorded");
    }

    #[test]
    fn apply_never_starts_the_tail_at_a_tool_result() {
        let temp = tempfile::tempdir().unwrap();
        let mut session = Session::create(temp.path(), temp.path().join("sessions")).unwrap();
        session
            .append_message(&Message::system("system prompt", Vec::new()))
            .unwrap();
        session
            .append_message(&Message::assistant(
                "api",
                "provider",
                "model",
                vec![Content::text("x".repeat(100_000))],
                StopReason::ToolUse,
            ))
            .unwrap();
        session
            .append_message(&Message::tool_result(
                "call",
                "read",
                vec![Content::text("result")],
                false,
            ))
            .unwrap();
        session.append_message(&Message::user("recent")).unwrap();

        let projected = apply(&mut session, &outcome(), None).unwrap();
        let compaction = session
            .entries()
            .iter()
            .rev()
            .find_map(|entry| entry.compaction())
            .expect("compaction");
        let first_kept = compaction.1.expect("first kept");
        let kept = session
            .entries()
            .iter()
            .find(|entry| entry.id() == first_kept)
            .unwrap();
        assert!(
            !matches!(kept.message(), Some(Message::ToolResult(_))),
            "tail must not start at a tool result"
        );
        assert!(
            projected
                .iter()
                .skip(1)
                .any(|message| message.as_assistant().is_some()),
            "the assistant tool call must be retained with its result"
        );
    }

    #[test]
    fn apply_reports_an_empty_session() {
        let temp = tempfile::tempdir().unwrap();
        let mut session = Session::create(temp.path(), temp.path().join("sessions")).unwrap();
        let error = apply(&mut session, &outcome(), None).unwrap_err();
        assert!(error.contains("nothing to compact"));
    }

    #[test]
    #[spec("CTX-004")]
    fn spec_ctx_004_summary_prompt_has_the_fixed_structure() {
        for section in [
            "## Goal",
            "## Constraints",
            "## Progress",
            "## Key Decisions",
            "## Next Steps",
            "## Critical Context",
            "<read-files>",
            "<modified-files>",
        ] {
            assert!(
                SUMMARY_INSTRUCTIONS.contains(section),
                "missing summary section {section}"
            );
        }
        assert!(SUMMARY_INSTRUCTIONS.contains("Omit thinking blocks"));
        assert!(SUMMARY_INSTRUCTIONS.contains("Truncate tool output"));
    }

    #[test]
    #[spec("CTX-001")]
    fn spec_ctx_001_threshold_accounts_for_reserve_and_prompt() {
        let history = vec![Message::user("x".repeat(40_000))];
        let prompt = Message::user("y".repeat(4_000));
        // 10,000 + 1,000 tokens exceeds a 20,000-token window minus 16,384 reserve.
        assert!(needs_compaction(&history, &prompt, 20_000));
        // A 100,000-token window leaves enough room.
        assert!(!needs_compaction(&history, &prompt, 100_000));
        assert!(!needs_compaction(&history, &prompt, 0));
    }

    #[test]
    #[spec("CTX-006")]
    fn spec_ctx_006_overflow_detection_matches_provider_wording() {
        let overflow = |text: &str| {
            Message::Assistant(codie_core::AssistantMessage::error(
                "api",
                "provider",
                "model",
                StopReason::Error,
                text,
            ))
        };
        assert!(is_context_overflow(&overflow(
            "This model's maximum context length is 128000 tokens"
        )));
        assert!(is_context_overflow(&overflow(
            "The prompt is too long for the context window"
        )));
        assert!(!is_context_overflow(&overflow("invalid API key")));
        assert!(!is_context_overflow(&Message::user("context length")));

        let mut length = codie_core::AssistantMessage::partial("api", "provider", "model");
        length.stop_reason = StopReason::Length;
        length.error_message = Some("maximum context length".to_owned());
        assert!(
            !is_context_overflow(&Message::Assistant(length)),
            "only terminal errors trigger recovery"
        );
    }
}
