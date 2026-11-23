//! Markdown rendering and lightweight code highlighting (`TUI-004`).

use std::sync::OnceLock;

use pulldown_cmark::{Alignment, CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use regex::Regex;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::theme::Palette;

/// Narrowest column a table cell may shrink to before the table switches to a stacked layout.
const MIN_COLUMN: usize = 3;

type Cell = Vec<Span<'static>>;
type Row = Vec<Cell>;

/// Renders Markdown into styled terminal lines that fit within `width` columns.
#[must_use]
pub fn render(markdown: &str, palette: &Palette, width: usize) -> Vec<Line<'static>> {
    render_with_base(
        markdown,
        palette,
        width,
        Style::default().fg(palette.assistant),
    )
}

/// Renders a user message: plain text uses `palette.user` and every line keeps
/// the raised surface background. Line breaks are preserved, so a pasted block
/// stays multi-line.
#[must_use]
pub fn render_user(markdown: &str, palette: &Palette, width: usize) -> Vec<Line<'static>> {
    let base = Style::default().fg(palette.user);
    let mut lines = render_with_base(markdown, palette, width, base);
    for line in &mut lines {
        line.style = line.style.bg(palette.surface);
        for span in &mut line.spans {
            span.style = span.style.bg(palette.surface);
        }
    }
    lines
}

fn render_with_base(
    markdown: &str,
    palette: &Palette,
    width: usize,
    base: Style,
) -> Vec<Line<'static>> {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);

    let mut renderer = Renderer {
        palette: *palette,
        width: width.max(1),
        base,
        lines: Vec::new(),
        current: Vec::new(),
        code: None,
        lists: Vec::new(),
        blockquote: false,
        strong: false,
        emphasis: false,
        link: false,
        heading: None,
        table: None,
    };

    for event in Parser::new_ext(markdown, options) {
        renderer.event(event);
    }
    renderer.flush();
    renderer.lines
}

struct Renderer {
    palette: Palette,
    width: usize,
    /// Style for plain body text; semantic styles override its foreground.
    base: Style,
    lines: Vec<Line<'static>>,
    current: Vec<Span<'static>>,
    code: Option<(String, String)>,
    lists: Vec<ListState>,
    blockquote: bool,
    strong: bool,
    emphasis: bool,
    link: bool,
    heading: Option<u8>,
    table: Option<TableState>,
}

struct ListState {
    ordered: bool,
    next: u64,
}

struct TableState {
    alignments: Vec<Alignment>,
    rows: Vec<Row>,
    row: Row,
    cell: Cell,
    in_cell: bool,
    header_rows: usize,
}

impl TableState {
    fn new(alignments: Vec<Alignment>) -> Self {
        Self {
            alignments,
            rows: Vec::new(),
            row: Vec::new(),
            cell: Vec::new(),
            in_cell: false,
            header_rows: 0,
        }
    }
}

enum Border {
    Top,
    Row,
    Bottom,
}

impl Renderer {
    fn event(&mut self, event: Event<'_>) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) => {
                if let Some((_, buffer)) = self.code.as_mut() {
                    buffer.push_str(&text);
                } else {
                    self.push_text(&text);
                }
            }
            Event::Code(text) => {
                let style = Style::default().fg(self.palette.code);
                self.push_span(text.to_string(), style);
            }
            Event::SoftBreak | Event::HardBreak => {
                if self.is_in_table_cell() {
                    self.push_text(" ");
                } else {
                    self.flush();
                }
            }
            Event::Rule => {
                self.flush();
                self.lines.push(Line::from(Span::styled(
                    "─".repeat(self.width.min(40)),
                    Style::default().fg(self.palette.rule),
                )));
            }
            Event::Html(text) | Event::InlineHtml(text) => self.push_text(&text),
            Event::TaskListMarker(checked) => {
                self.push_text(if checked { "[x] " } else { "[ ] " });
            }
            Event::FootnoteReference(name) => self.push_text(&format!("[{name}]")),
            _ => {}
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph => {}
            Tag::Heading { level, .. } => {
                self.flush();
                self.heading = Some(level as u8);
            }
            Tag::BlockQuote(_) => {
                self.flush();
                self.blockquote = true;
                self.push_prefix();
            }
            Tag::CodeBlock(kind) => {
                self.flush();
                let language = match kind {
                    CodeBlockKind::Fenced(language) => language.to_string(),
                    CodeBlockKind::Indented => String::new(),
                };
                self.code = Some((language, String::new()));
            }
            Tag::List(start) => {
                self.flush();
                self.lists.push(ListState {
                    ordered: start.is_some(),
                    next: start.unwrap_or(1),
                });
            }
            Tag::Item => {
                self.flush();
                self.push_prefix();
                let (marker, color) = match self.lists.last_mut() {
                    Some(list) if list.ordered => {
                        let number = list.next;
                        list.next += 1;
                        (format!("{number}. "), self.palette.list_enum)
                    }
                    _ => ("• ".to_owned(), self.palette.list),
                };
                self.push_span(marker, Style::default().fg(color));
            }
            Tag::Emphasis => self.emphasis = true,
            Tag::Strong => self.strong = true,
            Tag::Strikethrough => {
                self.current.push(Span::styled(
                    "~".to_owned(),
                    Style::default().fg(self.palette.muted),
                ));
            }
            Tag::Link { .. } => self.link = true,
            Tag::Image { .. } => {
                self.push_span(
                    "[image] ".to_owned(),
                    Style::default().fg(self.palette.muted),
                );
            }
            Tag::Table(alignments) => {
                self.flush();
                self.table = Some(TableState::new(alignments));
            }
            Tag::TableHead => {
                self.table_row_end();
                if let Some(table) = self.table.as_mut() {
                    table.in_cell = false;
                }
            }
            Tag::TableRow => self.table_row_end(),
            Tag::TableCell => {
                if let Some(table) = self.table.as_mut() {
                    table.cell.clear();
                    table.in_cell = true;
                }
            }
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => {
                if self.is_in_table_cell() {
                    return;
                }
                self.flush();
                self.blank();
            }
            TagEnd::Heading(_) => {
                self.heading = None;
                self.flush();
                self.blank();
            }
            TagEnd::BlockQuote(_) => {
                self.flush();
                self.blockquote = false;
                self.blank();
            }
            TagEnd::CodeBlock => {
                if let Some((language, buffer)) = self.code.take() {
                    for line in buffer.lines() {
                        self.lines
                            .push(highlight_code(line, &language, &self.palette));
                    }
                    self.blank();
                }
            }
            TagEnd::List(_) => {
                self.flush();
                self.lists.pop();
                if self.lists.is_empty() {
                    self.blank();
                }
            }
            TagEnd::Item => self.flush(),
            TagEnd::Emphasis => self.emphasis = false,
            TagEnd::Strong => self.strong = false,
            TagEnd::Strikethrough => {
                self.current.push(Span::styled(
                    "~".to_owned(),
                    Style::default().fg(self.palette.muted),
                ));
            }
            TagEnd::Link => self.link = false,
            TagEnd::TableCell => self.table_cell_end(),
            TagEnd::TableRow => self.table_row_end(),
            TagEnd::TableHead => {
                self.table_row_end();
                if let Some(table) = self.table.as_mut() {
                    table.header_rows += 1;
                    table.in_cell = false;
                }
            }
            TagEnd::Table => {
                self.flush();
                if let Some(table) = self.table.take() {
                    let lines = table_lines(table, &self.palette, self.width);
                    self.lines.extend(lines);
                }
                self.blank();
            }
            _ => {}
        }
    }

    fn table_cell_end(&mut self) {
        if let Some(table) = self.table.as_mut() {
            table.in_cell = false;
            let cell = trim_cell(std::mem::take(&mut table.cell));
            table.row.push(cell);
        }
    }

    fn table_row_end(&mut self) {
        if let Some(table) = self.table.as_mut() {
            let row = std::mem::take(&mut table.row);
            if !row.is_empty() {
                table.rows.push(row);
            }
        }
    }

    fn is_in_table_cell(&self) -> bool {
        self.table.as_ref().is_some_and(|table| table.in_cell)
    }

    fn push_text(&mut self, text: &str) {
        let mut style = self.base;
        if self.blockquote {
            style = style.fg(self.palette.quote);
        }
        if self.heading.is_some() {
            style = style.fg(self.palette.heading).add_modifier(Modifier::BOLD);
        }
        if self.strong {
            style = style.fg(self.palette.strong).add_modifier(Modifier::BOLD);
        }
        if self.emphasis {
            style = style
                .fg(self.palette.emphasis)
                .add_modifier(Modifier::ITALIC);
        }
        if self.link {
            style = style
                .fg(self.palette.link)
                .add_modifier(Modifier::UNDERLINED);
        }
        self.push_span(text.to_owned(), style);
    }

    fn push_span(&mut self, text: String, style: Style) {
        if text.is_empty() {
            return;
        }
        if let Some(table) = self.table.as_mut()
            && table.in_cell
        {
            table.cell.push(Span::styled(text, style));
            return;
        }
        if self.current.is_empty() {
            self.push_prefix();
        }
        self.current.push(Span::styled(text, style));
    }

    fn push_prefix(&mut self) {
        let indent = "  ".repeat(self.lists.len().saturating_sub(1));
        if self.blockquote {
            self.current.push(Span::styled(
                format!("{indent}│ "),
                Style::default().fg(self.palette.quote),
            ));
        } else if !indent.is_empty() {
            self.current.push(Span::raw(indent));
        }
    }

    fn flush(&mut self) {
        if self.current.is_empty() {
            return;
        }
        self.lines
            .push(Line::from(std::mem::take(&mut self.current)));
    }

    fn blank(&mut self) {
        if self.lines.last().is_some_and(|line| !line.spans.is_empty()) {
            self.lines.push(Line::default());
        }
    }
}

fn trim_cell(mut cell: Cell) -> Cell {
    while cell
        .last()
        .is_some_and(|span| span.content.trim().is_empty())
    {
        cell.pop();
    }
    if let Some(first) = cell.first_mut() {
        let trimmed = first.content.trim_start();
        if trimmed.len() != first.content.len() {
            *first = Span::styled(trimmed.to_owned(), first.style);
        }
    }
    if let Some(last) = cell.last_mut() {
        let trimmed = last.content.trim_end();
        if trimmed.len() != last.content.len() {
            *last = Span::styled(trimmed.to_owned(), last.style);
        }
    }
    cell
}

fn spans_width(spans: &[Span<'static>]) -> usize {
    spans
        .iter()
        .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
        .sum()
}

fn spans_text(spans: &[Span<'static>]) -> String {
    spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>()
}

fn table_lines(table: TableState, palette: &Palette, width: usize) -> Vec<Line<'static>> {
    let columns = table
        .alignments
        .len()
        .max(table.rows.iter().map(Vec::len).max().unwrap_or(0));
    if columns == 0 {
        return Vec::new();
    }

    let mut widths = vec![0usize; columns];
    for row in &table.rows {
        for (index, cell) in row.iter().enumerate() {
            widths[index] = widths[index].max(spans_width(cell));
        }
    }

    let overhead = 1 + columns * 3;
    let available = width.saturating_sub(overhead);
    if available < columns * MIN_COLUMN {
        return stacked_lines(&table, palette);
    }
    shrink_widths(&mut widths, available);
    grow_widths(&mut widths, available);

    let mut lines = Vec::new();
    lines.push(border_line(&widths, palette, Border::Top));
    for (index, row) in table.rows.iter().enumerate() {
        let header = index < table.header_rows;
        let wrapped = widths
            .iter()
            .enumerate()
            .map(|(column, width)| {
                row.get(column)
                    .map_or_else(|| vec![Vec::new()], |cell| wrap_cell(cell, *width))
            })
            .collect::<Vec<_>>();
        let height = wrapped.iter().map(Vec::len).max().unwrap_or(1).max(1);
        for line in 0..height {
            let cells = wrapped
                .iter()
                .map(|cell_lines| cell_lines.get(line).cloned().unwrap_or_default())
                .collect::<Vec<_>>();
            lines.push(row_line(
                &cells,
                &widths,
                &table.alignments,
                palette,
                header,
            ));
        }
        if index + 1 < table.rows.len() {
            lines.push(border_line(&widths, palette, Border::Row));
        }
    }
    lines.push(border_line(&widths, palette, Border::Bottom));
    lines
}

fn shrink_widths(widths: &mut [usize], available: usize) {
    let mut total: usize = widths.iter().sum();
    while total > available {
        let widest = widths
            .iter()
            .enumerate()
            .filter(|(_, width)| **width > MIN_COLUMN)
            .max_by_key(|(_, width)| **width)
            .map(|(index, _)| index);
        let Some(index) = widest else {
            break;
        };
        widths[index] -= 1;
        total -= 1;
    }
}

/// Distributes leftover width proportionally so the table spans the transcript.
fn grow_widths(widths: &mut [usize], available: usize) {
    let content: usize = widths.iter().sum();
    if content >= available {
        return;
    }
    let extra = available - content;
    let divisor = content.max(1);
    let last = widths.len().saturating_sub(1);
    let mut assigned = 0usize;
    for (index, width) in widths.iter_mut().enumerate() {
        let share = if index == last {
            extra - assigned
        } else {
            extra * *width / divisor
        };
        *width += share;
        assigned += share;
    }
}

/// Wraps a cell onto multiple lines at `width`, preserving span styles.
fn wrap_cell(cell: &[Span<'static>], width: usize) -> Vec<Cell> {
    if width == 0 {
        return vec![Vec::new()];
    }

    let mut lines: Vec<Cell> = Vec::new();
    let mut line: Cell = Vec::new();
    let mut line_width = 0usize;

    for (word, word_width) in cell_words(cell) {
        let separator = usize::from(!line.is_empty());
        if line_width + separator + word_width <= width {
            if separator == 1 {
                let style = line.last().map_or_else(Style::default, |span| span.style);
                push_styled(&mut line, " ", style);
                line_width += 1;
            }
            for span in word {
                push_styled(&mut line, span.content.as_ref(), span.style);
            }
            line_width += word_width;
            continue;
        }

        if !line.is_empty() {
            lines.push(std::mem::take(&mut line));
            line_width = 0;
        }
        for span in word {
            let mut segment = String::new();
            for character in span.content.chars() {
                let character_width = UnicodeWidthChar::width(character).unwrap_or(0);
                if line_width > 0 && line_width + character_width > width {
                    if !segment.is_empty() {
                        push_styled(&mut line, &segment, span.style);
                        segment.clear();
                    }
                    lines.push(std::mem::take(&mut line));
                    line_width = 0;
                }
                segment.push(character);
                line_width += character_width;
            }
            if !segment.is_empty() {
                push_styled(&mut line, &segment, span.style);
            }
        }
    }

    if !line.is_empty() || lines.is_empty() {
        lines.push(line);
    }
    lines
}

fn cell_words(cell: &[Span<'static>]) -> Vec<(Cell, usize)> {
    let mut words = Vec::new();
    let mut word: Cell = Vec::new();
    let mut width = 0usize;
    for span in cell {
        for character in span.content.chars() {
            if character.is_whitespace() {
                if !word.is_empty() {
                    words.push((std::mem::take(&mut word), width));
                    width = 0;
                }
                continue;
            }
            let mut buffer = [0u8; 4];
            push_styled(&mut word, character.encode_utf8(&mut buffer), span.style);
            width += UnicodeWidthChar::width(character).unwrap_or(0);
        }
    }
    if !word.is_empty() {
        words.push((word, width));
    }
    words
}

fn push_styled(cell: &mut Cell, text: &str, style: Style) {
    if text.is_empty() {
        return;
    }
    if let Some(last) = cell.last_mut()
        && last.style == style
    {
        let mut content = last.content.to_string();
        content.push_str(text);
        last.content = content.into();
        return;
    }
    cell.push(Span::styled(text.to_owned(), style));
}

fn row_line(
    cells: &[Cell],
    widths: &[usize],
    alignments: &[Alignment],
    palette: &Palette,
    header: bool,
) -> Line<'static> {
    let border = Style::default().fg(palette.border);
    let mut spans = vec![Span::styled("│", border)];

    for (index, width) in widths.iter().enumerate() {
        let empty: Cell = Vec::new();
        let cell = cells.get(index).unwrap_or(&empty);
        let fitted = if header {
            cell.iter()
                .map(|span| {
                    Span::styled(
                        span.content.to_string(),
                        Style::default()
                            .fg(palette.heading)
                            .add_modifier(Modifier::BOLD),
                    )
                })
                .collect::<Cell>()
        } else {
            cell.clone()
        };

        let padding = width.saturating_sub(spans_width(&fitted));
        let (left, right) = match alignments.get(index).copied().unwrap_or(Alignment::None) {
            Alignment::Right => (padding, 0),
            Alignment::Center => (padding / 2, padding - padding / 2),
            Alignment::Left | Alignment::None => (0, padding),
        };

        spans.push(Span::raw(" "));
        if left > 0 {
            spans.push(Span::raw(" ".repeat(left)));
        }
        spans.extend(fitted);
        if right > 0 {
            spans.push(Span::raw(" ".repeat(right)));
        }
        spans.push(Span::raw(" "));
        spans.push(Span::styled("│", border));
    }

    Line::from(spans)
}

fn border_line(widths: &[usize], palette: &Palette, border: Border) -> Line<'static> {
    let (left, join, right) = match border {
        Border::Top => ('┌', '┬', '┐'),
        Border::Row => ('├', '┼', '┤'),
        Border::Bottom => ('└', '┴', '┘'),
    };

    let mut text = String::new();
    text.push(left);
    for (index, width) in widths.iter().enumerate() {
        if index > 0 {
            text.push(join);
        }
        text.push_str(&"─".repeat(width + 2));
    }
    text.push(right);

    Line::from(Span::styled(text, Style::default().fg(palette.border)))
}

/// Falls back to one `Key: value` line per cell when a table cannot fit the terminal.
fn stacked_lines(table: &TableState, palette: &Palette) -> Vec<Line<'static>> {
    let headers = (table.header_rows > 0)
        .then(|| table.rows.first())
        .flatten();
    let start = table.header_rows.min(table.rows.len());

    let mut lines = Vec::new();
    for row in &table.rows[start..] {
        for (index, cell) in row.iter().enumerate() {
            if cell.is_empty() {
                continue;
            }
            let mut spans = Vec::new();
            if let Some(label) = headers.and_then(|headers| headers.get(index))
                && !label.is_empty()
            {
                spans.push(Span::styled(
                    format!("{}: ", spans_text(label)),
                    Style::default()
                        .fg(palette.heading)
                        .add_modifier(Modifier::BOLD),
                ));
            }
            spans.extend(cell.iter().cloned());
            lines.push(Line::from(spans));
        }
    }

    if lines.is_empty()
        && let Some(headers) = headers
    {
        for cell in headers {
            if !cell.is_empty() {
                lines.push(Line::from(cell.to_vec()));
            }
        }
    }

    lines
}

/// Highlights comments, strings, numbers, keywords, and function calls in
/// fenced code blocks.
///
/// This is intentionally lightweight: it is line-based and not a full parser.
#[must_use]
pub fn highlight_code(line: &str, language: &str, palette: &Palette) -> Line<'static> {
    let language = language.to_ascii_lowercase();
    if !matches!(
        language.as_str(),
        "rust"
            | "rs"
            | "python"
            | "py"
            | "javascript"
            | "js"
            | "typescript"
            | "ts"
            | "bash"
            | "sh"
            | "powershell"
            | "ps1"
    ) {
        return Line::from(Span::styled(
            line.to_owned(),
            Style::default().fg(palette.assistant),
        ));
    }

    let hash_comments = matches!(
        language.as_str(),
        "python" | "py" | "bash" | "sh" | "powershell" | "ps1"
    );
    let tokens = token_regex(hash_comments);

    let mut spans = Vec::new();
    let mut cursor = 0;
    for captures in tokens.captures_iter(line) {
        let Some(found) = captures.get(0) else {
            continue;
        };
        if found.start() > cursor {
            spans.push(Span::styled(
                line[cursor..found.start()].to_owned(),
                Style::default().fg(palette.assistant),
            ));
        }
        let token = found.as_str().to_owned();
        let style = if captures.name("comment").is_some() {
            Style::default().fg(palette.muted)
        } else if captures.name("string").is_some() {
            Style::default().fg(palette.code)
        } else if captures.name("number").is_some() {
            Style::default().fg(palette.warning)
        } else if captures.name("keyword").is_some() {
            Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(palette.primary)
        };
        spans.push(Span::styled(token, style));
        cursor = found.end();
    }
    if cursor < line.len() {
        spans.push(Span::styled(
            line[cursor..].to_owned(),
            Style::default().fg(palette.assistant),
        ));
    }
    if spans.is_empty() {
        spans.push(Span::raw(String::new()));
    }
    Line::from(spans)
}

fn token_regex(hash_comments: bool) -> &'static Regex {
    static SLASH_COMMENTS: OnceLock<Regex> = OnceLock::new();
    static HASH_COMMENTS: OnceLock<Regex> = OnceLock::new();

    let cell = if hash_comments {
        &HASH_COMMENTS
    } else {
        &SLASH_COMMENTS
    };
    cell.get_or_init(|| {
        let comment = if hash_comments { "#.*" } else { r"//.*|/\*.*?\*/" };
        Regex::new(&format!(
            r#"(?P<comment>{comment})|(?P<string>"(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])*')|(?P<number>\b\d+(?:\.\d+)?\b)|(?P<keyword>\b(fn|let|mut|pub|struct|enum|impl|trait|use|mod|match|if|else|for|while|loop|return|async|await|move|const|static|def|class|import|from|lambda|function|var|export|interface|type|echo|cd|git|cargo|npm|sudo)\b)|(?P<function>\b[A-Za-z_][A-Za-z0-9_]*\s*\()"#
        ))
        .expect("token regex")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use codie_testkit::spec;

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

    fn is_table_line(line: &str) -> bool {
        matches!(line.chars().next(), Some('┌' | '├' | '└' | '│'))
    }

    #[test]
    #[spec("TUI-004")]
    fn spec_tui_004_headings_lists_and_code_render() {
        let palette = Palette::dark();
        let lines = render(
            "# Title\n\n- item\n\n```rust\nfn main() {}\n```",
            &palette,
            80,
        );
        let text = plain(&lines);
        assert!(text.iter().any(|line| line.contains("Title")));
        assert!(!text.iter().any(|line| line.contains("# Title")));
        assert!(text.iter().any(|line| line.contains("• item")));
        assert!(text.iter().any(|line| line.contains("fn main")));
        assert!(
            !text.iter().any(|line| line.contains("```")),
            "code fences are not printed: {text:?}"
        );
    }

    #[test]
    #[spec("TUI-004")]
    fn spec_tui_004_user_messages_keep_lines_and_surface() {
        let palette = Palette::dark();
        let lines = render_user(
            "explain this\n\n```rust\nfn main() {}\nuse std::fs;\n```",
            &palette,
            80,
        );
        let text = plain(&lines);

        assert!(text.iter().any(|line| line == "explain this"), "{text:?}");
        assert!(text.iter().any(|line| line.contains("fn main")), "{text:?}");
        assert!(
            text.iter().any(|line| line.contains("use std::fs")),
            "{text:?}"
        );
        assert!(
            !text.iter().any(|line| line.contains("```")),
            "code fences are not printed: {text:?}"
        );
        for span in lines.iter().flat_map(|line| &line.spans) {
            assert_eq!(
                span.style.bg,
                Some(palette.surface),
                "every user span keeps the raised surface"
            );
        }
        let body = lines
            .iter()
            .flat_map(|line| &line.spans)
            .find(|span| span.content.as_ref() == "explain this")
            .expect("body span");
        assert_eq!(body.style.fg, Some(palette.user));
    }

    #[test]
    #[spec("TUI-004")]
    fn spec_tui_004_ordered_lists_are_numbered() {
        let palette = Palette::dark();
        let text = plain(&render("1. one\n2. two\n", &palette, 80));
        assert!(text.iter().any(|line| line.contains("1. one")));
        assert!(text.iter().any(|line| line.contains("2. two")));
    }

    #[test]
    #[spec("TUI-004")]
    fn spec_tui_004_tables_render_as_aligned_grid() {
        let palette = Palette::dark();
        let markdown = "| Item | Spec |\n|---|---|\n| Model | Mac mini |\n| Chip | Apple M4 |\n";
        let text = plain(&render(markdown, &palette, 60));

        assert!(text[0].starts_with('┌'), "{text:?}");
        assert!(text.iter().any(|line| line.contains("│ Item")), "{text:?}");
        assert!(text.iter().any(|line| line.starts_with("├")), "{text:?}");
        assert!(text.iter().any(|line| line.starts_with("└")), "{text:?}");
        assert!(text.iter().any(|line| line.contains("│ Model")), "{text:?}");

        let widths = text
            .iter()
            .filter(|line| is_table_line(line))
            .map(|line| UnicodeWidthStr::width(line.as_str()))
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(widths.len(), 1, "rows must share one width: {text:?}");
    }

    #[test]
    #[spec("TUI-004")]
    fn spec_tui_004_table_alignment_is_honored() {
        let palette = Palette::dark();
        let markdown = "| left | center | right |\n|:-----|:------:|------:|\n| a | b | c |\n";
        let text = plain(&render(markdown, &palette, 80));

        let row = text
            .iter()
            .find(|line| line.starts_with('│') && line.contains('a') && line.contains('c'))
            .expect("data row");
        let chars = row.chars().collect::<Vec<_>>();
        let borders = chars
            .iter()
            .enumerate()
            .filter(|(_, character)| **character == '│')
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        assert_eq!(borders.len(), 4, "{row:?}");

        let find = |target: char| {
            chars
                .iter()
                .position(|character| *character == target)
                .expect("cell content")
        };
        let a = find('a');
        let b = find('b');
        let c = find('c');
        assert_eq!(a, borders[0] + 2, "left cell hugs the border: {row:?}");
        assert_eq!(c, borders[3] - 2, "right cell hugs the border: {row:?}");
        let left_gap = b - borders[1];
        let right_gap = borders[2] - b;
        assert!(
            left_gap.abs_diff(right_gap) <= 1,
            "center cell is centered: {row:?}"
        );
    }

    #[test]
    #[spec("TUI-004")]
    fn spec_tui_004_tables_rule_every_row() {
        let palette = Palette::dark();
        let markdown = "| a | b |\n|---|---|\n| 1 | 2 |\n| 3 | 4 |\n";
        let text = plain(&render(markdown, &palette, 40));

        // One rule under the header and one between the two data rows.
        assert_eq!(
            text.iter().filter(|line| line.starts_with('├')).count(),
            2,
            "{text:?}"
        );
    }

    #[test]
    #[spec("TUI-004")]
    fn spec_tui_004_tables_fill_the_available_width() {
        let palette = Palette::dark();
        let markdown = "| a | b |\n|---|---|\n| 1 | 2 |\n";
        let text = plain(&render(markdown, &palette, 40));

        for line in text.iter().filter(|line| is_table_line(line)) {
            assert_eq!(UnicodeWidthStr::width(line.as_str()), 40, "{text:?}");
        }
    }

    #[test]
    #[spec("TUI-004")]
    fn spec_tui_004_table_widths_count_wide_characters() {
        let palette = Palette::dark();
        let markdown = "| 名前 | 値 |\n|---|---|\n| codie | テスト |\n";
        let text = plain(&render(markdown, &palette, 40));

        let widths = text
            .iter()
            .filter(|line| is_table_line(line))
            .map(|line| UnicodeWidthStr::width(line.as_str()))
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            widths.len(),
            1,
            "wide characters must not shift columns: {text:?}"
        );
    }

    #[test]
    #[spec("TUI-004")]
    fn spec_tui_004_narrow_tables_stack_key_value_rows() {
        let palette = Palette::dark();
        let markdown = "| Item | Spec |\n|---|---|\n| Model | Mac mini |\n";
        let text = plain(&render(markdown, &palette, 12));

        assert!(!text.iter().any(|line| line.starts_with('┌')), "{text:?}");
        assert!(text.iter().any(|line| line == "Item: Model"), "{text:?}");
        assert!(text.iter().any(|line| line == "Spec: Mac mini"), "{text:?}");
    }

    #[test]
    #[spec("TUI-004")]
    fn spec_tui_004_long_cells_wrap_instead_of_truncating() {
        let palette = Palette::dark();
        let markdown = "| key | value |\n|---|---|\n| a | a very long value that cannot fit |\n";
        let text = plain(&render(markdown, &palette, 24));

        assert!(!text.iter().any(|line| line.contains('…')), "{text:?}");
        assert!(text.iter().any(|line| line.contains("│ a ")), "{text:?}");
        assert!(
            text.iter().any(|line| line.contains("very long")),
            "{text:?}"
        );
        assert!(
            text.iter().any(|line| line.contains("cannot fit")),
            "{text:?}"
        );

        let widths = text
            .iter()
            .filter(|line| is_table_line(line))
            .map(|line| UnicodeWidthStr::width(line.as_str()))
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(widths.len(), 1, "wrapped rows stay aligned: {text:?}");
    }

    #[test]
    fn code_keywords_are_highlighted() {
        let palette = Palette::dark();
        let line = highlight_code("fn main() {}", "rust", &palette);
        assert!(
            line.spans
                .iter()
                .any(|span| span.content == "fn" && span.style.fg == Some(palette.accent))
        );
    }

    #[test]
    fn markdown_semantic_colors_are_distinct() {
        let palette = Palette::dark();
        let lines = render(
            "# Title\n\n**bold** *em* `code` [link](https://example.com)\n",
            &palette,
            80,
        );
        let spans = lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .collect::<Vec<_>>();
        let fg = |text: &str| {
            spans
                .iter()
                .find(|span| span.content.as_ref() == text)
                .and_then(|span| span.style.fg)
        };

        assert_eq!(fg("Title"), Some(palette.heading));
        assert_eq!(fg("bold"), Some(palette.strong));
        assert_eq!(fg("em"), Some(palette.emphasis));
        assert_eq!(fg("code"), Some(palette.code));
        assert_eq!(fg("link"), Some(palette.link));
    }

    #[test]
    fn inline_code_drops_backticks_and_background() {
        let palette = Palette::dark();
        let lines = render("Use `cargo test` now\n", &palette, 80);
        let span = lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .find(|span| span.content.contains("cargo test"))
            .expect("code span");

        assert_eq!(span.content, "cargo test");
        assert_eq!(span.style.fg, Some(palette.code));
        assert_eq!(span.style.bg, None);
        assert!(
            !lines
                .iter()
                .any(|line| { line.spans.iter().any(|span| span.content.contains('`')) }),
            "inline code must not keep its backticks"
        );
    }

    #[test]
    fn list_markers_use_distinct_colors() {
        let palette = Palette::dark();
        let lines = render("- bullet\n1. number\n", &palette, 80);
        let spans = lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .collect::<Vec<_>>();
        let fg = |text: &str| {
            spans
                .iter()
                .find(|span| span.content.as_ref() == text)
                .and_then(|span| span.style.fg)
        };

        assert_eq!(fg("• "), Some(palette.list));
        assert_eq!(fg("1. "), Some(palette.list_enum));
    }

    #[test]
    fn code_tokens_use_syntax_colors() {
        let palette = Palette::dark();
        let line = highlight_code("let answer = 42; // note", "rust", &palette);
        let fg = |text: &str| {
            line.spans
                .iter()
                .find(|span| span.content.as_ref() == text)
                .and_then(|span| span.style.fg)
        };

        assert_eq!(fg("let"), Some(palette.accent));
        assert_eq!(fg("42"), Some(palette.warning));
        assert_eq!(fg("// note"), Some(palette.muted));

        let strings = highlight_code("let name = \"codie\";", "rust", &palette);
        assert!(
            strings
                .spans
                .iter()
                .any(|span| { span.content == "\"codie\"" && span.style.fg == Some(palette.code) })
        );
    }

    #[test]
    fn links_are_underlined() {
        let palette = Palette::dark();
        let lines = render("[codie](https://example.com)", &palette, 80);
        assert!(lines.iter().any(|line| {
            line.spans.iter().any(|span| {
                span.content.contains("codie")
                    && span.style.add_modifier.contains(Modifier::UNDERLINED)
            })
        }));
    }
}
