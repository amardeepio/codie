//! Core types shared by every codie crate.
//!
//! `codie-core` contains no I/O, HTTP, terminal, or filesystem code. It defines
//! the vocabulary — messages, content, events, tools, and the agent loop — that
//! providers, tools, sessions, and interfaces agree on.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod agent;
pub mod content;
pub mod error;
pub mod event;
pub mod message;
pub mod tool;

pub use agent::{Agent, AgentConfig, ChatClient, ModelRef, QueueMode, StreamOptions};
pub use content::{Content, Cost, StopReason, ThinkingLevel, ToolCall, ToolDecl, Usage};
pub use error::{Error, ErrorKind, Result};
pub use event::{
    AgentEvent, AgentEventSender, AgentEventStream, AgentRunResult, AssistantDelta,
    AssistantEventStream, AssistantStreamSender, RunStatus,
};
pub use message::{
    AssistantMessage, Message, SystemMessage, ToolResultMessage, UserMessage,
    collapse_system_messages, current_tools, normalize_context, now_ms,
};
pub use tool::{ExecutionMode, Tool, ToolContext, ToolProgress, ToolResult, Toolbox};

/// The codie version compiled into this build.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_present() {
        assert_eq!(VERSION, env!("CARGO_PKG_VERSION"));
    }
}
