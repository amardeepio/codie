//! Markdown rendering and lightweight code highlighting (`TUI-004`).

use std::sync::OnceLock;

use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use regex::Regex;

use crate::theme::Palette;

/// Renders Markdown into styled terminal lines.
#[must_use]
pub fn render(markdown: &str, palette: &Palette) -> Vec<Line<'static>> {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);

    let mut renderer = Renderer {
        palette: *palette,
        lines: Vec::new(),
        current: Vec::new(),
        code: None,
        list_depth: 0,
        blockquote: false,
        strong: false,
        emphasis: false,
        link: false,
        heading: false,
    };

    for event in Parser::new_ext(markdown, options) {
        renderer.event(event);
    }
    renderer.flush();
    renderer.lines
}

struct Renderer {
    palette: Palette,
    lines: Vec<Line<'static>>,
    current: Vec<Span<'static>>,
    code: Option<(String, String)>,
    list_depth: usize,
    blockquote: bool,
    strong: bool,
    emphasis: bool,
    link: bool,
    heading: bool,
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
                let style = Style::default()
                    .fg(self.palette.tool)
                    .bg(self.palette.border);
                self.push_span(format!("`{text}`"), style);
            }
            Event::SoftBreak | Event::HardBreak => self.flush(),
            Event::Rule => {
                self.flush();
                self.lines.push(Line::from(Span::styled(
                    "─".repeat(40),
                    Style::default().fg(self.palette.muted),
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
                self.heading = true;
                let prefix = "#".repeat(usize::from(level as u8));
                self.current.push(Span::styled(
                    format!("{prefix} "),
                    Style::default()
                        .fg(self.palette.accent)
                        .add_modifier(Modifier::BOLD),
                ));
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
                self.lines.push(Line::from(Span::styled(
                    format!("```{language}"),
                    Style::default().fg(self.palette.muted),
                )));
                self.code = Some((language, String::new()));
            }
            Tag::List(_) => {
                self.flush();
                self.list_depth += 1;
            }
            Tag::Item => {
                self.flush();
                self.push_prefix();
                self.push_span("• ".to_owned(), Style::default().fg(self.palette.accent));
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
            Tag::TableHead => self.flush(),
            Tag::TableRow => self.flush(),
            Tag::TableCell => self.push_text("│ "),
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => {
                self.flush();
                self.blank();
            }
            TagEnd::Heading(_) => {
                self.heading = false;
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
                    self.lines.push(Line::from(Span::styled(
                        "```",
                        Style::default().fg(self.palette.muted),
                    )));
                    self.blank();
                }
            }
            TagEnd::List(_) => {
                self.flush();
                self.list_depth = self.list_depth.saturating_sub(1);
                if self.list_depth == 0 {
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
            TagEnd::TableHead | TagEnd::TableRow => self.flush(),
            TagEnd::TableCell => self.push_text(" "),
            _ => {}
        }
    }

    fn push_text(&mut self, text: &str) {
        let mut style = Style::default().fg(self.palette.assistant);
        if self.heading {
            style = style.fg(self.palette.accent).add_modifier(Modifier::BOLD);
        }
        if self.strong {
            style = style.add_modifier(Modifier::BOLD);
        }
        if self.emphasis {
            style = style.add_modifier(Modifier::ITALIC);
        }
        if self.link {
            style = style
                .fg(self.palette.accent)
                .add_modifier(Modifier::UNDERLINED);
        }
        self.push_span(text.to_owned(), style);
    }

    fn push_span(&mut self, text: String, style: Style) {
        if text.is_empty() {
            return;
        }
        if self.current.is_empty() {
            self.push_prefix();
        }
        self.current.push(Span::styled(text, style));
    }

    fn push_prefix(&mut self) {
        let indent = "  ".repeat(self.list_depth.saturating_sub(1));
        if self.blockquote {
            self.current.push(Span::styled(
                format!("{indent}│ "),
                Style::default().fg(self.palette.muted),
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

/// Highlights keywords in fenced code blocks.
///
/// This is intentionally lightweight: keywords are colored, the rest of the
/// line keeps the body color. It is not a full parser.
#[must_use]
pub fn highlight_code(line: &str, language: &str, palette: &Palette) -> Line<'static> {
    if !matches!(
        language.to_ascii_lowercase().as_str(),
        "rust" | "rs" | "python" | "py" | "javascript" | "js" | "typescript" | "ts" | "bash" | "sh"
    ) {
        return Line::from(Span::styled(
            line.to_owned(),
            Style::default().fg(palette.assistant),
        ));
    }

    static KEYWORDS: OnceLock<Regex> = OnceLock::new();
    let keywords = KEYWORDS.get_or_init(|| {
        Regex::new(
            r"\b(fn|let|mut|pub|struct|enum|impl|trait|use|mod|match|if|else|for|while|loop|return|async|await|move|const|static|def|class|import|from|lambda|function|var|export|interface|type|echo|cd|git|cargo|npm|sudo)\b",
        )
        .expect("keyword regex")
    });

    let mut spans = Vec::new();
    let mut cursor = 0;
    for found in keywords.find_iter(line) {
        if found.start() > cursor {
            spans.push(Span::styled(
                line[cursor..found.start()].to_owned(),
                Style::default().fg(palette.assistant),
            ));
        }
        spans.push(Span::styled(
            found.as_str().to_owned(),
            Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD),
        ));
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

    #[test]
    #[spec("TUI-004")]
    fn headings_lists_and_code_render() {
        let palette = Palette::dark();
        let lines = render("# Title\n\n- item\n\n```rust\nfn main() {}\n```", &palette);
        let text = plain(&lines);
        assert!(text.iter().any(|line| line.contains("Title")));
        assert!(text.iter().any(|line| line.contains("• item")));
        assert!(text.iter().any(|line| line.contains("fn main")));
        assert!(text.iter().any(|line| line.contains("```rust")));
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
    fn links_are_underlined() {
        let palette = Palette::dark();
        let lines = render("[codie](https://example.com)", &palette);
        assert!(lines.iter().any(|line| {
            line.spans.iter().any(|span| {
                span.content.contains("codie")
                    && span.style.add_modifier.contains(Modifier::UNDERLINED)
            })
        }));
    }
}
