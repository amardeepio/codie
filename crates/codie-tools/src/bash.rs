//! The `bash` tool (POSIX).

use async_trait::async_trait;
use codie_core::{ExecutionMode, Tool, ToolCall, ToolContext, ToolProgress, ToolResult};
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::shell::{self, ShellKind};

pub use crate::shell::MAX_TIMEOUT_SECONDS;

/// Executes shell commands with `/bin/bash` and keeps the tail of the output.
#[derive(Debug, Clone, Copy, Default)]
pub struct BashTool;

impl BashTool {
    /// Creates a bash tool.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for BashTool {
    fn name(&self) -> &str {
        "bash"
    }

    fn description(&self) -> &str {
        "Execute a bash command in the current working directory. Returns combined stdout and \
         stderr, truncated to the last 2000 lines or 50KB. Commands time out after 120 seconds \
         by default; raise timeout for long builds. A command that starts a server must background \
         it and redirect its output (`cmd > /tmp/out.log 2>&1 &`), otherwise the tool waits for it \
         and then times out. A non-zero exit code is an error."
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "Shell command to execute"
                },
                "timeout": {
                    "type": "number",
                    "minimum": 0,
                    "description": "Timeout in seconds (optional, defaults to 120)"
                }
            },
            "required": ["command"],
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
        shell::execute(ShellKind::Bash, call, context).await
    }
}
