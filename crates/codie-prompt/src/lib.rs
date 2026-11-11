//! Deterministic system prompt assembly for codie.
//!
//! The prompt is an ordered set of sections (PRMPT-001): the preamble is
//! emitted verbatim, every other section is XML-wrapped, and sections are
//! joined by a blank line. Given the same [`PromptInput`], assembly is
//! byte-identical (PRMPT-008).
//!
//! Project context discovery walks from the working directory to the
//! filesystem root, loading at most one instruction file per directory in the
//! order `AGENTS.override.md`, `AGENTS.md`, `CLAUDE.md` (PRMPT-003). The
//! topmost ancestor is emitted first and the working directory last, so the
//! nearest project instructions appear closest to the conversation.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

use std::fs;
use std::path::{Path, PathBuf};

use codie_core::{Message, ToolDecl};

/// The preamble used when [`PromptInput::preamble`] is `None`.
pub const DEFAULT_PREAMBLE: &str = "You are a coding assistant running inside codie, a coding agent harness. \
Help users inspect files, run commands, and modify code.";

/// Instruction file names checked in each directory, highest precedence first.
const CONTEXT_FILE_NAMES: [&str; 3] = ["AGENTS.override.md", "AGENTS.md", "CLAUDE.md"];

/// Baseline behavioral rules included in every system prompt.
const RULES: &str = "\
<rules>
- Use the available tools instead of guessing about files or commands.
- Read a file before editing it, and verify changes after writing them.
- Prefer small, focused edits over broad rewrites.
- Be concise in your responses.
- Show file paths clearly when working with files.
</rules>";

/// One project instruction file loaded into the prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextFile {
    /// Path recorded in the `<project_instructions>` tag.
    pub path: PathBuf,
    /// File contents with a leading UTF-8 BOM removed.
    pub content: String,
}

/// Everything required to assemble a system prompt.
#[derive(Debug, Clone, PartialEq)]
pub struct PromptInput {
    /// Working directory reported to the model.
    pub cwd: PathBuf,
    /// Tool declarations advertised in the `<tools>` section, in order.
    pub tools: Vec<ToolDecl>,
    /// Replacement preamble. `None` uses [`DEFAULT_PREAMBLE`].
    pub preamble: Option<String>,
    /// Extra instructions appended to the preamble.
    pub append: Option<String>,
    /// Project instruction files, topmost ancestor first.
    pub project_context: Vec<ContextFile>,
}

impl PromptInput {
    /// Creates an input with the default preamble and no context files.
    #[must_use]
    pub fn new(cwd: impl Into<PathBuf>) -> Self {
        Self {
            cwd: cwd.into(),
            tools: Vec::new(),
            preamble: None,
            append: None,
            project_context: Vec::new(),
        }
    }
}

/// Discovers project instruction files between `cwd` and the filesystem root.
///
/// At most one file is returned per directory, chosen in the order
/// `AGENTS.override.md`, `AGENTS.md`, `CLAUDE.md`. Only existing regular files
/// are returned. The topmost ancestor comes first and `cwd` comes last.
#[must_use]
pub fn discover_project_context(cwd: &Path) -> Vec<PathBuf> {
    // Canonicalizing makes ancestor traversal independent of how the caller
    // spelled the working directory (relative, trailing slash, symlinked).
    let cwd = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());

    let mut directories: Vec<&Path> = cwd.ancestors().collect();
    directories.reverse();

    let mut discovered = Vec::new();
    for directory in directories {
        for name in CONTEXT_FILE_NAMES {
            let candidate = directory.join(name);
            if candidate.is_file() {
                discovered.push(candidate);
                break;
            }
        }
    }
    discovered
}

/// Discovers and loads project instruction files for `cwd`.
///
/// Contents are read as UTF-8 and a leading byte-order mark is stripped. Files
/// that cannot be read as UTF-8 are skipped rather than failing prompt
/// assembly.
#[must_use]
pub fn load_project_context(cwd: &Path) -> Vec<ContextFile> {
    discover_project_context(cwd)
        .into_iter()
        .filter_map(|path| {
            let content = fs::read_to_string(&path).ok()?;
            Some(ContextFile {
                path,
                content: strip_bom(&content).to_string(),
            })
        })
        .collect()
}

/// Builds the system prompt for `input`.
///
/// Sections are emitted in this order: preamble, tools, rules, project context
/// (omitted when no files were supplied), and cwd. Sections are separated by a
/// blank line and the result has no trailing newline.
#[must_use]
pub fn build_system_prompt(input: &PromptInput) -> String {
    let mut sections: Vec<String> = Vec::new();
    sections.push(preamble_section(input));
    sections.push(tools_section(&input.tools));
    sections.push(RULES.to_string());
    if !input.project_context.is_empty() {
        sections.push(project_context_section(&input.project_context));
    }
    sections.push(format!("<cwd>{}</cwd>", input.cwd.display()));
    sections.join("\n\n")
}

/// Builds a system [`Message`] carrying the assembled prompt and `tools`.
///
/// `tools` is passed explicitly so a caller can persist the exact declaration
/// set that the prompt describes; it should normally equal `input.tools`.
#[must_use]
pub fn build_system_message(input: &PromptInput, tools: Vec<ToolDecl>) -> Message {
    Message::system(build_system_prompt(input), tools)
}

/// Returns the preamble with any appended instructions, separated by a blank
/// line.
fn preamble_section(input: &PromptInput) -> String {
    let mut preamble = match input.preamble.as_deref() {
        Some(preamble) => preamble.trim_end().to_string(),
        None => DEFAULT_PREAMBLE.to_string(),
    };

    if let Some(append) = input
        .append
        .as_deref()
        .map(str::trim)
        .filter(|append| !append.is_empty())
    {
        if !preamble.is_empty() {
            preamble.push_str("\n\n");
        }
        preamble.push_str(append);
    }

    preamble
}

/// Renders the `<tools>` section as one `- name: snippet` line per tool.
fn tools_section(tools: &[ToolDecl]) -> String {
    let mut section = String::from("<tools>");
    for tool in tools {
        let snippet = snippet(&tool.description);
        section.push('\n');
        if snippet.is_empty() {
            section.push_str(&format!("- {}", tool.name));
        } else {
            section.push_str(&format!("- {}: {snippet}", tool.name));
        }
    }
    section.push_str("\n</tools>");
    section
}

/// Renders one `<project_context>` section with one block per file.
fn project_context_section(files: &[ContextFile]) -> String {
    let mut section = String::from("<project_context>");
    for file in files {
        let path = file.path.display();
        let content = file.content.trim_end();
        section.push('\n');
        section.push_str(&format!("<project_instructions path=\"{path}\">"));
        if !content.is_empty() {
            section.push('\n');
            section.push_str(content);
            section.push('\n');
        }
        section.push_str("</project_instructions>");
    }
    section.push_str("\n</project_context>");
    section
}

/// Collapses a tool description into a single-line prompt snippet.
fn snippet(description: &str) -> String {
    description
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Removes a leading UTF-8 byte-order mark.
fn strip_bom(content: &str) -> &str {
    content.strip_prefix('\u{feff}').unwrap_or(content)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stripping_a_bom_is_idempotent() {
        assert_eq!(strip_bom("\u{feff}hello"), "hello");
        assert_eq!(strip_bom("hello"), "hello");
    }

    #[test]
    fn snippets_collapse_lines() {
        assert_eq!(
            snippet("Read a file.\n  More detail.  \n"),
            "Read a file. More detail."
        );
        assert_eq!(snippet("   "), "");
    }
}
