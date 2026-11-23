//! Ratatui rendering for the transcript, editor, footer, and overlays.

use std::collections::{BTreeSet, HashSet};

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Padding, Paragraph, Wrap};
use unicode_width::UnicodeWidthStr;

use codie_protocol::ext::ExtRenderKind;

use crate::app::{
    App, CachedEntry, CachedEntryKey, CachedToolView, CachedTranscript, EntryLayout, Hit,
    HitTarget, Overlay, RowIndex, Status, ToolViewKey,
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

const MAX_LIST_ITEMS: usize = 16;
const MAX_LIST_WIDTH: u16 = 88;
const NARROW_FOOTER_WIDTH: u16 = 80;
const DETAILED_FOOTER_WIDTH: u16 = 120;

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
    let footer_height = if area.width < NARROW_FOOTER_WIDTH {
        2
    } else {
        1
    };
    let chunks = Layout::vertical([
        Constraint::Min(3),
        Constraint::Length(editor_height),
        Constraint::Length(footer_height),
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

    if let Some(selection) = app.selection {
        highlight_selection(frame, selection);
    }
    if std::mem::take(&mut app.copy_pending)
        && let Some(selection) = app.selection
    {
        let text = selection_text(frame.buffer_mut(), selection);
        if !text.trim().is_empty() {
            let lines = text.lines().count();
            app.notice = Some(match crate::copy_to_clipboard(&text) {
                Ok(()) => format!("copied {lines} line(s)"),
                Err(error) => format!("clipboard unavailable: {error}"),
            });
            app.needs_redraw = true;
        }
    }
}

/// Marks every selected cell so the highlight is visible.
fn highlight_selection(frame: &mut Frame<'_>, selection: crate::app::Selection) {
    let ((top, left), (bottom, right)) = selection.bounds();
    let area = frame.area();
    let buffer = frame.buffer_mut();
    for row in top..=bottom {
        if !(area.top()..area.bottom()).contains(&row) {
            continue;
        }
        let from = if row == top { left } else { area.left() };
        let to = if row == bottom {
            right
        } else {
            area.right().saturating_sub(1)
        };
        for column in from.max(area.left())..=to.min(area.right().saturating_sub(1)) {
            if let Some(cell) = buffer.cell_mut((column, row)) {
                cell.modifier |= Modifier::REVERSED;
            }
        }
    }
}

/// Reads the selected cells as plain text, trimming row padding.
fn selection_text(buffer: &ratatui::buffer::Buffer, selection: crate::app::Selection) -> String {
    let ((top, left), (bottom, right)) = selection.bounds();
    let area = buffer.area;
    let mut rows = Vec::new();
    for row in top..=bottom {
        if !(area.top()..area.bottom()).contains(&row) {
            continue;
        }
        let from = if row == top { left } else { area.left() };
        let to = if row == bottom {
            right
        } else {
            area.right().saturating_sub(1)
        };
        let mut line = String::new();
        for column in from.max(area.left())..=to.min(area.right().saturating_sub(1)) {
            line.push_str(buffer[(column, row)].symbol());
        }
        rows.push(line.trim_end().to_owned());
    }
    while rows.first().is_some_and(String::is_empty) {
        rows.remove(0);
    }
    while rows.last().is_some_and(String::is_empty) {
        rows.pop();
    }
    rows.join("\n")
}

fn draw_list_overlay(frame: &mut Frame<'_>, area: Rect, list: &crate::app::ChoiceList, app: &App) {
    let palette = app.theme.palette;
    let width = area.width.saturating_sub(4).clamp(30, MAX_LIST_WIDTH);
    let item_rows = list.visible.len().clamp(1, MAX_LIST_ITEMS);
    let height = u16::try_from(item_rows.saturating_add(6))
        .unwrap_or(u16::MAX)
        .max(8);
    let rect = centered(area, width, height);
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
        "—"
    } else {
        list.query.as_str()
    };
    let query_style = if list.query.is_empty() {
        palette.muted
    } else {
        palette.foreground
    };
    let mut lines = vec![
        Line::from(vec![
            Span::styled("filter: ", Style::default().fg(palette.muted)),
            Span::styled(query, Style::default().fg(query_style)),
        ]),
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
                Style::default().fg(palette.foreground),
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
        Style::default().fg(palette.foreground),
    )));
    frame.render_widget(
        Paragraph::new(lines).style(
            Style::default()
                .fg(palette.foreground)
                .bg(palette.background),
        ),
        inner,
    );

    if list.selected >= offset
        && let Some(selected_row) = list.selected.checked_sub(offset)
        && selected_row < visible_height
        && selected_row < list.visible.len()
    {
        let row = Rect {
            x: inner.x,
            y: inner.y + 2 + u16::try_from(selected_row).unwrap_or(u16::MAX),
            width: inner.width,
            height: 1,
        };
        frame.buffer_mut().set_style(
            row,
            Style::default().fg(palette.background).bg(palette.primary),
        );
    }
}

fn draw_transcript(frame: &mut Frame<'_>, area: Rect, app: &mut App) {
    let width = area.width.max(1);
    let total = ensure_transcript_cache(app, width);
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
        if app.transcript.is_empty() {
            visible.extend(
                cache.banner[start.min(cache.banner.len())..end.min(cache.banner.len())]
                    .iter()
                    .cloned(),
            );
        } else {
            let mut remaining = end.saturating_sub(start);
            let (mut index, mut offset) = cache
                .row_index
                .find(start)
                .unwrap_or((cache.entries.len(), 0));
            while remaining > 0 && index < cache.entries.len() {
                let entry = &cache.entries[index];
                let from = offset.min(entry.rows.len());
                let to = from.saturating_add(remaining).min(entry.rows.len());
                for (row, target) in entry.rows[from..to].iter().zip(&entry.targets[from..to]) {
                    if let Some(target) = target {
                        app.hits.push(Hit {
                            row: area
                                .y
                                .saturating_add(u16::try_from(visible.len()).unwrap_or(u16::MAX)),
                            target: *target,
                        });
                    }
                    visible.push(row.clone());
                }
                remaining = remaining.saturating_sub(to - from);
                index += 1;
                offset = 0;
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

/// Rebuilds only the transcript entries whose content, layout, or animation
/// changed, and returns the total number of laid-out rows.
fn ensure_transcript_cache(app: &mut App, width: u16) -> usize {
    let dirty = app.transcript.take_dirty();
    let layout_generation = app.transcript.layout_generation();
    let tick = app.tick;

    let mut slot = app.transcript_cache.borrow_mut();
    let cache = slot.get_or_insert_with(|| CachedTranscript {
        width,
        theme_revision: app.theme_revision,
        layout_generation,
        tick,
        entries: Vec::new(),
        row_index: RowIndex::default(),
        animated: HashSet::new(),
        banner: Vec::new(),
    });
    let reset = cache.width != width
        || cache.theme_revision != app.theme_revision
        || cache.layout_generation != layout_generation;
    let tick_changed = cache.tick != tick;
    if reset {
        cache.width = width;
        cache.theme_revision = app.theme_revision;
        cache.layout_generation = layout_generation;
        cache.entries.clear();
        cache.row_index = RowIndex::default();
        cache.animated.clear();
        cache.banner.clear();
    }
    cache.tick = tick;

    if app.transcript.is_empty() {
        cache.entries.clear();
        cache.row_index = RowIndex::default();
        cache.animated.clear();
        if cache.banner.is_empty() {
            cache.banner = wrap_lines(
                empty_state_lines(&app.theme.palette, width.saturating_sub(1)),
                usize::from(width),
            );
        }
        return cache.banner.len();
    }

    let app: &App = app;
    let transcript = &app.transcript;
    let len = transcript.entries().len();
    let full = reset || cache.entries.len() > len;
    let appended = !full && cache.entries.len() < len;
    let mut work = if full {
        (0..len).collect::<BTreeSet<_>>()
    } else {
        dirty.into_iter().collect::<BTreeSet<_>>()
    };
    if appended {
        work.extend(cache.entries.len()..len);
    }
    if !full && tick_changed {
        work.extend(cache.animated.iter().copied());
    }
    if full {
        cache.entries.clear();
        cache.row_index = RowIndex::default();
        cache.animated.clear();
    }

    let layout_width = width.saturating_sub(1);
    let rebuild_index = full || appended;
    for index in work {
        if index >= len {
            continue;
        }
        let entry = &transcript.entries()[index];
        let layout = entry_layout(app, index);
        let animation_tick = if layout == EntryLayout::Skipped {
            0
        } else {
            entry_animation_tick(entry, app.tick)
        };
        let key = CachedEntryKey {
            revision: transcript.entry_revision(index),
            layout,
            animation_tick,
        };
        let unchanged = !rebuild_index
            && cache
                .entries
                .get(index)
                .is_some_and(|cached| cached.key == key);
        if unchanged {
            continue;
        }
        let (lines, targets) = entry_lines(app, index, layout, layout_width);
        let (rows, targets) = wrap_rows(lines, targets, usize::from(width));
        let previous = cache
            .entries
            .get(index)
            .map_or(0, |cached| cached.rows.len());
        let cached = CachedEntry { key, rows, targets };
        if index < cache.entries.len() {
            cache.entries[index] = cached;
        } else {
            cache.entries.push(cached);
        }
        if animation_tick == 0 {
            cache.animated.remove(&index);
        } else {
            cache.animated.insert(index);
        }
        if !rebuild_index {
            cache
                .row_index
                .set(index, cache.entries[index].rows.len(), previous);
        }
    }
    cache.entries.truncate(len);
    cache.animated.retain(|index| *index < len);
    if rebuild_index || cache.row_index.len() != len {
        cache.row_index = RowIndex::from_counts(cache.entries.iter().map(|entry| entry.rows.len()));
    }
    cache.row_index.total()
}

/// Returns how one entry should be laid out in the current turn state.
fn entry_layout(app: &App, index: usize) -> EntryLayout {
    if app.transcript.is_skipped(index) {
        EntryLayout::Skipped
    } else if app.transcript.is_turn_start(index) && app.transcript.is_turn_collapsed(index) {
        EntryLayout::Collapsed
    } else {
        EntryLayout::Visible
    }
}

/// Returns the animation tick an entry needs, or zero when it is static.
fn entry_animation_tick(entry: &Entry, tick: u64) -> u64 {
    match entry {
        Entry::Tool(block) if block.status == ToolStatus::Running => tick,
        Entry::Assistant(assistant) if assistant.streaming && !assistant.thinking.is_empty() => {
            tick
        }
        _ => 0,
    }
}

fn wrap_lines(lines: Vec<Line<'static>>, width: usize) -> Vec<Line<'static>> {
    lines
        .into_iter()
        .flat_map(|line| tool_view::wrap_line(line, width))
        .collect()
}

fn wrap_rows(
    lines: Vec<Line<'static>>,
    targets: Vec<Option<HitTarget>>,
    width: usize,
) -> (Vec<Line<'static>>, Vec<Option<HitTarget>>) {
    let mut rows = Vec::new();
    let mut row_targets = Vec::new();
    for (line, target) in lines.into_iter().zip(targets) {
        for row in tool_view::wrap_line(line, width) {
            rows.push(row);
            row_targets.push(target);
        }
    }
    (rows, row_targets)
}

fn empty_state_lines(palette: &Palette, width: u16) -> Vec<Line<'static>> {
    let mut lines = vec![Line::default()];
    if width >= 40 {
        for row in BANNER {
            lines.push(Line::from(Span::styled(
                row,
                Style::default()
                    .fg(palette.primary)
                    .add_modifier(Modifier::BOLD),
            )));
        }
    } else {
        lines.push(Line::from(Span::styled(
            "Codie!",
            Style::default()
                .fg(palette.primary)
                .add_modifier(Modifier::BOLD),
        )));
    }
    lines.push(Line::default());
    lines.push(Line::from(Span::styled(
        "Type a prompt, or press / for the command palette.",
        Style::default().fg(palette.foreground),
    )));
    lines.push(Line::from(vec![
        Span::styled(
            "Enter",
            Style::default()
                .fg(palette.primary)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" submit  ·  ", Style::default().fg(palette.muted)),
        Span::styled(
            "Shift+Enter",
            Style::default()
                .fg(palette.primary)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" newline  ·  ", Style::default().fg(palette.muted)),
        Span::styled(
            "@",
            Style::default()
                .fg(palette.primary)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" attach files", Style::default().fg(palette.muted)),
    ]));
    lines.push(Line::default());
    lines
}

/// Lays out one transcript entry in isolation.
fn entry_lines(
    app: &App,
    index: usize,
    layout: EntryLayout,
    width: u16,
) -> (Vec<Line<'static>>, Vec<Option<HitTarget>>) {
    if layout == EntryLayout::Skipped {
        return (Vec::new(), Vec::new());
    }
    let palette = app.theme.palette;
    let entry = &app.transcript.entries()[index];
    let mut body = TranscriptBody::default();

    if layout == EntryLayout::Collapsed {
        let Entry::User(text) = entry else {
            return (Vec::new(), Vec::new());
        };
        body.push(Line::default());
        let label = match text.split_once('\n') {
            Some((first, _)) => format!("{first} …"),
            None => text.clone(),
        };
        body.push_target(
            Line::from(vec![
                Span::styled(
                    "▸ ",
                    Style::default()
                        .fg(palette.primary)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(label, Style::default().fg(palette.user).bg(palette.surface)),
                Span::styled(
                    format!("  {}", turn_summary(&app.transcript, index)),
                    Style::default().fg(palette.muted),
                ),
            ]),
            HitTarget::Turn(index),
        );
        return (body.lines, body.targets);
    }

    match entry {
        Entry::User(text) => {
            body.push(Line::default());
            let mut lines = markdown::render_user(text, &palette, usize::from(width));
            if lines.is_empty() {
                lines.push(Line::default());
            }
            lines[0].spans.insert(
                0,
                Span::styled(
                    "▾ ",
                    Style::default()
                        .fg(palette.primary)
                        .add_modifier(Modifier::BOLD),
                ),
            );
            let turn_start = app.transcript.is_turn_start(index);
            for (offset, line) in lines.into_iter().enumerate() {
                if turn_start && offset == 0 {
                    body.push_target(line, HitTarget::Turn(index));
                } else {
                    body.push(line);
                }
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
    if block.name == "task" {
        return task_header(block, tick, palette);
    }
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

/// Renders a delegated run as a first-class row: agent, label, live activity,
/// and the terminal status, turns, cost, and child session.
fn task_header(
    block: &crate::transcript::ToolBlock,
    tick: u64,
    palette: &Palette,
) -> Vec<Span<'static>> {
    let agent = block
        .arguments
        .get("agent")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("general");
    let description = block
        .arguments
        .get("description")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("delegated task");
    let (marker, color) = match block.status {
        ToolStatus::Running => (SPINNER[tick as usize % SPINNER.len()], palette.primary),
        ToolStatus::Success => ("✓", palette.success),
        ToolStatus::Error => ("✗", palette.error),
    };
    let mut spans = vec![
        Span::styled(
            format!("{marker} "),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!("Task · {agent}"), Style::default().fg(palette.tool)),
        Span::styled(
            format!(" · {}", truncate(description, 48)),
            Style::default().fg(palette.foreground),
        ),
    ];

    if block.status == ToolStatus::Running {
        if let Some(progress) = block.progress.as_deref() {
            spans.push(Span::styled(
                format!("  {progress}"),
                Style::default().fg(palette.muted),
            ));
        }
        if let Some(elapsed) = block.elapsed_label() {
            spans.push(Span::styled(
                format!("  {elapsed}"),
                Style::default().fg(palette.muted),
            ));
        }
        return spans;
    }

    let details = block.details.as_ref();
    let status = details
        .and_then(|details| details.get("status"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or(if block.status == ToolStatus::Error {
            "error"
        } else {
            "completed"
        });
    let turns = details
        .and_then(|details| details.get("turns"))
        .and_then(serde_json::Value::as_u64);
    let cost = details
        .and_then(|details| details.get("usage"))
        .and_then(|usage| usage.get("cost_usd"))
        .and_then(serde_json::Value::as_f64);
    let session = details
        .and_then(|details| details.get("session_id"))
        .and_then(serde_json::Value::as_str);

    let mut suffix = status.to_owned();
    if let Some(turns) = turns {
        suffix.push_str(&format!(
            " · {turns} turn{}",
            if turns == 1 { "" } else { "s" }
        ));
    }
    if let Some(cost) = cost
        && cost > 0.0
    {
        suffix.push_str(&format!(" · ${cost:.4}"));
    }
    if let Some(session) = session {
        suffix.push_str(&format!(" · {}", &session[..session.len().min(12)]));
    }
    if let Some(error) = details
        .and_then(|details| details.get("error"))
        .and_then(serde_json::Value::as_str)
        .filter(|error| !error.is_empty())
    {
        suffix.push_str(&format!(" · {}", truncate(error, 60)));
    }
    spans.push(Span::styled(
        format!("  {suffix}"),
        Style::default().fg(palette.muted),
    ));
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
        .title(editor_title(title, app))
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

/// Builds the editor title, appending a transient notice when present.
fn editor_title(title: String, app: &App) -> Line<'static> {
    let palette = app.theme.palette;
    let mut spans = vec![Span::styled(title, Style::default().fg(palette.muted))];
    if let Some(notice) = &app.notice {
        spans.push(Span::styled(
            format!("· {notice} "),
            Style::default().fg(palette.success),
        ));
    }
    Line::from(spans)
}

fn draw_footer(frame: &mut Frame<'_>, area: Rect, app: &mut App) {
    let palette = app.theme.palette;
    let location = footer_location(app);

    if area.height > 1 {
        let rows = Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).split(area);
        let location = truncate(&location, usize::from(rows[0].width));
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                location,
                Style::default().fg(palette.muted),
            ))),
            rows[0],
        );
        let spans = footer_spans(
            footer_parts(app, area.width),
            usize::from(rows[1].width),
            &palette,
        );
        frame.render_widget(Paragraph::new(Line::from(spans)), rows[1]);
        for row in rows.iter() {
            app.hits.push(Hit {
                row: row.y,
                target: crate::app::HitTarget::Usage,
            });
        }
        return;
    }

    let max_location_width = area.width.saturating_mul(2) / 5;
    let location_width = u16::try_from(
        UnicodeWidthStr::width(location.as_str())
            .saturating_add(1)
            .min(usize::from(max_location_width)),
    )
    .unwrap_or(max_location_width);
    let columns =
        Layout::horizontal([Constraint::Length(location_width), Constraint::Min(0)]).split(area);
    let location = truncate(&location, usize::from(columns[0].width));
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            location,
            Style::default().fg(palette.muted),
        ))),
        columns[0],
    );

    let spans = footer_spans(
        footer_parts(app, area.width),
        usize::from(columns[1].width),
        &palette,
    );
    frame.render_widget(
        Paragraph::new(Line::from(spans)).alignment(Alignment::Right),
        columns[1],
    );
    app.hits.push(Hit {
        row: area.y,
        target: crate::app::HitTarget::Usage,
    });
}

fn footer_location(app: &App) -> String {
    let mut location = app.footer.cwd.clone();
    if let Some(branch) = &app.footer.git_branch {
        location.push_str(&format!(" ({branch})"));
    }
    location
}

fn footer_parts(app: &App, width: u16) -> Vec<(String, Style)> {
    let palette = app.theme.palette;
    let detailed = width >= DETAILED_FOOTER_WIDTH;
    let model = if width < NARROW_FOOTER_WIDTH || app.footer.provider.is_empty() {
        app.footer.model.clone()
    } else {
        format!("{}/{}", app.footer.provider, app.footer.model)
    };
    let mut parts = vec![(model, Style::default().fg(palette.secondary))];
    if app.auto_approve {
        parts.push((
            "FULL ACCESS".to_owned(),
            Style::default().fg(palette.warning),
        ));
    }
    let running = app.subagent_runtime.as_ref().map_or(0, |runtime| {
        runtime
            .runs()
            .iter()
            .filter(|run| {
                matches!(
                    run.state,
                    codie_subagents::RunState::Running | codie_subagents::RunState::Queued
                )
            })
            .count()
    });
    if app.subagent_mode != codie_subagents::DelegationMode::Off || running > 0 {
        let badge = match app.subagent_mode {
            codie_subagents::DelegationMode::Off => "sub:off",
            codie_subagents::DelegationMode::Auto => "sub:auto",
            codie_subagents::DelegationMode::Force => "sub:force",
        };
        parts.push((badge.to_owned(), Style::default().fg(palette.muted)));
        if running > 0 {
            parts.push((format!("▶ {running}"), Style::default().fg(palette.primary)));
        }
    }
    if let Some(percent) = app.footer.context_percent {
        parts.push((
            format!("ctx {} {percent}%", context_meter(percent)),
            Style::default().fg(context_color(percent, app.context_window, &palette)),
        ));
    }
    if detailed {
        parts.push((
            app.footer.thinking.clone(),
            Style::default().fg(palette.muted),
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
    if detailed {
        if let Some(rate) = app.footer.tokens_per_sec {
            parts.push((format!("{rate:.0} t/s"), Style::default().fg(palette.muted)));
        }
        if let Some(session) = &app.footer.session {
            parts.push((
                session[..session.len().min(8)].to_owned(),
                Style::default().fg(palette.muted),
            ));
        }
    }
    parts
}

fn footer_spans(
    parts: Vec<(String, Style)>,
    available: usize,
    palette: &Palette,
) -> Vec<Span<'static>> {
    if available == 0 {
        return Vec::new();
    }
    let mut spans = Vec::new();
    let mut used = 0_usize;
    for (text, style) in parts {
        let separator = usize::from(!spans.is_empty()) * 3;
        let text = if spans.is_empty() {
            truncate(&text, available)
        } else {
            text
        };
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
    spans
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

/// Rows a capped permission preview shows.
const PERMISSION_CODE_ROWS: usize = 40;
/// Rows a capped permission diff shows.
const PERMISSION_DIFF_ROWS: usize = 48;
/// Rows a permission preview shows once expanded.
const PERMISSION_EXPANDED_ROWS: usize = 400;
/// Widest permission dialog.
const PERMISSION_MAX_WIDTH: u16 = 88;
/// Narrowest permission dialog that keeps the actions and footer readable.
const PERMISSION_MIN_WIDTH: u16 = 56;
/// Space used by the border and horizontal padding.
const PERMISSION_FRAME_WIDTH: usize = 4;

/// One built permission dialog body.
struct PermissionBody {
    /// Intent line and operation context.
    header: Vec<Line<'static>>,
    /// Preview title, rows, and truncation state.
    preview: Vec<Line<'static>>,
    /// True when the preview is capped.
    truncated: bool,
}

fn permission_dialog_width(area: Rect, prompt: &crate::app::PermissionPrompt, app: &App) -> u16 {
    let maximum = area.width.min(PERMISSION_MAX_WIDTH);
    let minimum = PERMISSION_MIN_WIDTH.min(maximum);
    let max_content = usize::from(PERMISSION_MAX_WIDTH).saturating_sub(PERMISSION_FRAME_WIDTH);
    let mut content = 52_usize;
    content = content.max(UnicodeWidthStr::width(app.footer.cwd.as_str()));
    if let Some(branch) = &app.footer.git_branch {
        content = content.max(
            UnicodeWidthStr::width(app.footer.cwd.as_str())
                .saturating_add(UnicodeWidthStr::width(branch.as_str()))
                .saturating_add(4),
        );
    }
    if let Some(preview) = &prompt.preview {
        if let Some(title) = &preview.title {
            content = content.max(UnicodeWidthStr::width(title.as_str()));
        }
        let longest_line = preview
            .text
            .lines()
            .map(UnicodeWidthStr::width)
            .max()
            .unwrap_or(0);
        content = match preview.kind {
            ExtRenderKind::Diff => max_content,
            ExtRenderKind::Code => content.max(longest_line.saturating_add(
                if matches!(prompt.tool.as_str(), "bash" | "powershell") {
                    2
                } else {
                    6
                },
            )),
            _ => content.max(longest_line),
        };
    }
    let desired = u16::try_from(
        content
            .min(max_content)
            .saturating_add(PERMISSION_FRAME_WIDTH),
    )
    .unwrap_or(maximum);
    desired.clamp(minimum, maximum)
}

fn draw_permission_overlay(
    frame: &mut Frame<'_>,
    area: Rect,
    prompt: &crate::app::PermissionPrompt,
    app: &App,
) {
    let palette = app.theme.palette;
    let width = permission_dialog_width(area, prompt, app);
    let inner_width = usize::from(width).saturating_sub(PERMISSION_FRAME_WIDTH);
    let danger = destructive_command(prompt);
    let origin = delegated_origin(app, prompt.session_id.as_deref());
    let PermissionBody {
        header,
        preview,
        truncated,
    } = permission_body(
        prompt,
        inner_width,
        &palette,
        danger,
        &app.footer.cwd,
        app.footer.git_branch.as_deref(),
        origin.as_deref(),
    );

    // Borders, header, two gaps, and a pinned footer. The dialog hugs its
    // content instead of filling a fixed fraction of the screen.
    let overhead = 2 + header.len() + 2 + 2;
    let capacity = usize::from(area.height).saturating_sub(overhead).max(1);
    let height = u16::try_from(overhead + preview.len().min(capacity))
        .unwrap_or(u16::MAX)
        .min(area.height);
    let rect = centered(area, width, height);
    frame.render_widget(Clear, rect);
    dim_outside(frame, rect);

    let border = if danger {
        palette.error
    } else {
        palette.warning
    };
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .padding(Padding::horizontal(1))
        .title(Line::from(vec![
            Span::styled(
                " permission ",
                Style::default().fg(border).add_modifier(Modifier::BOLD),
            ),
            Span::styled("· ", Style::default().fg(palette.muted)),
            Span::styled(
                format!("{} ", prompt.tool),
                Style::default()
                    .fg(palette.tool)
                    .add_modifier(Modifier::BOLD),
            ),
        ]))
        .border_style(Style::default().fg(border));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);

    let gap = u16::from(!preview.is_empty());
    let rows = Layout::vertical([
        Constraint::Length(u16::try_from(header.len()).unwrap_or(0)),
        Constraint::Length(gap),
        Constraint::Min(0),
        Constraint::Length(gap),
        Constraint::Length(2),
    ])
    .split(inner);

    frame.render_widget(
        Paragraph::new(header).style(
            Style::default()
                .fg(palette.foreground)
                .bg(palette.background),
        ),
        rows[0],
    );

    let preview_rows = usize::from(rows[2].height);
    let max_scroll = preview.len().saturating_sub(preview_rows);
    let scroll = usize::from(prompt.scroll).min(max_scroll);
    frame.render_widget(
        Block::default().style(Style::default().bg(palette.surface)),
        rows[2],
    );
    frame.render_widget(
        Paragraph::new(preview)
            .style(Style::default().bg(palette.surface))
            .scroll((u16::try_from(scroll).unwrap_or(0), 0)),
        rows[2],
    );

    let footer = Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).split(rows[4]);
    frame.render_widget(
        Paragraph::new(Line::from(permission_actions(
            inner_width,
            &palette,
            prompt.agent.is_some(),
        ))),
        footer[0],
    );
    let mut hints = String::from("Enter allow once · Esc deny");
    if max_scroll > 0 {
        hints.push_str(" · ↑↓ scroll");
    }
    if prompt.expanded {
        hints.push_str(" · Ctrl+O collapse");
    } else if truncated {
        hints.push_str(" · Ctrl+O full");
    }
    let shift_tab = " · Shift+Tab full access";
    if UnicodeWidthStr::width(hints.as_str()) + UnicodeWidthStr::width(shift_tab) <= inner_width {
        hints.push_str(shift_tab);
    }
    if UnicodeWidthStr::width(hints.as_str()) > inner_width {
        hints = hints.replacen("Enter allow once", "Enter allow", 1);
    }
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            truncate(&hints, inner_width),
            Style::default().fg(palette.foreground),
        ))),
        footer[1],
    );
}

/// Returns the run label for a delegated session, when it is still known.
fn delegated_origin(app: &App, session_id: Option<&str>) -> Option<String> {
    let session_id = session_id?;
    app.subagent_runtime.as_ref().and_then(|runtime| {
        runtime
            .runs()
            .into_iter()
            .find(|run| run.session_id.as_deref() == Some(session_id))
            .map(|run| run.description)
    })
}

/// Dims every cell outside the modal so the transcript recedes.
fn dim_outside(frame: &mut Frame<'_>, rect: Rect) {
    let area = frame.area();
    let buffer = frame.buffer_mut();
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            if x >= rect.x && x < rect.right() && y >= rect.y && y < rect.bottom() {
                continue;
            }
            if let Some(cell) = buffer.cell_mut((x, y)) {
                cell.modifier |= Modifier::DIM;
            }
        }
    }
}

/// Builds the intent header and the preview rows for one prompt.
fn permission_body(
    prompt: &crate::app::PermissionPrompt,
    width: usize,
    palette: &Palette,
    danger: bool,
    cwd: &str,
    branch: Option<&str>,
    origin: Option<&str>,
) -> PermissionBody {
    let intent = match prompt.tool.as_str() {
        "bash" => "Run this shell command?",
        "powershell" => "Run this PowerShell command?",
        "edit" => "Apply these edits?",
        "write" => "Write this file?",
        _ => "Allow this tool to run?",
    };
    let mut header = vec![Line::from(Span::styled(
        intent,
        Style::default()
            .fg(palette.foreground)
            .add_modifier(Modifier::BOLD),
    ))];
    if let Some(agent) = prompt.agent.as_deref() {
        let label = origin
            .filter(|label| !label.trim().is_empty())
            .map(|label| format!(" · {label}"))
            .unwrap_or_default();
        header.push(Line::from(Span::styled(
            truncate(
                &format!(
                    "Requested by subagent `{agent}`{label} · depth {}",
                    prompt.depth
                ),
                width,
            ),
            Style::default().fg(palette.warning),
        )));
    }

    let mut context: Vec<(String, Style)> =
        vec![(cwd.to_owned(), Style::default().fg(palette.foreground))];
    if let Some(branch) = branch {
        context.push((format!("  ⑂ {branch}"), Style::default().fg(palette.info)));
    }
    if let Some(preview) = &prompt.preview
        && preview.kind == ExtRenderKind::Diff
    {
        let (added, removed) = diff_stats(&preview.text);
        context.push((format!("  +{added}"), Style::default().fg(palette.diff_add)));
        context.push((
            format!(" −{removed}"),
            Style::default().fg(palette.diff_del),
        ));
    }
    header.push(Line::from(clip_spans(context, width)));
    if danger {
        header.push(Line::from(Span::styled(
            truncate(
                "⚠ destructive command — review every line before allowing",
                width,
            ),
            Style::default()
                .fg(palette.error)
                .add_modifier(Modifier::BOLD),
        )));
    }

    let mut preview = Vec::new();
    let truncated;
    match &prompt.preview {
        Some(hint) => {
            if let Some(title) = &hint.title {
                preview.push(Line::from(Span::styled(
                    title.clone(),
                    Style::default()
                        .fg(palette.heading)
                        .add_modifier(Modifier::BOLD),
                )));
            }
            let limit = if prompt.expanded {
                PERMISSION_EXPANDED_ROWS
            } else {
                match hint.kind {
                    ExtRenderKind::Diff => PERMISSION_DIFF_ROWS,
                    _ => PERMISSION_CODE_ROWS,
                }
            };
            let path = prompt
                .arguments
                .get("path")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            let (rows, capped) = match hint.kind {
                ExtRenderKind::Diff if width < 110 => {
                    tool_view::render_diff_unified(&hint.text, path, width, palette, limit)
                }
                ExtRenderKind::Diff => {
                    tool_view::render_diff(&hint.text, path, width, palette, limit)
                }
                ExtRenderKind::Code if matches!(prompt.tool.as_str(), "bash" | "powershell") => {
                    let shell = if prompt.tool == "powershell" {
                        "command.ps1"
                    } else {
                        "command.sh"
                    };
                    tool_view::render_shell(&hint.text, shell, width, palette, limit)
                }
                ExtRenderKind::Code => {
                    tool_view::render_code(&hint.text, path, width, palette, limit)
                }
                _ => (
                    hint.text
                        .lines()
                        .map(|line| Line::from(line.to_owned()))
                        .collect(),
                    false,
                ),
            };
            truncated = capped;
            preview.extend(rows);
        }
        None => {
            let json = serde_json::to_string_pretty(&prompt.arguments).unwrap_or_default();
            for line in json.lines().take(PERMISSION_CODE_ROWS) {
                preview.push(Line::from(Span::styled(
                    format!("  {line}"),
                    Style::default().fg(palette.foreground),
                )));
            }
            truncated = json.lines().count() > PERMISSION_CODE_ROWS;
        }
    }
    PermissionBody {
        header,
        preview,
        truncated,
    }
}

/// Returns the three decision chips, each in its own color, with the
/// `Enter` target emphasized.
fn permission_actions(width: usize, palette: &Palette, delegated: bool) -> Vec<Span<'static>> {
    let compact = width < 48;
    let always = if compact {
        "Always"
    } else if delegated {
        "Always for agent"
    } else {
        "Always allow"
    };
    let actions: [(&str, &str, ratatui::style::Color, bool); 3] = [
        (
            "y",
            if compact { "Allow" } else { "Allow once" },
            palette.success,
            true,
        ),
        ("A", always, palette.warning, false),
        ("n", "Deny", palette.error, false),
    ];
    let mut spans = Vec::new();
    for (index, (key, label, color, selected)) in actions.into_iter().enumerate() {
        if index > 0 {
            spans.push(Span::raw(if compact { "  " } else { "   " }));
        }
        spans.push(Span::styled(
            format!("[{key}]"),
            Style::default()
                .fg(palette.background)
                .bg(color)
                .add_modifier(Modifier::BOLD),
        ));
        let mut label_style = Style::default().fg(color);
        if selected {
            label_style = label_style.add_modifier(Modifier::BOLD);
        }
        spans.push(Span::styled(format!(" {label}"), label_style));
    }
    spans
}

/// Counts added and removed rows in a unified diff.
fn diff_stats(text: &str) -> (usize, usize) {
    text.lines().fold((0, 0), |(added, removed), line| {
        if line.starts_with("+++") || line.starts_with("---") {
            (added, removed)
        } else if line.starts_with('+') {
            (added + 1, removed)
        } else if line.starts_with('-') {
            (added, removed + 1)
        } else {
            (added, removed)
        }
    })
}

/// Truncates styled fragments to a display width.
fn clip_spans(spans: Vec<(String, Style)>, width: usize) -> Vec<Span<'static>> {
    let mut clipped = Vec::new();
    let mut used = 0;
    for (text, style) in spans {
        let remaining = width.saturating_sub(used);
        if remaining == 0 {
            break;
        }
        let text = truncate(&text, remaining);
        used += UnicodeWidthStr::width(text.as_str());
        clipped.push(Span::styled(text, style));
    }
    clipped
}

/// Detects shell commands that commonly destroy work.
fn destructive_command(prompt: &crate::app::PermissionPrompt) -> bool {
    if !matches!(prompt.tool.as_str(), "bash" | "powershell") {
        return false;
    }
    let Some(command) = prompt
        .arguments
        .get("command")
        .and_then(serde_json::Value::as_str)
    else {
        return false;
    };
    const PATTERNS: [&str; 16] = [
        "rm -rf",
        "rm -fr",
        "rm -r",
        "sudo ",
        "git reset --hard",
        "git clean -f",
        "git checkout -- .",
        "mkfs",
        "dd if=",
        "chmod -r",
        "chown -r",
        "truncate -s 0",
        "remove-item",
        "format-volume",
        "stop-computer",
        ":(){",
    ];
    let lowered = command.to_ascii_lowercase();
    PATTERNS.iter().any(|pattern| lowered.contains(pattern))
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

    let mut scroll = 0_u16;
    let (title, body) = match overlay {
        Overlay::Child {
            title,
            lines,
            scroll: offset,
            ..
        } => {
            scroll = *offset;
            (title.as_str(), lines.join("\n"))
        }
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
                "/connect [provider] add or replace an API key",
                "/provider [id]     switch provider",
                "/logout [provider] remove a stored API key",
                "/auth              show credential status",
                "/model [id]        change the model",
                "/thinking [level]  change reasoning effort",
                "/compact [note]    compact the context",
                "/usage             tokens, cost, and context",
                "/subagents [cmd]   configure subagent delegation",
                "/stop              stop the active run",
                "/skill:<name>      invoke a skill",
                "/template:<name>   expand a prompt template",
                "/reload            reload skills, templates, and project memory",
                "/init              create PLAN.MD and CODIE.MD",
                "/quit              exit",
            ]
            .join("\n"),
        ),
        Overlay::Hotkeys => (" keys ", app.bindings.describe()),
        Overlay::Permission(prompt) => (
            " permission ",
            format!(
                "Allow `{}`?\n\n{}\n\n[y] yes   [n] no   [A] Always allow   [Shift+Tab] full access (no prompts)",
                prompt.tool,
                serde_json::to_string_pretty(&prompt.arguments).unwrap_or_default()
            ),
        ),
        Overlay::Question(prompt) => {
            let origin = delegated_origin(app, prompt.session_id.as_deref());
            let who = match (&prompt.agent, origin) {
                (Some(agent), Some(label)) => format!("from subagent `{agent}` · {label}\n\n"),
                (Some(agent), None) => format!("from subagent `{agent}`\n\n"),
                (None, _) => String::new(),
            };
            (
                " question ",
                format!(
                    "{who}{}\n\n> {}\u{258c}\n\nEnter send · Esc dismiss (the model continues without an answer)",
                    prompt.question, prompt.input
                ),
            )
        }
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
        Overlay::Connect(connect) => {
            let key = truncate(&connect.display(), 64);
            let body = match &connect.phase {
                crate::app::ConnectPhase::Editing => format!(
                    "Provider: {} ({})\n\nPaste the API key below. It is stored only after the provider accepts it.\n\n  {key}\n\nEnter check · Ctrl+V paste · Ctrl+R reveal · Esc cancel",
                    connect.provider_name, connect.provider
                ),
                crate::app::ConnectPhase::Checking => format!(
                    "Provider: {} ({})\n\nChecking the key with {}…\n\nEsc cancel",
                    connect.provider_name, connect.provider, connect.provider_name
                ),
                crate::app::ConnectPhase::Error(message) => format!(
                    "Provider: {} ({})\n\n{message}\n\nEnter edit and retry · Ctrl+S save without checking · Esc cancel",
                    connect.provider_name, connect.provider
                ),
            };
            (" connect provider ", body)
        }
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
            .scroll((scroll, 0))
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
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

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
        assert!(text.contains("Shift+Enter"));
        assert!(text.contains("attach files"));
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
    #[spec("TUI-004")]
    fn spec_tui_004_user_messages_preserve_line_breaks() {
        let mut app = app();
        app.transcript
            .push_user("line one\nline two\n\n```rust\nfn main() {}\n```");
        let palette = app.theme.palette;
        let buffer = buffer_at(&mut app, 80, 24);
        let row = |y: u16| (0..80).map(|x| buffer[(x, y)].symbol()).collect::<String>();

        assert!(row(1).contains("line one"), "{:?}", row(1));
        assert!(row(2).contains("line two"), "{:?}", row(2));
        assert!(!row(1).contains("line two"), "{:?}", row(1));
        assert_eq!(buffer[(0, 2)].bg, palette.surface);

        let text = (0..24).map(row).collect::<Vec<_>>().join("\n");
        assert!(text.contains("fn main"), "{text}");
        assert!(!text.contains("```"), "code fences are not printed: {text}");
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
    #[spec("TUI-014")]
    fn spec_tui_014_list_overlay_hugs_results_and_highlights_the_full_row() {
        let mut app = app();
        app.handle_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE));
        let palette = app.theme.palette;
        let buffer = buffer_at(&mut app, 100, 40);
        let row = |y: u16| {
            (0..100)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        };
        let top = (0..40)
            .find(|y| row(*y).contains(" commands "))
            .expect("palette title row");
        let bottom = (top..40)
            .find(|y| row(*y).contains('└'))
            .expect("palette bottom border");
        assert_eq!(bottom - top + 1, 22, "palette should cap at 16 items");

        let description_row = (0..40)
            .find(|y| row(*y).contains("Choose a provider"))
            .expect("command description row");
        let description_column = row(description_row)
            .find("Choose a provider")
            .expect("description column") as u16;
        assert_eq!(
            buffer[(description_column, description_row)].fg,
            palette.foreground,
            "command descriptions should not use muted text"
        );

        let Some(Overlay::List(list)) = app.overlay.as_mut() else {
            panic!("expected the command palette");
        };
        list.query = "usage".to_owned();
        list.refresh();
        let buffer = buffer_at(&mut app, 80, 24);
        let row = |y: u16| (0..80).map(|x| buffer[(x, y)].symbol()).collect::<String>();
        let top = (0..24)
            .find(|y| row(*y).contains(" commands "))
            .expect("filtered palette title row");
        let bottom = (top..24)
            .find(|y| row(*y).contains('└'))
            .expect("filtered palette bottom border");
        assert_eq!(bottom - top + 1, 8, "palette should fit one result");

        let selected = (0..24)
            .find(|y| row(*y).contains("/usage"))
            .expect("selected command row");
        assert!(
            (3..77).all(|x| buffer[(x, selected)].bg == palette.primary),
            "the entire inner row should have the selected background"
        );
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
        assert!(text.contains("136 t/s"), "footer throughput: {text}");
        assert!(text.contains("0190f3a1"), "footer session: {text}");
        assert!(text.contains("medium"), "footer thinking level: {text}");
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
    #[spec("TUI-001")]
    fn spec_tui_001_narrow_footer_uses_two_rows() {
        let mut app = app();
        app.footer.context_percent = Some(42);
        app.footer.tokens = 1_200;
        app.footer.cost = 8_000_000;
        app.footer.tokens_per_sec = Some(136.0);
        app.footer.session = Some("0190f3a1".into());
        let buffer = buffer_at(&mut app, 60, 20);
        let row = |y: u16| (0..60).map(|x| buffer[(x, y)].symbol()).collect::<String>();
        assert!(
            row(18).contains("/tmp/project"),
            "location row: {}",
            row(18)
        );
        assert!(
            row(19).contains("deepseek-flash"),
            "status row: {}",
            row(19)
        );
        assert!(row(19).contains("42%"), "context meter: {}", row(19));
        assert!(!row(19).contains("deepseek/deepseek-flash"));
        assert!(!row(19).contains("136 t/s"));
        assert!(!row(19).contains("0190f3a1"));
        assert!(!row(19).contains("medium"));
    }

    #[test]
    #[spec("TUI-006")]
    fn spec_tui_006_medium_footer_drops_metadata_before_essentials() {
        let mut app = app();
        app.footer.provider = "deepseek".into();
        app.footer.context_percent = Some(42);
        app.footer.tokens = 1_200;
        app.footer.cost = 8_000_000;
        app.footer.tokens_per_sec = Some(136.0);
        app.footer.session = Some("0190f3a1".into());
        let buffer = buffer_at(&mut app, 100, 20);
        let row = (0..100)
            .map(|x| buffer[(x, 19)].symbol())
            .collect::<String>();
        assert!(row.contains("deepseek/deepseek-flash"), "{row}");
        assert!(row.contains("42%"), "{row}");
        assert!(row.contains("1.2k tok"), "{row}");
        assert!(row.contains("$0.0080"), "{row}");
        assert!(!row.contains("136 t/s"), "{row}");
        assert!(!row.contains("0190f3a1"), "{row}");
        assert!(!row.contains("medium"), "{row}");
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
            None,
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
    fn selected_cells_copy_as_trimmed_text() {
        let mut buffer = ratatui::buffer::Buffer::empty(Rect::new(0, 0, 12, 3));
        for (y, text) in ["hello world", "second line", ""].iter().enumerate() {
            for (x, character) in text.chars().enumerate() {
                buffer[(x as u16, y as u16)].set_symbol(&character.to_string());
            }
        }
        let selection = crate::app::Selection {
            anchor: (1, 6),
            cursor: (0, 2),
        };
        assert_eq!(selection_text(&buffer, selection), "llo world\nsecond");

        let trailing = crate::app::Selection {
            anchor: (2, 0),
            cursor: (0, 0),
        };
        assert_eq!(
            selection_text(&buffer, trailing),
            "hello world\nsecond line"
        );
    }

    #[test]
    fn selection_highlights_the_rendered_screen() {
        let mut app = app();
        app.transcript.push_user("hello");
        app.selection = Some(crate::app::Selection {
            anchor: (0, 0),
            cursor: (3, 10),
        });
        let buffer = buffer_at(&mut app, 80, 20);
        let reversed = buffer
            .content()
            .iter()
            .filter(|cell| cell.modifier.contains(Modifier::REVERSED))
            .count();
        assert!(reversed > 0, "the selection is not highlighted");
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
            agent: None,
            depth: 0,
            session_id: None,
            reply,
        });
        let text = buffer(&mut app);
        assert!(text.contains("Edit file.txt"), "{text}");
        assert!(text.contains("beta"), "{text}");
        assert!(text.contains("BETA"), "{text}");
    }

    #[test]
    #[spec("TUI-043")]
    fn footer_shows_the_delegation_mode() {
        let mut app = app();
        app.subagent_mode = codie_subagents::DelegationMode::Auto;
        let text = buffer(&mut app);
        assert!(text.contains("sub:auto"), "{text}");

        app.subagent_mode = codie_subagents::DelegationMode::Off;
        let text = buffer(&mut app);
        assert!(!text.contains("sub:auto"), "{text}");
    }

    #[test]
    #[spec("TUI-040")]
    fn task_rows_show_live_activity_and_result_details() {
        let mut app = app();
        app.transcript.tool_start(
            "call_1".into(),
            "task".into(),
            serde_json::json!({
                "description": "News feeds",
                "agent": "general",
                "prompt": "research"
            }),
        );
        app.transcript
            .tool_progress("call_1", "turn 2 · grep markets".into(), None);
        let text = buffer(&mut app);
        assert!(text.contains("Task · general · News feeds"), "{text}");
        assert!(text.contains("turn 2 · grep markets"), "{text}");

        app.transcript.tool_end(
            "call_1",
            "done".into(),
            false,
            None,
            Some(serde_json::json!({
                "status": "completed",
                "turns": 3,
                "usage": {"cost_usd": 0.0123},
                "session_id": "ses_child"
            })),
        );
        let text = buffer(&mut app);
        assert!(
            text.contains("completed · 3 turns · $0.0123 · ses_child"),
            "{text}"
        );
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
        app.transcript
            .tool_end("call_1", "ok".into(), false, None, None);
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
    fn collapsed_summary_refreshes_when_a_hidden_entry_changes() {
        let mut app = app();
        app.transcript.push_user("first question");
        app.transcript.tool_start(
            "call_1".into(),
            "bash".into(),
            serde_json::json!({"command": "cargo test"}),
        );
        app.transcript
            .tool_end("call_1", "ok".into(), false, None, None);
        app.transcript.push_user("second question");

        let collapsed = buffer(&mut app);
        assert!(collapsed.contains("1 tool"), "{collapsed}");

        app.transcript
            .tool_end("call_1", "failed".into(), true, None, None);
        let refreshed = buffer(&mut app);
        assert!(refreshed.contains("1 error"), "{refreshed}");
    }

    #[test]
    fn layout_cache_rebuilds_only_changed_entries() {
        let mut app = app();
        app.transcript.push_user("question");
        app.transcript.start_assistant();
        app.transcript.set_text("first".into());
        let _ = buffer(&mut app);

        let (user_before, assistant_before) = {
            let cache = app.transcript_cache.borrow();
            let cache = cache.as_ref().expect("cache");
            (cache.entries[0].key, cache.entries[1].key)
        };
        app.transcript.set_text("second".into());
        let text = buffer(&mut app);
        assert!(text.contains("second"), "{text}");

        let cache = app.transcript_cache.borrow();
        let cache = cache.as_ref().expect("cache");
        assert_eq!(cache.entries[0].key, user_before);
        assert_ne!(cache.entries[1].key, assistant_before);
    }

    #[test]
    fn scrolling_tracks_row_counts_after_an_old_entry_grows() {
        let mut app = app();
        for index in 0..20 {
            app.transcript.push_info(format!("before {index}"));
        }
        app.transcript.tool_start(
            "call_1".into(),
            "bash".into(),
            serde_json::json!({"command": "seq 100"}),
        );
        app.transcript.tool_end(
            "call_1",
            (0..100)
                .map(|line| format!("output {line}"))
                .collect::<Vec<_>>()
                .join("\n"),
            false,
            None,
            None,
        );
        for index in 0..20 {
            app.transcript.push_info(format!("after {index}"));
        }

        let _ = buffer(&mut app);
        let bottom_before = app.transcript.scroll();
        assert!(app.transcript.toggle_tool(20));
        let expanded = buffer(&mut app);
        let bottom_after = app.transcript.scroll();
        assert!(bottom_after > bottom_before);
        assert!(expanded.contains("after 19"), "{expanded}");

        app.transcript.scroll_up(u16::MAX);
        let top = buffer(&mut app);
        assert!(top.contains("before 0"), "{top}");
    }

    #[test]
    fn clearing_the_transcript_does_not_reuse_stale_rows() {
        let mut app = app();
        app.transcript.push_info("old line");
        let first = buffer(&mut app);
        assert!(first.contains("old line"), "{first}");

        app.transcript.clear();
        app.transcript.push_info("new line");
        let second = buffer(&mut app);
        assert!(second.contains("new line"), "{second}");
        assert!(!second.contains("old line"), "{second}");
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

    fn buffer_at(app: &mut App, width: u16, height: u16) -> ratatui::buffer::Buffer {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, app)).unwrap();
        terminal.backend().buffer().clone()
    }

    fn permission_app() -> App {
        let mut app = app();
        app.footer = Footer {
            cwd: "/tmp/project".into(),
            git_branch: Some("main".into()),
            model: "deepseek-flash".into(),
            thinking: "medium".into(),
            ..Footer::default()
        };
        app
    }

    #[test]
    #[spec("TUI-035")]
    fn spec_tui_035_connect_overlay_never_renders_a_masked_key() {
        let mut app = app();
        let mut connect = crate::app::ConnectState::new("openrouter");
        connect.input = "sk-or-very-secret".to_owned();
        app.overlay = Some(Overlay::Connect(connect));

        let text = buffer(&mut app);
        assert!(text.contains("connect provider"), "{text}");
        assert!(text.contains('•'), "{text}");
        assert!(!text.contains("sk-or-very-secret"), "{text}");
    }

    #[test]
    #[spec("TUI-025")]
    fn spec_tui_025_permission_overlay_hugs_content_and_pads_the_preview() {
        let mut app = permission_app();
        let (reply, _answer) = tokio::sync::oneshot::channel();
        app.push_permission(crate::permissions::PermissionRequest {
            tool: "bash".to_owned(),
            arguments: serde_json::json!({"command": "cargo test"}),
            agent: None,
            depth: 0,
            session_id: None,
            reply,
        });
        let text = buffer(&mut app);
        assert!(text.contains("Run this shell command?"), "{text}");
        assert!(text.contains("/tmp/project"), "{text}");
        assert!(text.contains("⑂ main"), "{text}");
        assert!(text.contains("$ cargo test"), "{text}");
        assert!(text.contains("[y] Allow once"), "{text}");
        assert!(text.contains("[A] Always allow"), "{text}");
        assert!(text.contains("[n] Deny"), "{text}");
        assert!(text.contains("Enter allow once"), "{text}");
        assert!(text.contains("full access"), "{text}");

        let palette = app.theme.palette;
        let buffer = buffer_at(&mut app, 80, 30);
        let row_text = |y: u16| (0..80).map(|x| buffer[(x, y)].symbol()).collect::<String>();
        let top = (0..30)
            .find(|y| row_text(*y).contains("permission · bash"))
            .expect("dialog title row");
        let bottom = (top..30)
            .find(|y| (0..80).any(|x| buffer[(x, *y)].symbol() == "╰"))
            .expect("dialog bottom border");
        let height = bottom - top + 1;
        assert!(
            height <= 12,
            "dialog is {height} rows tall, not hugging content"
        );

        let title = row_text(top);
        let left = title
            .chars()
            .position(|character| character == '╭')
            .expect("rounded left border") as u16;
        let right = title
            .chars()
            .position(|character| character == '╮')
            .expect("rounded right border") as u16;
        assert_eq!(
            right - left + 1,
            56,
            "short commands should not use a wide dialog"
        );
        let question = (top..bottom)
            .find(|y| row_text(*y).contains("Run this shell command?"))
            .expect("question row");
        let question_text = row_text(question);
        let question_column = question_text
            .find("Run this shell command?")
            .map(|index| question_text[..index].chars().count())
            .expect("question column") as u16;
        assert_eq!(
            question_column,
            left + 2,
            "content should have horizontal padding"
        );
        let command = (top..bottom)
            .find(|y| row_text(*y).contains("$ cargo test"))
            .expect("command row");
        assert_eq!(buffer[(left + 2, command)].bg, palette.surface);
        assert_eq!(buffer[(left + 53, command)].bg, palette.surface);
    }

    #[test]
    fn permission_overlay_colors_each_decision_and_dims_the_transcript() {
        let mut app = permission_app();
        let (reply, _answer) = tokio::sync::oneshot::channel();
        app.push_permission(crate::permissions::PermissionRequest {
            tool: "bash".to_owned(),
            arguments: serde_json::json!({"command": "cargo test"}),
            agent: None,
            depth: 0,
            session_id: None,
            reply,
        });
        let palette = crate::theme::Theme::load(Some("dark")).unwrap().palette;
        let buffer = buffer_at(&mut app, 100, 24);

        let count = |wanted: ratatui::style::Color| {
            buffer
                .content()
                .iter()
                .filter(|cell| cell.bg == wanted)
                .count()
        };
        assert!(count(palette.success) > 0, "allow once is not green");
        assert!(count(palette.warning) > 0, "always allow is not amber");
        assert!(count(palette.error) > 0, "deny is not red");

        let dimmed = buffer
            .content()
            .iter()
            .filter(|cell| cell.modifier.contains(Modifier::DIM))
            .count();
        assert!(dimmed > 0, "the transcript behind the dialog is not dimmed");
    }

    #[test]
    fn permission_overlay_warns_before_destructive_commands() {
        let mut app = permission_app();
        let (reply, _answer) = tokio::sync::oneshot::channel();
        app.push_permission(crate::permissions::PermissionRequest {
            tool: "bash".to_owned(),
            arguments: serde_json::json!({"command": "rm -rf target && cargo build"}),
            agent: None,
            depth: 0,
            session_id: None,
            reply,
        });
        let text = buffer(&mut app);
        assert!(text.contains("destructive"), "{text}");
        assert!(text.contains("review every line"), "{text}");
    }

    #[test]
    fn permission_overlay_scrolls_and_expands_long_previews() {
        let mut app = permission_app();
        let command = (0..80)
            .map(|index| format!("echo line-{index}"))
            .collect::<Vec<_>>()
            .join("\n");
        let (reply, _answer) = tokio::sync::oneshot::channel();
        app.push_permission(crate::permissions::PermissionRequest {
            tool: "bash".to_owned(),
            arguments: serde_json::json!({"command": command}),
            agent: None,
            depth: 0,
            session_id: None,
            reply,
        });
        let text = buffer(&mut app);
        assert!(text.contains("Ctrl+O full"), "{text}");
        assert!(text.contains("↑↓ scroll"), "{text}");

        let before = buffer(&mut app);
        app.handle_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
        let after = buffer(&mut app);
        assert_ne!(before, after, "scrolling did not move the preview");

        app.handle_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL));
        let expanded = buffer(&mut app);
        assert!(expanded.contains("Ctrl+O collapse"), "{expanded}");
    }

    #[test]
    fn permission_overlay_renders() {
        let mut app = app();
        let (reply, _answer) = tokio::sync::oneshot::channel();
        app.push_permission(crate::permissions::PermissionRequest {
            tool: "bash".to_owned(),
            arguments: serde_json::json!({"command": "cargo test"}),
            agent: None,
            depth: 0,
            session_id: None,
            reply,
        });
        let text = buffer(&mut app);
        assert!(text.contains("permission"));
        assert!(text.contains("[y] Allow once"));
        assert!(text.contains("[n] Deny"));
        assert!(text.contains("full access"));
        assert!(text.contains("cargo test"));
    }
}
