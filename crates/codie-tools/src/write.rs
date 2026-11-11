//! The `write` tool.

use async_trait::async_trait;
use codie_core::{ExecutionMode, Tool, ToolCall, ToolContext, ToolProgress, ToolResult};
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::{file_mutation::with_file_mutation, path::resolve_path};

/// Creates or overwrites a UTF-8 text file, creating parent directories.
#[derive(Debug, Clone, Copy, Default)]
pub struct WriteTool;

impl WriteTool {
    /// Creates a write tool.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for WriteTool {
    fn name(&self) -> &str {
        "write"
    }

    fn description(&self) -> &str {
        "Write content to a file. Creates the file if it does not exist, overwrites it if it \
         does, and creates parent directories automatically. Use write for new files or full \
         rewrites, and edit for targeted changes."
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to write (relative or absolute)"
                },
                "content": {
                    "type": "string",
                    "description": "Content to write to the file"
                }
            },
            "required": ["path", "content"],
            "additionalProperties": false
        })
    }

    fn execution_mode(&self) -> ExecutionMode {
        ExecutionMode::Parallel
    }

    async fn execute(
        &self,
        call: &ToolCall,
        context: &ToolContext,
        _updates: Option<mpsc::UnboundedSender<ToolProgress>>,
    ) -> ToolResult {
        let Some(raw_path) = call.arguments.get("path").and_then(Value::as_str) else {
            return ToolResult::error("Missing required argument: path");
        };
        let Some(content) = call.arguments.get("content").and_then(Value::as_str) else {
            return ToolResult::error("Missing required argument: content");
        };
        if context.cancel.is_cancelled() {
            return ToolResult::error("Operation aborted");
        }

        let path = resolve_path(raw_path, &context.cwd);
        let result = with_file_mutation(&path, async {
            if context.cancel.is_cancelled() {
                return Err("Operation aborted".to_owned());
            }
            if let Some(parent) = path.parent()
                && !parent.as_os_str().is_empty()
                && let Err(error) = tokio::fs::create_dir_all(parent).await
            {
                return Err(format!(
                    "Could not create parent directory for: {raw_path}. {error}"
                ));
            }
            if context.cancel.is_cancelled() {
                return Err("Operation aborted".to_owned());
            }
            if let Err(error) = tokio::fs::write(&path, content).await {
                return Err(format!("Could not write file: {raw_path}. {error}"));
            }
            Ok(())
        })
        .await;

        match result {
            Ok(()) => ToolResult::text(format!("Successfully wrote to {raw_path}")),
            Err(message) => ToolResult::error(message),
        }
    }
}
