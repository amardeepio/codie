//! The `project_memory` tool.
//!
//! Appends or replaces one section of `CODIE.MD` (the project notebook) or
//! `PLAN.MD` (the project plan). The notebook is the durable, human-readable
//! memory codie loads into every session; this tool is the only write path so
//! edits stay section-scoped, deduplicated, and redacted.

use std::path::Path;
use std::sync::LazyLock;

use async_trait::async_trait;
use codie_core::{ExecutionMode, Tool, ToolCall, ToolContext, ToolProgress, ToolResult};
use codie_prompt::{
    MAX_PROJECT_MEMORY_BYTES, PROJECT_MEMORY_FILE, PROJECT_PLAN_FILE, append_entry, project_root,
    replace_section,
};
use regex::Regex;
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::file_mutation::with_file_mutation;

/// Maximum size of `PLAN.MD` after an edit.
pub const MAX_PROJECT_PLAN_BYTES: usize = 128 * 1024;

/// Maximum length of one memory entry.
pub const MAX_ENTRY_BYTES: usize = 1024;

/// Known `CODIE.MD` sections, offered as schema hints.
const MEMORY_SECTIONS: [&str; 7] = [
    "Overview",
    "Stack and layout",
    "Commands",
    "Conventions",
    "Decisions",
    "Pitfalls",
    "Open threads",
];

/// Known `PLAN.MD` sections.
const PLAN_SECTIONS: [&str; 6] = [
    "Goal",
    "Non-goals",
    "Scope and phases",
    "Tasks",
    "Decisions",
    "Risks and open questions",
];

static SECRET_ASSIGNMENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)((?:api[_-]?key|token|secret|password|passwd)\s*[:=]\s*)([^\s`'"]{6,})"#)
        .expect("valid secret assignment pattern")
});
static BEARER_TOKEN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(bearer\s+)[A-Za-z0-9._~+/-]{10,}").expect("valid bearer token pattern")
});
static PRIVATE_KEY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z ]*PRIVATE KEY-----")
        .expect("valid private key pattern")
});

/// Records durable project memory in `CODIE.MD` or `PLAN.MD`.
#[derive(Debug, Clone, Copy, Default)]
pub struct ProjectMemoryTool;

impl ProjectMemoryTool {
    /// Creates the tool.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for ProjectMemoryTool {
    fn name(&self) -> &str {
        "project_memory"
    }

    fn description(&self) -> &str {
        "Record durable project memory in CODIE.MD, or update PLAN.MD. Use it after establishing a \
         convention, command, decision, or pitfall, and before starting a new project. Entries are \
         deduplicated and credentials are redacted. Do not record chatter or secrets."
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "file": {
                    "type": "string",
                    "enum": ["codie", "plan"],
                    "description": "Which file to update: `codie` for CODIE.MD (default), `plan` for PLAN.MD"
                },
                "section": {
                    "type": "string",
                    "description": format!(
                        "Heading to update. CODIE.MD sections: {}. PLAN.MD sections: {}. A missing section is created.",
                        MEMORY_SECTIONS.join(", "),
                        PLAN_SECTIONS.join(", ")
                    )
                },
                "text": {
                    "type": "string",
                    "description": "Entry to append, or the full replacement body when mode is `replace`. Markdown is allowed."
                },
                "mode": {
                    "type": "string",
                    "enum": ["append", "replace"],
                    "description": "`append` adds one entry and deduplicates it (default); `replace` replaces the whole section body"
                }
            },
            "required": ["section", "text"],
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
        let Some(section) = call.arguments.get("section").and_then(Value::as_str) else {
            return ToolResult::error("Missing required argument: section");
        };
        let Some(text) = call.arguments.get("text").and_then(Value::as_str) else {
            return ToolResult::error("Missing required argument: text");
        };
        let file = call
            .arguments
            .get("file")
            .and_then(Value::as_str)
            .unwrap_or("codie");
        let replace = call
            .arguments
            .get("mode")
            .and_then(Value::as_str)
            .unwrap_or("append")
            == "replace";

        if section.trim().is_empty() || section.contains('\n') || section.len() > 64 {
            return ToolResult::error(
                "Section must be one line of at most 64 characters, for example `Pitfalls`.",
            );
        }
        let text = redact(text.trim());
        if text.is_empty() {
            return ToolResult::error("Text is empty after trimming.");
        }
        if text.len() > MAX_ENTRY_BYTES {
            return ToolResult::error(format!(
                "Entry is {} bytes; the limit is {MAX_ENTRY_BYTES}. Split it into smaller facts.",
                text.len()
            ));
        }
        if context.cancel.is_cancelled() {
            return ToolResult::error("Operation aborted");
        }

        let root = project_root(&context.cwd);
        let (name, limit) = match file {
            "codie" => (PROJECT_MEMORY_FILE, MAX_PROJECT_MEMORY_BYTES),
            "plan" => (PROJECT_PLAN_FILE, MAX_PROJECT_PLAN_BYTES),
            other => {
                return ToolResult::error(format!(
                    "Unknown file `{other}`; use `codie` or `plan`."
                ));
            }
        };
        let path = root.join(name);
        let existing = read_or_header(&path, &root, name);

        let updated = if replace {
            replace_section(&existing, section, &text)
        } else {
            append_entry(&existing, section, &text)
        };
        if updated.len() > limit {
            return ToolResult::error(format!(
                "{name} would be {} bytes; the limit is {limit}. Replace a section with `mode: replace` to condense it.",
                updated.len()
            ));
        }

        let result =
            with_file_mutation(&path, async { write_atomically(&path, &updated).await }).await;
        if let Err(message) = result {
            return ToolResult::error(message);
        }

        let verb = if replace { "Replaced" } else { "Recorded" };
        let body = section_body(&updated, section).unwrap_or_default();
        let note = if updated == existing {
            "\n\nThe entry already existed, so nothing changed."
        } else {
            ""
        };
        ToolResult::text(format!(
            "{verb} memory in {name} under \"{}\".{note}\n\n{}",
            section.trim(),
            body.trim()
        ))
    }
}

/// Returns a notebook header used when the file does not exist yet.
fn header(root: &Path, file: &str) -> String {
    let name = root
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "project".to_owned());
    let title = if file == PROJECT_PLAN_FILE {
        "plan"
    } else {
        "codie memory"
    };
    format!("# {name} — {title}\n")
}

fn read_or_header(path: &Path, root: &Path, file: &str) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|_| header(root, file))
}

/// Removes credentials from an entry before it is stored.
#[must_use]
pub fn redact(text: &str) -> String {
    let text = PRIVATE_KEY.replace_all(text, "[redacted private key]");
    let text = BEARER_TOKEN.replace_all(&text, "${1}[redacted]");
    SECRET_ASSIGNMENT
        .replace_all(&text, "${1}[redacted]")
        .into_owned()
}

async fn write_atomically(path: &Path, contents: &str) -> Result<(), String> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let temporary = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "memory".to_owned()),
        std::process::id()
    ));
    tokio::fs::write(&temporary, contents)
        .await
        .map_err(|error| format!("Could not write {}: {error}", temporary.display()))?;
    tokio::fs::rename(&temporary, path)
        .await
        .map_err(|error| format!("Could not replace {}: {error}", path.display()))
}

/// Returns the body of `## section`, or `None` when the section is absent.
#[must_use]
pub fn section_body(content: &str, section: &str) -> Option<String> {
    let mut lines = content.lines();
    let mut found = false;
    let mut body = Vec::new();
    for line in lines.by_ref() {
        let trimmed = line.trim_start();
        if let Some(title) = trimmed.strip_prefix("## ") {
            if found {
                break;
            }
            if title.trim().eq_ignore_ascii_case(section.trim()) {
                found = true;
            }
            continue;
        }
        if found && (trimmed.starts_with("# ") || trimmed.starts_with("## ")) {
            break;
        }
        if found {
            body.push(line);
        }
    }
    found.then(|| body.join("\n").trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credentials_are_redacted() {
        assert_eq!(
            redact("set API_KEY=sk-live-1234567890"),
            "set API_KEY=[redacted]"
        );
        assert_eq!(
            redact("Authorization: Bearer abcdefghijklmnop"),
            "Authorization: Bearer [redacted]"
        );
        assert!(
            redact("-----BEGIN RSA PRIVATE KEY-----\nabc\n-----END RSA PRIVATE KEY-----")
                .contains("[redacted private key]")
        );
    }

    #[test]
    fn ordinary_text_is_untouched() {
        assert_eq!(redact("Use `cargo test`."), "Use `cargo test`.");
    }

    #[test]
    fn section_bodies_are_extracted() {
        let content = "# Memory\n\n## Commands\n\n- `cargo test`\n\n## Pitfalls\n\n- one\n";
        assert_eq!(
            section_body(content, "commands").as_deref(),
            Some("- `cargo test`")
        );
        assert_eq!(section_body(content, "missing"), None);
    }
}
