//! Multiline editor with grapheme-correct movement (`TUI-003`).

use std::sync::OnceLock;

use regex::Regex;
use unicode_segmentation::UnicodeSegmentation;

/// A cursor position: line index and byte offset within that line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Cursor {
    /// Zero-based line index.
    pub row: usize,
    /// Byte offset on a grapheme boundary.
    pub col: usize,
}

#[derive(Debug, Clone)]
struct Snapshot {
    lines: Vec<String>,
    cursor: Cursor,
}

/// Maximum characters inserted inline before a paste is collapsed.
pub const PASTE_COLLAPSE_CHARS: usize = 2_000;

/// A multiline text editor.
#[derive(Debug, Clone)]
pub struct Editor {
    lines: Vec<String>,
    cursor: Cursor,
    history: Vec<String>,
    history_index: Option<usize>,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    kill_ring: String,
    pastes: Vec<String>,
}

impl Default for Editor {
    fn default() -> Self {
        Self::new()
    }
}

impl Editor {
    /// Creates an empty editor with one empty line.
    #[must_use]
    pub fn new() -> Self {
        Self {
            lines: vec![String::new()],
            cursor: Cursor::default(),
            history: Vec::new(),
            history_index: None,
            undo: Vec::new(),
            redo: Vec::new(),
            kill_ring: String::new(),
            pastes: Vec::new(),
        }
    }

    /// Returns the current cursor.
    #[must_use]
    pub const fn cursor(&self) -> Cursor {
        self.cursor
    }

    /// Returns the lines for rendering.
    #[must_use]
    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    /// Returns true when the editor has no visible text.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.lines.len() == 1 && self.lines[0].is_empty()
    }

    /// Returns the raw text, including paste markers.
    #[must_use]
    pub fn raw_text(&self) -> String {
        self.lines.join("\n")
    }

    /// Returns the submitted text with paste markers expanded.
    #[must_use]
    pub fn text(&self) -> String {
        self.expand_pastes(&self.raw_text())
    }

    /// Replaces the editor contents and resets undo history.
    pub fn set_text(&mut self, text: &str) {
        self.lines = text.split('\n').map(str::to_owned).collect();
        if self.lines.is_empty() {
            self.lines.push(String::new());
        }
        self.cursor = Cursor {
            row: self.lines.len() - 1,
            col: self.lines[self.lines.len() - 1].len(),
        };
        self.undo.clear();
        self.redo.clear();
        self.history_index = None;
    }

    /// Clears the editor and its paste buffer.
    pub fn clear(&mut self) {
        self.lines = vec![String::new()];
        self.cursor = Cursor::default();
        self.pastes.clear();
        self.history_index = None;
    }

    /// Inserts one character at the cursor.
    pub fn insert_char(&mut self, character: char) {
        self.checkpoint();
        let col = self.cursor.col;
        self.lines[self.cursor.row].insert(col, character);
        self.cursor.col += character.len_utf8();
        self.history_index = None;
    }

    /// Inserts a string, splitting on newlines.
    pub fn insert_str(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.checkpoint();
        for (index, segment) in text.split('\n').enumerate() {
            if index > 0 {
                self.newline_inner();
            }
            let col = self.cursor.col;
            self.lines[self.cursor.row].insert_str(col, segment);
            self.cursor.col += segment.len();
        }
        self.history_index = None;
    }

    /// Inserts a newline at the cursor.
    pub fn insert_newline(&mut self) {
        self.checkpoint();
        self.newline_inner();
        self.history_index = None;
    }

    fn newline_inner(&mut self) {
        let col = self.cursor.col;
        let tail = self.lines[self.cursor.row].split_off(col);
        self.cursor.row += 1;
        self.lines.insert(self.cursor.row, tail);
        self.cursor.col = 0;
    }

    /// Deletes the grapheme before the cursor.
    pub fn backspace(&mut self) {
        if self.cursor.col == 0 {
            if self.cursor.row == 0 {
                return;
            }
            self.checkpoint();
            let line = self.lines.remove(self.cursor.row);
            let previous = self.cursor.row - 1;
            let previous_len = self.lines[previous].len();
            self.lines[previous].push_str(&line);
            self.cursor.row = previous;
            self.cursor.col = previous_len;
            self.history_index = None;
            return;
        }

        self.checkpoint();
        let line = &mut self.lines[self.cursor.row];
        let start = previous_boundary(line, self.cursor.col);
        line.replace_range(start..self.cursor.col, "");
        self.cursor.col = start;
        self.history_index = None;
    }

    /// Deletes the grapheme after the cursor.
    pub fn delete_forward(&mut self) {
        let line_len = self.lines[self.cursor.row].len();
        if self.cursor.col < line_len {
            self.checkpoint();
            let line = &mut self.lines[self.cursor.row];
            let end = next_boundary(line, self.cursor.col);
            line.replace_range(self.cursor.col..end, "");
            self.history_index = None;
            return;
        }
        if self.cursor.row + 1 < self.lines.len() {
            self.checkpoint();
            let next = self.lines.remove(self.cursor.row + 1);
            self.lines[self.cursor.row].push_str(&next);
            self.history_index = None;
        }
    }

    /// Moves one grapheme left.
    pub fn move_left(&mut self) {
        if self.cursor.col > 0 {
            self.cursor.col = previous_boundary(&self.lines[self.cursor.row], self.cursor.col);
        } else if self.cursor.row > 0 {
            self.cursor.row -= 1;
            self.cursor.col = self.lines[self.cursor.row].len();
        }
        self.history_index = None;
    }

    /// Moves one grapheme right.
    pub fn move_right(&mut self) {
        let line = &self.lines[self.cursor.row];
        if self.cursor.col < line.len() {
            self.cursor.col = next_boundary(line, self.cursor.col);
        } else if self.cursor.row + 1 < self.lines.len() {
            self.cursor.row += 1;
            self.cursor.col = 0;
        }
        self.history_index = None;
    }

    /// Moves up one line, clamping the column.
    pub fn move_up(&mut self) {
        if self.cursor.row == 0 {
            self.cursor.col = 0;
            return;
        }
        self.cursor.row -= 1;
        self.cursor.col = clamp_col(&self.lines[self.cursor.row], self.cursor.col);
        self.history_index = None;
    }

    /// Moves down one line, clamping the column.
    pub fn move_down(&mut self) {
        if self.cursor.row + 1 >= self.lines.len() {
            self.cursor.col = self.lines[self.cursor.row].len();
            return;
        }
        self.cursor.row += 1;
        self.cursor.col = clamp_col(&self.lines[self.cursor.row], self.cursor.col);
        self.history_index = None;
    }

    /// Moves to the start of the line.
    pub fn home(&mut self) {
        self.cursor.col = 0;
        self.history_index = None;
    }

    /// Moves to the end of the line.
    pub fn end(&mut self) {
        self.cursor.col = self.lines[self.cursor.row].len();
        self.history_index = None;
    }

    /// Deletes from the cursor to the end of the line and stores it in the kill ring.
    pub fn kill_to_end(&mut self) {
        self.checkpoint();
        let col = self.cursor.col;
        self.kill_ring = self.lines[self.cursor.row].split_off(col);
        self.history_index = None;
    }

    /// Inserts the kill ring at the cursor.
    pub fn yank(&mut self) {
        if self.kill_ring.is_empty() {
            return;
        }
        let kill = self.kill_ring.clone();
        self.insert_str(&kill);
    }

    /// Undoes the last edit.
    pub fn undo(&mut self) {
        let Some(snapshot) = self.undo.pop() else {
            return;
        };
        self.redo.push(self.snapshot());
        self.lines = snapshot.lines;
        self.cursor = snapshot.cursor;
    }

    /// Redoes the last undone edit.
    pub fn redo(&mut self) {
        let Some(snapshot) = self.redo.pop() else {
            return;
        };
        self.undo.push(self.snapshot());
        self.lines = snapshot.lines;
        self.cursor = snapshot.cursor;
    }

    /// Inserts pasted text, collapsing very large pastes into a marker.
    pub fn paste(&mut self, text: &str) {
        let characters = text.chars().count();
        if characters > PASTE_COLLAPSE_CHARS {
            self.pastes.push(text.to_owned());
            let marker = format!("[paste #{}: {characters} chars]", self.pastes.len());
            self.insert_str(&marker);
        } else {
            self.insert_str(text);
        }
    }

    /// Replaces the text between `start_col` and the cursor on the current line.
    pub fn replace_token(&mut self, start_col: usize, replacement: &str) {
        self.checkpoint();
        let col = self.cursor.col.min(self.lines[self.cursor.row].len());
        let start = start_col.min(col);
        self.lines[self.cursor.row].replace_range(start..col, replacement);
        self.cursor.col = start + replacement.len();
        self.history_index = None;
    }

    /// Recalls the previous submitted prompt.
    pub fn history_previous(&mut self) {
        if self.history.is_empty() {
            return;
        }
        let index = match self.history_index {
            Some(index) if index > 0 => index - 1,
            Some(index) => index,
            None => self.history.len() - 1,
        };
        self.history_index = Some(index);
        self.lines = self.history[index].split('\n').map(str::to_owned).collect();
        self.cursor.row = self.lines.len() - 1;
        self.cursor.col = self.lines[self.cursor.row].len();
    }

    /// Recalls the next submitted prompt, or clears after the newest one.
    pub fn history_next(&mut self) {
        let Some(index) = self.history_index else {
            return;
        };
        if index + 1 >= self.history.len() {
            self.history_index = None;
            self.lines = vec![String::new()];
            self.cursor = Cursor::default();
            return;
        }
        let index = index + 1;
        self.history_index = Some(index);
        self.lines = self.history[index].split('\n').map(str::to_owned).collect();
        self.cursor.row = self.lines.len() - 1;
        self.cursor.col = self.lines[self.cursor.row].len();
    }

    /// Takes the submitted text with paste markers expanded.
    pub fn submit(&mut self) -> String {
        let text = self.text();
        if !text.trim().is_empty() {
            self.history.push(text.clone());
            if self.history.len() > 100 {
                self.history.remove(0);
            }
        }
        self.clear();
        text
    }

    fn checkpoint(&mut self) {
        self.undo.push(self.snapshot());
        if self.undo.len() > 200 {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            lines: self.lines.clone(),
            cursor: self.cursor,
        }
    }

    fn expand_pastes(&self, text: &str) -> String {
        if self.pastes.is_empty() {
            return text.to_owned();
        }
        static MARKER: OnceLock<Regex> = OnceLock::new();
        let marker = MARKER.get_or_init(|| {
            Regex::new(r"\[paste #(\d+): \d+ chars\]").expect("paste marker regex")
        });
        marker
            .replace_all(text, |captures: &regex::Captures<'_>| {
                captures
                    .get(1)
                    .and_then(|index| index.as_str().parse::<usize>().ok())
                    .and_then(|index| self.pastes.get(index.wrapping_sub(1)))
                    .cloned()
                    .unwrap_or_else(|| captures[0].to_owned())
            })
            .into_owned()
    }
}

fn previous_boundary(line: &str, col: usize) -> usize {
    line[..col]
        .grapheme_indices(true)
        .next_back()
        .map_or(0, |(index, _)| index)
}

fn next_boundary(line: &str, col: usize) -> usize {
    line[col..]
        .graphemes(true)
        .next()
        .map_or(line.len(), |grapheme| col + grapheme.len())
}

fn clamp_col(line: &str, col: usize) -> usize {
    let col = col.min(line.len());
    let mut boundary = 0;
    for (index, grapheme) in line.grapheme_indices(true) {
        if index > col {
            break;
        }
        boundary = if index + grapheme.len() <= col {
            index + grapheme.len()
        } else {
            index
        };
    }
    boundary
}

#[cfg(test)]
mod tests {
    use super::*;
    use codie_testkit::spec;

    #[test]
    fn typing_and_newlines_build_multiline_text() {
        let mut editor = Editor::new();
        editor.insert_str("hello");
        editor.insert_newline();
        editor.insert_str("world");
        assert_eq!(editor.text(), "hello\nworld");
        assert_eq!(editor.cursor(), Cursor { row: 1, col: 5 });
    }

    #[test]
    fn backspace_merges_lines() {
        let mut editor = Editor::new();
        editor.set_text("ab\ncd");
        editor.home();
        editor.backspace();
        assert_eq!(editor.text(), "abcd");
        assert_eq!(editor.cursor(), Cursor { row: 0, col: 2 });
    }

    #[test]
    #[spec("TUI-003")]
    fn movement_is_grapheme_correct() {
        let mut editor = Editor::new();
        editor.set_text("a👨‍👩‍👧b");
        editor.home();
        editor.move_right();
        assert_eq!(editor.cursor().col, 1);
        editor.move_right();
        assert_eq!(editor.cursor().col, "a👨‍👩‍👧".len());
        editor.move_left();
        assert_eq!(editor.cursor().col, 1);
    }

    #[test]
    #[spec("TUI-003")]
    fn undo_and_redo_restore_snapshots() {
        let mut editor = Editor::new();
        editor.insert_str("one");
        editor.insert_str(" two");
        assert_eq!(editor.text(), "one two");
        editor.undo();
        assert_eq!(editor.text(), "one");
        editor.redo();
        assert_eq!(editor.text(), "one two");
    }

    #[test]
    fn kill_and_yank_round_trip() {
        let mut editor = Editor::new();
        editor.set_text("hello world");
        editor.home();
        editor.move_right();
        editor.move_right();
        editor.move_right();
        editor.move_right();
        editor.move_right();
        editor.kill_to_end();
        assert_eq!(editor.text(), "hello");
        editor.end();
        editor.yank();
        assert_eq!(editor.text(), "hello world");
    }

    #[test]
    #[spec("TUI-003")]
    fn large_pastes_collapse_and_expand_on_submit() {
        let mut editor = Editor::new();
        let paste = "x".repeat(PASTE_COLLAPSE_CHARS + 1);
        editor.paste(&paste);
        assert!(editor.raw_text().contains("[paste #1:"));
        assert_eq!(editor.submit(), paste);
    }

    #[test]
    #[spec("TUI-003")]
    fn history_recalls_submitted_prompts() {
        let mut editor = Editor::new();
        editor.insert_str("first");
        editor.submit();
        editor.insert_str("second");
        editor.submit();
        editor.history_previous();
        assert_eq!(editor.text(), "second");
        editor.history_previous();
        assert_eq!(editor.text(), "first");
        editor.history_next();
        assert_eq!(editor.text(), "second");
    }
}
