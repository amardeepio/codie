//! Tool contracts shared by the agent loop and tool implementations.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::content::{Content, ToolCall, ToolDecl};

/// How a tool may be scheduled inside a batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    /// May run concurrently with other parallel tools.
    #[default]
    Parallel,
    /// Forces the whole batch to run sequentially.
    Sequential,
}

/// Progress emitted while a tool is running.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolProgress {
    /// Partial model-visible content.
    pub content: Vec<Content>,
    /// Host-only details.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

/// The final result of a tool call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolResult {
    /// Model-visible content.
    pub content: Vec<Content>,
    /// Host-only structured details.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
    /// True when the tool failed.
    #[serde(default)]
    pub is_error: bool,
    /// Requests early termination of the batch when every call agrees.
    #[serde(default)]
    pub terminate: bool,
    /// Execution time in milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
}

impl ToolResult {
    /// Creates a successful text result.
    #[must_use]
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            content: vec![Content::text(text)],
            details: None,
            is_error: false,
            terminate: false,
            duration_ms: None,
        }
    }

    /// Creates an error text result.
    #[must_use]
    pub fn error(text: impl Into<String>) -> Self {
        Self {
            content: vec![Content::text(text)],
            details: None,
            is_error: true,
            terminate: false,
            duration_ms: None,
        }
    }

    /// Attaches structured details.
    #[must_use]
    pub fn with_details(mut self, details: serde_json::Value) -> Self {
        self.details = Some(details);
        self
    }

    /// Attaches a duration.
    #[must_use]
    pub const fn with_duration(mut self, duration_ms: u64) -> Self {
        self.duration_ms = Some(duration_ms);
        self
    }
}

/// Context available to every tool execution.
#[derive(Debug, Clone)]
pub struct ToolContext {
    /// Working directory for relative paths and shell commands.
    pub cwd: PathBuf,
    /// Cancellation signal for the current run.
    pub cancel: CancellationToken,
    /// Session identifier, when the session is persisted.
    pub session_id: Option<String>,
}

/// A built-in or extension tool.
#[async_trait]
pub trait Tool: Send + Sync {
    /// Name used on the wire.
    fn name(&self) -> &str;

    /// Description shown to the model.
    fn description(&self) -> &str;

    /// JSON Schema for the arguments object.
    fn schema(&self) -> serde_json::Value;

    /// Scheduling mode for this tool.
    fn execution_mode(&self) -> ExecutionMode {
        ExecutionMode::Parallel
    }

    /// Prompt snippet used by the system prompt builder.
    fn prompt_snippet(&self) -> &str {
        self.description()
    }

    /// Executes the call. Errors are returned as `is_error` results, not panics.
    async fn execute(
        &self,
        call: &ToolCall,
        context: &ToolContext,
        updates: Option<mpsc::UnboundedSender<ToolProgress>>,
    ) -> ToolResult;
}

/// A collection of tools available to a run.
pub trait Toolbox: Send + Sync {
    /// Returns a tool by name.
    fn get(&self, name: &str) -> Option<Arc<dyn Tool>>;

    /// Returns declarations for every available tool.
    fn declarations(&self) -> Vec<ToolDecl>;

    /// Returns a human-readable denial reason when a tool is not permitted.
    ///
    /// The agent turns a denial into an error tool result so the model can
    /// recover instead of treating the tool as missing.
    fn denial(&self, _name: &str) -> Option<String> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_results_are_flagged() {
        let result = ToolResult::error("nope");
        assert!(result.is_error);
        assert_eq!(result.content[0].as_text(), Some("nope"));
    }

    #[test]
    fn text_results_are_not_errors() {
        let result = ToolResult::text("ok").with_details(serde_json::json!({"a": 1}));
        assert!(!result.is_error);
        assert!(result.details.is_some());
    }
}
