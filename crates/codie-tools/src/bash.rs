//! The `bash` tool.

use std::{
    process::{ExitStatus, Stdio},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use async_trait::async_trait;
use codie_core::{ExecutionMode, Tool, ToolCall, ToolContext, ToolProgress, ToolResult};
use serde_json::{Value, json};
use tokio::{io::AsyncReadExt, process::Command, sync::mpsc};

use crate::truncate::{DEFAULT_MAX_BYTES, TruncationResult, format_size, truncate_tail};

/// Maximum timeout accepted by the tool, in seconds.
pub const MAX_TIMEOUT_SECONDS: f64 = 2_147_483.647;

/// Default timeout for commands that do not provide one, in seconds.
pub const DEFAULT_TIMEOUT_SECONDS: f64 = 120.0;

/// How long to wait for inherited output pipes to close after the shell exits.
const PIPE_GRACE: Duration = Duration::from_millis(300);

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

/// A running output reader and the buffer it fills.
type OutputReader = (tokio::task::JoinHandle<()>, Arc<Mutex<Vec<u8>>>);

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

        // A dedicated process group lets a timeout kill the shell and every
        // child it started, instead of leaving a server running.
        #[cfg(unix)]
        process.process_group(0);

        let mut child = match process.spawn() {
            Ok(child) => child,
            Err(error) => {
                return ToolResult::error(format!("Could not execute command: {error}"));
            }
        };

        let stdout = child.stdout.take().map(spawn_reader);
        let stderr = child.stderr.take().map(spawn_reader);

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
                        "Command timed out after {} seconds. If this starts a long-running process, \
                         background it with output redirected (`cmd > /tmp/out.log 2>&1 &`).",
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

        let mut output_bytes = collect_reader(stdout).await;
        output_bytes.extend(collect_reader(stderr).await);
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

/// Kills the child's process group and reaps the shell.
async fn stop_child(child: &mut tokio::process::Child) {
    #[cfg(unix)]
    if let Some(pid) = child.id() {
        // `process_group(0)` makes the pid the group id. Shelling out keeps
        // this crate free of unsafe code; a missing group is harmless.
        let _ = Command::new("kill")
            .arg("-KILL")
            .arg(format!("-{pid}"))
            .kill_on_drop(true)
            .status()
            .await;
    }
    let _ = child.start_kill();
    let _ = child.wait().await;
}

/// Spawns a reader that appends output to a shared buffer until EOF.
fn spawn_reader<R>(mut stream: R) -> OutputReader
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    let buffer = Arc::new(Mutex::new(Vec::new()));
    let shared = Arc::clone(&buffer);
    let handle = tokio::spawn(async move {
        let mut chunk = [0_u8; 8192];
        loop {
            match stream.read(&mut chunk).await {
                Ok(0) | Err(_) => break,
                Ok(count) => shared
                    .lock()
                    .expect("output buffer")
                    .extend_from_slice(&chunk[..count]),
            }
        }
    });
    (handle, buffer)
}

/// Collects buffered output, giving inherited pipes a short grace period.
///
/// A background process may keep the pipe open after the shell exits; waiting
/// for EOF would hang the turn forever, so the reader is aborted after
/// [`PIPE_GRACE`] and whatever arrived is returned.
async fn collect_reader(reader: Option<OutputReader>) -> Vec<u8> {
    let Some((mut handle, buffer)) = reader else {
        return Vec::new();
    };
    if tokio::time::timeout(PIPE_GRACE, &mut handle).await.is_err() {
        handle.abort();
    }
    buffer.lock().expect("output buffer").clone()
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
        return Ok(Some(DEFAULT_TIMEOUT_SECONDS));
    };
    if value.is_null() {
        return Ok(Some(DEFAULT_TIMEOUT_SECONDS));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeout_defaults_to_two_minutes() {
        assert_eq!(
            parse_timeout(&json!({})).expect("default timeout"),
            Some(DEFAULT_TIMEOUT_SECONDS)
        );
        assert_eq!(
            parse_timeout(&json!({"timeout": null})).expect("null timeout"),
            Some(DEFAULT_TIMEOUT_SECONDS)
        );
    }
}
