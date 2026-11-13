//! The `edit` tool.

use async_trait::async_trait;
use codie_core::{ExecutionMode, Tool, ToolCall, ToolContext, ToolProgress, ToolResult};
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::{file_mutation::with_file_mutation, path::resolve_path};

/// Applies one or more exact-text replacements to a UTF-8 text file.
#[derive(Debug, Clone, Copy, Default)]
pub struct EditTool;

impl EditTool {
    /// Creates an edit tool.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

/// One replacement parsed from the tool arguments.
struct ParsedEdit {
    old_text: String,
    new_text: String,
}

/// One matched replacement in normalized content.
struct MatchedEdit {
    edit_index: usize,
    index: usize,
    length: usize,
    new_text: String,
}

#[async_trait]
impl Tool for EditTool {
    fn name(&self) -> &str {
        "edit"
    }

    fn description(&self) -> &str {
        "Edit a single file using exact text replacement. Every edits[].oldText must match a \
         unique, non-overlapping region of the original file. If two changes affect nearby \
         lines, merge them into one edit. All edits are matched against the original content, \
         not incrementally."
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to edit (relative or absolute)"
                },
                "edits": {
                    "type": "array",
                    "minItems": 1,
                    "description": "One or more targeted replacements. Each edit is matched against the original file, not incrementally. Do not include overlapping or nested edits.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "oldText": {
                                "type": "string",
                                "description": "Exact text for one targeted replacement. It must be unique in the original file and must not overlap with any other edits[].oldText in the same call."
                            },
                            "newText": {
                                "type": "string",
                                "description": "Replacement text for this targeted edit."
                            }
                        },
                        "required": ["oldText", "newText"],
                        "additionalProperties": false
                    }
                }
            },
            "required": ["path", "edits"],
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
        let Some(raw_path) = call.arguments.get("path").and_then(Value::as_str) else {
            return ToolResult::error("Missing required argument: path");
        };
        let edits = match parse_edits(call.arguments.get("edits")) {
            Ok(edits) => edits,
            Err(message) => return ToolResult::error(message),
        };
        if context.cancel.is_cancelled() {
            return ToolResult::error("Operation aborted");
        }

        let path = resolve_path(raw_path, &context.cwd);
        let result = with_file_mutation(&path, async {
            edit_file(&path, raw_path, &edits, context.cancel.is_cancelled())
        })
        .await;

        match result {
            Ok((count, first_changed_line, render)) => {
                let mut details = json!({ "first_changed_line": first_changed_line });
                if let Some(render) = render {
                    details["codie_render"] = render;
                }
                ToolResult::text(format!(
                    "Successfully replaced {count} block(s) in {raw_path}."
                ))
                .with_details(details)
            }
            Err(message) => ToolResult::error(message),
        }
    }
}

/// Builds a compact host-only render hint for the transcript diff view.
///
/// The hint is omitted for unusually large diffs so a single edit cannot bloat
/// the session or the renderer's work.
fn render_hint(raw_path: &str, old: &str, new: &str) -> Option<Value> {
    const CONTEXT: usize = 3;
    const MAX_LINES: usize = 600;

    let diff = similar::TextDiff::from_lines(old, new);
    let mut unified = diff.unified_diff();
    unified.context_radius(CONTEXT).header(raw_path, raw_path);
    let text = unified.to_string();
    if text.lines().count() > MAX_LINES {
        return None;
    }
    Some(json!({
        "kind": "diff",
        "title": format!("Edit {raw_path}"),
        "text": text,
    }))
}

/// Reads, edits, and writes the file.
///
/// Returns the edit count, first changed line, and an optional render hint.
fn edit_file(
    path: &std::path::Path,
    raw_path: &str,
    edits: &[ParsedEdit],
    cancelled: bool,
) -> Result<(usize, usize, Option<Value>), String> {
    if cancelled {
        return Err("Operation aborted".to_owned());
    }
    let bytes =
        std::fs::read(path).map_err(|error| format!("Could not edit file: {raw_path}. {error}"))?;
    let raw_content = String::from_utf8(bytes).map_err(|_| {
        format!("Could not edit file: {raw_path}. The file is not valid UTF-8 text.")
    })?;

    let had_bom = raw_content.starts_with('\u{feff}');
    let content = raw_content.strip_prefix('\u{feff}').unwrap_or(&raw_content);
    let normalized_content = normalize_to_lf(content);

    let normalized_edits: Vec<ParsedEdit> = edits
        .iter()
        .map(|edit| ParsedEdit {
            old_text: normalize_to_lf(&edit.old_text),
            new_text: normalize_to_lf(&edit.new_text),
        })
        .collect();

    let matched = match_edits(&normalized_content, &normalized_edits, raw_path)?;
    let mut new_content = normalized_content.clone();
    for replacement in matched.iter().rev() {
        new_content.replace_range(
            replacement.index..replacement.index + replacement.length,
            &replacement.new_text,
        );
    }

    if new_content == normalized_content {
        return Err(format!(
            "No changes made to {raw_path}. The replacement produced identical content."
        ));
    }

    let final_content = if content == normalized_content {
        new_content
    } else {
        apply_replacements_to_original(content, &matched)
    };
    let mut output = String::with_capacity(final_content.len() + usize::from(had_bom) * 3);
    if had_bom {
        output.push('\u{feff}');
    }
    output.push_str(&final_content);
    if cancelled {
        return Err("Operation aborted".to_owned());
    }
    std::fs::write(path, output)
        .map_err(|error| format!("Could not write file: {raw_path}. {error}"))?;

    let first_changed_line = matched.first().map_or(1, |replacement| {
        normalized_content[..replacement.index]
            .matches('\n')
            .count()
            + 1
    });
    let normalized_final = normalize_to_lf(&final_content);
    let render = render_hint(raw_path, &normalized_content, &normalized_final);
    Ok((edits.len(), first_changed_line, render))
}

/// Parses and validates the `edits` argument.
fn parse_edits(value: Option<&Value>) -> Result<Vec<ParsedEdit>, String> {
    let Some(value) = value else {
        return Err(
            "Edit tool input is invalid. edits must contain at least one replacement.".to_owned(),
        );
    };
    let Some(items) = value.as_array() else {
        return Err(
            "Edit tool input is invalid. edits must contain at least one replacement.".to_owned(),
        );
    };
    if items.is_empty() {
        return Err(
            "Edit tool input is invalid. edits must contain at least one replacement.".to_owned(),
        );
    }
    items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let old_text = item
                .get("oldText")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("edits[{index}].oldText must be a string"))?;
            let new_text = item
                .get("newText")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("edits[{index}].newText must be a string"))?;
            Ok(ParsedEdit {
                old_text: old_text.to_owned(),
                new_text: new_text.to_owned(),
            })
        })
        .collect()
}

/// Finds every edit in the normalized original content and validates uniqueness.
fn match_edits(
    content: &str,
    edits: &[ParsedEdit],
    raw_path: &str,
) -> Result<Vec<MatchedEdit>, String> {
    let mut matched = Vec::with_capacity(edits.len());
    for (index, edit) in edits.iter().enumerate() {
        if edit.old_text.is_empty() {
            return if edits.len() == 1 {
                Err(format!("oldText must not be empty in {raw_path}."))
            } else {
                Err(format!(
                    "edits[{index}].oldText must not be empty in {raw_path}."
                ))
            };
        }
        let occurrences: Vec<usize> = content
            .match_indices(edit.old_text.as_str())
            .map(|(position, _)| position)
            .collect();
        let Some(position) = occurrences.first().copied() else {
            return if edits.len() == 1 {
                Err(format!(
                    "Could not find the exact text in {raw_path}. The old text must match exactly including all whitespace and newlines."
                ))
            } else {
                Err(format!(
                    "Could not find edits[{index}] in {raw_path}. The old text must match exactly including all whitespace and newlines."
                ))
            };
        };
        if occurrences.len() > 1 {
            return if edits.len() == 1 {
                Err(format!(
                    "Found {} occurrences of the text in {raw_path}. The text must be unique. Please provide more context to make it unique.",
                    occurrences.len()
                ))
            } else {
                Err(format!(
                    "Found {} occurrences of edits[{index}] in {raw_path}. Each oldText must be unique. Please provide more context to make it unique.",
                    occurrences.len()
                ))
            };
        }
        matched.push(MatchedEdit {
            edit_index: index,
            index: position,
            length: edit.old_text.len(),
            new_text: edit.new_text.clone(),
        });
    }

    matched.sort_by_key(|replacement| replacement.index);
    for pair in matched.windows(2) {
        let previous = &pair[0];
        let current = &pair[1];
        if previous.index + previous.length > current.index {
            return Err(format!(
                "edits[{}] and edits[{}] overlap in {raw_path}. Merge them into one edit or target disjoint regions.",
                previous.edit_index, current.edit_index
            ));
        }
    }
    Ok(matched)
}

/// Normalizes `\r\n` and lone `\r` line endings to `\n`.
fn normalize_to_lf(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// Applies replacements matched in normalized content to the original text.
///
/// Untouched bytes stay as they were. Inserted line breaks use the line ending
/// of the line where the replacement starts.
fn apply_replacements_to_original(original: &str, replacements: &[MatchedEdit]) -> String {
    let offsets = normalized_offset_map(original);
    let mut output = String::with_capacity(original.len());
    let mut cursor = 0;
    let mut ends_with_cr = false;

    let push = |output: &mut String, part: &str, ends_with_cr: &mut bool| {
        if part.is_empty() {
            return;
        }
        if *ends_with_cr && part.starts_with('\n') {
            output.push('\n');
        }
        output.push_str(part);
        *ends_with_cr = part.ends_with('\r');
    };

    for replacement in replacements {
        let Some(&start) = offsets.get(replacement.index) else {
            continue;
        };
        let Some(&end) = offsets.get(replacement.index + replacement.length) else {
            continue;
        };
        push(&mut output, &original[cursor..start], &mut ends_with_cr);
        let ending = line_ending_at(original, start);
        let inserted = if ending == "\n" {
            replacement.new_text.clone()
        } else {
            replacement.new_text.replace('\n', ending)
        };
        push(&mut output, &inserted, &mut ends_with_cr);
        cursor = end;
    }
    push(&mut output, &original[cursor..], &mut ends_with_cr);
    output
}

/// Maps normalized byte offsets to original byte offsets.
///
/// Entries are meaningful at character boundaries, which is where exact text
/// replacements begin and end.
fn normalized_offset_map(original: &str) -> Vec<usize> {
    let bytes = original.as_bytes();
    let mut offsets = vec![0usize; original.len() + 1];
    let mut original_index = 0;
    let mut normalized_index = 0;
    while original_index < bytes.len() {
        offsets[normalized_index] = original_index;
        if bytes[original_index] == b'\r' {
            original_index += if bytes.get(original_index + 1) == Some(&b'\n') {
                2
            } else {
                1
            };
        } else {
            original_index += 1;
        }
        normalized_index += 1;
    }
    offsets[normalized_index] = original.len();
    offsets
}

/// Returns the line ending used at `offset`.
fn line_ending_at(content: &str, offset: usize) -> &'static str {
    let bytes = content.as_bytes();
    let line_feed = content[offset..]
        .find('\n')
        .map(|index| offset + index)
        .or_else(|| content[..offset].rfind('\n'));
    match line_feed {
        Some(index) if index > 0 && bytes[index - 1] == b'\r' => "\r\n",
        Some(_) => "\n",
        None if content.contains('\r') => "\r",
        None => "\n",
    }
}
