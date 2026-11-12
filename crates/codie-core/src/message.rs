//! Message model, normalization, and system-message replay.

use serde::{Deserialize, Serialize};

use crate::content::{Content, StopReason, ToolCall, ToolDecl, Usage};

/// A system message, including the tool declarations effective at that point.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SystemMessage {
    /// Prompt text.
    pub content: String,
    /// Tool declarations effective from this message onward.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ToolDecl>,
    /// Creation time in Unix milliseconds.
    #[serde(default)]
    pub timestamp: i64,
}

/// A user message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserMessage {
    /// Text and image blocks.
    pub content: Vec<Content>,
    /// Creation time in Unix milliseconds.
    #[serde(default)]
    pub timestamp: i64,
}

/// An assistant message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssistantMessage {
    /// Text, thinking, and tool-call blocks in source order.
    pub content: Vec<Content>,
    /// Wire API identifier.
    pub api: String,
    /// Provider identifier.
    pub provider: String,
    /// Model identifier used for the request.
    pub model: String,
    /// Model identifier reported by the provider, when it differs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_model: Option<String>,
    /// Provider response identifier, when supplied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_id: Option<String>,
    /// Requested thinking level.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_level: Option<String>,
    /// Provider-native thinking level, when it differs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_thinking_level: Option<String>,
    /// Normalized token usage.
    #[serde(default)]
    pub usage: Usage,
    /// Why the turn stopped.
    pub stop_reason: StopReason,
    /// Error message when `stop_reason` is `error`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    /// Raw provider stop reason.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_stop_reason: Option<String>,
    /// Request duration in milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// Creation time in Unix milliseconds.
    #[serde(default)]
    pub timestamp: i64,
}

impl AssistantMessage {
    /// Creates an empty partial message for the given model.
    #[must_use]
    pub fn partial(
        api: impl Into<String>,
        provider: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            content: Vec::new(),
            api: api.into(),
            provider: provider.into(),
            model: model.into(),
            response_model: None,
            response_id: None,
            thinking_level: None,
            provider_thinking_level: None,
            usage: Usage::default(),
            stop_reason: StopReason::Pending,
            error_message: None,
            raw_stop_reason: None,
            duration_ms: None,
            timestamp: now_ms(),
        }
    }

    /// Creates a final error message.
    #[must_use]
    pub fn error(
        api: impl Into<String>,
        provider: impl Into<String>,
        model: impl Into<String>,
        reason: StopReason,
        message: impl Into<String>,
    ) -> Self {
        let mut assistant = Self::partial(api, provider, model);
        assistant.stop_reason = reason;
        assistant.error_message = Some(message.into());
        assistant
    }

    /// Returns every tool call in content order.
    #[must_use]
    pub fn tool_calls(&self) -> Vec<ToolCall> {
        self.content
            .iter()
            .filter_map(Content::as_tool_call)
            .collect()
    }

    /// Concatenates all text blocks.
    #[must_use]
    pub fn text(&self) -> String {
        self.content
            .iter()
            .filter_map(Content::as_text)
            .collect::<Vec<_>>()
            .join("")
    }
}

/// The result of one tool call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolResultMessage {
    /// Identifier of the call this result answers.
    pub tool_call_id: String,
    /// Name of the tool that ran.
    pub tool_name: String,
    /// Model-visible content.
    pub content: Vec<Content>,
    /// Host-only structured details.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
    /// True when the tool failed.
    #[serde(default)]
    pub is_error: bool,
    /// Execution time in milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// Creation time in Unix milliseconds.
    #[serde(default)]
    pub timestamp: i64,
}

/// One transcript message.
///
/// The assistant variant is intentionally stored inline: it is the dominant
/// message type and boxing it would add an allocation to every turn. The
/// resulting size difference is accepted and documented.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "snake_case")]
pub enum Message {
    /// System prompt and tool declarations.
    System(SystemMessage),
    /// User input.
    User(UserMessage),
    /// Model output.
    Assistant(AssistantMessage),
    /// Tool output.
    ToolResult(ToolResultMessage),
}

impl Message {
    /// Creates a system message.
    #[must_use]
    pub fn system(content: impl Into<String>, tools: Vec<ToolDecl>) -> Self {
        Self::System(SystemMessage {
            content: content.into(),
            tools,
            timestamp: 0,
        })
    }

    /// Creates a text-only user message.
    #[must_use]
    pub fn user(text: impl Into<String>) -> Self {
        Self::User(UserMessage {
            content: vec![Content::text(text)],
            timestamp: now_ms(),
        })
    }

    /// Creates a user message from pre-built content blocks.
    #[must_use]
    pub fn user_with_content(content: Vec<Content>) -> Self {
        Self::User(UserMessage {
            content,
            timestamp: now_ms(),
        })
    }

    /// Creates an assistant message from content and a stop reason.
    #[must_use]
    pub fn assistant(
        api: impl Into<String>,
        provider: impl Into<String>,
        model: impl Into<String>,
        content: Vec<Content>,
        stop_reason: StopReason,
    ) -> Self {
        let mut message = AssistantMessage::partial(api, provider, model);
        message.content = content;
        message.stop_reason = stop_reason;
        Self::Assistant(message)
    }

    /// Creates a tool result message.
    #[must_use]
    pub fn tool_result(
        tool_call_id: impl Into<String>,
        tool_name: impl Into<String>,
        content: Vec<Content>,
        is_error: bool,
    ) -> Self {
        Self::ToolResult(ToolResultMessage {
            tool_call_id: tool_call_id.into(),
            tool_name: tool_name.into(),
            content,
            details: None,
            is_error,
            duration_ms: None,
            timestamp: now_ms(),
        })
    }

    /// Returns the message timestamp in Unix milliseconds.
    #[must_use]
    pub const fn timestamp(&self) -> i64 {
        match self {
            Self::System(message) => message.timestamp,
            Self::User(message) => message.timestamp,
            Self::Assistant(message) => message.timestamp,
            Self::ToolResult(message) => message.timestamp,
        }
    }

    /// Returns the assistant payload when this is an assistant message.
    #[must_use]
    pub const fn as_assistant(&self) -> Option<&AssistantMessage> {
        match self {
            Self::Assistant(message) => Some(message),
            _ => None,
        }
    }

    /// Returns the role name used on the wire and in JSON events.
    #[must_use]
    pub const fn role(&self) -> &'static str {
        match self {
            Self::System(_) => "system",
            Self::User(_) => "user",
            Self::Assistant(_) => "assistant",
            Self::ToolResult(_) => "tool_result",
        }
    }
}

/// Ensures a transcript starts with exactly one system message.
///
/// When the transcript already starts with a system message it is returned
/// unchanged. Otherwise a leading system message is inserted.
#[must_use]
pub fn normalize_context(
    system_prompt: &str,
    tools: Vec<ToolDecl>,
    messages: Vec<Message>,
) -> Vec<Message> {
    if matches!(messages.first(), Some(Message::System(_))) {
        return messages;
    }

    let mut normalized = Vec::with_capacity(messages.len() + 1);
    normalized.push(Message::system(system_prompt, tools));
    normalized.extend(messages);
    normalized
}

/// Collapses every system message into one leading system message.
///
/// Providers that reject mid-conversation system turns use this projection.
/// Content is concatenated in order and the last tool declaration set wins.
#[must_use]
pub fn collapse_system_messages(messages: Vec<Message>) -> Vec<Message> {
    let mut system_content: Vec<String> = Vec::new();
    let mut system_tools = Vec::new();
    let mut timestamp = 0;
    let mut rest = Vec::with_capacity(messages.len());

    for message in messages {
        match message {
            Message::System(system) => {
                if !system.content.is_empty() {
                    system_content.push(system.content);
                }
                if !system.tools.is_empty() {
                    system_tools = system.tools;
                }
                timestamp = system.timestamp;
            }
            other => rest.push(other),
        }
    }

    if system_content.is_empty() && system_tools.is_empty() {
        return rest;
    }

    let mut collapsed = Vec::with_capacity(rest.len() + 1);
    collapsed.push(Message::System(SystemMessage {
        content: system_content.join("\n\n"),
        tools: system_tools,
        timestamp,
    }));
    collapsed.extend(rest);
    collapsed
}

/// Returns the tool declarations currently in effect.
#[must_use]
pub fn current_tools(messages: &[Message]) -> Vec<ToolDecl> {
    messages
        .iter()
        .rev()
        .find_map(|message| match message {
            Message::System(system) if !system.tools.is_empty() => Some(system.tools.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

/// Current time in Unix milliseconds, saturating at zero before the epoch.
#[must_use]
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::Usage;

    #[test]
    fn normalization_prepends_one_system_message() {
        let messages = normalize_context("prompt", Vec::new(), vec![Message::user("hi")]);
        assert_eq!(messages.len(), 2);
        assert!(matches!(messages[0], Message::System(_)));
        assert_eq!(messages[0].role(), "system");
    }

    #[test]
    fn normalization_is_idempotent() {
        let once = normalize_context("prompt", Vec::new(), vec![Message::user("hi")]);
        let twice = normalize_context("prompt", Vec::new(), once.clone());
        assert_eq!(once, twice);
    }

    #[test]
    fn collapsing_merges_system_messages_and_keeps_order() {
        let messages = vec![
            Message::system("one", Vec::new()),
            Message::user("hi"),
            Message::system("two", Vec::new()),
            Message::assistant(
                "openai-completions",
                "p",
                "m",
                vec![Content::text("ok")],
                StopReason::Stop,
            ),
        ];
        let collapsed = collapse_system_messages(messages);
        assert_eq!(collapsed.len(), 3);
        match &collapsed[0] {
            Message::System(system) => assert_eq!(system.content, "one\n\ntwo"),
            _ => panic!("expected system message"),
        }
        assert_eq!(collapsed[1].role(), "user");
        assert_eq!(collapsed[2].role(), "assistant");
    }

    #[test]
    fn assistant_extracts_tool_calls_and_text() {
        let message = Message::assistant(
            "api",
            "provider",
            "model",
            vec![
                Content::text("hello "),
                Content::Text {
                    text: "world".into(),
                    signature: None,
                },
                Content::ToolCall {
                    id: "call_1".into(),
                    name: "read".into(),
                    arguments: serde_json::json!({"path": "a"}),
                    thought_signature: None,
                },
            ],
            StopReason::ToolUse,
        );
        let assistant = message.as_assistant().unwrap();
        assert_eq!(assistant.text(), "hello world");
        assert_eq!(assistant.tool_calls().len(), 1);
    }

    #[test]
    fn usage_is_always_present_on_assistant_messages() {
        let message = Message::assistant("a", "p", "m", Vec::new(), StopReason::Stop);
        let assistant = message.as_assistant().unwrap();
        assert_eq!(assistant.usage, Usage::default());
    }

    #[test]
    fn timestamps_are_milliseconds() {
        let before = now_ms();
        let message = Message::user("x");
        let after = now_ms();
        let timestamp = message.timestamp();
        assert!(timestamp >= before && timestamp <= after);
        assert!(timestamp > 1_000_000_000_000);
    }
}
