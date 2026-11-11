//! The `bash` tool.

use std::{
    process::{ExitStatus, Stdio},
    time::{Duration, Instant},
};

use async_trait::async_trait;
use codie_core::{ExecutionMode, Tool, ToolCall, ToolContext, ToolProgress, ToolResult};
use serde_json::{Value, json};
use tokio::{io::AsyncReadExt, process::Command, sync::mpsc};

use crate::truncate::{DEFAULT_MAX_BYTES, TruncationResult, format_size, truncate_tail};

/// Maximum timeout accepted by the tool, in seconds.
pub const MAX_TIMEOUT_SECONDS: f64 = 2_147_483.647;

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

/// How waiting for the child process ended.
enum WaitOutcome {
    /// The child exited (or waiting failed).
    Exited(std::io::Result<ExitStatus>),
    /// The timeout expired.
    TimedOut,
    /// The run was cancelled.
    Aborted,
}

#[async_trait]
impl Tool for BashTool {
    fn name(&self) -> &str {
        "bash"
    }

    fn description(&self) -> &str {
        "Execute a bash command in the current working directory. Returns combined stdout and \
         stderr, truncated to the last 2000 lines or 50KB. There is no default timeout; provide \
         timeout in seconds when a command may run too long. A non-zero exit code is an error."
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
                    "description": "Timeout in seconds (optional, no default timeout)"
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
        let Some(command) = call.arguments.get("command").and_then(Value::as_str) else {
            return ToolResult::error("Missing required argument: command");
        };
        let timeout = match parse_timeout(&call.arguments) {
            Ok(timeout) => timeout,
            Err(message) => return ToolResult::error(message),
        };
        if context.cancel.is_cancelled() {
            return ToolResult::error("Command aborted");
        }

        let started = Instant::now();
        let mut process = Command::new("/bin/bash");
        process
            .arg("-c")
            .arg(command)
            .current_dir(&context.cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let mut child = match process.spawn() {
            Ok(child) => child,
            Err(error) => {
                return ToolResult::error(format!("Could not execute command: {error}"));
            }
        };

        let stdout_task = child.stdout.take().map(|mut stream| {
            tokio::spawn(async move {
                let mut buffer = Vec::new();
                let _ = stream.read_to_end(&mut buffer).await;
                buffer
            })
        });
        let stderr_task = child.stderr.take().map(|mut stream| {
            tokio::spawn(async move {
                let mut buffer = Vec::new();
                let _ = stream.read_to_end(&mut buffer).await;
                buffer
            })
        });

        let outcome = wait_for_child(&mut child, timeout, context).await;
        let (exit_code, status, is_error, include_empty_placeholder) = match outcome {
            WaitOutcome::Exited(Ok(status)) => {
                let code = exit_code_of(status);
                if code == 0 {
                    (code, None, false, true)
                } else {
                    (
                        code,
                        Some(format!("Command exited with code {code}")),
                        true,
                        true,
                    )
                }
            }
            WaitOutcome::Exited(Err(error)) => {
                stop_child(&mut child).await;
                (
                    -1,
                    Some(format!("Could not wait for command: {error}")),
                    true,
                    false,
                )
            }
            WaitOutcome::TimedOut => {
                stop_child(&mut child).await;
                let seconds = timeout.unwrap_or_default();
                (
                    -1,
                    Some(format!(
                        "Command timed out after {} seconds",
                        format_seconds(seconds)
                    )),
                    true,
                    false,
                )
            }
            WaitOutcome::Aborted => {
                stop_child(&mut child).await;
                (-1, Some("Command aborted".to_owned()), true, false)
            }
        };

        let mut output_bytes = join_task(stdout_task).await;
        output_bytes.extend(join_task(stderr_task).await);
        let output = String::from_utf8_lossy(&output_bytes).into_owned();
        let truncation = truncate_tail(&output);
        let mut text = if truncation.content.is_empty() {
            if include_empty_placeholder {
                "(no output)".to_owned()
            } else {
                String::new()
            }
        } else {
            truncation.content.clone()
        };

        if truncation.truncated {
            if !text.is_empty() {
                text.push_str("\n\n");
            }
            text.push_str(&truncation_notice(&truncation));
        }
        if let Some(status) = status {
            if !text.is_empty() {
                text.push_str("\n\n");
            }
            text.push_str(&status);
        }

        let wall_time_seconds = (started.elapsed().as_secs_f64() * 10.0).round() / 10.0;
        ToolResult {
            content: vec![codie_core::Content::text(text)],
            details: Some(json!({
                "exit_code": exit_code,
                "wall_time_seconds": wall_time_seconds,
                "truncated": truncation.truncated,
            })),
            is_error,
            terminate: false,
            duration_ms: Some(started.elapsed().as_millis() as u64),
        }
    }
}

/// Waits for the child, a timeout, or cancellation.
async fn wait_for_child(
    child: &mut tokio::process::Child,
    timeout: Option<f64>,
    context: &ToolContext,
) -> WaitOutcome {
    if let Some(seconds) = timeout {
        let deadline = tokio::time::sleep(Duration::from_secs_f64(seconds));
        tokio::select! {
            status = child.wait() => WaitOutcome::Exited(status),
            () = deadline => WaitOutcome::TimedOut,
            () = context.cancel.cancelled() => WaitOutcome::Aborted,
        }
    } else {
        tokio::select! {
            status = child.wait() => WaitOutcome::Exited(status),
            () = context.cancel.cancelled() => WaitOutcome::Aborted,
        }
    }
}

/// Kills and reaps a child process.
async fn stop_child(child: &mut tokio::process::Child) {
    let _ = child.start_kill();
    let _ = child.wait().await;
}

/// Awaits an output reader task, using an empty buffer if the task failed.
async fn join_task(task: Option<tokio::task::JoinHandle<Vec<u8>>>) -> Vec<u8> {
    match task {
        Some(task) => task.await.unwrap_or_default(),
        None => Vec::new(),
    }
}

/// Builds the tail-truncation notice.
fn truncation_notice(truncation: &TruncationResult) -> String {
    let end = truncation.total_lines;
    let start = truncation
        .total_lines
        .saturating_sub(truncation.output_lines)
        + 1;
    if truncation.last_line_partial {
        format!(
            "[Showing last {} of line {end}. Output truncated.]",
            format_size(truncation.output_bytes)
        )
    } else if truncation.truncated_by == Some(crate::truncate::TruncatedBy::Bytes) {
        format!(
            "[Showing last {start}-{end} of {} lines ({} limit). Output truncated.]",
            truncation.total_lines,
            format_size(DEFAULT_MAX_BYTES)
        )
    } else {
        format!(
            "[Showing last {start}-{end} of {} lines. Output truncated.]",
            truncation.total_lines
        )
    }
}

/// Parses and validates the optional timeout argument.
fn parse_timeout(arguments: &Value) -> Result<Option<f64>, String> {
    let Some(value) = arguments.get("timeout") else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let Some(seconds) = value.as_f64() else {
        return Err("Invalid timeout: must be a positive finite number of seconds".to_owned());
    };
    if !seconds.is_finite() || seconds <= 0.0 {
        return Err("Invalid timeout: must be a positive finite number of seconds".to_owned());
    }
    if seconds > MAX_TIMEOUT_SECONDS {
        return Err(format!(
            "Invalid timeout: maximum is {} seconds",
            format_seconds(MAX_TIMEOUT_SECONDS)
        ));
    }
    Ok(Some(seconds))
}

/// Formats seconds without unnecessary trailing zeros.
fn format_seconds(seconds: f64) -> String {
    if seconds.fract() == 0.0 {
        format!("{seconds:.0}")
    } else {
        format!("{seconds}")
    }
}

/// Returns a shell-style exit code, mapping signal termination to 128 + signal.
fn exit_code_of(status: ExitStatus) -> i32 {
    if let Some(code) = status.code() {
        return code;
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        128 + status.signal().unwrap_or(0)
    }
    #[cfg(not(unix))]
    {
        -1
    }
}
