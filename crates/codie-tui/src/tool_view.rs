//! Rich transcript bodies for file-editing tools.
//!
//! `edit` results carry a compact unified diff and `write` results carry the
//! final file content. This module turns those inputs into line-numbered,
//! syntax-highlighted terminal rows. Diffs render side by side when the
//! transcript is wide enough and fall back to a single-column unified view on
//! narrow terminals.

use std::collections::VecDeque;

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::markdown;
use crate::theme::Palette;

/// Rows shown before a rich body is truncated.
pub(crate) const PREVIEW_ROWS: usize = 80;

/// Narrowest transcript that still gets a side-by-side diff.
const SIDE_BY_SIDE_MIN: usize = 72;

/// One aligned row in a parsed diff.
#[derive(Debug, Clone, PartialEq, Eq)]
struct DiffRow {
    old_no: Option<usize>,
    new_no: Option<usize>,
    old: String,
    new: String,
    kind: DiffRowKind,
}

/// How a parsed diff row changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DiffRowKind {
    Equal,
    Delete,
    Insert,
    Modify,
}

/// One contiguous hunk from a unified diff.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Hunk {
    rows: Vec<DiffRow>,
}

/// Renders one parsed `edit` diff.
pub(crate) fn render_diff(
    text: &str,
    path: &str,
    width: usize,
    palette: &Palette,
    limit: usize,
) -> (Vec<Line<'static>>, bool) {
    let hunks = parse_unified(text);
    if hunks_empty(&hunks) {
        return (Vec::new(), false);
    }
    if width < SIDE_BY_SIDE_MIN {
        render_unified(&hunks, path, width, palette, limit)
    } else {
        render_side_by_side(&hunks, path, width, palette, limit)
    }
}

/// Renders written file content with line numbers.
pub(crate) fn render_code(
    content: &str,
    path: &str,
    width: usize,
    palette: &Palette,
    limit: usize,
) -> (Vec<Line<'static>>, bool) {
    let language = language_for(path);
    let number_width = digits(content.lines().count().max(1)).max(2);
    let gutter = number_width + 1;
    let content_width = width.saturating_sub(gutter).max(1);
    let number_style = Style::default().fg(palette.muted);

    let mut lines = Vec::new();
    let mut truncated = false;
    'source: for (index, source) in content.lines().enumerate() {
        let highlighted = markdown::highlight_code(source, language, palette);
        for (row_index, mut spans) in wrap_spans(&highlighted.spans, content_width)
            .into_iter()
            .enumerate()
        {
            if lines.len() >= limit {
                truncated = true;
                break 'source;
            }
            let number = if row_index == 0 {
                format!("{:>number_width$} ", index + 1)
            } else {
                " ".repeat(gutter)
            };
            let used = spans_width(&spans);
            let mut line = vec![Span::styled(number, number_style)];
            line.append(&mut spans);
            if used < content_width {
                line.push(Span::raw(" ".repeat(content_width - used)));
            }
            lines.push(Line::from(line));
        }
    }
    (lines, truncated)
}

/// Returns the lightweight highlighter language for a path.
fn language_for(path: &str) -> &'static str {
    let extension = path
        .rsplit_once('.')
        .map_or("", |(_, extension)| extension)
        .to_ascii_lowercase();
    match extension.as_str() {
        "rs" => "rust",
        "py" => "python",
        "js" | "mjs" | "cjs" | "jsx" => "javascript",
        "ts" | "mts" | "cts" | "tsx" => "typescript",
        "sh" | "bash" | "zsh" => "bash",
        "ps1" | "powershell" => "powershell",
        _ => "",
    }
}

fn hunks_empty(hunks: &[Hunk]) -> bool {
    hunks.iter().all(|hunk| hunk.rows.is_empty())
}

fn parse_unified(text: &str) -> Vec<Hunk> {
    let mut hunks = Vec::new();
    let mut rows = Vec::new();
    let mut pending: VecDeque<(usize, String)> = VecDeque::new();
    let mut old_no = 1_usize;
    let mut new_no = 1_usize;

    for line in text.lines() {
        if line.starts_with("--- ") || line.starts_with("+++ ") || line.starts_with('\\') {
            continue;
        }
        if let Some((old_start, new_start)) = parse_hunk_header(line) {
            flush_pending(&mut rows, &mut pending);
            if !rows.is_empty() {
                hunks.push(Hunk {
                    rows: std::mem::take(&mut rows),
                });
            }
            old_no = old_start;
            new_no = new_start;
            continue;
        }
        let Some(tag) = line.chars().next() else {
            continue;
        };
        let body = line.chars().skip(1).collect::<String>();
        match tag {
            ' ' => {
                flush_pending(&mut rows, &mut pending);
                rows.push(DiffRow {
                    old_no: Some(old_no),
                    new_no: Some(new_no),
                    old: body.clone(),
                    new: body,
                    kind: DiffRowKind::Equal,
                });
                old_no += 1;
                new_no += 1;
            }
            '-' => {
                pending.push_back((old_no, body));
                old_no += 1;
            }
            '+' => {
                if let Some((old_no, old)) = pending.pop_front() {
                    rows.push(DiffRow {
                        old_no: Some(old_no),
                        new_no: Some(new_no),
                        old,
                        new: body,
                        kind: DiffRowKind::Modify,
                    });
                } else {
                    rows.push(DiffRow {
                        old_no: None,
                        new_no: Some(new_no),
                        old: String::new(),
                        new: body,
                        kind: DiffRowKind::Insert,
                    });
                }
                new_no += 1;
            }
            _ => {}
        }
    }
    flush_pending(&mut rows, &mut pending);
    if !rows.is_empty() {
        hunks.push(Hunk { rows });
    }
    hunks
}

fn flush_pending(rows: &mut Vec<DiffRow>, pending: &mut VecDeque<(usize, String)>) {
    for (old_no, old) in pending.drain(..) {
        rows.push(DiffRow {
            old_no: Some(old_no),
            new_no: None,
            old,
            new: String::new(),
            kind: DiffRowKind::Delete,
        });
    }
}

fn parse_hunk_header(line: &str) -> Option<(usize, usize)> {
    let rest = line.strip_prefix("@@")?.trim_start();
    let old = rest.strip_prefix('-')?;
    let old_end = old.find(' ')?;
    let old_range = &old[..old_end];
    let new = old[old_end..].trim_start().strip_prefix('+')?;
    let new_end = new.find(' ').unwrap_or(new.len());
    let new_range = &new[..new_end];
    Some((range_start(old_range), range_start(new_range)))
}

fn range_start(range: &str) -> usize {
    range
        .split(',')
        .next()
        .and_then(|value| value.parse().ok())
        .unwrap_or(1)
}

fn render_side_by_side(
    hunks: &[Hunk],
    path: &str,
    width: usize,
    palette: &Palette,
    limit: usize,
) -> (Vec<Line<'static>>, bool) {
    let max_line = hunks
        .iter()
        .flat_map(|hunk| &hunk.rows)
        .flat_map(|row| [row.old_no, row.new_no])
        .flatten()
        .max()
        .unwrap_or(1);
    let number_width = digits(max_line).max(3);
    let gutter = number_width + 1;
    let gap = 1;
    let usable = width.saturating_sub(gap).max(2);
    let left_total = usable / 2;
    let right_total = usable - left_total;
    let left_content = left_total.saturating_sub(gutter).max(1);
    let right_content = right_total.saturating_sub(gutter).max(1);

    let language = language_for(path);
    let add_bg = tint(palette.diff_add, palette.background, 0.22);
    let delete_bg = tint(palette.diff_del, palette.background, 0.22);

    let mut lines = Vec::new();
    let mut truncated = false;
    'outer: for (hunk_index, hunk) in hunks.iter().enumerate() {
        if hunk_index > 0 {
            if lines.len() >= limit {
                truncated = true;
                break;
            }
            lines.push(Line::from(Span::styled(
                "…",
                Style::default().fg(palette.muted),
            )));
        }
        for row in &hunk.rows {
            if lines.len() >= limit {
                truncated = true;
                break 'outer;
            }
            let (left_bg, right_bg) = match row.kind {
                DiffRowKind::Equal => (None, None),
                DiffRowKind::Delete => (Some(delete_bg), None),
                DiffRowKind::Insert => (None, Some(add_bg)),
                DiffRowKind::Modify => (Some(delete_bg), Some(add_bg)),
            };
            let left = side_rows(
                row.old_no,
                &row.old,
                language,
                left_content,
                number_width,
                left_bg,
                palette,
            );
            let right = side_rows(
                row.new_no,
                &row.new,
                language,
                right_content,
                number_width,
                right_bg,
                palette,
            );
            let height = left.len().max(right.len()).max(1);
            for index in 0..height {
                if lines.len() >= limit {
                    truncated = true;
                    break 'outer;
                }
                let mut spans = left
                    .get(index)
                    .cloned()
                    .unwrap_or_else(|| blank_side(left_total, None, palette));
                spans.push(Span::raw(" "));
                spans.extend(
                    right
                        .get(index)
                        .cloned()
                        .unwrap_or_else(|| blank_side(right_total, None, palette)),
                );
                lines.push(Line::from(spans));
            }
        }
    }
    (lines, truncated)
}

#[allow(clippy::too_many_arguments)]
fn side_rows(
    number: Option<usize>,
    text: &str,
    language: &str,
    content_width: usize,
    number_width: usize,
    background: Option<Color>,
    palette: &Palette,
) -> Vec<Vec<Span<'static>>> {
    let highlighted = markdown::highlight_code(text, language, palette);
    let mut rows = wrap_spans(&highlighted.spans, content_width);
    if rows.is_empty() {
        rows.push(Vec::new());
    }
    rows.into_iter()
        .enumerate()
        .map(|(index, mut content)| {
            let gutter = if index == 0 {
                number.map_or_else(
                    || " ".repeat(number_width + 1),
                    |number| format!("{number:>number_width$} "),
                )
            } else {
                " ".repeat(number_width + 1)
            };
            let mut number_style = Style::default().fg(palette.muted);
            if let Some(background) = background {
                number_style = number_style.bg(background);
            }
            let used = spans_width(&content);
            if let Some(background) = background {
                content = content
                    .into_iter()
                    .map(|span| {
                        let style = span.style.bg(background);
                        Span::styled(span.content.into_owned(), style)
                    })
                    .collect();
            }
            let mut spans = vec![Span::styled(gutter, number_style)];
            spans.append(&mut content);
            if used < content_width {
                let style = match background {
                    Some(background) => Style::default().bg(background),
                    None => Style::default(),
                };
                spans.push(Span::styled(" ".repeat(content_width - used), style));
            }
            spans
        })
        .collect()
}

fn blank_side(width: usize, background: Option<Color>, palette: &Palette) -> Vec<Span<'static>> {
    let style = match background {
        Some(background) => Style::default().bg(background),
        None => Style::default().fg(palette.background),
    };
    vec![Span::styled(" ".repeat(width), style)]
}

fn render_unified(
    hunks: &[Hunk],
    path: &str,
    width: usize,
    palette: &Palette,
    limit: usize,
) -> (Vec<Line<'static>>, bool) {
    let max_line = hunks
        .iter()
        .flat_map(|hunk| &hunk.rows)
        .flat_map(|row| [row.old_no, row.new_no])
        .flatten()
        .max()
        .unwrap_or(1);
    let number_width = digits(max_line).max(2);
    let language = language_for(path);

    let mut lines = Vec::new();
    let mut truncated = false;
    'outer: for (hunk_index, hunk) in hunks.iter().enumerate() {
        if hunk_index > 0 {
            if lines.len() >= limit {
                truncated = true;
                break;
            }
            lines.push(Line::from(Span::styled(
                "…",
                Style::default().fg(palette.muted),
            )));
        }
        for row in &hunk.rows {
            for part in unified_parts(row, palette) {
                if lines.len() >= limit {
                    truncated = true;
                    break 'outer;
                }
                let prefix = format!(
                    "{} {:>number_width$} ",
                    part.sign,
                    part.number
                        .map_or_else(|| "·".to_owned(), |number| number.to_string())
                );
                let prefix_width = UnicodeWidthStr::width(prefix.as_str());
                let content_width = width.saturating_sub(prefix_width).max(1);
                let highlighted = markdown::highlight_code(part.text, language, palette);
                let mut spans = truncate_spans(&highlighted.spans, content_width);
                if let Some(background) = part.background {
                    spans = spans
                        .into_iter()
                        .map(|span| {
                            let style = span.style.bg(background);
                            Span::styled(span.content.into_owned(), style)
                        })
                        .collect();
                }
                let mut style = Style::default().fg(part.color);
                if let Some(background) = part.background {
                    style = style.bg(background);
                }
                let mut line = vec![Span::styled(prefix, style)];
                line.append(&mut spans);
                lines.push(Line::from(line));
            }
        }
    }
    (lines, truncated)
}

struct UnifiedPart<'a> {
    sign: &'static str,
    number: Option<usize>,
    text: &'a str,
    background: Option<Color>,
    color: Color,
}

fn unified_parts<'a>(row: &'a DiffRow, palette: &Palette) -> Vec<UnifiedPart<'a>> {
    let add_bg = tint(palette.diff_add, palette.background, 0.22);
    let delete_bg = tint(palette.diff_del, palette.background, 0.22);
    match row.kind {
        DiffRowKind::Equal => vec![UnifiedPart {
            sign: " ",
            number: row.old_no,
            text: row.old.as_str(),
            background: None,
            color: palette.muted,
        }],
        DiffRowKind::Delete => vec![UnifiedPart {
            sign: "-",
            number: row.old_no,
            text: row.old.as_str(),
            background: Some(delete_bg),
            color: palette.diff_del,
        }],
        DiffRowKind::Insert => vec![UnifiedPart {
            sign: "+",
            number: row.new_no,
            text: row.new.as_str(),
            background: Some(add_bg),
            color: palette.diff_add,
        }],
        DiffRowKind::Modify => vec![
            UnifiedPart {
                sign: "-",
                number: row.old_no,
                text: row.old.as_str(),
                background: Some(delete_bg),
                color: palette.diff_del,
            },
            UnifiedPart {
                sign: "+",
                number: row.new_no,
                text: row.new.as_str(),
                background: Some(add_bg),
                color: palette.diff_add,
            },
        ],
    }
}

/// Wraps styled spans at `width` display columns.
fn wrap_spans(spans: &[Span<'static>], width: usize) -> Vec<Vec<Span<'static>>> {
    if width == 0 {
        return vec![Vec::new()];
    }
    let mut rows: Vec<Vec<Span<'static>>> = Vec::new();
    let mut row: Vec<Span<'static>> = Vec::new();
    let mut row_width = 0_usize;
    let mut buffer = String::new();
    let mut buffer_style: Option<Style> = None;

    fn flush(buffer: &mut String, style: &mut Option<Style>, row: &mut Vec<Span<'static>>) {
        if buffer.is_empty() {
            return;
        }
        let style = style.take().unwrap_or_default();
        row.push(Span::styled(std::mem::take(buffer), style));
    }

    for span in spans {
        let style = span.style;
        for character in span.content.chars() {
            let char_width = character.width().unwrap_or(0);
            if row_width + char_width > width && row_width > 0 {
                flush(&mut buffer, &mut buffer_style, &mut row);
                rows.push(std::mem::take(&mut row));
                row_width = 0;
            }
            if buffer_style != Some(style) {
                flush(&mut buffer, &mut buffer_style, &mut row);
                buffer_style = Some(style);
            }
            buffer.push(character);
            row_width += char_width;
        }
    }
    flush(&mut buffer, &mut buffer_style, &mut row);
    if !row.is_empty() || rows.is_empty() {
        rows.push(row);
    }
    rows
}

/// Wraps one fully styled transcript line on word boundaries.
pub(crate) fn wrap_line(line: Line<'static>, width: usize) -> Vec<Line<'static>> {
    let width = width.max(1);
    if line.width() <= width {
        return vec![line];
    }
    let mut graphemes = Vec::new();
    for span in &line.spans {
        let style = line.style.patch(span.style);
        for grapheme in span.content.graphemes(true) {
            graphemes.push((grapheme.to_owned(), style));
        }
    }
    wrap_graphemes(graphemes, width)
        .into_iter()
        .map(|row| Line::from(group_spans(row)))
        .collect()
}

fn wrap_graphemes(graphemes: Vec<(String, Style)>, width: usize) -> Vec<Vec<(String, Style)>> {
    let mut rows = Vec::new();
    let mut current: Vec<(String, Style)> = Vec::new();
    let mut current_width = 0_usize;

    for (grapheme, style) in graphemes {
        let grapheme_width = UnicodeWidthStr::width(grapheme.as_str());
        if current_width + grapheme_width > width && current_width > 0 {
            let whitespace = current
                .iter()
                .rposition(|(value, _)| value.chars().all(char::is_whitespace))
                .filter(|position| *position > 0);
            if let Some(position) = whitespace {
                let mut carry = current.split_off(position + 1);
                while current
                    .last()
                    .is_some_and(|(value, _)| value.chars().all(char::is_whitespace))
                {
                    current.pop();
                }
                if !current.is_empty() {
                    rows.push(current);
                }
                while carry
                    .first()
                    .is_some_and(|(value, _)| value.chars().all(char::is_whitespace))
                {
                    carry.remove(0);
                }
                current = carry;
                current_width = current
                    .iter()
                    .map(|(value, _)| UnicodeWidthStr::width(value.as_str()))
                    .sum();
                if current_width + grapheme_width > width {
                    if !current.is_empty() {
                        rows.push(std::mem::take(&mut current));
                    }
                    current_width = 0;
                }
            } else {
                rows.push(std::mem::take(&mut current));
                current_width = 0;
            }
        }
        current_width += grapheme_width;
        current.push((grapheme, style));
    }

    if !current.is_empty() || rows.is_empty() {
        rows.push(current);
    }
    rows
}

fn group_spans(row: Vec<(String, Style)>) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut text = String::new();
    let mut style: Option<Style> = None;
    for (grapheme, grapheme_style) in row {
        if style != Some(grapheme_style) {
            if !text.is_empty() {
                spans.push(Span::styled(
                    std::mem::take(&mut text),
                    style.unwrap_or_default(),
                ));
            }
            style = Some(grapheme_style);
        }
        text.push_str(&grapheme);
    }
    if !text.is_empty() {
        spans.push(Span::styled(text, style.unwrap_or_default()));
    }
    spans
}

fn truncate_spans(spans: &[Span<'static>], width: usize) -> Vec<Span<'static>> {
    let mut output = Vec::new();
    let mut used = 0_usize;
    for span in spans {
        let mut text = String::new();
        for character in span.content.chars() {
            let char_width = character.width().unwrap_or(0);
            if used + char_width > width.saturating_sub(1) {
                if used < width {
                    text.push('…');
                }
                if !text.is_empty() {
                    output.push(Span::styled(std::mem::take(&mut text), span.style));
                }
                return output;
            }
            text.push(character);
            used += char_width;
        }
        if !text.is_empty() {
            output.push(Span::styled(text, span.style));
        }
    }
    output
}

fn spans_width(spans: &[Span<'_>]) -> usize {
    spans
        .iter()
        .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
        .sum()
}

fn digits(number: usize) -> usize {
    number.max(1).to_string().len()
}

/// Blends `color` toward `background`; `weight` is the share kept of `color`.
fn tint(color: Color, background: Color, weight: f32) -> Color {
    let (Color::Rgb(red, green, blue), Color::Rgb(bg_red, bg_green, bg_blue)) = (color, background)
    else {
        return color;
    };
    let blend = |value: u8, base: u8| {
        (f32::from(value) * weight + f32::from(base) * (1.0 - weight)).round() as u8
    };
    Color::Rgb(
        blend(red, bg_red),
        blend(green, bg_green),
        blend(blue, bg_blue),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIFF: &str = "\
--- src/lib.rs
+++ src/lib.rs
@@ -1,3 +1,3 @@
 fn main() {
-    let old = 1;
+    let new = 2;
 }
";

    fn plain(lines: &[Line<'static>]) -> Vec<String> {
        lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect()
    }

    #[test]
    fn parses_unified_diff_into_aligned_rows() {
        let hunks = parse_unified(DIFF);
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].rows.len(), 3);
        assert!(matches!(hunks[0].rows[0].kind, DiffRowKind::Equal));
        assert!(matches!(hunks[0].rows[1].kind, DiffRowKind::Modify));
        assert!(matches!(hunks[0].rows[2].kind, DiffRowKind::Equal));
        assert_eq!(hunks[0].rows[1].old_no, Some(2));
        assert_eq!(hunks[0].rows[1].new_no, Some(2));
    }

    #[test]
    fn side_by_side_renders_both_sides_with_numbers() {
        let palette = Palette::dark();
        let (lines, truncated) = render_diff(DIFF, "src/lib.rs", 100, &palette, 80);
        assert!(!truncated);
        let text = plain(&lines).join("\n");
        assert!(text.contains("let old = 1;"), "{text}");
        assert!(text.contains("let new = 2;"), "{text}");
        assert!(text.contains(" 1 "), "old line number: {text}");
        assert!(text.contains(" 2 "), "changed line number: {text}");
    }

    #[test]
    fn narrow_diff_falls_back_to_unified_rows() {
        let palette = Palette::dark();
        let (lines, _) = render_diff(DIFF, "src/lib.rs", 40, &palette, 80);
        let text = plain(&lines).join("\n");
        let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(compact.contains("- 2 let old"), "{text}");
        assert!(compact.contains("+ 2 let new"), "{text}");
    }

    #[test]
    fn code_view_uses_line_numbers_and_highlighting() {
        let palette = Palette::dark();
        let (lines, truncated) = render_code(
            "fn main() {}\nlet value = 1;\n",
            "src/main.rs",
            80,
            &palette,
            80,
        );
        assert!(!truncated);
        let text = plain(&lines).join("\n");
        assert!(text.contains("1 fn main"), "{text}");
        assert!(text.contains("2 let value"), "{text}");
    }

    #[test]
    fn preview_limit_reports_truncation() {
        let palette = Palette::dark();
        let content = (1..=10)
            .map(|index| format!("line {index}"))
            .collect::<Vec<_>>()
            .join("\n");
        let (lines, truncated) = render_code(&content, "notes.txt", 80, &palette, 3);
        assert_eq!(lines.len(), 3);
        assert!(truncated);
    }
}
