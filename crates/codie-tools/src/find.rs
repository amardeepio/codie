//! The `find` tool.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use codie_core::{ExecutionMode, Tool, ToolCall, ToolContext, ToolProgress, ToolResult};
use globset::{GlobBuilder, GlobMatcher};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::{
    search::{
        FIND_DEFAULT_MAX_RESULTS, FIND_MAX_RESULTS, cap_notice, display_path,
        optional_positive_integer, optional_string, relative_to, resolve_root, walker,
    },
    truncate::truncate_head,
};

/// Finds files by glob pattern.
#[derive(Debug, Clone, Copy, Default)]
pub struct FindTool;

impl FindTool {
    /// Creates a find tool.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for FindTool {
    fn name(&self) -> &str {
        "find"
    }

    fn description(&self) -> &str {
        "Find files by glob pattern. Patterns without a slash match file names at any depth \
         (*.rs); patterns with a slash match paths relative to path (src/**/*.toml). Respects \
         .gitignore and skips hidden entries and directories."
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Glob pattern to match, for example *.rs, **/*.toml, or src/**/mod.rs"
                },
                "path": {
                    "type": "string",
                    "description": "Directory to search (relative or absolute); defaults to the working directory"
                },
                "max_results": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Maximum number of paths to return; defaults to 1000 and is capped at 5000"
                }
            },
            "required": ["pattern"],
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
        let Some(pattern) = call.arguments.get("pattern").and_then(Value::as_str) else {
            return ToolResult::error("Missing required argument: pattern");
        };
        let max_results =
            match optional_positive_integer(&call.arguments, "max_results", FIND_MAX_RESULTS) {
                Ok(value) => value.unwrap_or(FIND_DEFAULT_MAX_RESULTS),
                Err(message) => return ToolResult::error(message),
            };
        let raw_root = match optional_string(&call.arguments, "path") {
            Ok(value) => value,
            Err(message) => return ToolResult::error(message),
        };

        // A pattern without a slash matches the file name at any depth; a pattern
        // with a slash matches the path relative to the search root.
        let path_relative = pattern.contains('/');
        let matcher = match GlobBuilder::new(pattern).literal_separator(true).build() {
            Ok(glob) => glob.compile_matcher(),
            Err(error) => return ToolResult::error(format!("Invalid glob pattern: {error}")),
        };

        let root = resolve_root(raw_root.as_deref(), &context.cwd);
        if !root.exists() {
            return ToolResult::error(match raw_root.as_deref() {
                Some(raw) => format!("Path does not exist: {raw}"),
                None => format!("Path does not exist: {}", root.display()),
            });
        }
        if context.cancel.is_cancelled() {
            return ToolResult::error("Operation aborted");
        }

        let request = FindRequest {
            root,
            matcher,
            path_relative,
            max_results,
        };
        let cwd = context.cwd.clone();
        let cancel = context.cancel.clone();
        let outcome = tokio::task::spawn_blocking(move || request.run(&cwd, &cancel)).await;

        match outcome {
            Ok(Ok(result)) => format_result(result, max_results),
            Ok(Err(message)) => ToolResult::error(message),
            Err(error) => ToolResult::error(format!("Search failed: {error}")),
        }
    }
}

/// A validated find request executed on the blocking pool.
struct FindRequest {
    root: PathBuf,
    matcher: GlobMatcher,
    path_relative: bool,
    max_results: usize,
}

/// Collected paths and counters.
struct FindOutcome {
    paths: Vec<String>,
    capped: bool,
    cancelled: bool,
}

impl FindRequest {
    fn run(self, cwd: &Path, cancel: &CancellationToken) -> Result<FindOutcome, String> {
        let mut outcome = FindOutcome {
            paths: Vec::new(),
            capped: false,
            cancelled: false,
        };

        if self.root.is_file() {
            let display = display_path(&self.root, cwd);
            if self.matches(&self.root, &display) {
                outcome.paths.push(display);
            }
            return Ok(outcome);
        }

        for entry in walker(&self.root) {
            if cancel.is_cancelled() {
                outcome.cancelled = true;
                break;
            }
            let Ok(entry) = entry else { continue };
            if !entry.file_type().is_some_and(|kind| kind.is_file()) {
                continue;
            }
            let display = display_path(entry.path(), cwd);
            if !self.matches(entry.path(), &display) {
                continue;
            }
            outcome.paths.push(display);
            if outcome.paths.len() >= self.max_results {
                outcome.capped = true;
                break;
            }
        }

        outcome.paths.sort();
        Ok(outcome)
    }

    fn matches(&self, path: &Path, display: &str) -> bool {
        if self.path_relative {
            let relative = relative_to(path, &self.root).unwrap_or_else(|| display.to_owned());
            return self.matcher.is_match(relative);
        }
        path.file_name()
            .is_some_and(|name| self.matcher.is_match(name))
    }
}

/// Converts a find outcome into a tool result.
fn format_result(outcome: FindOutcome, max_results: usize) -> ToolResult {
    if outcome.cancelled {
        return ToolResult::error("Operation aborted");
    }
    if outcome.paths.is_empty() {
        return ToolResult::text("No files found.").with_details(json!({
            "count": 0,
            "capped": false,
        }));
    }

    let count = outcome.paths.len();
    let truncated = truncate_head(&outcome.paths.join("\n"));
    let mut output = truncated.content.clone();
    if outcome.capped {
        output.push_str(&cap_notice(max_results, "file"));
    } else if truncated.truncated {
        output.push_str(&format!(
            "\n\n[Output truncated; showing {shown} of {count} paths.]",
            shown = truncated.output_lines
        ));
    }

    ToolResult::text(output).with_details(json!({
        "count": count,
        "capped": outcome.capped,
        "truncated": truncated.truncated,
    }))
}
