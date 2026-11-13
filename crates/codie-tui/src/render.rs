//! Ratatui rendering for the transcript, editor, footer, and overlays.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph, Wrap};
use unicode_width::UnicodeWidthStr;

use crate::app::{
    App, CachedToolView, CachedTranscript, Hit, HitTarget, Overlay, Status, ToolViewKey,
    TranscriptCacheKey,
};
use crate::markdown;
use crate::theme::Palette;
use crate::tool_view;
use crate::transcript::{Entry, ToolStatus};

const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// The empty-state banner, 40 columns wide.
const BANNER: [&str; 5] = [
    " ██████╗  ██████╗ ██████╗ ██╗███████╗██╗",
    "██╔════╝ ██╔═══██╗██╔══██╗██║██╔════╝██║",
    "██║      ██║   ██║██║  ██║██║█████╗  ██║",
    "╚██████╗ ╚██████╔╝██████╔╝╚═╝██╔══╝  ╚═╝",
    " ╚═════╝  ╚═════╝ ╚═════╝    ███████╗   ",
];

/// Draws one frame.
pub fn draw(frame: &mut Frame<'_>, app: &mut App) {
    let area = frame.area();
    let palette = app.theme.palette;
    frame.render_widget(
        Block::default().style(Style::default().bg(palette.background)),
        area,
    );
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
        match overlay {
            Overlay::List(list) => draw_list_overlay(frame, area, list, app),
            Overlay::Picker(picker) => draw_picker_overlay(frame, area, picker, app),
            _ => draw_overlay(frame, area, overlay, app),
        }
    }
}

fn draw_list_overlay(frame: &mut Frame<'_>, area: Rect, list: &crate::app::ChoiceList, app: &App) {
    let palette = app.theme.palette;
    let width = area.width.saturating_mul(4) / 5;
    let height = area.height.saturating_mul(3) / 5;
    let rect = centered(area, width.max(30), height.max(8));
    frame.render_widget(Clear, rect);

    let block = Block::bordered()
        .title(list.title.clone())
        .border_style(Style::default().fg(palette.primary));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);

    let visible_height = usize::from(inner.height).saturating_sub(4).max(1);
    let offset = if list.selected >= visible_height {
        list.selected + 1 - visible_height
    } else {
        0
    };

    let query = if list.query.is_empty() {
        "—".to_owned()
    } else {
        list.query.clone()
    };
    let mut lines = vec![
        Line::from(Span::styled(
            format!("filter: {query}"),
            Style::default().fg(palette.muted),
        )),
        Line::default(),
    ];
    if list.visible.is_empty() {
        lines.push(Line::from(Span::styled(
            "No matches.",
            Style::default().fg(palette.muted),
        )));
    }
    for (position, index) in list
        .visible
        .iter()
        .enumerate()
        .skip(offset)
        .take(visible_height)
    {
        let Some(item) = list.items.get(*index) else {
            continue;
        };
        let selected = position == list.selected;
        let marker = if selected { "› " } else { "  " };
        let (style, detail_style) = if selected {
            (
                Style::default()
                    .fg(palette.background)
                    .bg(palette.primary)
                    .add_modifier(Modifier::BOLD),
                Style::default().fg(palette.background).bg(palette.primary),
            )
        } else {
            (
                Style::default().fg(palette.foreground),
                Style::default().fg(palette.muted),
            )
        };
        let mut spans = vec![Span::styled(format!("{marker}{}", item.label), style)];
        if !item.detail.is_empty() {
            spans.push(Span::styled(format!("  {}", item.detail), detail_style));
        }
        lines.push(Line::from(spans));
    }
    lines.push(Line::default());
    lines.push(Line::from(Span::styled(
        list.hint.clone(),
        Style::default().fg(palette.muted),
    )));
    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }).style(
            Style::default()
                .fg(palette.foreground)
                .bg(palette.background),
        ),
        inner,
    );
}

fn draw_transcript(frame: &mut Frame<'_>, area: Rect, app: &mut App) {
    let width = area.width.max(1);
    let key = TranscriptCacheKey {
        revision: app.transcript.revision(),
        width,
        theme_revision: app.theme_revision,
        animation_tick: if app.transcript.is_animated() {
            app.tick
        } else {
            0
        },
    };
    let stale = app
        .transcript_cache
        .borrow()
        .as_ref()
        .is_none_or(|cache| cache.key != key);
    if stale {
        let (lines, targets) = transcript_lines(app, area.width.saturating_sub(1));
        let mut rows = Vec::new();
        let mut row_targets = Vec::new();
        for (line, target) in lines.into_iter().zip(targets) {
            for row in tool_view::wrap_line(line, usize::from(width)) {
                rows.push(row);
                row_targets.push(target);
            }
        }
        *app.transcript_cache.borrow_mut() = Some(CachedTranscript {
            key,
            rows,
            targets: row_targets,
        });
    }

    let total = app
        .transcript_cache
        .borrow()
        .as_ref()
        .map_or(0, |cache| cache.rows.len());
    let maximum = u16::try_from(total.saturating_sub(usize::from(area.height))).unwrap_or(u16::MAX);
    app.transcript.clamp_scroll(maximum);
    let start = usize::from(app.transcript.scroll()).min(total);
    let end = start.saturating_add(usize::from(area.height)).min(total);

    app.hits.clear();
    let mut visible = Vec::with_capacity(end.saturating_sub(start));
    {
        let cache = app.transcript_cache.borrow();
        let Some(cache) = cache.as_ref() else {
            return;
        };
        for (offset, (line, target)) in cache.rows[start..end]
            .iter()
            .zip(&cache.targets[start..end])
            .enumerate()
        {
            visible.push(line.clone());
            if let Some(target) = target {
                app.hits.push(Hit {
                    row: area
                        .y
                        .saturating_add(u16::try_from(offset).unwrap_or(u16::MAX)),
                    target: *target,
                });
            }
        }
    }

    let paragraph = Paragraph::new(visible).style(
        Style::default()
            .fg(app.theme.palette.foreground)
            .bg(app.theme.palette.background),
    );
    frame.render_widget(paragraph, area);
}

fn transcript_lines(app: &App, width: u16) -> (Vec<Line<'static>>, Vec<Option<HitTarget>>) {
    let palette = app.theme.palette;
    let mut body = TranscriptBody::default();

    if app.transcript.is_empty() {
        body.push(Line::default());
        if width >= 40 {
            for row in BANNER {
                body.push(Line::from(Span::styled(
                    row,
                    Style::default()
                        .fg(palette.primary)
                        .add_modifier(Modifier::BOLD),
                )));
            }
        } else {
            body.push(Line::from(Span::styled(
                "Codie!",
                Style::default()
                    .fg(palette.primary)
                    .add_modifier(Modifier::BOLD),
            )));
        }
        body.push(Line::default());
        body.push(Line::from(Span::styled(
            "Type a prompt, or press / for the command palette.",
            Style::default().fg(palette.muted),
        )));
        body.push(Line::default());
    }

    let mut skip_turn = false;
    for (index, entry) in app.transcript.entries().iter().enumerate() {
        let turn_start = app.transcript.is_turn_start(index);
        if turn_start {
            skip_turn = false;
        }
        if skip_turn && !turn_start {
            continue;
        }
        if let Entry::User(text) = entry
            && turn_start
            && app.transcript.is_turn_collapsed(index)
        {
            body.push(Line::default());
            body.push_target(
                Line::from(vec![
                    Span::styled(
                        "▸ ",
                        Style::default()
                            .fg(palette.primary)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        text.clone(),
                        Style::default().fg(palette.user).bg(palette.surface),
                    ),
                    Span::styled(
                        format!("  {}", turn_summary(&app.transcript, index)),
                        Style::default().fg(palette.muted),
                    ),
                ]),
                HitTarget::Turn(index),
            );
            skip_turn = true;
            continue;
        }
        match entry {
            Entry::User(text) => {
                body.push(Line::default());
                let line = Line::from(vec![
                    Span::styled(
                        "▾ ",
                        Style::default()
                            .fg(palette.primary)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        text.clone(),
                        Style::default().fg(palette.user).bg(palette.surface),
                    ),
                ]);
                if app.transcript.is_turn_start(index) {
                    body.push_target(line, HitTarget::Turn(index));
                } else {
                    body.push(line);
                }
            }
            Entry::Assistant(assistant) => {
                if !assistant.thinking.is_empty() {
                    let marker = if assistant.streaming {
                        SPINNER[app.tick as usize % SPINNER.len()]
                    } else {
                        "✻"
                    };
                    let mut spans = vec![Span::styled(
                        format!("{marker} "),
                        Style::default()
                            .fg(palette.primary)
                            .add_modifier(Modifier::BOLD),
                    )];
                    if assistant.thinking_expanded {
                        spans.push(Span::styled("Thinking", Style::default().fg(palette.muted)));
                        spans.push(Span::styled(
                            "  (Esc to hide)",
                            Style::default().fg(palette.muted),
                        ));
                    } else {
                        spans.push(Span::styled(
                            if assistant.streaming {
                                "Thinking…"
                            } else {
                                "Thought"
                            },
                            Style::default()
                                .fg(palette.muted)
                                .add_modifier(Modifier::ITALIC),
                        ));
                        spans.push(Span::styled(
                            "  (click to view)",
                            Style::default().fg(palette.muted),
                        ));
                    }
                    body.push_target(Line::from(spans), HitTarget::Thinking(index));
                    if assistant.thinking_expanded {
                        for line in assistant.thinking.lines() {
                            body.push(Line::from(Span::styled(
                                format!("  {line}"),
                                Style::default()
                                    .fg(palette.muted)
                                    .add_modifier(Modifier::ITALIC),
                            )));
                        }
                        body.push(Line::default());
                    }
                }
                body.extend(markdown::render(
                    &assistant.text,
                    &palette,
                    usize::from(width),
                ));
                if assistant.streaming {
                    body.push(Line::from(Span::styled(
                        "▌",
                        Style::default().fg(palette.primary),
                    )));
                }
                body.push(Line::default());
            }
            Entry::Tool(block) => {
                body.push_target(
                    Line::from(tool_header(block, app.tick, &palette)),
                    HitTarget::Tool(index),
                );

                let mut rendered = false;
                if let Some((lines, truncated)) =
                    rich_tool_body(app, block, width, &palette, block.expanded)
                {
                    let body_len = lines.len();
                    body.extend(lines);
                    rendered = true;
                    if truncated {
                        body.push(Line::from(Span::styled(
                            "  … more lines · Ctrl+O to expand",
                            Style::default().fg(palette.muted),
                        )));
                    } else if block.expanded && body_len > tool_view::PREVIEW_ROWS {
                        body.push(Line::from(Span::styled(
                            "  Ctrl+O to collapse",
                            Style::default().fg(palette.muted),
                        )));
                    }
                    body.push(Line::default());
                }

                if !rendered && block.expanded {
                    let mut expanded = Vec::new();
                    if let Some(render) = &block.render {
                        if let Some(title) = &render.title {
                            expanded.push(Line::from(Span::styled(
                                format!("  {title}"),
                                Style::default().fg(palette.heading),
                            )));
                        }
                        match render.kind {
                            codie_protocol::ext::ExtRenderKind::Diff => {
                                let (lines, _) = tool_view::render_diff(
                                    &render.text,
                                    tool_path(block),
                                    usize::from(width),
                                    &palette,
                                    usize::MAX,
                                );
                                expanded.extend(lines);
                            }
                            codie_protocol::ext::ExtRenderKind::Code => {
                                let (lines, _) = tool_view::render_code(
                                    &render.text,
                                    tool_path(block),
                                    usize::from(width),
                                    &palette,
                                    usize::MAX,
                                );
                                expanded.extend(lines);
                            }
                            _ => {
                                for line in render.text.lines().take(200) {
                                    expanded.push(Line::from(Span::styled(
                                        format!("  {line}"),
                                        Style::default().fg(palette.foreground),
                                    )));
                                }
                            }
                        }
                    } else if !block.output.is_empty() {
                        for line in block.output.lines().take(200) {
                            let color = if line.starts_with('+') {
                                palette.diff_add
                            } else if line.starts_with('-') {
                                palette.diff_del
                            } else {
                                palette.foreground
                            };
                            expanded.push(Line::from(Span::styled(
                                format!("  {line}"),
                                Style::default().fg(color),
                            )));
                        }
                    }
                    body.extend(expanded);
                }

                if block.status != ToolStatus::Running && !block.expanded && !rendered {
                    if let Some(last) = body.lines.last_mut() {
                        last.spans.push(Span::styled(
                            "  (Ctrl+O to expand)",
                            Style::default().fg(palette.muted),
                        ));
                    }
                }
            }
            Entry::Info(text) => body.push(Line::from(Span::styled(
                text.clone(),
                Style::default().fg(palette.muted),
            ))),
            Entry::Error(text) => body.push(Line::from(Span::styled(
                format!("! {text}"),
                Style::default().fg(palette.error),
            ))),
        }
    }
    (body.lines, body.targets)
}

#[derive(Default)]
struct TranscriptBody {
    lines: Vec<Line<'static>>,
    targets: Vec<Option<HitTarget>>,
}

impl TranscriptBody {
    fn push(&mut self, line: Line<'static>) {
        self.lines.push(line);
        self.targets.push(None);
    }

    fn push_target(&mut self, line: Line<'static>, target: HitTarget) {
        self.lines.push(line);
        self.targets.push(Some(target));
    }

    fn extend(&mut self, lines: impl IntoIterator<Item = Line<'static>>) {
        for line in lines {
            self.push(line);
        }
    }
}

fn turn_summary(transcript: &crate::transcript::Transcript, start: usize) -> String {
    let mut tools = 0_usize;
    let mut edits = 0_usize;
    let mut errors = 0_usize;
    let mut replies = 0_usize;
    for (index, entry) in transcript.entries().iter().enumerate().skip(start + 1) {
        if transcript.is_turn_start(index) {
            break;
        }
        match entry {
            Entry::User(_) => {}
            Entry::Tool(block) => {
                tools += 1;
                if matches!(block.name.as_str(), "edit" | "write") {
                    edits += 1;
                }
                if block.status == ToolStatus::Error {
                    errors += 1;
                }
            }
            Entry::Assistant(assistant) if !assistant.text.is_empty() => replies += 1,
            Entry::Error(_) => errors += 1,
            _ => {}
        }
    }
    let mut parts = Vec::new();
    if tools > 0 {
        parts.push(format!("{tools} tool{}", plural(tools)));
    }
    if edits > 0 {
        parts.push(format!("{edits} edit{}", plural(edits)));
    }
    if errors > 0 {
        parts.push(format!("{errors} error{}", plural(errors)));
    }
    if parts.is_empty() {
        if replies > 0 {
            "reply hidden".to_owned()
        } else {
            "no output".to_owned()
        }
    } else {
        parts.join(" · ")
    }
}

fn plural(count: usize) -> &'static str {
    if count == 1 { "" } else { "s" }
}

fn tool_path(block: &crate::transcript::ToolBlock) -> &str {
    block
        .arguments
        .get("path")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
}

fn tool_header(
    block: &crate::transcript::ToolBlock,
    tick: u64,
    palette: &Palette,
) -> Vec<Span<'static>> {
    let path = tool_path(block);
    let rich = matches!(block.name.as_str(), "edit" | "write");
    let (marker, color) = match block.status {
        ToolStatus::Running => (SPINNER[tick as usize % SPINNER.len()], palette.primary),
        ToolStatus::Success => {
            if block.name == "edit" {
                ("→", palette.tool)
            } else if block.name == "write" {
                ("#", palette.tool)
            } else {
                ("✓", palette.success)
            }
        }
        ToolStatus::Error => ("✗", palette.error),
    };
    let label = if rich {
        match block.name.as_str() {
            "edit" => block
                .render
                .as_ref()
                .and_then(|render| render.title.clone())
                .unwrap_or_else(|| format!("Edit {path}")),
            "write" if block.status == ToolStatus::Success => format!("Wrote {path}"),
            "write" => format!("Write {path}"),
            _ => block.summary(),
        }
    } else {
        block.summary()
    };
    let mut spans = vec![
        Span::styled(
            format!("{marker} "),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
        Span::styled(label, Style::default().fg(palette.tool)),
    ];
    if block.status == ToolStatus::Running
        && let Some(elapsed) = block.elapsed_label()
    {
        spans.push(Span::styled(
            format!("  {elapsed}"),
            Style::default().fg(palette.muted),
        ));
    }
    spans
}

fn rich_tool_body(
    app: &App,
    block: &crate::transcript::ToolBlock,
    width: u16,
    palette: &Palette,
    expanded: bool,
) -> Option<(Vec<Line<'static>>, bool)> {
    if block.status != ToolStatus::Success {
        return None;
    }
    let key = ToolViewKey {
        id: block.id.clone(),
        width,
        expanded,
        theme_revision: app.theme_revision,
    };
    if let Some(cached) = app.tool_views.borrow().get(&key) {
        return Some((cached.lines.clone(), cached.truncated));
    }

    let limit = if expanded {
        usize::MAX
    } else {
        tool_view::PREVIEW_ROWS
    };
    let rendered = match block.name.as_str() {
        "edit" => {
            let render = block.render.as_ref()?;
            if !matches!(render.kind, codie_protocol::ext::ExtRenderKind::Diff) {
                return None;
            }
            Some(tool_view::render_diff(
                &render.text,
                tool_path(block),
                usize::from(width),
                palette,
                limit,
            ))
        }
        "write" => {
            let content = block.arguments.get("content")?.as_str()?;
            Some(tool_view::render_code(
                content,
                tool_path(block),
                usize::from(width),
                palette,
                limit,
            ))
        }
        _ => None,
    };
    if let Some((lines, truncated)) = &rendered {
        app.tool_views.borrow_mut().insert(
            key,
            CachedToolView {
                lines: lines.clone(),
                truncated: *truncated,
            },
        );
    }
    rendered
}

fn draw_editor(frame: &mut Frame<'_>, area: Rect, app: &mut App) {
    let palette = app.theme.palette;
    let (title, border) = match app.status {
        Status::Idle => (
            if app.transcript.is_following() {
                " prompt ".to_owned()
            } else {
                " prompt · scrolled (Esc to bottom) ".to_owned()
            },
            palette.border_active,
        ),
        Status::Streaming => {
            let frame = SPINNER[app.tick as usize % SPINNER.len()];
            let title = match app.transcript.running_tool() {
                Some(tool) => {
                    let elapsed = tool
                        .elapsed_label()
                        .map_or_else(String::new, |elapsed| format!(" {elapsed}"));
                    format!(" {frame} {} {elapsed} · Esc to stop ", tool.name)
                }
                None if app.transcript.thinking_in_progress() => {
                    format!(" {frame} thinking · Esc to stop ")
                }
                None => format!(" {frame} streaming · Esc to stop "),
            };
            (title, palette.primary)
        }
        Status::Waiting => (" waiting ".to_owned(), palette.warning),
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
    frame.render_widget(
        Paragraph::new(lines).style(
            Style::default()
                .fg(palette.foreground)
                .bg(palette.background),
        ),
        inner,
    );

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

fn draw_footer(frame: &mut Frame<'_>, area: Rect, app: &mut App) {
    let palette = app.theme.palette;
    let columns =
        Layout::horizontal([Constraint::Percentage(38), Constraint::Percentage(62)]).split(area);

    let mut left = app.footer.cwd.clone();
    if let Some(branch) = &app.footer.git_branch {
        left.push_str(&format!(" ({branch})"));
    }
    let left = truncate(&left, usize::from(columns[0].width));
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            left,
            Style::default().fg(palette.muted),
        ))),
        columns[0],
    );

    let mut parts: Vec<(String, Style)> = Vec::new();
    if let Some(percent) = app.footer.context_percent {
        parts.push((
            format!("ctx {} {percent}%", context_meter(percent)),
            Style::default().fg(context_color(percent, app.context_window, &palette)),
        ));
    }
    parts.push((
        app.footer.model.clone(),
        Style::default().fg(palette.secondary),
    ));
    parts.push((
        app.footer.thinking.clone(),
        Style::default().fg(palette.muted),
    ));
    if app.auto_approve {
        parts.push((
            "FULL ACCESS".to_owned(),
            Style::default().fg(palette.warning),
        ));
    }
    if app.footer.tokens > 0 {
        parts.push((
            format!("{} tok", format_tokens(app.footer.tokens)),
            Style::default().fg(palette.secondary),
        ));
        parts.push((
            format!("${:.4}", app.footer.cost as f64 / 1_000_000_000.0),
            Style::default().fg(palette.secondary),
        ));
    }
    if let Some(rate) = app.footer.tokens_per_sec {
        parts.push((format!("{rate:.0} t/s"), Style::default().fg(palette.muted)));
    }
    if let Some(session) = &app.footer.session {
        parts.push((
            session[..session.len().min(8)].to_owned(),
            Style::default().fg(palette.muted),
        ));
    }

    let available = usize::from(columns[1].width);
    let mut spans = Vec::new();
    let mut used = 0_usize;
    for (text, style) in parts {
        let separator = usize::from(!spans.is_empty()) * 3;
        let width = UnicodeWidthStr::width(text.as_str());
        if used + separator + width > available {
            continue;
        }
        if separator > 0 {
            spans.push(Span::styled(" · ", Style::default().fg(palette.muted)));
            used += separator;
        }
        spans.push(Span::styled(text, style));
        used += width;
    }
    if spans.is_empty() {
        spans.push(Span::styled(
            truncate(&app.footer.model, available),
            Style::default().fg(palette.secondary),
        ));
    }
    frame.render_widget(
        Paragraph::new(Line::from(spans)).alignment(Alignment::Right),
        columns[1],
    );
    app.hits.push(Hit {
        row: area.y,
        target: crate::app::HitTarget::Usage,
    });
}

fn context_meter(percent: u8) -> String {
    const CELLS: usize = 8;
    let filled = (usize::from(percent.min(100)) * CELLS / 100).min(CELLS);
    format!("{}{}", "█".repeat(filled), "░".repeat(CELLS - filled))
}

fn context_color(percent: u8, context_window: u32, palette: &Palette) -> ratatui::style::Color {
    let window = u64::from(context_window.max(1));
    let threshold = u8::try_from(
        window
            .saturating_sub(crate::compaction::DEFAULT_RESERVE_TOKENS)
            .saturating_mul(100)
            / window,
    )
    .unwrap_or(100);
    if percent >= threshold {
        palette.error
    } else if percent >= threshold.saturating_sub(10).min(70) {
        palette.warning
    } else {
        palette.success
    }
}

fn format_count(value: u64) -> String {
    let digits = value.to_string();
    let mut output = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, character) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            output.push(',');
        }
        output.push(character);
    }
    output
}

fn usage_body(app: &App) -> String {
    let window = u64::from(app.context_window.max(1));
    let tokens = app.footer.context_tokens.unwrap_or_default();
    let percent = app.footer.context_percent.unwrap_or_default();
    let reserve = crate::compaction::DEFAULT_RESERVE_TOKENS;
    let threshold = window.saturating_sub(reserve);
    let threshold_percent = u8::try_from(threshold.saturating_mul(100) / window).unwrap_or(100);
    let mut body = format!(
        "Context\n  estimated   {} / {} tok ({}%)\n  budget      {} tok ({}%)\n  reserve     {} tok\n  {}\n\nSession\n  input       {} tok\n  output      {} tok\n  cache read  {} tok\n  cache write {} tok\n  total       {} tok\n  cost        ${:.4}",
        format_count(tokens),
        format_count(window),
        percent,
        format_count(threshold),
        threshold_percent,
        format_count(reserve),
        context_meter(percent),
        format_count(app.footer.input_tokens),
        format_count(app.footer.output_tokens),
        format_count(app.footer.cache_read_tokens),
        format_count(app.footer.cache_write_tokens),
        format_count(app.footer.tokens),
        app.footer.cost as f64 / 1_000_000_000.0,
    );
    if let Some(rate) = app.footer.tokens_per_sec {
        body.push_str(&format!("\n  throughput  {rate:.0} tok/s"));
    }
    if let Some(session) = &app.footer.session {
        body.push_str(&format!("\n  session     {session}"));
    }
    body.push_str("\n\n[ Enter ] close");
    body
}

/// Abbreviates large counts for the footer: `999`, `1k`, `70.2k`, `1.2M`.
fn format_tokens(tokens: u64) -> String {
    const KILO: f64 = 1_000.0;
    const MEGA: f64 = 1_000_000.0;
    const GIGA: f64 = 1_000_000_000.0;

    let scaled = tokens as f64;
    let (value, suffix) = if tokens < 1_000 {
        return tokens.to_string();
    } else if scaled < 999_950.0 {
        (scaled / KILO, "k")
    } else if scaled < 999_950_000.0 {
        (scaled / MEGA, "M")
    } else {
        (scaled / GIGA, "B")
    };

    let rounded = (value * 10.0).round() / 10.0;
    if rounded.fract() == 0.0 {
        format!("{rounded:.0}{suffix}")
    } else {
        format!("{rounded:.1}{suffix}")
    }
}

fn draw_picker_overlay(frame: &mut Frame<'_>, area: Rect, picker: &crate::app::Picker, app: &App) {
    let palette = app.theme.palette;
    let width = area.width.saturating_mul(9) / 10;
    let height = area.height.saturating_mul(7) / 10;
    let rect = centered(area, width.max(50), height.max(14));
    frame.render_widget(Clear, rect);
    let block = Block::bordered()
        .title(" sessions ")
        .border_style(Style::default().fg(palette.primary));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    let columns =
        Layout::horizontal([Constraint::Percentage(46), Constraint::Percentage(54)]).split(inner);

    let visible_height = usize::from(columns[0].height).saturating_sub(4).max(1);
    let offset = if picker.selected >= visible_height {
        picker.selected + 1 - visible_height
    } else {
        0
    };
    let mut list = vec![Line::from(Span::styled(
        format!(
            "filter: {}   sort: {}",
            if picker.query.is_empty() {
                "—"
            } else {
                picker.query.as_str()
            },
            if picker.sort_by_name {
                "name"
            } else {
                "recent"
            }
        ),
        Style::default().fg(palette.muted),
    ))];
    if let Some(buffer) = picker.rename.as_ref() {
        list.push(Line::from(Span::styled(
            format!("name: {buffer}▌   (Enter save, Esc cancel)"),
            Style::default().fg(palette.warning),
        )));
    } else {
        list.push(Line::default());
    }
    if picker.visible.is_empty() {
        list.push(Line::from(Span::styled(
            "No sessions match.",
            Style::default().fg(palette.muted),
        )));
    }
    for (position, index) in picker
        .visible
        .iter()
        .enumerate()
        .skip(offset)
        .take(visible_height)
    {
        let Some(summary) = picker.entries.get(*index) else {
            continue;
        };
        let selected = position == picker.selected;
        let marker = if selected { "› " } else { "  " };
        let name = summary.name.as_deref().unwrap_or("unnamed");
        let style = if selected {
            Style::default()
                .fg(palette.background)
                .bg(palette.primary)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(palette.foreground)
        };
        list.push(Line::from(Span::styled(
            format!(
                "{marker}{:<22} {:>4} msg",
                truncate(name, 22),
                summary.message_count
            ),
            style,
        )));
    }
    if picker.confirm_delete {
        list.push(Line::default());
        list.push(Line::from(Span::styled(
            "Delete this session? [y] yes   [n] no",
            Style::default().fg(palette.error),
        )));
    }
    frame.render_widget(
        Paragraph::new(list).wrap(Wrap { trim: false }).style(
            Style::default()
                .fg(palette.foreground)
                .bg(palette.background),
        ),
        columns[0],
    );

    let mut preview = Vec::new();
    if let Some(summary) = picker.current() {
        let name = summary.name.as_deref().unwrap_or("unnamed");
        preview.push(Line::from(Span::styled(
            truncate(name, usize::from(columns[1].width)),
            Style::default()
                .fg(palette.heading)
                .add_modifier(Modifier::BOLD),
        )));
        preview.push(Line::from(Span::styled(
            format!(
                "{} · {} messages · {} ctx tok · ${:.4}",
                summary.id,
                summary.message_count,
                format_tokens(summary.tokens),
                summary.cost as f64 / 1_000_000_000.0
            ),
            Style::default().fg(palette.muted),
        )));
        if let Some(model) = &summary.model {
            preview.push(Line::from(Span::styled(
                format!("model: {model}"),
                Style::default().fg(palette.secondary),
            )));
        }
        preview.push(Line::from(Span::styled(
            format!(
                "cwd: {}",
                truncate(&summary.cwd, usize::from(columns[1].width))
            ),
            Style::default().fg(palette.muted),
        )));
        preview.push(Line::from(Span::styled(
            format!(
                "created {} · updated {}",
                truncate(&summary.created_at, 20),
                format_age(summary.updated_at)
            ),
            Style::default().fg(palette.muted),
        )));
        preview.push(Line::default());
        preview.push(Line::from(Span::styled(
            "Last prompt",
            Style::default().fg(palette.heading),
        )));
        for line in summary.last_user.as_deref().unwrap_or("—").lines().take(6) {
            preview.push(Line::from(Span::styled(
                format!("  {line}"),
                Style::default().fg(palette.foreground),
            )));
        }
        preview.push(Line::default());
        preview.push(Line::from(Span::styled(
            "Last reply",
            Style::default().fg(palette.heading),
        )));
        for line in summary
            .last_assistant
            .as_deref()
            .unwrap_or("—")
            .lines()
            .take(10)
        {
            preview.push(Line::from(Span::styled(
                format!("  {line}"),
                Style::default().fg(palette.foreground),
            )));
        }
    } else {
        preview.push(Line::from(Span::styled(
            "No session selected.",
            Style::default().fg(palette.muted),
        )));
    }
    frame.render_widget(
        Paragraph::new(preview).wrap(Wrap { trim: false }).style(
            Style::default()
                .fg(palette.foreground)
                .bg(palette.background),
        ),
        columns[1],
    );
}

fn format_age(updated: std::time::SystemTime) -> String {
    let Ok(age) = std::time::SystemTime::now().duration_since(updated) else {
        return "now".to_owned();
    };
    let seconds = age.as_secs();
    if seconds < 60 {
        format!("{seconds}s ago")
    } else if seconds < 3_600 {
        format!("{}m ago", seconds / 60)
    } else if seconds < 86_400 {
        format!("{}h ago", seconds / 3_600)
    } else {
        format!("{}d ago", seconds / 86_400)
    }
}

fn draw_permission_overlay(
    frame: &mut Frame<'_>,
    area: Rect,
    prompt: &crate::app::PermissionPrompt,
    app: &App,
) {
    let palette = app.theme.palette;
    let width = area.width.saturating_mul(4) / 5;
    let height = area.height.saturating_mul(3) / 5;
    let rect = centered(area, width.max(40), height.max(12));
    frame.render_widget(Clear, rect);
    let block = Block::bordered()
        .title(" permission ")
        .border_style(Style::default().fg(palette.warning));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);

    let path = prompt
        .arguments
        .get("path")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let mut body = vec![
        Line::from(vec![
            Span::styled("Allow ", Style::default().fg(palette.foreground)),
            Span::styled(
                format!("`{}`", prompt.tool),
                Style::default()
                    .fg(palette.warning)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("?", Style::default().fg(palette.foreground)),
        ]),
        Line::default(),
    ];

    if let Some(preview) = &prompt.preview {
        if let Some(title) = &preview.title {
            body.push(Line::from(Span::styled(
                title.clone(),
                Style::default()
                    .fg(palette.heading)
                    .add_modifier(Modifier::BOLD),
            )));
        }
        match preview.kind {
            codie_protocol::ext::ExtRenderKind::Diff => {
                let (lines, truncated) = tool_view::render_diff(
                    &preview.text,
                    path,
                    usize::from(inner.width),
                    &palette,
                    48,
                );
                body.extend(lines);
                if truncated {
                    body.push(Line::from(Span::styled(
                        "  … diff truncated",
                        Style::default().fg(palette.muted),
                    )));
                }
            }
            codie_protocol::ext::ExtRenderKind::Code => {
                let language_path = if prompt.tool == "bash" {
                    "command.sh"
                } else {
                    path
                };
                let (lines, truncated) = tool_view::render_code(
                    &preview.text,
                    language_path,
                    usize::from(inner.width),
                    &palette,
                    40,
                );
                body.extend(lines);
                if truncated {
                    body.push(Line::from(Span::styled(
                        "  … preview truncated",
                        Style::default().fg(palette.muted),
                    )));
                }
            }
            _ => {}
        }
    } else {
        let json = serde_json::to_string_pretty(&prompt.arguments).unwrap_or_default();
        for line in json.lines().take(40) {
            body.push(Line::from(Span::styled(
                format!("  {line}"),
                Style::default().fg(palette.foreground),
            )));
        }
    }

    let maximum = usize::from(inner.height).max(1);
    let preview_limit = maximum.saturating_sub(3).max(1);
    if body.len() > preview_limit {
        body.truncate(preview_limit.saturating_sub(1));
        body.push(Line::from(Span::styled(
            "…",
            Style::default().fg(palette.muted),
        )));
    }
    body.push(Line::default());
    body.push(Line::from(Span::styled(
        "[y] allow once   [A] always allow   [n] deny   [Enter] allow   [Esc] deny   [Shift+Tab] full access",
        Style::default().fg(palette.muted),
    )));
    frame.render_widget(
        Paragraph::new(body).wrap(Wrap { trim: false }).style(
            Style::default()
                .fg(palette.foreground)
                .bg(palette.background),
        ),
        inner,
    );
}

fn draw_overlay(frame: &mut Frame<'_>, area: Rect, overlay: &Overlay, app: &App) {
    if let Overlay::Permission(prompt) = overlay {
        draw_permission_overlay(frame, area, prompt, app);
        return;
    }
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
                "/tree              show the session tree",
                "/fork [entry]      branch at an entry",
                "/clone             clone this session",
                "/name [text]       set or clear the name",
                "/export [path]     export JSONL",
                "/export html [p]   export a self-contained HTML transcript",
                "/copy              copy the last assistant message",
                "/debug             print diagnostics",
                "/update            install a newer release",
                "/theme [name]      choose a theme",
                "/model [id]        change the model",
                "/thinking [level]  change reasoning effort",
                "/compact [note]    compact the context",
                "/usage             tokens, cost, and context",
                "/stop              stop the active run",
                "/skill:<name>      invoke a skill",
                "/template:<name>   expand a prompt template",
                "/reload            reload skills and templates",
                "/quit              exit",
            ]
            .join("\n"),
        ),
        Overlay::Hotkeys => (" keys ", app.bindings.describe()),
        Overlay::Permission(prompt) => (
            " permission ",
            format!(
                "Allow `{}`?\n\n{}\n\n[y] yes   [n] no   [A] always allow   [Shift+Tab] full access (no prompts)",
                prompt.tool,
                serde_json::to_string_pretty(&prompt.arguments).unwrap_or_default()
            ),
        ),
        Overlay::Tree(entries) => (
            " session tree ",
            entries
                .iter()
                .map(|entry| {
                    let indent = "  ".repeat(entry.depth.min(8));
                    let marker = if entry.on_branch { "●" } else { "○" };
                    format!(
                        "{indent}{marker} {} {:<16} {}",
                        entry.id, entry.kind, entry.description
                    )
                })
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        Overlay::List(_) => (" list ", String::new()),
        Overlay::Usage => (" usage ", usage_body(app)),
        Overlay::Notice { title, body, ok } => (
            title.as_str(),
            if *ok {
                format!("{body}\n\n[ Enter ] OK")
            } else {
                body.clone()
            },
        ),
        Overlay::Picker(picker) => {
            let mut lines = vec![format!(
                "search: {}   sort: {}   (↑/↓ select, Enter resume, r rename, d delete, s sort, Esc close)",
                if picker.query.is_empty() {
                    "—"
                } else {
                    picker.query.as_str()
                },
                if picker.sort_by_name {
                    "name"
                } else {
                    "recent"
                }
            )];
            if let Some(buffer) = picker.rename.as_ref() {
                lines.push(format!("name: {buffer}▌   (Enter save, Esc cancel)"));
            }
            lines.push(String::new());
            if picker.visible.is_empty() {
                lines.push("No sessions match.".to_owned());
            }
            for (position, index) in picker.visible.iter().enumerate() {
                let summary = &picker.entries[*index];
                let selected = if position == picker.selected {
                    "›"
                } else {
                    " "
                };
                let name = summary.name.as_deref().unwrap_or("unnamed");
                lines.push(format!(
                    "{selected} {:<20} {:>4} msg  {}  {}",
                    truncate(name, 20),
                    summary.message_count,
                    summary.id,
                    summary.cwd
                ));
            }
            if picker.confirm_delete {
                lines.push(String::new());
                lines.push("Delete this session? [y] yes   [n] no".to_owned());
            }
            (" sessions ", lines.join("\n"))
        }
    };

    let block = Block::bordered()
        .title(title)
        .border_style(Style::default().fg(palette.primary));
    frame.render_widget(
        Paragraph::new(body)
            .block(block)
            .wrap(Wrap { trim: false })
            .style(
                Style::default()
                    .fg(palette.foreground)
                    .bg(palette.background),
            ),
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
            128_000,
            Vec::new(),
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
        assert!(text.contains("██████╗"), "banner missing: {text}");
        assert!(text.contains("Type a prompt"));
        assert!(text.contains("deepseek-flash"));
        assert!(text.contains("/tmp/project"));
    }

    #[test]
    fn empty_state_banner_uses_primary() {
        let mut app = app();
        let palette = app.theme.palette;
        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &mut app)).unwrap();
        let buffer = terminal.backend().buffer();
        assert_eq!(buffer[(1, 1)].fg, palette.primary);
    }

    #[test]
    fn editor_text_uses_the_theme_foreground() {
        let mut app = app();
        app.editor.insert_str("hello");
        let palette = app.theme.palette;
        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &mut app)).unwrap();
        let buffer = terminal.backend().buffer();
        assert_eq!(buffer[(1, 17)].fg, palette.foreground);
    }

    #[test]
    fn user_messages_use_a_raised_surface() {
        let mut app = app();
        app.transcript.push_user("hello");
        let palette = app.theme.palette;
        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &mut app)).unwrap();
        let buffer = terminal.backend().buffer();
        assert_eq!(buffer[(0, 1)].fg, palette.primary);
        assert_eq!(buffer[(2, 1)].bg, palette.surface);
    }

    #[test]
    fn narrow_empty_state_falls_back_to_text_banner() {
        let mut app = app();
        let backend = TestBackend::new(30, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &mut app)).unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("Codie!"), "{text}");
    }

    #[test]
    fn token_counts_use_compact_suffixes() {
        assert_eq!(format_tokens(0), "0");
        assert_eq!(format_tokens(999), "999");
        assert_eq!(format_tokens(1_000), "1k");
        assert_eq!(format_tokens(1_234), "1.2k");
        assert_eq!(format_tokens(70_218), "70.2k");
        assert_eq!(format_tokens(999_949), "999.9k");
        assert_eq!(format_tokens(999_999), "1M");
        assert_eq!(format_tokens(1_200_000), "1.2M");
        assert_eq!(format_tokens(1_500_000_000), "1.5B");
    }

    #[test]
    #[spec("TUI-006")]
    fn spec_tui_006_footer_abbreviates_tokens() {
        let mut app = app();
        app.footer.tokens = 70_218;
        app.footer.cost = 8_000_000;
        app.footer.tokens_per_sec = Some(136.0);
        app.footer.context_percent = Some(0);
        app.footer.session = Some("0190f3a1".into());

        let backend = TestBackend::new(160, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &mut app)).unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();

        assert!(text.contains("70.2k tok"), "footer tokens: {text}");
        assert!(text.contains("$0.0080"), "footer cost: {text}");
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
    #[spec("TUI-021")]
    fn scrolled_transcript_shows_a_return_hint() {
        let mut app = app();
        for index in 0..30 {
            app.transcript.push_user(format!("message {index}"));
        }
        app.transcript.clamp_scroll(40);
        app.transcript.scroll_up(100);
        let text = buffer(&mut app);
        assert!(text.contains("scrolled"), "{text}");
        assert!(text.contains("Esc to bottom"), "{text}");
    }

    #[test]
    #[spec("TUI-020")]
    fn running_tool_replaces_the_streaming_title() {
        let mut app = app();
        app.status = crate::app::Status::Streaming;
        app.transcript.tool_start(
            "call_1".into(),
            "bash".into(),
            serde_json::json!({"command": "bun run dev"}),
        );
        let text = buffer(&mut app);
        assert!(text.contains("bun run dev"), "{text}");
        assert!(
            text.contains("bash"),
            "the running tool should title the editor"
        );
    }

    #[test]
    #[spec("TUI-005")]
    fn edit_results_render_a_side_by_side_diff() {
        let mut app = app();
        app.transcript.tool_start(
            "call_1".into(),
            "edit".into(),
            serde_json::json!({"path": "src/lib.rs"}),
        );
        app.transcript.tool_end(
            "call_1",
            "Successfully replaced 1 block(s).".into(),
            false,
            Some(codie_protocol::ext::ExtRenderHint {
                kind: codie_protocol::ext::ExtRenderKind::Diff,
                title: Some("Edit src/lib.rs".into()),
                text: "--- src/lib.rs\n+++ src/lib.rs\n@@ -1,3 +1,3 @@\n fn main() {\n-    let old = 1;\n+    let new = 2;\n }\n"
                    .into(),
            }),
        );
        let text = buffer(&mut app);
        assert!(text.contains("→ Edit src/lib.rs"), "{text}");
        assert!(text.contains("let old = 1;"), "{text}");
        assert!(text.contains("let new = 2;"), "{text}");
        assert!(text.contains("1"), "{text}");
    }

    #[test]
    #[spec("TUI-005")]
    fn write_results_render_numbered_code() {
        let mut app = app();
        app.transcript.tool_start(
            "call_1".into(),
            "write".into(),
            serde_json::json!({
                "path": "src/lib.rs",
                "content": "fn main() {}\nlet value = 1;\n"
            }),
        );
        app.transcript.tool_end(
            "call_1",
            "Successfully wrote to src/lib.rs".into(),
            false,
            None,
        );
        let text = buffer(&mut app);
        assert!(text.contains("# Wrote src/lib.rs"), "{text}");
        assert!(text.contains("1 fn main"), "{text}");
        assert!(text.contains("2 let value"), "{text}");
    }

    #[test]
    #[spec("TUI-022")]
    fn thinking_starts_collapsed_and_toggles_with_a_click() {
        let mut app = app();
        app.transcript.start_assistant();
        app.transcript.set_thinking("secret reasoning".into());

        let collapsed = buffer(&mut app);
        assert!(collapsed.contains("Thinking"), "{collapsed}");
        assert!(collapsed.contains("click to view"), "{collapsed}");
        assert!(!collapsed.contains("secret reasoning"), "{collapsed}");

        let hit = app.hits.first().copied().expect("thinking hit row");
        assert!(app.handle_click(0, hit.row));

        let expanded = buffer(&mut app);
        assert!(expanded.contains("secret reasoning"), "{expanded}");
        assert!(expanded.contains("Esc to hide"), "{expanded}");

        app.handle_key(ratatui::crossterm::event::KeyEvent::new(
            ratatui::crossterm::event::KeyCode::Esc,
            ratatui::crossterm::event::KeyModifiers::NONE,
        ));
        let collapsed = buffer(&mut app);
        assert!(!collapsed.contains("secret reasoning"), "{collapsed}");
        assert!(collapsed.contains("click to view"), "{collapsed}");
    }

    #[test]
    #[spec("TUI-024")]
    fn footer_shows_a_context_meter_and_usage_overlay() {
        let mut app = app();
        app.footer.context_percent = Some(81);
        app.footer.context_tokens = Some(104_000);
        app.footer.input_tokens = 900_000;
        app.footer.output_tokens = 120_000;
        app.footer.cache_read_tokens = 700_000;
        app.footer.cache_write_tokens = 80_000;
        let text = buffer(&mut app);
        assert!(text.contains("ctx"), "{text}");
        assert!(text.contains("81%"), "{text}");

        app.overlay = Some(Overlay::Usage);
        let text = buffer(&mut app);
        assert!(text.contains("Context"), "{text}");
        assert!(text.contains("104,000"), "{text}");
        assert!(text.contains("cache read"), "{text}");
        assert!(text.contains("900,000"), "{text}");
    }

    #[test]
    #[spec("TUI-025")]
    fn permission_overlay_renders_an_edit_diff() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("file.txt"), "alpha\nbeta\ngamma\n").unwrap();
        let mut app = App::new(
            directory.path(),
            codie_core::ModelRef {
                api: "openai-completions".into(),
                provider: "deepseek".into(),
                model: "deepseek-flash".into(),
            },
            codie_core::ThinkingLevel::Medium,
            128_000,
            Vec::new(),
            crate::theme::Theme::load(Some("dark")).unwrap(),
            None,
            Vec::new(),
            directory.path().join("sessions"),
            std::sync::Arc::new(std::sync::atomic::AtomicI32::new(0)),
            crate::keybindings::Bindings::default(),
        );
        let (reply, _answer) = tokio::sync::oneshot::channel();
        app.push_permission(crate::permissions::PermissionRequest {
            tool: "edit".to_owned(),
            arguments: serde_json::json!({
                "path": "file.txt",
                "edits": [{"oldText": "beta", "newText": "BETA"}]
            }),
            reply,
        });
        let text = buffer(&mut app);
        assert!(text.contains("Edit file.txt"), "{text}");
        assert!(text.contains("beta"), "{text}");
        assert!(text.contains("BETA"), "{text}");
    }

    #[test]
    #[spec("TUI-026")]
    fn tool_headers_toggle_on_click_and_all_at_once() {
        let mut app = app();
        app.transcript.tool_start(
            "call_1".into(),
            "bash".into(),
            serde_json::json!({"command": "cargo test"}),
        );
        app.transcript.tool_end("call_1", "ok".into(), false, None);
        let _ = buffer(&mut app);
        let row = app
            .hits
            .iter()
            .find_map(|hit| matches!(hit.target, HitTarget::Tool(0)).then_some(hit.row))
            .expect("tool hit row");
        assert!(app.handle_click(0, row));
        assert!(matches!(
            &app.transcript.entries()[0],
            Entry::Tool(block) if block.expanded
        ));
        assert!(app.transcript.toggle_all_tools());
        assert!(matches!(
            &app.transcript.entries()[0],
            Entry::Tool(block) if !block.expanded
        ));
    }

    #[test]
    #[spec("TUI-027")]
    fn older_turns_collapse_and_expand_on_click() {
        let mut app = app();
        app.transcript.push_user("first question");
        app.transcript.start_assistant();
        app.transcript.set_text("first answer".into());
        app.transcript.finish_assistant();
        app.transcript.push_user("second question");

        let collapsed = buffer(&mut app);
        assert!(collapsed.contains("▸ first question"), "{collapsed}");
        assert!(collapsed.contains("reply hidden"), "{collapsed}");
        assert!(!collapsed.contains("first answer"), "{collapsed}");

        let row = app
            .hits
            .iter()
            .find_map(|hit| matches!(hit.target, HitTarget::Turn(0)).then_some(hit.row))
            .expect("turn hit row");
        assert!(app.handle_click(0, row));
        let expanded = buffer(&mut app);
        assert!(expanded.contains("first answer"), "{expanded}");
    }

    #[test]
    #[spec("TUI-028")]
    fn session_picker_shows_a_preview() {
        let directory = tempfile::tempdir().unwrap();
        let cwd = directory.path().join("project");
        std::fs::create_dir_all(&cwd).unwrap();
        let sessions = directory.path().join("sessions");
        let mut session = codie_session::Session::create(&cwd, &sessions).unwrap();
        session
            .append_message(&codie_core::Message::user("hello prompt"))
            .unwrap();
        let mut assistant = codie_core::AssistantMessage::partial(
            "openai-completions",
            "deepseek",
            "deepseek-flash",
        );
        assistant.content = vec![codie_core::Content::text("hello answer")];
        assistant.stop_reason = codie_core::StopReason::Stop;
        session
            .append_message(&codie_core::Message::Assistant(assistant))
            .unwrap();
        drop(session);
        let entries = codie_session::Session::list(&sessions).unwrap();
        let mut app = App::new(
            &cwd,
            codie_core::ModelRef {
                api: "openai-completions".into(),
                provider: "deepseek".into(),
                model: "deepseek-flash".into(),
            },
            codie_core::ThinkingLevel::Medium,
            128_000,
            Vec::new(),
            crate::theme::Theme::load(Some("dark")).unwrap(),
            None,
            Vec::new(),
            sessions,
            std::sync::Arc::new(std::sync::atomic::AtomicI32::new(0)),
            crate::keybindings::Bindings::default(),
        );
        app.overlay = Some(Overlay::Picker(crate::app::Picker::new(entries)));

        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &mut app)).unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("Last prompt"), "{text}");
        assert!(text.contains("hello prompt"), "{text}");
        assert!(text.contains("Last reply"), "{text}");
        assert!(text.contains("hello answer"), "{text}");
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
        assert!(text.contains("[y] allow once"));
        assert!(text.contains("[n] deny"));
        assert!(text.contains("full access"));
        assert!(text.contains("cargo test"));
    }
}
