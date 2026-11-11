//! Shared output truncation helpers.
//!
//! Tool output is limited by two independent caps: a line count and a byte
//! count. Whichever is reached first wins. Head truncation keeps the beginning
//! of the output and never returns a partial line. Tail truncation keeps the
//! end of the output and only breaks a line when a single line is larger than
//! the byte cap.

/// Default maximum number of lines in tool output.
pub const DEFAULT_MAX_LINES: usize = 2000;

/// Default maximum number of bytes in tool output (50 KiB).
pub const DEFAULT_MAX_BYTES: usize = 50 * 1024;

/// Default maximum number of characters in a single presented line.
pub const DEFAULT_MAX_LINE_CHARS: usize = 500;

/// Which limit caused truncation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TruncatedBy {
    /// The line cap was reached.
    Lines,
    /// The byte cap was reached.
    Bytes,
}

impl TruncatedBy {
    /// Returns the stable lowercase spelling used in structured details.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Lines => "lines",
            Self::Bytes => "bytes",
        }
    }
}

/// The result of truncating tool output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TruncationResult {
    /// The retained content.
    pub content: String,
    /// Whether anything was removed.
    pub truncated: bool,
    /// Which cap removed content, or `None` when the output fits.
    pub truncated_by: Option<TruncatedBy>,
    /// Total number of lines in the original content.
    pub total_lines: usize,
    /// Total number of bytes in the original content.
    pub total_bytes: usize,
    /// Number of complete lines in [`Self::content`].
    pub output_lines: usize,
    /// Number of bytes in [`Self::content`].
    pub output_bytes: usize,
    /// Whether a single oversized line was partially retained by tail truncation.
    pub last_line_partial: bool,
    /// Whether the first line alone exceeded the byte cap during head truncation.
    pub first_line_exceeds_limit: bool,
    /// Line cap that was applied.
    pub max_lines: usize,
    /// Byte cap that was applied.
    pub max_bytes: usize,
}

/// Splits text into lines for counting.
///
/// A single trailing newline is treated as a terminator, not as an extra empty
/// line. Empty content has zero lines.
fn split_lines_for_counting(content: &str) -> Vec<&str> {
    if content.is_empty() {
        return Vec::new();
    }
    let mut lines: Vec<&str> = content.split('\n').collect();
    if content.ends_with('\n') {
        lines.pop();
    }
    lines
}

/// Formats a byte count as `B`, `KB`, or `MB` with one decimal for larger units.
#[must_use]
pub fn format_size(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{bytes}B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1}KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1}MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

/// Truncates content from the head with the default limits.
#[must_use]
pub fn truncate_head(content: &str) -> TruncationResult {
    truncate_head_with_limits(content, DEFAULT_MAX_LINES, DEFAULT_MAX_BYTES)
}

/// Truncates content from the head, keeping the first lines that fit both caps.
///
/// A line is only included when its complete text and separating newline fit in
/// the byte budget. If the first line alone exceeds the byte cap, the result is
/// empty and [`TruncationResult::first_line_exceeds_limit`] is true.
#[must_use]
pub fn truncate_head_with_limits(
    content: &str,
    max_lines: usize,
    max_bytes: usize,
) -> TruncationResult {
    let total_bytes = content.len();
    let lines = split_lines_for_counting(content);
    let total_lines = lines.len();

    if total_lines <= max_lines && total_bytes <= max_bytes {
        return TruncationResult {
            content: content.to_owned(),
            truncated: false,
            truncated_by: None,
            total_lines,
            total_bytes,
            output_lines: total_lines,
            output_bytes: total_bytes,
            last_line_partial: false,
            first_line_exceeds_limit: false,
            max_lines,
            max_bytes,
        };
    }

    if lines.first().is_some_and(|line| line.len() > max_bytes) {
        return TruncationResult {
            content: String::new(),
            truncated: true,
            truncated_by: Some(TruncatedBy::Bytes),
            total_lines,
            total_bytes,
            output_lines: 0,
            output_bytes: 0,
            last_line_partial: false,
            first_line_exceeds_limit: true,
            max_lines,
            max_bytes,
        };
    }

    let mut output: Vec<&str> = Vec::new();
    let mut output_bytes = 0usize;
    let mut truncated_by = TruncatedBy::Lines;

    for (index, line) in lines.iter().enumerate() {
        if index >= max_lines {
            truncated_by = TruncatedBy::Lines;
            break;
        }
        let line_bytes = line.len() + usize::from(index > 0);
        if output_bytes + line_bytes > max_bytes {
            truncated_by = TruncatedBy::Bytes;
            break;
        }
        output.push(line);
        output_bytes += line_bytes;
    }

    if output.len() >= max_lines && output_bytes <= max_bytes {
        truncated_by = TruncatedBy::Lines;
    }

    let output_content = output.join("\n");
    let final_bytes = output_content.len();

    TruncationResult {
        content: output_content,
        truncated: true,
        truncated_by: Some(truncated_by),
        total_lines,
        total_bytes,
        output_lines: output.len(),
        output_bytes: final_bytes,
        last_line_partial: false,
        first_line_exceeds_limit: false,
        max_lines,
        max_bytes,
    }
}

/// Truncates content from the tail with the default limits.
#[must_use]
pub fn truncate_tail(content: &str) -> TruncationResult {
    truncate_tail_with_limits(content, DEFAULT_MAX_LINES, DEFAULT_MAX_BYTES)
}

/// Truncates content from the tail, keeping the last lines that fit both caps.
///
/// When the final line alone is larger than the byte cap, its tail is retained
/// on a UTF-8 boundary and [`TruncationResult::last_line_partial`] is true.
#[must_use]
pub fn truncate_tail_with_limits(
    content: &str,
    max_lines: usize,
    max_bytes: usize,
) -> TruncationResult {
    let total_bytes = content.len();
    let lines = split_lines_for_counting(content);
    let total_lines = lines.len();

    if total_lines <= max_lines && total_bytes <= max_bytes {
        return TruncationResult {
            content: content.to_owned(),
            truncated: false,
            truncated_by: None,
            total_lines,
            total_bytes,
            output_lines: total_lines,
            output_bytes: total_bytes,
            last_line_partial: false,
            first_line_exceeds_limit: false,
            max_lines,
            max_bytes,
        };
    }

    let mut output: Vec<&str> = Vec::new();
    let mut output_bytes = 0usize;
    let mut truncated_by = TruncatedBy::Lines;
    let mut last_line_partial = false;

    for line in lines.iter().rev() {
        if output.len() >= max_lines {
            truncated_by = TruncatedBy::Lines;
            break;
        }
        let line_bytes = line.len() + usize::from(!output.is_empty());
        if output_bytes + line_bytes > max_bytes {
            truncated_by = TruncatedBy::Bytes;
            if output.is_empty() {
                let partial = truncate_to_bytes_from_end(line, max_bytes);
                output.push(partial);
                output_bytes = partial.len();
                last_line_partial = true;
            }
            break;
        }
        output.push(line);
        output_bytes += line_bytes;
    }

    if !last_line_partial && output.len() >= max_lines && output_bytes <= max_bytes {
        truncated_by = TruncatedBy::Lines;
    }

    output.reverse();
    let output_content = output.join("\n");
    let final_bytes = output_content.len();

    TruncationResult {
        content: output_content,
        truncated: true,
        truncated_by: Some(truncated_by),
        total_lines,
        total_bytes,
        output_lines: output.len(),
        output_bytes: final_bytes,
        last_line_partial,
        first_line_exceeds_limit: false,
        max_lines,
        max_bytes,
    }
}

/// Returns the longest suffix of `text` that fits in `max_bytes` without
/// splitting a UTF-8 sequence.
fn truncate_to_bytes_from_end(text: &str, max_bytes: usize) -> &str {
    if text.len() <= max_bytes {
        return text;
    }
    let mut start = text.len() - max_bytes;
    while start < text.len() && !text.is_char_boundary(start) {
        start += 1;
    }
    &text[start..]
}

/// Truncates one line to the default character cap.
///
/// The result ends with `... [truncated]` when content was removed.
#[must_use]
pub fn truncate_line(line: &str) -> String {
    truncate_line_with_limit(line, DEFAULT_MAX_LINE_CHARS)
}

/// Truncates one line to `max_chars` Unicode scalar values.
///
/// The result ends with `... [truncated]` when content was removed. The suffix
/// is not counted against `max_chars`.
#[must_use]
pub fn truncate_line_with_limit(line: &str, max_chars: usize) -> String {
    if line.chars().count() <= max_chars {
        return line.to_owned();
    }
    let head: String = line.chars().take(max_chars).collect();
    format!("{head}... [truncated]")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_size_uses_binary_units() {
        assert_eq!(format_size(512), "512B");
        assert_eq!(format_size(1024), "1.0KB");
        assert_eq!(format_size(DEFAULT_MAX_BYTES), "50.0KB");
        assert_eq!(format_size(1024 * 1024), "1.0MB");
    }

    #[test]
    fn head_keeps_complete_lines() {
        let content = (1..=2100)
            .map(|line| format!("line {line}"))
            .collect::<Vec<_>>()
            .join("\n");
        let result = truncate_head(&content);
        assert!(result.truncated);
        assert_eq!(result.truncated_by, Some(TruncatedBy::Lines));
        assert_eq!(result.total_lines, 2100);
        assert_eq!(result.output_lines, DEFAULT_MAX_LINES);
        assert!(result.content.ends_with("line 2000"));
    }

    #[test]
    fn head_respects_byte_cap_without_partial_line() {
        let line = "x".repeat(100);
        let content = std::iter::repeat_n(line.as_str(), 600)
            .collect::<Vec<_>>()
            .join("\n");
        let result = truncate_head(&content);
        assert!(result.truncated);
        assert_eq!(result.truncated_by, Some(TruncatedBy::Bytes));
        assert!(result.output_bytes <= DEFAULT_MAX_BYTES);
        assert_eq!(result.content.lines().count(), result.output_lines);
        assert_eq!(result.output_bytes, result.output_lines * 101 - 1);
    }

    #[test]
    fn head_reports_first_line_too_large() {
        let content = "x".repeat(DEFAULT_MAX_BYTES + 1);
        let result = truncate_head(&content);
        assert!(result.first_line_exceeds_limit);
        assert_eq!(result.output_lines, 0);
        assert!(result.content.is_empty());
    }

    #[test]
    fn tail_keeps_the_end() {
        let content = (1..=2100)
            .map(|line| format!("line {line}"))
            .collect::<Vec<_>>()
            .join("\n");
        let result = truncate_tail(&content);
        assert!(result.truncated);
        assert_eq!(result.truncated_by, Some(TruncatedBy::Lines));
        assert_eq!(result.output_lines, DEFAULT_MAX_LINES);
        assert!(result.content.starts_with("line 101"));
        assert!(result.content.ends_with("line 2100"));
    }

    #[test]
    fn tail_partial_line_stays_on_a_utf8_boundary() {
        let content = "é".repeat(100);
        let result = truncate_tail_with_limits(&content, 10, 15);
        assert!(result.truncated);
        assert!(result.last_line_partial);
        assert_eq!(result.truncated_by, Some(TruncatedBy::Bytes));
        assert!(result.output_bytes <= 15);
        assert!(result.content.chars().all(|character| character == 'é'));
    }

    #[test]
    fn lines_are_truncated_with_a_suffix() {
        let line = "a".repeat(600);
        let result = truncate_line(&line);
        assert!(result.ends_with("... [truncated]"));
        assert_eq!(result.chars().count(), DEFAULT_MAX_LINE_CHARS + 15);
        assert_eq!(truncate_line("short"), "short");
    }
}
