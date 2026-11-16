//! Project memory: `CODIE.MD`, the agent-maintained project notebook.
//!
//! The notebook lives at the project root and is injected into the system
//! prompt as a `<project_memory>` block. It is project data, not instructions:
//! the rules section tells the model to ignore directives found inside it.
//!
//! Writes are plain Markdown edits so the notebook stays reviewable in a diff.
//! [`append_entry`] and [`replace_section`] are pure string functions shared by
//! the `project_memory` tool and `/init`.

use std::fs;
use std::path::{Path, PathBuf};

use crate::ContextFile;

/// The notebook codie reads and updates.
pub const PROJECT_MEMORY_FILE: &str = "CODIE.MD";

/// The plan file `/init` scaffolds next to the notebook.
pub const PROJECT_PLAN_FILE: &str = "PLAN.MD";

/// Maximum number of bytes read from the notebook into the prompt.
pub const MAX_PROJECT_MEMORY_BYTES: usize = 32 * 1024;

/// A project notebook loaded for a working directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectMemory {
    /// Project root the notebook belongs to.
    pub root: PathBuf,
    /// Absolute path of the notebook.
    pub path: PathBuf,
    /// File contents, truncated to [`MAX_PROJECT_MEMORY_BYTES`].
    pub content: String,
    /// True when the file was longer than the prompt cap.
    pub truncated: bool,
}

impl ProjectMemory {
    /// Renders the `<project_memory>` block for the system prompt.
    #[must_use]
    pub fn section(&self) -> String {
        let path = self.path.display();
        let mut section = format!("<project_memory path=\"{path}\">");
        let content = self.content.trim_end();
        if !content.is_empty() {
            section.push('\n');
            section.push_str(content);
        }
        if self.truncated {
            section.push('\n');
            if !content.is_empty() {
                section.push('\n');
            }
            section.push_str(&format!(
                "…(truncated at {} KiB; keep {PROJECT_MEMORY_FILE} shorter)",
                MAX_PROJECT_MEMORY_BYTES / 1024
            ));
        }
        section.push_str("\n</project_memory>");
        section
    }

    /// Returns the notebook as a [`ContextFile`].
    #[must_use]
    pub fn as_context_file(&self) -> ContextFile {
        ContextFile {
            path: self.path.clone(),
            content: self.content.clone(),
        }
    }
}

/// Returns the project root for `cwd`.
///
/// The root is the nearest ancestor containing a `.git` entry, so a git
/// worktree and its subdirectories share one notebook. Outside a repository the
/// working directory itself is the root. Ancestor walking stops there: unlike
/// `AGENTS.md`, project memory is never inherited from a parent directory.
#[must_use]
pub fn project_root(cwd: &Path) -> PathBuf {
    let cwd = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
    cwd.ancestors()
        .find(|directory| directory.join(".git").exists())
        .map_or_else(|| cwd.clone(), Path::to_path_buf)
}

/// Loads `CODIE.MD` from the project root of `cwd`.
///
/// Returns `None` when the notebook is missing, unreadable, or not UTF-8.
#[must_use]
pub fn load_project_memory(cwd: &Path) -> Option<ProjectMemory> {
    let root = project_root(cwd);
    let path = root.join(PROJECT_MEMORY_FILE);
    let content = fs::read_to_string(&path).ok()?;
    let content = strip_bom(&content);
    let (content, truncated) = truncate_utf8(content, MAX_PROJECT_MEMORY_BYTES);
    Some(ProjectMemory {
        root,
        path,
        content: content.to_owned(),
        truncated,
    })
}

/// Inserts, replaces, or removes the `<project_memory>` block in a prompt.
///
/// Used by `/reload` to refresh a prompt that was already assembled and
/// persisted, without rebuilding the other sections.
#[must_use]
pub fn replace_project_memory(system: &str, memory: Option<&ProjectMemory>) -> String {
    let existing = memory_block_range(system);
    let block = memory.map(ProjectMemory::section);

    match (existing, block) {
        (Some((start, end)), Some(block)) => {
            format!("{}{block}{}", &system[..start], &system[end..])
        }
        (Some((start, end)), None) => {
            let mut start = start;
            if start >= 2 && &system[start - 2..start] == "\n\n" {
                start -= 2;
            }
            format!("{}{}", &system[..start], &system[end..])
        }
        (None, Some(block)) => match system.rfind("\n\n<cwd>") {
            Some(position) => {
                let position = position + 2;
                format!("{}{block}\n\n{}", &system[..position], &system[position..])
            }
            None => format!("{system}\n\n{block}"),
        },
        (None, None) => system.to_owned(),
    }
}

/// Appends one entry under a `##` section, creating the section when absent.
///
/// Entries are deduplicated by normalized text, so repeating a fact is a no-op.
/// The result always ends with exactly one newline.
#[must_use]
pub fn append_entry(content: &str, section: &str, entry: &str) -> String {
    let entry = clean_entry(entry);
    if entry.is_empty() {
        return with_trailing_newline(content);
    }
    let wanted = normalize(&entry);
    if content
        .lines()
        .any(|line| normalize(clean_entry(line).as_str()) == wanted)
    {
        return with_trailing_newline(content);
    }

    let mut lines = trimmed_lines(content);
    if let Some(heading) = find_section(&lines, section) {
        let end = section_end(&lines, heading);
        let mut insert = end;
        while insert > heading + 1 && lines[insert - 1].trim().is_empty() {
            insert -= 1;
        }
        lines.insert(insert, format!("- {entry}"));
    } else {
        if !lines.is_empty() {
            lines.push(String::new());
        }
        lines.push(format!("## {}", section.trim()));
        lines.push(String::new());
        lines.push(format!("- {entry}"));
    }
    join_lines(&lines)
}

/// Replaces the body of a `##` section, creating the section when absent.
#[must_use]
pub fn replace_section(content: &str, section: &str, body: &str) -> String {
    let mut lines = trimmed_lines(content);
    let body_lines: Vec<String> = body
        .trim()
        .lines()
        .map(str::trim_end)
        .map(str::to_owned)
        .collect();

    if let Some(heading) = find_section(&lines, section) {
        let end = section_end(&lines, heading);
        let mut replacement = vec![lines[heading].clone(), String::new()];
        replacement.extend(body_lines);
        replacement.push(String::new());
        lines.splice(heading..end, replacement);
        return join_lines(&lines);
    }

    if !lines.is_empty() {
        lines.push(String::new());
    }
    lines.push(format!("## {}", section.trim()));
    lines.push(String::new());
    lines.extend(body_lines);
    join_lines(&lines)
}

/// Returns true when both texts say the same thing after normalization.
#[must_use]
pub fn same_entry(left: &str, right: &str) -> bool {
    normalize(&clean_entry(left)) == normalize(&clean_entry(right))
}

fn memory_block_range(system: &str) -> Option<(usize, usize)> {
    let start = system.find("<project_memory path=")?;
    let end = system[start..].find("</project_memory>")?;
    Some((start, start + end + "</project_memory>".len()))
}

fn truncate_utf8(text: &str, max: usize) -> (&str, bool) {
    if text.len() <= max {
        return (text, false);
    }
    let mut end = max;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    (&text[..end], true)
}

fn clean_entry(entry: &str) -> String {
    entry
        .trim()
        .trim_start_matches(['-', '*'])
        .trim_start()
        .to_owned()
}

fn normalize(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
        .trim_end_matches(['.', ';', '!'])
        .to_owned()
}

fn find_section(lines: &[String], section: &str) -> Option<usize> {
    let section = section.trim();
    lines.iter().position(|line| {
        line.trim_start()
            .strip_prefix("## ")
            .is_some_and(|title| title.trim().eq_ignore_ascii_case(section))
    })
}

fn section_end(lines: &[String], heading: usize) -> usize {
    lines
        .iter()
        .enumerate()
        .skip(heading + 1)
        .find(|(_, line)| {
            let trimmed = line.trim_start();
            trimmed.starts_with("# ") || trimmed.starts_with("## ")
        })
        .map_or(lines.len(), |(index, _)| index)
}

fn trimmed_lines(content: &str) -> Vec<String> {
    let mut lines: Vec<String> = content.lines().map(str::to_owned).collect();
    while lines.last().is_some_and(|line| line.trim().is_empty()) {
        lines.pop();
    }
    while lines.first().is_some_and(|line| line.trim().is_empty()) {
        lines.remove(0);
    }
    lines
}

fn join_lines(lines: &[String]) -> String {
    let mut text = lines.join("\n");
    text.push('\n');
    text
}

fn with_trailing_newline(content: &str) -> String {
    let mut text = content.trim_end().to_owned();
    text.push('\n');
    text
}

fn strip_bom(content: &str) -> &str {
    content.strip_prefix('\u{feff}').unwrap_or(content)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_append_under_existing_sections() {
        let content = "# Memory\n\n## Pitfalls\n\n- old\n\n## Open threads\n";
        let updated = append_entry(content, "Pitfalls", "new hazard");
        assert_eq!(
            updated,
            "# Memory\n\n## Pitfalls\n\n- old\n- new hazard\n\n## Open threads\n"
        );
    }

    #[test]
    fn missing_sections_are_created() {
        let updated = append_entry("# Memory\n", "Decisions", "use Postgres");
        assert!(updated.ends_with("## Decisions\n\n- use Postgres\n"));
    }

    #[test]
    fn duplicate_entries_are_ignored() {
        let content = "## Pitfalls\n\n- Do not edit generated files.\n";
        let updated = append_entry(content, "Pitfalls", "do not  EDIT generated files");
        assert_eq!(updated, content);
    }

    #[test]
    fn replacing_a_section_keeps_the_heading() {
        let content = "# Memory\n\n## Commands\n\n- old\n\n## Pitfalls\n\n- keep\n";
        let updated = replace_section(content, "Commands", "- new");
        assert_eq!(
            updated,
            "# Memory\n\n## Commands\n\n- new\n\n## Pitfalls\n\n- keep\n"
        );
    }

    #[test]
    fn memory_blocks_replace_in_place() {
        let memory = ProjectMemory {
            root: PathBuf::from("/work/app"),
            path: PathBuf::from("/work/app/CODIE.MD"),
            content: "fresh\n".to_owned(),
            truncated: false,
        };
        let prompt = "preamble\n\n<project_memory path=\"/work/app/CODIE.MD\">\nold\n</project_memory>\n\n<cwd>/work/app</cwd>";
        let updated = replace_project_memory(prompt, Some(&memory));
        assert_eq!(
            updated,
            "preamble\n\n<project_memory path=\"/work/app/CODIE.MD\">\nfresh\n</project_memory>\n\n<cwd>/work/app</cwd>"
        );
        let removed = replace_project_memory(&updated, None);
        assert_eq!(removed, "preamble\n\n<cwd>/work/app</cwd>");
    }

    #[test]
    fn missing_blocks_are_inserted_before_cwd() {
        let memory = ProjectMemory {
            root: PathBuf::from("/work/app"),
            path: PathBuf::from("/work/app/CODIE.MD"),
            content: "facts\n".to_owned(),
            truncated: false,
        };
        let prompt = "preamble\n\n<cwd>/work/app</cwd>";
        let updated = replace_project_memory(prompt, Some(&memory));
        assert_eq!(
            updated,
            "preamble\n\n<project_memory path=\"/work/app/CODIE.MD\">\nfacts\n</project_memory>\n\n<cwd>/work/app</cwd>"
        );
    }

    #[test]
    fn truncation_stays_on_a_character_boundary() {
        let text = "éééé";
        let (truncated, was_truncated) = truncate_utf8(text, 5);
        assert!(was_truncated);
        assert_eq!(truncated, "éé");
    }
}
