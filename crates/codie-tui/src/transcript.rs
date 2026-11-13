//! Transcript entries and tool blocks (`TUI-005`, `TUI-006`).

use std::time::{Duration, Instant};

use serde_json::Value;

/// State of one tool call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolStatus {
    /// The call is executing.
    Running,
    /// The call returned successfully.
    Success,
    /// The call failed, was denied, or panicked.
    Error,
}

/// A stateful tool block.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolBlock {
    /// Provider call identifier.
    pub id: String,
    /// Tool name.
    pub name: String,
    /// Parsed arguments.
    pub arguments: Value,
    /// Current state.
    pub status: ToolStatus,
    /// Model-visible output, when finished.
    pub output: String,
    /// Optional structured render hint from an extension.
    pub render: Option<codie_protocol::ext::ExtRenderHint>,
    /// Whether the full output is visible.
    pub expanded: bool,
    /// When the call started, for the live elapsed indicator.
    pub started: Option<Instant>,
}

impl ToolBlock {
    /// Returns a one-line summary for the collapsed view.
    #[must_use]
    pub fn summary(&self) -> String {
        let arguments = self
            .arguments
            .as_object()
            .and_then(|object| {
                object
                    .get("path")
                    .or_else(|| object.get("command"))
                    .and_then(Value::as_str)
            })
            .unwrap_or_default();
        if arguments.is_empty() {
            self.name.clone()
        } else {
            format!("{} {}", self.name, truncate(arguments, 60))
        }
    }

    /// Returns how long the call has been running.
    #[must_use]
    pub fn elapsed(&self) -> Option<Duration> {
        self.started.map(|started| started.elapsed())
    }

    /// Returns a compact elapsed label such as `12s` or `1m 03s`.
    #[must_use]
    pub fn elapsed_label(&self) -> Option<String> {
        let elapsed = self.elapsed()?;
        Some(format_elapsed(elapsed))
    }
}

/// Formats a duration as `Ns`, `Nm SSs`, or `Nh MMm`.
fn format_elapsed(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs();
    if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 3600 {
        format!("{}m {:02}s", seconds / 60, seconds % 60)
    } else {
        format!("{}h {:02}m", seconds / 3600, (seconds % 3600) / 60)
    }
}

/// One assistant turn being streamed.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AssistantEntry {
    /// Visible text.
    pub text: String,
    /// Reasoning text.
    pub thinking: String,
    /// True while the provider stream is live.
    pub streaming: bool,
}

/// One transcript entry.
#[derive(Debug, Clone, PartialEq)]
pub enum Entry {
    /// A user message.
    User(String),
    /// An assistant message, possibly streaming.
    Assistant(AssistantEntry),
    /// A tool call block.
    Tool(ToolBlock),
    /// Informational system text.
    Info(String),
    /// An error.
    Error(String),
}

/// The scrolling transcript.
#[derive(Debug, Clone, Default)]
pub struct Transcript {
    entries: Vec<Entry>,
    scroll: u16,
    follow: bool,
    max_scroll: u16,
}

impl Transcript {
    /// Creates an empty transcript that follows new output.
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            scroll: 0,
            follow: true,
            max_scroll: 0,
        }
    }

    /// Returns the entries.
    #[must_use]
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Returns the current scroll offset in lines.
    #[must_use]
    pub const fn scroll(&self) -> u16 {
        self.scroll
    }

    /// Returns true when the view follows new output.
    #[must_use]
    pub const fn is_following(&self) -> bool {
        self.follow
    }

    /// Adds a user message.
    pub fn push_user(&mut self, text: impl Into<String>) {
        self.entries.push(Entry::User(text.into()));
        self.follow = true;
    }

    /// Adds an informational line.
    pub fn push_info(&mut self, text: impl Into<String>) {
        self.entries.push(Entry::Info(text.into()));
        self.follow = true;
    }

    /// Adds an error line.
    pub fn push_error(&mut self, text: impl Into<String>) {
        self.entries.push(Entry::Error(text.into()));
        self.follow = true;
    }

    /// Starts a streaming assistant entry and returns its index.
    pub fn start_assistant(&mut self) -> usize {
        self.entries.push(Entry::Assistant(AssistantEntry {
            streaming: true,
            ..AssistantEntry::default()
        }));
        self.follow = true;
        self.entries.len() - 1
    }

    /// Appends visible text to the streaming assistant entry.
    pub fn append_text(&mut self, text: &str) {
        if let Some(Entry::Assistant(assistant)) = self.entries.last_mut() {
            assistant.text.push_str(text);
            self.follow = true;
        }
    }

    /// Sets the visible text of the streaming assistant entry.
    pub fn set_text(&mut self, text: String) {
        if let Some(Entry::Assistant(assistant)) = self.entries.last_mut() {
            assistant.text = text;
            self.follow = true;
        }
    }

    /// Appends reasoning text to the streaming assistant entry.
    pub fn append_thinking(&mut self, text: &str) {
        if let Some(Entry::Assistant(assistant)) = self.entries.last_mut() {
            assistant.thinking.push_str(text);
            self.follow = true;
        }
    }

    /// Sets the reasoning text of the streaming assistant entry.
    pub fn set_thinking(&mut self, text: String) {
        if let Some(Entry::Assistant(assistant)) = self.entries.last_mut() {
            assistant.thinking = text;
            self.follow = true;
        }
    }

    /// Marks the streaming assistant entry as finished.
    pub fn finish_assistant(&mut self) {
        if let Some(Entry::Assistant(assistant)) = self.entries.last_mut() {
            assistant.streaming = false;
        }
    }

    /// Starts a tool block.
    pub fn tool_start(&mut self, id: String, name: String, arguments: Value) {
        self.entries.push(Entry::Tool(ToolBlock {
            id,
            name,
            arguments,
            status: ToolStatus::Running,
            output: String::new(),
            render: None,
            expanded: false,
            started: Some(Instant::now()),
        }));
        self.follow = true;
    }

    /// Returns the most recent tool that is still running.
    #[must_use]
    pub fn running_tool(&self) -> Option<&ToolBlock> {
        self.entries.iter().rev().find_map(|entry| match entry {
            Entry::Tool(block) if block.status == ToolStatus::Running => Some(block),
            _ => None,
        })
    }

    /// Finishes a tool block by id.
    pub fn tool_end(
        &mut self,
        id: &str,
        output: String,
        is_error: bool,
        render: Option<codie_protocol::ext::ExtRenderHint>,
    ) {
        if let Some(block) = self.entries.iter_mut().rev().find_map(|entry| match entry {
            Entry::Tool(block) if block.id == id => Some(block),
            _ => None,
        }) {
            block.status = if is_error {
                ToolStatus::Error
            } else {
                ToolStatus::Success
            };
            block.output = output;
            block.render = render;
            self.follow = true;
        }
    }

    /// Toggles the most recent tool block.
    pub fn toggle_last_tool(&mut self) -> bool {
        for entry in self.entries.iter_mut().rev() {
            if let Entry::Tool(block) = entry {
                block.expanded = !block.expanded;
                return true;
            }
        }
        false
    }

    /// Scrolls up by `lines`, leaving follow mode.
    pub fn scroll_up(&mut self, lines: u16) {
        self.scroll = self.scroll.saturating_sub(lines);
        self.follow = false;
    }

    /// Scrolls down by `lines`, resuming follow mode at the bottom.
    pub fn scroll_down(&mut self, lines: u16) {
        self.scroll = self.scroll.saturating_add(lines).min(self.max_scroll);
        if self.scroll >= self.max_scroll {
            self.scroll = self.max_scroll;
            self.follow = true;
        }
    }

    /// Resumes following new output.
    pub fn scroll_to_bottom(&mut self) {
        self.follow = true;
        self.scroll = 0;
    }

    /// Records the content height and clamps the offset.
    ///
    /// Following means the view sticks to the newest output. Scrolling to the
    /// bottom resumes follow mode.
    pub fn clamp_scroll(&mut self, maximum: u16) {
        self.max_scroll = maximum;
        if self.follow {
            self.scroll = maximum;
        } else if self.scroll >= maximum {
            self.scroll = maximum;
            self.follow = true;
        }
    }

    /// Clears every entry.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.scroll = 0;
        self.follow = true;
        self.max_scroll = 0;
    }

    /// Returns true when there is no content.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

fn truncate(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_owned();
    }
    let head: String = text.chars().take(limit).collect();
    format!("{head}…")
}

#[cfg(test)]
mod tests {
    use super::*;
    use codie_testkit::spec;
    use serde_json::json;

    #[test]
    fn streaming_text_accumulates() {
        let mut transcript = Transcript::new();
        transcript.start_assistant();
        transcript.append_text("hel");
        transcript.append_text("lo");
        transcript.finish_assistant();
        match &transcript.entries()[0] {
            Entry::Assistant(assistant) => {
                assert_eq!(assistant.text, "hello");
                assert!(!assistant.streaming);
            }
            other => panic!("unexpected entry: {other:?}"),
        }
    }

    #[test]
    #[spec("TUI-020")]
    fn running_tool_reports_progress_until_it_finishes() {
        let mut transcript = Transcript::new();
        transcript.tool_start(
            "call_1".into(),
            "bash".into(),
            json!({"command": "cargo test"}),
        );
        let running = transcript.running_tool().expect("running tool");
        assert_eq!(running.name, "bash");
        assert!(running.elapsed_label().is_some());

        transcript.tool_end("call_1", "ok".into(), false, None);
        assert!(transcript.running_tool().is_none());
    }

    #[test]
    #[spec("TUI-005")]
    fn tool_blocks_transition_and_expand() {
        let mut transcript = Transcript::new();
        transcript.tool_start(
            "call_1".into(),
            "edit".into(),
            json!({"path": "src/lib.rs"}),
        );
        transcript.tool_end("call_1", "ok".into(), false, None);
        assert!(matches!(
            &transcript.entries()[0],
            Entry::Tool(block)
                if block.status == ToolStatus::Success && block.output == "ok"
        ));
        assert!(transcript.toggle_last_tool());
        assert!(matches!(
            &transcript.entries()[0],
            Entry::Tool(block) if block.expanded
        ));
    }

    #[test]
    fn scrolling_leaves_follow_mode() {
        let mut transcript = Transcript::new();
        transcript.push_info("one");
        transcript.scroll_up(3);
        assert!(!transcript.is_following());
        transcript.scroll_to_bottom();
        assert!(transcript.is_following());
    }
}
