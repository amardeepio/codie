//! The `powershell` tool (Windows).

use async_trait::async_trait;
use codie_core::{ExecutionMode, Tool, ToolCall, ToolContext, ToolProgress, ToolResult};
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::shell::{self, ShellKind};

/// Executes PowerShell commands and keeps the tail of the output.
///
/// The tool prefers PowerShell 7 (`pwsh.exe`) and falls back to Windows
/// PowerShell 5.1 (`powershell.exe`). It is registered on Windows in place of
/// the POSIX `bash` tool.
#[derive(Debug, Clone, Copy, Default)]
pub struct PowerShellTool;

impl PowerShellTool {
    /// Creates a PowerShell tool.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for PowerShellTool {
    fn name(&self) -> &str {
        "powershell"
    }

    fn description(&self) -> &str {
        "Execute a PowerShell command in the current working directory. Returns combined stdout \
         and stderr, truncated to the last 2000 lines or 50KB. Commands time out after 120 seconds \
         by default; raise timeout for long builds. A command that starts a server must background \
         it with Start-Process and redirect its output, otherwise the tool waits for it and then \
         times out. A non-zero exit code is an error."
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "PowerShell command to execute"
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
        shell::execute(ShellKind::PowerShell, call, context).await
    }
}
