//! The `ls` tool.

use async_trait::async_trait;
use codie_core::{ExecutionMode, Tool, ToolCall, ToolContext, ToolProgress, ToolResult};
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::{
    search::{display_path, optional_bool, optional_string, resolve_root},
    truncate::{format_size, truncate_head},
};

/// Lists one directory without recursing.
#[derive(Debug, Clone, Copy, Default)]
pub struct LsTool;

impl LsTool {
    /// Creates an ls tool.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for LsTool {
    fn name(&self) -> &str {
        "ls"
    }

    fn description(&self) -> &str {
        "List a directory without recursing. Directories end with /, files show their size, \
         and symlinks show their target. Hidden entries are omitted unless all is true."
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Directory to list (relative or absolute); defaults to the working directory"
                },
                "all": {
                    "type": "boolean",
                    "description": "Include hidden entries; defaults to false"
                }
            },
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
        let raw_path = match optional_string(&call.arguments, "path") {
            Ok(value) => value,
            Err(message) => return ToolResult::error(message),
        };
        let all = match optional_bool(&call.arguments, "all") {
            Ok(value) => value.unwrap_or(false),
            Err(message) => return ToolResult::error(message),
        };
        if context.cancel.is_cancelled() {
            return ToolResult::error("Operation aborted");
        }

        let path = resolve_root(raw_path.as_deref(), &context.cwd);
        if !path.exists() {
            return ToolResult::error(match raw_path.as_deref() {
                Some(raw) => format!("Path does not exist: {raw}"),
                None => format!("Path does not exist: {}", path.display()),
            });
        }
        if !path.is_dir() {
            let display = raw_path
                .clone()
                .unwrap_or_else(|| display_path(&path, &context.cwd));
            return ToolResult::error(format!("Not a directory: {display}"));
        }

        let mut entries = match read_entries(&path, all, &context.cancel).await {
            Ok(entries) => entries,
            Err(message) => return ToolResult::error(message),
        };
        entries.sort_by(|left, right| {
            right
                .is_dir
                .cmp(&left.is_dir)
                .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
                .then_with(|| left.name.cmp(&right.name))
        });

        if entries.is_empty() {
            return ToolResult::text("(empty directory)").with_details(json!({ "entries": 0 }));
        }

        let count = entries.len();
        let lines = entries
            .into_iter()
            .map(|entry| entry.line)
            .collect::<Vec<_>>();
        let truncated = truncate_head(&lines.join("\n"));
        let mut output = truncated.content.clone();
        if truncated.truncated {
            output.push_str(&format!(
                "\n\n[Listing truncated; showing {shown} of {count} entries.]",
                shown = truncated.output_lines
            ));
        }

        ToolResult::text(output).with_details(json!({
            "entries": count,
            "truncated": truncated.truncated,
        }))
    }
}

/// One rendered directory entry.
struct EntryLine {
    name: String,
    is_dir: bool,
    line: String,
}

/// Reads direct children of `path`, skipping hidden entries unless `all` is set.
async fn read_entries(
    path: &std::path::Path,
    all: bool,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<Vec<EntryLine>, String> {
    let mut reader = match tokio::fs::read_dir(path).await {
        Ok(reader) => reader,
        Err(error) => return Err(format!("Could not read directory: {error}")),
    };
    let mut entries = Vec::new();

    loop {
        if cancel.is_cancelled() {
            return Err("Operation aborted".to_owned());
        }
        let entry = match reader.next_entry().await {
            Ok(Some(entry)) => entry,
            Ok(None) => break,
            Err(error) => return Err(format!("Could not read directory: {error}")),
        };
        let name = entry.file_name().to_string_lossy().into_owned();
        if !all && name.starts_with('.') {
            continue;
        }
        let file_type = match entry.file_type().await {
            Ok(file_type) => file_type,
            Err(error) => return Err(format!("Could not inspect {name}: {error}")),
        };
        let line = if file_type.is_dir() {
            format!("{name}/")
        } else if file_type.is_symlink() {
            let target = tokio::fs::read_link(entry.path())
                .await
                .map(|target| target.to_string_lossy().into_owned())
                .unwrap_or_else(|_| "?".to_owned());
            format!("{name} -> {target}")
        } else {
            let size = entry.metadata().await.map_or(0, |metadata| metadata.len());
            format!("{name} ({})", format_size(size as usize))
        };
        entries.push(EntryLine {
            name,
            is_dir: file_type.is_dir(),
            line,
        });
    }

    Ok(entries)
}
