//! Provider deltas, agent events, and their stream primitives.

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::content::{StopReason, ToolCall};
use crate::message::{AssistantMessage, Message, ToolResultMessage};
use crate::tool::{ToolProgress, ToolResult};

/// One event from a provider stream.
///
/// Terminal variants carry the full assistant message; the size difference is
/// accepted to keep streaming allocation-free.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AssistantDelta {
    /// The stream started. The partial message has `stop_reason = pending`.
    Start {
        /// Live partial message.
        partial: AssistantMessage,
    },
    /// A text block started at `index`.
    TextStart {
        /// Content index.
        index: usize,
    },
    /// Text was appended to the block at `index`.
    TextDelta {
        /// Content index.
        index: usize,
        /// Appended text.
        delta: String,
    },
    /// A text block ended with authoritative content.
    TextEnd {
        /// Content index.
        index: usize,
        /// Full block text.
        content: String,
    },
    /// A thinking block started at `index`.
    ThinkingStart {
        /// Content index.
        index: usize,
    },
    /// Thinking text was appended.
    ThinkingDelta {
        /// Content index.
        index: usize,
        /// Appended thinking.
        delta: String,
    },
    /// A thinking block ended with authoritative content.
    ThinkingEnd {
        /// Content index.
        index: usize,
        /// Full thinking text.
        content: String,
    },
    /// A tool call block started at `index`.
    ToolCallStart {
        /// Content index.
        index: usize,
    },
    /// Tool-call argument JSON was appended.
    ToolCallDelta {
        /// Content index.
        index: usize,
        /// Appended JSON text.
        delta: String,
    },
    /// A tool call block ended with the complete call.
    ToolCallEnd {
        /// Content index.
        index: usize,
        /// Parsed call.
        call: ToolCall,
    },
    /// The stream completed successfully.
    Done {
        /// Terminal stop reason.
        reason: StopReason,
        /// Final assistant message.
        message: AssistantMessage,
    },
    /// The stream failed or was aborted.
    Error {
        /// Terminal stop reason (`error` or `aborted`).
        reason: StopReason,
        /// Final assistant message.
        message: AssistantMessage,
    },
}

impl AssistantDelta {
    /// Returns the terminal message, when this is a terminal event.
    #[must_use]
    pub const fn terminal_message(&self) -> Option<&AssistantMessage> {
        match self {
            Self::Done { message, .. } | Self::Error { message, .. } => Some(message),
            _ => None,
        }
    }

    /// Returns the content index, when the event targets a block.
    #[must_use]
    pub const fn content_index(&self) -> Option<usize> {
        match self {
            Self::TextStart { index }
            | Self::TextDelta { index, .. }
            | Self::TextEnd { index, .. }
            | Self::ThinkingStart { index }
            | Self::ThinkingDelta { index, .. }
            | Self::ThinkingEnd { index, .. }
            | Self::ToolCallStart { index }
            | Self::ToolCallDelta { index, .. }
            | Self::ToolCallEnd { index, .. } => Some(*index),
            _ => None,
        }
    }
}

/// The receiving half of a provider stream.
#[derive(Debug)]
pub struct AssistantEventStream {
    receiver: mpsc::Receiver<AssistantDelta>,
}

impl AssistantEventStream {
    /// Creates a stream with the given channel capacity.
    #[must_use]
    pub fn channel(capacity: usize) -> (Self, AssistantStreamSender) {
        let (sender, receiver) = mpsc::channel(capacity);
        (Self { receiver }, AssistantStreamSender { sender })
    }

    /// Receives the next event.
    pub async fn recv(&mut self) -> Option<AssistantDelta> {
        self.receiver.recv().await
    }

    /// Drains the stream and returns the final assistant message.
    ///
    /// If the stream ends without a terminal event, a synthetic error message
    /// is returned so callers never hang.
    pub async fn result(mut self) -> AssistantMessage {
        let mut last_partial = None;
        while let Some(event) = self.receiver.recv().await {
            match event {
                AssistantDelta::Start { partial } => last_partial = Some(partial),
                AssistantDelta::Done { message, .. } | AssistantDelta::Error { message, .. } => {
                    return message;
                }
                _ => {}
            }
        }

        let mut message = last_partial.unwrap_or_else(|| AssistantMessage::partial("", "", ""));
        message.stop_reason = StopReason::Error;
        message.error_message = Some("provider stream ended without a terminal event".to_owned());
        message
    }
}

/// The sending half of a provider stream.
#[derive(Debug, Clone)]
pub struct AssistantStreamSender {
    sender: mpsc::Sender<AssistantDelta>,
}

impl AssistantStreamSender {
    /// Sends an event, returning false when the consumer is gone.
    pub async fn send(&self, event: AssistantDelta) -> bool {
        self.sender.send(event).await.is_ok()
    }

    /// Tries to send without waiting.
    #[must_use]
    pub fn try_send(&self, event: AssistantDelta) -> bool {
        self.sender.try_send(event).is_ok()
    }

    /// Returns true when the consumer has dropped the stream.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.sender.is_closed()
    }
}

/// High-level agent events consumed by interfaces and persistence.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    /// A run started.
    AgentStart,
    /// A model request round started.
    TurnStart,
    /// A message was appended to the transcript.
    MessageStart {
        /// The message.
        message: Message,
    },
    /// A streaming assistant message changed.
    MessageUpdate {
        /// Cumulative message.
        message: Message,
        /// The provider delta that caused the update.
        delta: AssistantDelta,
    },
    /// A finalized message was appended.
    MessageEnd {
        /// The finalized message.
        message: Message,
    },
    /// A tool started.
    ToolExecutionStart {
        /// Call identifier.
        tool_call_id: String,
        /// Tool name.
        tool_name: String,
        /// Parsed arguments.
        args: serde_json::Value,
    },
    /// A tool emitted progress.
    ToolExecutionUpdate {
        /// Call identifier.
        tool_call_id: String,
        /// Tool name.
        tool_name: String,
        /// Parsed arguments.
        args: serde_json::Value,
        /// Partial output.
        partial: ToolProgress,
    },
    /// A tool finished.
    ToolExecutionEnd {
        /// Call identifier.
        tool_call_id: String,
        /// Tool name.
        tool_name: String,
        /// Final output.
        result: ToolResult,
        /// True when the tool failed.
        is_error: bool,
        /// Duration in milliseconds.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        duration_ms: Option<u64>,
    },
    /// A model request round finished.
    TurnEnd {
        /// Final assistant message.
        message: Message,
        /// Tool results produced in this round, in source order.
        tool_results: Vec<ToolResultMessage>,
    },
    /// The run finished. This is always the last event.
    AgentEnd {
        /// Every message produced by this run.
        messages: Vec<Message>,
        /// Why the run ended.
        status: RunStatus,
    },
}

/// Why an agent run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    /// The model stopped without pending work.
    Completed,
    /// The turn budget was exhausted.
    TurnLimit,
    /// The caller aborted the run.
    Aborted,
    /// A request or hook failed.
    Error,
}

/// The result of draining an agent event stream.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentRunResult {
    /// Messages produced by the run.
    pub messages: Vec<Message>,
    /// Why the run ended.
    pub status: RunStatus,
}

/// The receiving half of an agent event stream.
#[derive(Debug)]
pub struct AgentEventStream {
    receiver: mpsc::Receiver<AgentEvent>,
}

impl AgentEventStream {
    /// Creates a stream with the given channel capacity.
    #[must_use]
    pub fn channel(capacity: usize) -> (Self, AgentEventSender) {
        let (sender, receiver) = mpsc::channel(capacity);
        (Self { receiver }, AgentEventSender { sender })
    }

    /// Receives the next event.
    pub async fn recv(&mut self) -> Option<AgentEvent> {
        self.receiver.recv().await
    }

    /// Drains the stream and returns the terminal result.
    pub async fn result(mut self) -> AgentRunResult {
        while let Some(event) = self.receiver.recv().await {
            if let AgentEvent::AgentEnd { messages, status } = event {
                return AgentRunResult { messages, status };
            }
        }
        AgentRunResult {
            messages: Vec::new(),
            status: RunStatus::Error,
        }
    }
}

/// The sending half of an agent event stream.
#[derive(Debug, Clone)]
pub struct AgentEventSender {
    sender: mpsc::Sender<AgentEvent>,
}

impl AgentEventSender {
    /// Sends an event, returning false when the consumer is gone.
    pub async fn send(&self, event: AgentEvent) -> bool {
        self.sender.send(event).await.is_ok()
    }

    /// Tries to send without waiting.
    #[must_use]
    pub fn try_send(&self, event: AgentEvent) -> bool {
        self.sender.try_send(event).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::Content;

    #[tokio::test]
    async fn assistant_stream_returns_terminal_message() {
        let (stream, sender) = AssistantEventStream::channel(4);
        let mut message = AssistantMessage::partial("api", "provider", "model");
        message.content.push(Content::text("hello"));
        message.stop_reason = StopReason::Stop;

        let message_clone = message.clone();
        tokio::spawn(async move {
            sender
                .send(AssistantDelta::Start {
                    partial: AssistantMessage::partial("api", "provider", "model"),
                })
                .await;
            sender
                .send(AssistantDelta::Done {
                    reason: StopReason::Stop,
                    message: message_clone,
                })
                .await;
        });

        let result = stream.result().await;
        assert_eq!(result.text(), "hello");
        assert_eq!(result.stop_reason, StopReason::Stop);
    }

    #[tokio::test]
    async fn assistant_stream_synthesizes_error_on_eof() {
        let (stream, sender) = AssistantEventStream::channel(1);
        drop(sender);
        let result = stream.result().await;
        assert_eq!(result.stop_reason, StopReason::Error);
        assert!(result.error_message.unwrap().contains("without a terminal"));
    }

    #[tokio::test]
    async fn agent_stream_returns_status_and_messages() {
        let (mut stream, sender) = AgentEventStream::channel(4);
        tokio::spawn(async move {
            sender.send(AgentEvent::AgentStart).await;
            sender
                .send(AgentEvent::AgentEnd {
                    messages: vec![Message::user("hi")],
                    status: RunStatus::Completed,
                })
                .await;
        });

        assert_eq!(stream.recv().await, Some(AgentEvent::AgentStart));
        let result = stream.result().await;
        assert_eq!(result.status, RunStatus::Completed);
        assert_eq!(result.messages.len(), 1);
    }
}
