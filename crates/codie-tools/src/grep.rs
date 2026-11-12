//! The `grep` tool.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use codie_core::{ExecutionMode, Tool, ToolCall, ToolContext, ToolProgress, ToolResult};
use regex::{Regex, RegexBuilder};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::{
    search::{
        GREP_DEFAULT_MAX_RESULTS, GREP_MAX_RESULTS, MAX_SEARCH_FILE_BYTES, cap_notice,
        display_path, optional_bool, optional_positive_integer, optional_string, relative_to,
        resolve_root, walker,
    },
    truncate::truncate_line,
};

/// Searches file contents with a regular expression.
#[derive(Debug, Clone, Copy, Default)]
pub struct GrepTool;

impl GrepTool {
    /// Creates a grep tool.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for GrepTool {
    fn name(&self) -> &str {
        "grep"
    }

    fn description(&self) -> &str {
        "Search file contents with a regular expression. Returns matches as path:line:text. \
         Respects .gitignore and skips hidden files and binaries by default. Use glob to \
         restrict the searched files, and narrow path instead of raising max_results when \
         output is too large."
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Regular expression to search for"
                },
                "path": {
                    "type": "string",
                    "description": "File or directory to search (relative or absolute); defaults to the working directory"
                },
                "glob": {
                    "type": "string",
                    "description": "Only search files whose path matches this glob, for example *.rs or src/**/*.toml"
                },
                "case_sensitive": {
                    "type": "boolean",
                    "description": "Match case exactly; defaults to true"
                },
                "max_results": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Maximum number of matches to return; defaults to 100 and is capped at 1000"
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
        let case_sensitive = match optional_bool(&call.arguments, "case_sensitive") {
            Ok(value) => value.unwrap_or(true),
            Err(message) => return ToolResult::error(message),
        };
        let max_results =
            match optional_positive_integer(&call.arguments, "max_results", GREP_MAX_RESULTS) {
                Ok(value) => value.unwrap_or(GREP_DEFAULT_MAX_RESULTS),
                Err(message) => return ToolResult::error(message),
            };
        let glob = match optional_string(&call.arguments, "glob") {
            Ok(value) => value,
            Err(message) => return ToolResult::error(message),
        };
        let raw_root = match optional_string(&call.arguments, "path") {
            Ok(value) => value,
            Err(message) => return ToolResult::error(message),
        };

        let regex = match RegexBuilder::new(pattern)
            .case_insensitive(!case_sensitive)
            .build()
        {
            Ok(regex) => regex,
            Err(error) => return ToolResult::error(format!("Invalid regular expression: {error}")),
        };
        let matcher = match glob.as_deref().map(globset::Glob::new) {
            None => None,
            Some(Ok(glob)) => Some(glob.compile_matcher()),
            Some(Err(error)) => return ToolResult::error(format!("Invalid glob pattern: {error}")),
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

        let request = GrepRequest {
            root,
            regex,
            matcher,
            max_results,
        };
        let cwd = context.cwd.clone();
        let cancel = context.cancel.clone();
        let outcome = tokio::task::spawn_blocking(move || request.run(&cwd, &cancel)).await;

        match outcome {
            Ok(Ok(result)) => format_result(result),
            Ok(Err(message)) => ToolResult::error(message),
            Err(error) => ToolResult::error(format!("Search failed: {error}")),
        }
    }
}

/// A validated grep request executed on the blocking pool.
struct GrepRequest {
    root: PathBuf,
    regex: Regex,
    matcher: Option<globset::GlobMatcher>,
    max_results: usize,
}

/// Collected matches and counters.
struct GrepOutcome {
    lines: Vec<String>,
    files: usize,
    capped: bool,
    cancelled: bool,
}

impl GrepRequest {
    fn run(self, cwd: &Path, cancel: &CancellationToken) -> Result<GrepOutcome, String> {
        let mut outcome = GrepOutcome {
            lines: Vec::new(),
            files: 0,
            capped: false,
            cancelled: false,
        };
        let mut counted = HashSet::new();

        if self.root.is_file() {
            self.search_file(&self.root, cwd, &mut outcome, &mut counted, cancel);
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
            if !self.matches_glob(entry.path(), cwd) {
                continue;
            }
            self.search_file(entry.path(), cwd, &mut outcome, &mut counted, cancel);
            if outcome.capped {
                break;
            }
        }
        Ok(outcome)
    }

    fn matches_glob(&self, path: &Path, cwd: &Path) -> bool {
        let Some(matcher) = &self.matcher else {
            return true;
        };
        let display = display_path(path, cwd);
        let relative = relative_to(path, &self.root).unwrap_or(display);
        matcher.is_match(relative)
    }

    fn search_file(
        &self,
        path: &Path,
        cwd: &Path,
        outcome: &mut GrepOutcome,
        counted: &mut HashSet<String>,
        cancel: &CancellationToken,
    ) {
        if cancel.is_cancelled() {
            outcome.cancelled = true;
            return;
        }
        if std::fs::metadata(path).is_ok_and(|metadata| metadata.len() > MAX_SEARCH_FILE_BYTES) {
            return;
        }
        let Ok(bytes) = std::fs::read(path) else {
            return;
        };
        let Ok(text) = String::from_utf8(bytes) else {
            return;
        };

        let display = display_path(path, cwd);
        for (index, line) in text.lines().enumerate() {
            if self.regex.is_match(line) {
                if !counted.contains(&display) {
                    counted.insert(display.clone());
                    outcome.files += 1;
                }
                outcome.lines.push(format!(
                    "{display}:{}:{}",
                    index + 1,
                    truncate_line(line.trim_end_matches('\r'))
                ));
                if outcome.lines.len() >= self.max_results {
                    outcome.capped = true;
                    return;
                }
            }
        }
    }
}

/// Converts a grep outcome into a tool result.
fn format_result(outcome: GrepOutcome) -> ToolResult {
    if outcome.cancelled {
        return ToolResult::error("Operation aborted");
    }
    if outcome.lines.is_empty() {
        return ToolResult::text("No matches found.").with_details(json!({
            "matches": 0,
            "files": 0,
            "capped": false,
        }));
    }

    let matches = outcome.lines.len();
    let truncated = crate::truncate::truncate_head(&outcome.lines.join("\n"));
    let mut output = truncated.content.clone();
    if outcome.capped {
        output.push_str(&cap_notice(matches, "match"));
    } else if truncated.truncated {
        output.push_str(&format!(
            "\n\n[Output truncated; showing {shown} of {matches} matched lines.]",
            shown = truncated.output_lines
        ));
    }

    ToolResult::text(output).with_details(json!({
        "matches": matches,
        "files": outcome.files,
        "capped": outcome.capped,
        "truncated": truncated.truncated,
    }))
}
