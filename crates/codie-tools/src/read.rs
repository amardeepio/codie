//! The `read` tool.

use async_trait::async_trait;
use codie_core::{Content, ExecutionMode, Tool, ToolCall, ToolContext, ToolProgress, ToolResult};
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::{
    path::resolve_path,
    truncate::{DEFAULT_MAX_BYTES, TruncationResult, format_size, truncate_head},
};

/// Reads UTF-8 text files with offset/limit support and head truncation.
#[derive(Debug, Clone, Copy, Default)]
pub struct ReadTool;

impl ReadTool {
    /// Creates a read tool.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for ReadTool {
    fn name(&self) -> &str {
        "read"
    }

    fn description(&self) -> &str {
        "Read the contents of a UTF-8 text file. Output is truncated to 2000 lines or \
         50KB, whichever is hit first. Use offset and limit for large files, and continue \
         with the offset from the notice until the file is complete."
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to read (relative or absolute)"
                },
                "offset": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Line number to start reading from (1-indexed)"
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Maximum number of lines to read"
                }
            },
            "required": ["path"],
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
        if context.cancel.is_cancelled() {
            return ToolResult::error("Operation aborted");
        }

        let Some(raw_path) = call.arguments.get("path").and_then(Value::as_str) else {
            return ToolResult::error("Missing required argument: path");
        };
        let offset = match optional_positive_integer(&call.arguments, "offset") {
            Ok(value) => value.unwrap_or(1),
            Err(message) => return ToolResult::error(message),
        };
        let limit = match optional_positive_integer(&call.arguments, "limit") {
            Ok(value) => value,
            Err(message) => return ToolResult::error(message),
        };

        let path = resolve_path(raw_path, &context.cwd);
        let bytes = match tokio::fs::read(&path).await {
            Ok(bytes) => bytes,
            Err(error) => {
                return ToolResult::error(format!("Could not read file: {raw_path}. {error}"));
            }
        };
        if context.cancel.is_cancelled() {
            return ToolResult::error("Operation aborted");
        }
        let text = match String::from_utf8(bytes) {
            Ok(text) => text,
            Err(_) => {
                return ToolResult::error(format!(
                    "Could not read file: {raw_path}. The file is not valid UTF-8 text."
                ));
            }
        };

        let lines: Vec<&str> = text.split('\n').collect();
        let total_file_lines = lines.len();
        let start = offset - 1;
        if start >= total_file_lines {
            return ToolResult::error(format!(
                "Offset {offset} is beyond end of file ({total_file_lines} lines total)"
            ));
        }

        let end = limit.map_or(total_file_lines, |limit| {
            start.saturating_add(limit).min(total_file_lines)
        });
        let selected = lines[start..end].join("\n");
        let truncation = truncate_head(&selected);
        let start_display = start + 1;
        let truncated = truncation.truncated || truncation.first_line_exceeds_limit;

        let output = if truncation.first_line_exceeds_limit {
            let first_line_bytes = lines[start].len();
            let first_line_size = format_size(first_line_bytes);
            let limit_size = format_size(DEFAULT_MAX_BYTES);
            format!(
                "[Line {start_display} is {first_line_size}, exceeds {limit_size} limit. \
                 Use bash: sed -n '{start_display}p' {raw_path} | head -c {DEFAULT_MAX_BYTES}]"
            )
        } else if truncation.truncated {
            let end_display = start_display + truncation.output_lines - 1;
            let next_offset = end_display + 1;
            let mut output = truncation.content.clone();
            if truncation.truncated_by == Some(crate::truncate::TruncatedBy::Lines) {
                output.push_str(&format!(
                    "\n\n[Showing lines {start_display}-{end_display} of {total_file_lines}. \
                     Use offset={next_offset} to continue.]"
                ));
            } else {
                output.push_str(&format!(
                    "\n\n[Showing lines {start_display}-{end_display} of {total_file_lines} \
                     ({limit_size} limit). Use offset={next_offset} to continue.]",
                    limit_size = format_size(DEFAULT_MAX_BYTES),
                ));
            }
            output
        } else if limit.is_some_and(|limit| start.saturating_add(limit) < total_file_lines) {
            let selected_lines = end - start;
            let remaining = total_file_lines - (start + selected_lines);
            let next_offset = start + selected_lines + 1;
            format!(
                "{}\n\n[{remaining} more lines in file. Use offset={next_offset} to continue.]",
                truncation.content
            )
        } else {
            truncation.content.clone()
        };

        ToolResult {
            content: vec![Content::text(output)],
            details: truncated.then(|| json!({ "truncation": truncation_json(&truncation) })),
            is_error: false,
            terminate: false,
            duration_ms: None,
        }
    }
}

/// Reads an optional positive integer argument.
fn optional_positive_integer(arguments: &Value, name: &str) -> Result<Option<usize>, String> {
    match arguments.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_u64()
            .and_then(|number| usize::try_from(number).ok())
            .filter(|number| *number > 0)
            .map(Some)
            .ok_or_else(|| format!("{name} must be a positive integer")),
    }
}

/// Converts a truncation result into JSON for host-only details.
fn truncation_json(truncation: &TruncationResult) -> Value {
    json!({
        "truncated": truncation.truncated,
        "truncated_by": truncation.truncated_by.map(|reason| reason.as_str()),
        "total_lines": truncation.total_lines,
        "total_bytes": truncation.total_bytes,
        "output_lines": truncation.output_lines,
        "output_bytes": truncation.output_bytes,
        "first_line_exceeds_limit": truncation.first_line_exceeds_limit,
        "last_line_partial": truncation.last_line_partial,
    })
}
