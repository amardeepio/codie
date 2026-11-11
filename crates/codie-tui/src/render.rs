//! Ratatui rendering for the transcript, editor, footer, and overlays.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph, Wrap};
use unicode_width::UnicodeWidthStr;

use crate::app::{App, Overlay, Status};
use crate::markdown;
use crate::theme::Palette;
use crate::transcript::{Entry, ToolStatus};

const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Draws one frame.
pub fn draw(frame: &mut Frame<'_>, app: &mut App) {
    let area = frame.area();
    let editor_height = u16::try_from(app.editor.lines().len())
        .unwrap_or(1)
        .saturating_add(2)
        .clamp(3, 10);
    let chunks = Layout::vertical([
        Constraint::Min(3),
        Constraint::Length(editor_height),
        Constraint::Length(1),
    ])
    .split(area);

    draw_transcript(frame, chunks[0], app);
    draw_editor(frame, chunks[1], app);
    draw_footer(frame, chunks[2], app);

    if let Some(overlay) = &app.overlay {
        draw_overlay(frame, area, overlay, app);
    }
}

fn draw_transcript(frame: &mut Frame<'_>, area: Rect, app: &mut App) {
    let lines = transcript_lines(app, area.width.saturating_sub(1));
    let width = usize::from(area.width.max(1));
    let total = lines
        .iter()
        .map(|line| {
            let line_width = line.width();
            if line_width == 0 {
                1
            } else {
                line_width.div_ceil(width)
            }
        })
        .sum::<usize>();
    let maximum = u16::try_from(total.saturating_sub(usize::from(area.height))).unwrap_or(u16::MAX);
    app.transcript.clamp_scroll(maximum);
    let paragraph = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .scroll((app.transcript.scroll(), 0));
    frame.render_widget(paragraph, area);
}

fn transcript_lines(app: &App, width: u16) -> Vec<Line<'static>> {
    let palette = app.theme.palette;
    let mut lines = Vec::new();
    let _ = width;

    if app.transcript.is_empty() {
        lines.push(Line::from(Span::styled(
            "codie",
            Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(Span::styled(
            "Type a prompt, or /help for commands. Enter submits; Shift+Enter adds a line.",
            Style::default().fg(palette.muted),
        )));
        lines.push(Line::default());
    }

    for entry in app.transcript.entries() {
        match entry {
            Entry::User(text) => {
                lines.push(Line::default());
                lines.push(Line::from(vec![
                    Span::styled(
                        "› ",
                        Style::default()
                            .fg(palette.accent)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(text.clone(), Style::default().fg(palette.user)),
                ]));
            }
            Entry::Assistant(assistant) => {
                if !assistant.thinking.is_empty() {
                    for line in assistant.thinking.lines() {
                        lines.push(Line::from(Span::styled(
                            format!("· {line}"),
                            Style::default()
                                .fg(palette.muted)
                                .add_modifier(Modifier::ITALIC),
                        )));
                    }
                }
                lines.extend(markdown::render(&assistant.text, &palette));
                if assistant.streaming {
                    lines.push(Line::from(Span::styled(
                        "▌",
                        Style::default().fg(palette.accent),
                    )));
                }
                lines.push(Line::default());
            }
            Entry::Tool(block) => {
                let (marker, color) = match block.status {
                    ToolStatus::Running => ("…", palette.tool),
                    ToolStatus::Success => ("✓", palette.success),
                    ToolStatus::Error => ("✗", palette.error),
                };
                let mut spans = vec![
                    Span::styled(
                        format!("{marker} "),
                        Style::default().fg(color).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(block.summary(), Style::default().fg(palette.tool)),
                ];
                if block.status != ToolStatus::Running && !block.expanded {
                    spans.push(Span::styled(
                        "  (Ctrl+O to expand)",
                        Style::default().fg(palette.muted),
                    ));
                }
                lines.push(Line::from(spans));
                if block.expanded && !block.output.is_empty() {
                    for line in block.output.lines().take(200) {
                        let color = if line.starts_with('+') {
                            palette.diff_add
                        } else if line.starts_with('-') {
                            palette.diff_del
                        } else {
                            palette.foreground
                        };
                        lines.push(Line::from(Span::styled(
                            format!("  {line}"),
                            Style::default().fg(color),
                        )));
                    }
                }
            }
            Entry::Info(text) => lines.push(Line::from(Span::styled(
                text.clone(),
                Style::default().fg(palette.muted),
            ))),
            Entry::Error(text) => lines.push(Line::from(Span::styled(
                format!("! {text}"),
                Style::default().fg(palette.error),
            ))),
        }
    }
    lines
}

fn draw_editor(frame: &mut Frame<'_>, area: Rect, app: &mut App) {
    let palette = app.theme.palette;
    let (title, border) = match app.status {
        Status::Idle => (" prompt ".to_owned(), palette.border),
        Status::Streaming => (
            format!(" {} streaming ", SPINNER[app.tick as usize % SPINNER.len()]),
            palette.accent,
        ),
        Status::Waiting => (" waiting ".to_owned(), palette.tool),
        Status::Error => (" error ".to_owned(), palette.error),
    };
    let block = Block::bordered()
        .title(title)
        .border_style(Style::default().fg(border));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let visible = usize::from(inner.height.max(1));
    let cursor_row = app.editor.cursor().row;
    let offset = cursor_row.saturating_sub(visible.saturating_sub(1));
    let lines = app
        .editor
        .lines()
        .iter()
        .skip(offset)
        .take(visible)
        .map(|line| Line::from(line.clone()))
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(lines), inner);

    let row = cursor_row.saturating_sub(offset);
    if row < visible {
        let line = app
            .editor
            .lines()
            .get(cursor_row)
            .map_or("", String::as_str);
        let column = UnicodeWidthStr::width(&line[..app.editor.cursor().col.min(line.len())]);
        let x = inner
            .x
            .saturating_add(u16::try_from(column).unwrap_or(u16::MAX))
            .min(inner.x.saturating_add(inner.width.saturating_sub(1)));
        let y = inner
            .y
            .saturating_add(u16::try_from(row).unwrap_or(u16::MAX))
            .min(inner.y.saturating_add(inner.height.saturating_sub(1)));
        frame.set_cursor_position((x, y));
    }
}

fn draw_footer(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let palette = app.theme.palette;
    let columns =
        Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)]).split(area);

    let left = app.footer.cwd.clone();
    let left = truncate(&left, usize::from(columns[0].width));
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            left,
            Style::default().fg(palette.muted),
        ))),
        columns[0],
    );

    let mut right = format!("{} · {}", app.footer.model, app.footer.thinking);
    if app.footer.tokens > 0 {
        right.push_str(&format!(
            " · {} tok · ${:.4}",
            app.footer.tokens,
            app.footer.cost as f64 / 1_000_000_000.0
        ));
    }
    if let Some(percent) = app.footer.context_percent {
        right.push_str(&format!(" · {percent}% ctx"));
    }
    if let Some(session) = &app.footer.session {
        right.push_str(&format!(" · {}", &session[..session.len().min(8)]));
    }
    let right = truncate(&right, usize::from(columns[1].width));
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            right,
            Style::default().fg(palette.muted),
        )))
        .alignment(Alignment::Right),
        columns[1],
    );
}

fn draw_overlay(frame: &mut Frame<'_>, area: Rect, overlay: &Overlay, app: &App) {
    let palette = app.theme.palette;
    let width = area.width.saturating_mul(4) / 5;
    let height = area.height.saturating_mul(3) / 5;
    let rect = centered(area, width.max(20), height.max(6));
    frame.render_widget(Clear, rect);

    let (title, body) = match overlay {
        Overlay::Help => (
            " commands ",
            [
                "/help              this help",
                "/hotkeys           keybindings",
                "/new               start a new session",
                "/session           show the current session",
                "/model [id]        change the model",
                "/thinking [level]  change reasoning effort",
                "/compact           compaction (Phase 3)",
                "/quit              exit",
            ]
            .join("\n"),
        ),
        Overlay::Hotkeys => (" keys ", app.bindings.describe()),
        Overlay::Permission(prompt) => (
            " permission ",
            format!(
                "Allow `{}`?\n\n{}\n\n[a] allow once   [A] always   [d] deny",
                prompt.tool,
                serde_json::to_string_pretty(&prompt.arguments).unwrap_or_default()
            ),
        ),
    };

    let block = Block::bordered()
        .title(title)
        .border_style(Style::default().fg(palette.accent));
    frame.render_widget(
        Paragraph::new(body)
            .block(block)
            .wrap(Wrap { trim: false })
            .style(Style::default().fg(palette.foreground)),
        rect,
    );
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    }
}

fn truncate(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_owned();
    }
    let head: String = text.chars().take(width.saturating_sub(1)).collect();
    format!("{head}…")
}

/// Returns the palette used by a rendered app.
#[must_use]
pub fn palette(app: &App) -> Palette {
    app.theme.palette
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Footer;
    use codie_testkit::spec;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn app() -> App {
        let mut app = App::new(
            "/tmp/project",
            codie_core::ModelRef {
                api: "openai-completions".into(),
                provider: "deepseek".into(),
                model: "deepseek-flash".into(),
            },
            codie_core::ThinkingLevel::Medium,
            crate::theme::Theme::load(Some("dark")).unwrap(),
            None,
            Vec::new(),
            "/tmp/sessions".into(),
            std::sync::Arc::new(std::sync::atomic::AtomicI32::new(0)),
            crate::keybindings::Bindings::default(),
        );
        app.footer = Footer {
            cwd: "/tmp/project".into(),
            model: "deepseek-flash".into(),
            thinking: "medium".into(),
            ..Footer::default()
        };
        app
    }

    fn buffer(app: &mut App) -> String {
        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, app)).unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    #[test]
    #[spec("TUI-006")]
    fn empty_state_and_footer_render() {
        let mut app = app();
        let text = buffer(&mut app);
        assert!(text.contains("codie"));
        assert!(text.contains("deepseek-flash"));
        assert!(text.contains("/tmp/project"));
    }

    #[test]
    #[spec("TUI-001")]
    fn spec_tui_001_fixed_editor_and_footer() {
        let mut app = app();
        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &mut app)).unwrap();
        let buffer = terminal.backend().buffer();
        let row = |y: u16| (0..80).map(|x| buffer[(x, y)].symbol()).collect::<String>();
        assert!(row(16).contains("prompt"), "editor dock missing");
        assert!(row(19).contains("deepseek-flash"), "footer missing");
    }

    #[test]
    fn transcript_entries_render() {
        let mut app = app();
        app.transcript.push_user("fix the bug");
        app.transcript.start_assistant();
        app.transcript
            .set_text("# Done\n\nThe test passes.".to_owned());
        app.transcript.finish_assistant();
        let text = buffer(&mut app);
        assert!(text.contains("fix the bug"));
        assert!(text.contains("Done"));
        assert!(text.contains("test passes"));
    }

    #[test]
    fn permission_overlay_renders() {
        let mut app = app();
        let (reply, _answer) = tokio::sync::oneshot::channel();
        app.push_permission(crate::permissions::PermissionRequest {
            tool: "bash".to_owned(),
            arguments: serde_json::json!({"command": "cargo test"}),
            reply,
        });
        let text = buffer(&mut app);
        assert!(text.contains("permission"));
        assert!(text.contains("allow once"));
        assert!(text.contains("cargo test"));
    }
}
