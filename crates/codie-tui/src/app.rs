//! Application state, key handling, and the interactive run loop.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use codie_ai::{
    AuthStore, CredentialSource, ProbeOutcome, ProviderAuthStatus, auth_status, probe_key,
    provider_name,
};
use codie_core::{
    Agent, AgentConfig, AgentEvent, ChatClient, Content, Message, ModelRef, RunStatus,
    ThinkingLevel, ToolResultMessage, Toolbox, Usage,
};
use codie_prompt::{
    Customizations, PromptInput, build_system_message, load_project_context, load_project_memory,
};
use codie_session::{Session, SessionSummary, TreeEntry, estimate_message_tokens};
use codie_subagents::{
    DelegationIntent, DelegationMode, FORCED_DIRECTIVE, NudgeState, RunSnapshot, SubagentConfig,
    SubagentRuntime, detect_intent, effective_mode, mode_directive,
};
use codie_tools::Registry;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{
    DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture, Event,
    MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::text::Line;
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};

use crate::commands::{Command, parse as parse_command, parse_thinking};
use crate::editor::Editor;
use crate::keybindings::{ActionId, Bindings};
use crate::permissions::{Decision, PermissionBroker, PermissionRequest};
use crate::plain;
use crate::render;
use crate::theme::Theme;
use crate::transcript::{Entry, Transcript};

/// Rendering mode for interactive runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TuiRunMode {
    /// Alternate screen with a fixed editor dock.
    Full,
    /// Non-alternate-screen rendering.
    Inline,
    /// Append-only output for screen readers.
    Plain,
}

/// One model available in the interactive picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelChoice {
    /// Provider identifier.
    pub provider: String,
    /// Model identifier.
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// Context window in tokens.
    pub context_window: u32,
}

/// One provider offered by the `/provider` picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderChoice {
    /// Provider identifier used on the wire.
    pub id: String,
    /// Human-readable provider name.
    pub name: String,
}

/// The state returned after a successful provider switch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderRuntime {
    /// Models offered by the newly active provider.
    pub models: Vec<ModelChoice>,
    /// Model to select after the switch.
    pub default_model: String,
}

/// Reloads the picker models for one provider.
pub type ModelsLoader = Arc<dyn Fn(&str) -> Vec<ModelChoice> + Send + Sync>;

/// Switches the active provider client and returns its runtime state.
pub type ProviderSwitcher = Arc<dyn Fn(&str) -> Result<ProviderRuntime, String> + Send + Sync>;

/// Persists the last provider and model selected in the TUI.
pub type SelectionPersister = Arc<dyn Fn(&str, &str) + Send + Sync>;

/// One selectable row in a list overlay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChoiceItem {
    /// Primary text.
    pub label: String,
    /// Secondary text shown after the label.
    pub detail: String,
    /// Value applied when the row is selected.
    pub value: String,
}

/// What selecting a list row does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChoiceKind {
    /// Slash commands.
    Commands,
    /// Models for the active provider.
    Models,
    /// Supported providers.
    Providers,
    /// Providers offered by the connect flow.
    ConnectProviders,
    /// Built-in themes.
    Themes,
    /// Reasoning levels.
    Thinking,
    /// Tool blocks in the transcript.
    Tools,
}

/// One file attached through the editor and shown as a compact label.
///
/// The editor holds `[Image 1]` rather than the full path; the label is replaced
/// with an `@path` token when the prompt is submitted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attachment {
    /// Text shown in the editor, such as `[Image 1]`.
    pub label: String,
    /// Absolute path the label resolves to.
    pub path: PathBuf,
}

/// A filterable, scrollable list overlay.
#[derive(Debug, Clone)]
pub struct ChoiceList {
    /// Overlay title.
    pub title: String,
    /// Selection behavior.
    pub kind: ChoiceKind,
    /// Every item.
    pub items: Vec<ChoiceItem>,
    /// Indices matching the query, in display order.
    pub visible: Vec<usize>,
    /// Selected position within `visible`.
    pub selected: usize,
    /// Filter text.
    pub query: String,
    /// Footer hint.
    pub hint: String,
}

impl ChoiceList {
    /// Builds a list and applies the empty query.
    #[must_use]
    pub fn new(
        kind: ChoiceKind,
        title: impl Into<String>,
        items: Vec<ChoiceItem>,
        hint: impl Into<String>,
    ) -> Self {
        let mut list = Self {
            title: title.into(),
            kind,
            items,
            visible: Vec::new(),
            selected: 0,
            query: String::new(),
            hint: hint.into(),
        };
        list.refresh();
        list
    }

    /// Recomputes visible rows after a query change.
    pub fn refresh(&mut self) {
        let query = self.query.to_lowercase();
        self.visible = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| {
                query.is_empty()
                    || item.label.to_lowercase().contains(&query)
                    || item.detail.to_lowercase().contains(&query)
            })
            .map(|(index, _)| index)
            .collect();
        self.selected = self.selected.min(self.visible.len().saturating_sub(1));
    }

    /// Returns the selected item.
    #[must_use]
    pub fn current(&self) -> Option<&ChoiceItem> {
        self.visible
            .get(self.selected)
            .and_then(|index| self.items.get(*index))
    }

    /// Moves the selection by `delta`, clamped to the list.
    pub fn move_by(&mut self, delta: isize) {
        if self.visible.is_empty() {
            self.selected = 0;
            return;
        }
        let next = self.selected as isize + delta;
        self.selected = next.clamp(0, self.visible.len() as isize - 1) as usize;
    }
}

/// Progress of one provider key check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectPhase {
    /// The user is typing or pasting a key.
    Editing,
    /// A probe is in flight.
    Checking,
    /// The last probe failed; the message is safe to display.
    Error(String),
}

/// State for the `/connect` key-entry overlay.
#[derive(Clone, PartialEq, Eq)]
pub struct ConnectState {
    /// Provider identifier.
    pub provider: String,
    /// Human-readable provider name.
    pub provider_name: String,
    /// Current key buffer. Never rendered unless `reveal` is true.
    pub input: String,
    /// True when the key is shown instead of masked.
    pub reveal: bool,
    /// Current phase.
    pub phase: ConnectPhase,
    /// Attempt counter used to ignore stale probe results.
    pub generation: u64,
}

impl ConnectState {
    /// Creates an editing state for `provider`.
    #[must_use]
    pub fn new(provider: impl Into<String>) -> Self {
        let provider = provider.into();
        let name = provider_name(&provider).to_owned();
        Self {
            provider,
            provider_name: name,
            input: String::new(),
            reveal: false,
            phase: ConnectPhase::Editing,
            generation: 0,
        }
    }

    /// Returns the masked or revealed key display.
    #[must_use]
    pub fn display(&self) -> String {
        if self.input.is_empty() {
            return "—".to_owned();
        }
        if self.reveal {
            return self.input.clone();
        }
        "•".repeat(self.input.chars().count())
    }
}

impl std::fmt::Debug for ConnectState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ConnectState")
            .field("provider", &self.provider)
            .field("provider_name", &self.provider_name)
            .field("input", &"<redacted>")
            .field("reveal", &self.reveal)
            .field("phase", &self.phase)
            .field("generation", &self.generation)
            .finish()
    }
}

/// Everything the interactive app needs from the CLI.
pub struct TuiOptions {
    /// Provider client.
    pub client: Arc<dyn ChatClient>,
    /// Working directory.
    pub cwd: PathBuf,
    /// Model identity.
    pub model: ModelRef,
    /// Context window in tokens for the selected model.
    pub context_window: u32,
    /// Models offered by the picker.
    pub models: Vec<ModelChoice>,
    /// Reloads the picker models when `/model` opens, so a completed catalog
    /// refresh is visible without restarting.
    pub models_loader: Option<ModelsLoader>,
    /// Providers offered by the `/provider` picker.
    pub providers: Vec<ProviderChoice>,
    /// Switches the active provider client and returns its models.
    pub switch_provider: Option<ProviderSwitcher>,
    /// Initial thinking level.
    pub thinking: ThinkingLevel,
    /// Maximum assistant requests per run.
    pub max_turns: usize,
    /// Resolved subagent configuration.
    pub subagents: SubagentConfig,
    /// Session delegation mode.
    pub subagent_mode: DelegationMode,
    /// Force delegation for the next prompt only.
    pub subagent_force_once: bool,
    /// Loaded session, or `None` for `--no-session`.
    pub session: Option<Session>,
    /// Conversation history projected from the session.
    pub history: Vec<Message>,
    /// Optional first prompt submitted as soon as the UI starts.
    pub initial_prompt: Option<String>,
    /// Tools that never prompt.
    pub auto_allowed: Vec<String>,
    /// Theme name or TOML path.
    pub theme: Option<String>,
    /// Rendering mode.
    pub mode: TuiRunMode,
    /// Session root, used by `/new`.
    pub session_dir: PathBuf,
    /// Show the session picker on startup (`--resume` with no identifier).
    pub resume_picker: bool,
    /// Exit code observed from a signal, if any.
    pub signal: Arc<std::sync::atomic::AtomicI32>,
    /// Tool registry selected by the CLI.
    pub registry: Arc<Registry>,
    /// Optional keybindings file.
    pub keybindings: Option<PathBuf>,
    /// Replacement system preamble, or `None` for the default.
    pub preamble: Option<String>,
    /// Extra system instructions appended after the preamble.
    pub append: Option<String>,
    /// Skills and templates available to slash commands.
    pub customizations: Customizations,
    /// Loaded out-of-process extensions.
    pub extensions: Arc<codie_ext::ExtensionSet>,
    /// Path to `auth.json`, used by the connect overlay.
    pub auth_path: PathBuf,
    /// Optional provider base URL override used for key checks.
    pub base_url: Option<String>,
    /// Provider whose key came from `--api-key` for this run.
    pub explicit_key_provider: Option<String>,
    /// Open the connect overlay on startup because no provider is configured.
    pub startup_connect: bool,
    /// Persists the last provider/model selection.
    pub persist_selection: Option<SelectionPersister>,
}

/// Live status shown in the editor border.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Waiting for input.
    Idle,
    /// A model request or tool call is running.
    Streaming,
    /// Waiting for a permission decision.
    Waiting,
    /// The last run failed.
    Error,
}

/// A transcript element that responds to a mouse click.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HitTarget {
    /// The reasoning header for one assistant entry.
    Thinking(usize),
    /// One tool block header, by transcript entry index.
    Tool(usize),
    /// One user-turn header, by transcript entry index.
    Turn(usize),
    /// The footer row.
    Usage,
}

/// One clickable screen row recorded by the last frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Hit {
    /// Absolute terminal row.
    pub row: u16,
    /// What the row toggles.
    pub target: HitTarget,
}

/// Identity of one cached rich tool body.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct ToolViewKey {
    /// Tool call identifier.
    pub id: String,
    /// Transcript width in columns.
    pub width: u16,
    /// Whether the full body is shown.
    pub expanded: bool,
    /// Bumped whenever the palette changes.
    pub theme_revision: u64,
}

/// A rendered `edit` diff or `write` body.
#[derive(Debug, Clone)]
pub(crate) struct CachedToolView {
    /// Styled rows.
    pub lines: Vec<Line<'static>>,
    /// True when the preview cap hid rows.
    pub truncated: bool,
}

/// How one transcript entry is laid out in the current turn state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EntryLayout {
    /// Rendered in full.
    Visible,
    /// Hidden inside a collapsed turn.
    Skipped,
    /// Collapsed to a single summary row.
    Collapsed,
}

/// Identity of one laid-out transcript entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CachedEntryKey {
    /// Entry revision, or the turn fingerprint for a collapsed summary.
    pub revision: u64,
    /// Current layout mode.
    pub layout: EntryLayout,
    /// Animation tick, or zero when the entry has no spinner.
    pub animation_tick: u64,
}

/// One laid-out transcript entry.
#[derive(Debug, Clone)]
pub(crate) struct CachedEntry {
    /// Cache identity.
    pub key: CachedEntryKey,
    /// Wrapped rows for this entry.
    pub rows: Vec<Line<'static>>,
    /// One optional target per row.
    pub targets: Vec<Option<HitTarget>>,
}

/// Pre-wrapped transcript rows, cached per entry.
#[derive(Debug, Clone)]
pub(crate) struct CachedTranscript {
    /// Transcript width in columns.
    pub width: u16,
    /// Palette revision.
    pub theme_revision: u64,
    /// Laid-out entries, aligned with `Transcript::entries`.
    pub entries: Vec<CachedEntry>,
    /// Empty-state banner rows.
    pub banner: Vec<Line<'static>>,
}

/// One overlay on top of the transcript.
pub enum Overlay {
    /// Slash-command help.
    Help,
    /// Keybinding reference.
    Hotkeys,
    /// A pending permission prompt.
    Permission(PermissionPrompt),
    /// A pending question from a delegated run.
    Question(QuestionPrompt),
    /// The current session tree.
    Tree(Vec<TreeEntry>),
    /// The session picker.
    Picker(Picker),
    /// A command, model, theme, or reasoning list.
    List(ChoiceList),
    /// API-key entry and verification for one provider.
    Connect(ConnectState),
    /// Token, cost, and context usage details.
    Usage,
    /// A read-only child transcript opened from `/subagents`.
    Child {
        /// Overlay title with the agent and label.
        title: String,
        /// Child session identifier, used to refresh while it runs.
        session_id: String,
        /// Pre-rendered transcript lines.
        lines: Vec<String>,
        /// Vertical scroll offset.
        scroll: u16,
        /// Last refresh time, for a one-second poll.
        updated: Option<std::time::Instant>,
    },
    /// A transient banner with an optional OK affordance.
    Notice {
        /// Banner title.
        title: String,
        /// Banner body.
        body: String,
        /// True when the banner can be dismissed with OK.
        ok: bool,
    },
}

/// Interactive session picker state (`SESS-008`).
pub struct Picker {
    /// Every session in the directory.
    pub entries: Vec<SessionSummary>,
    /// Indices of entries matching the query, in display order.
    pub visible: Vec<usize>,
    /// Selected position within `visible`.
    pub selected: usize,
    /// Search text; empty shows everything.
    pub query: String,
    /// Sort by name instead of modification time.
    pub sort_by_name: bool,
    /// Rename buffer, when editing a name.
    pub rename: Option<String>,
    /// True after `d`, waiting for confirmation.
    pub confirm_delete: bool,
}

impl Picker {
    /// Creates a picker over the given summaries.
    #[must_use]
    pub fn new(entries: Vec<SessionSummary>) -> Self {
        let mut picker = Self {
            entries,
            visible: Vec::new(),
            selected: 0,
            query: String::new(),
            sort_by_name: false,
            rename: None,
            confirm_delete: false,
        };
        picker.refresh();
        picker
    }

    /// Recomputes the visible list after a filter or sort change.
    pub fn refresh(&mut self) {
        if self.sort_by_name {
            self.entries.sort_by(|left, right| {
                left.name
                    .as_deref()
                    .unwrap_or(left.id.as_str())
                    .cmp(right.name.as_deref().unwrap_or(right.id.as_str()))
            });
        } else {
            self.entries.sort_by(|left, right| {
                right
                    .updated_at
                    .cmp(&left.updated_at)
                    .then_with(|| right.created_at.cmp(&left.created_at))
            });
        }
        let query = self.query.to_lowercase();
        self.visible = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, summary)| {
                query.is_empty()
                    || summary.id.to_lowercase().contains(&query)
                    || summary
                        .name
                        .as_deref()
                        .is_some_and(|name| name.to_lowercase().contains(&query))
                    || summary.cwd.to_lowercase().contains(&query)
            })
            .map(|(index, _)| index)
            .collect();
        self.selected = self.selected.min(self.visible.len().saturating_sub(1));
    }

    /// Returns the selected summary, if any.
    #[must_use]
    pub fn current(&self) -> Option<&SessionSummary> {
        self.visible
            .get(self.selected)
            .and_then(|index| self.entries.get(*index))
    }

    /// Moves the selection by `delta`, clamped to the list.
    pub fn move_by(&mut self, delta: isize) {
        if self.visible.is_empty() {
            self.selected = 0;
            return;
        }
        let next = self.selected as isize + delta;
        self.selected = next.clamp(0, self.visible.len() as isize - 1) as usize;
    }
}

/// A permission prompt shown to the user.
pub struct PermissionPrompt {
    /// Tool name.
    pub tool: String,
    /// Parsed arguments.
    pub arguments: Value,
    /// Agent type that asked for the call, when delegated.
    pub agent: Option<String>,
    /// Delegation depth of the caller.
    pub depth: usize,
    /// Session id of the asking run, used to look up its label.
    pub session_id: Option<String>,
    /// Structured preview for edit, write, and bash calls.
    pub preview: Option<codie_protocol::ext::ExtRenderHint>,
    /// Vertical scroll offset for the preview body.
    pub scroll: u16,
    /// True when the preview shows every row instead of the capped view.
    pub expanded: bool,
    /// Reply channel.
    reply: Option<oneshot::Sender<Decision>>,
}

/// A question from a delegated run, answered with one line of text.
pub struct QuestionPrompt {
    /// Question text.
    pub question: String,
    /// Agent type that asked, when delegated.
    pub agent: Option<String>,
    /// Session id of the asking run, used to look up its label.
    pub session_id: Option<String>,
    /// Input buffer.
    pub input: String,
    /// Reply channel; `None` means dismissed.
    reply: Option<oneshot::Sender<Option<String>>>,
}

/// Footer metadata.
#[derive(Debug, Clone, Default)]
pub struct Footer {
    /// Working directory.
    pub cwd: String,
    /// Git branch, when inside a repository.
    pub git_branch: Option<String>,
    /// Provider identifier.
    pub provider: String,
    /// Model identifier.
    pub model: String,
    /// Thinking level.
    pub thinking: String,
    /// Total tokens accumulated in this session.
    pub tokens: u64,
    /// Uncached input tokens.
    pub input_tokens: u64,
    /// Output tokens.
    pub output_tokens: u64,
    /// Tokens served from the provider cache.
    pub cache_read_tokens: u64,
    /// Tokens written to the provider cache.
    pub cache_write_tokens: u64,
    /// Total cost in nano-USD.
    pub cost: u64,
    /// Most recent or live tokens per second.
    pub tokens_per_sec: Option<f64>,
    /// Session identifier.
    pub session: Option<String>,
    /// Estimated context usage percentage.
    pub context_percent: Option<u8>,
    /// Estimated tokens currently in the context.
    pub context_tokens: Option<u64>,
}

/// An action requested by a key press.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Nothing to do.
    None,
    /// Submit a prompt as a new run (or steering while streaming).
    Submit(String),
    /// Queue a follow-up message.
    FollowUp(String),
    /// Cancel the current run.
    Cancel,
    /// Toggle automatic approval for mutating tools.
    ToggleAutoApprove,
    /// Check the API key currently held by the connect overlay.
    VerifyKey,
    /// Store the connect overlay's API key without checking it.
    SaveKey,
    /// Quit the app.
    Quit,
}

/// A mouse text selection over the rendered screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    /// Row and column where the drag started.
    pub anchor: (u16, u16),
    /// Row and column of the pointer.
    pub cursor: (u16, u16),
}

impl Selection {
    /// Returns the top-left and bottom-right corners in reading order.
    #[must_use]
    pub fn bounds(&self) -> ((u16, u16), (u16, u16)) {
        if self.cursor < self.anchor {
            (self.cursor, self.anchor)
        } else {
            (self.anchor, self.cursor)
        }
    }

    /// Returns true when both corners sit on the same cell.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.anchor == self.cursor
    }
}

/// The interactive application state.
pub struct App {
    /// Scrolling transcript.
    pub transcript: Transcript,
    /// Multiline editor.
    pub editor: Editor,
    /// Active theme.
    pub theme: Theme,
    /// Theme to restore if the `/theme` live preview is cancelled.
    theme_preview: Option<Theme>,
    /// Footer metadata.
    pub footer: Footer,
    /// Current status.
    pub status: Status,
    /// Active overlay.
    pub overlay: Option<Overlay>,
    /// Active mouse selection, if any.
    pub selection: Option<Selection>,
    /// True while the primary mouse button is held for a drag.
    dragging: bool,
    /// Set when the selection should be copied on the next frame.
    pub copy_pending: bool,
    /// Set when one more frame is needed, for example after a copy notice.
    pub needs_redraw: bool,
    /// Transient message shown in the editor title until the next input.
    pub notice: Option<String>,
    /// True when the app should exit.
    pub should_quit: bool,
    /// True when mutating tools run without a prompt.
    pub auto_approve: bool,
    /// Animation tick.
    pub tick: u64,
    /// Current model identity.
    pub model: ModelRef,
    /// Current thinking level.
    pub thinking: ThinkingLevel,
    /// Context window used for the threshold and footer.
    pub context_window: u32,
    /// Models offered by the picker.
    pub models: Vec<ModelChoice>,
    /// Optional reloader for the picker models.
    pub models_loader: Option<ModelsLoader>,
    /// Providers offered by the `/provider` picker.
    pub providers: Vec<ProviderChoice>,
    /// Optional provider switcher.
    pub switch_provider: Option<ProviderSwitcher>,
    /// Credential store used by the connect overlay.
    pub auth_path: Option<PathBuf>,
    /// Provider whose key came from `--api-key` for this run.
    pub explicit_key_provider: Option<String>,
    /// Monotonic counter for connect attempts, so stale results never match.
    connect_generation: u64,
    /// Persists the last provider/model selection.
    persist_selection: Option<SelectionPersister>,
    /// When the current run started, for live tokens/sec.
    pub run_started: Option<std::time::Instant>,
    /// Conversation history.
    pub history: Vec<Message>,
    /// Configuration before project agent definitions are applied.
    pub subagent_base: SubagentConfig,
    /// Active subagent configuration, including project personas.
    pub subagent_config: SubagentConfig,
    /// Effective session delegation mode.
    pub subagent_mode: DelegationMode,
    /// Force delegation for the next prompt only.
    pub subagent_force_once: bool,
    /// Runtime used by `/subagents` when the tool is enabled.
    pub subagent_runtime: Option<Arc<SubagentRuntime>>,
    /// True when the active run was forced to delegate.
    forced_run: bool,
    /// True when the active run called `task` at least once.
    run_had_task: bool,
    /// One-shot forced-delegation nudge (`AGENT-030`).
    nudge: NudgeState,
    /// Follow-up to start after the current run ends.
    pending_nudge: Option<String>,
    /// Active session, when persisted.
    pub session: Option<Session>,
    /// Pending manual compaction instructions.
    pub pending_compact: Option<String>,
    /// Prompt to run after a compaction finishes.
    pub pending_prompt: Option<String>,
    /// True when `pending_prompt` is a retry of the current transcript.
    pub pending_retry: bool,
    /// Last submitted prompt, used for one overflow retry.
    pub last_prompt: Option<String>,
    /// True after the one allowed overflow recovery attempt.
    pub overflow_retried: bool,
    /// True when `/update` asked the event loop to start a check.
    pub update_check_requested: bool,
    /// Session root for `/new`.
    session_dir: PathBuf,
    /// Working directory for file completion.
    cwd: PathBuf,
    /// Pending permission prompts.
    permissions: VecDeque<PermissionPrompt>,
    /// Pending subagent questions.
    questions: VecDeque<QuestionPrompt>,
    /// Exit code from a signal.
    pub signal: Arc<std::sync::atomic::AtomicI32>,
    /// Cancellation token for the current run.
    pub cancel: tokio_util::sync::CancellationToken,
    /// Active keymap.
    pub bindings: Bindings,
    /// Optional watcher for a TOML theme file.
    theme_watcher: Option<crate::theme::ThemeWatcher>,
    /// Skills and templates available to slash commands.
    pub customizations: Customizations,
    /// Prompt produced by a customization command, submitted by the event loop.
    pub command_prompt: Option<String>,
    /// Files attached through the editor, keyed by their compact label.
    pub attachments: Vec<Attachment>,
    /// Next attachment number used to build labels.
    next_attachment: usize,
    /// Slash commands declared by extensions.
    pub extension_commands: Vec<String>,
    /// True after `/new` starts a fresh session.
    pub extension_session_reset: bool,
    /// True when the event loop should drop queued steering and follow-ups.
    pub(crate) drop_queued: bool,
    /// Clickable rows recorded by the last transcript draw.
    pub(crate) hits: Vec<Hit>,
    /// Rendered rich tool bodies, keyed by call, width, and theme.
    pub(crate) tool_views: RefCell<HashMap<ToolViewKey, CachedToolView>>,
    /// Fully laid-out transcript rows, reused while scrolling.
    pub(crate) transcript_cache: RefCell<Option<CachedTranscript>>,
    /// Incremented when the active palette changes.
    pub(crate) theme_revision: u64,
}

impl App {
    /// Creates an app for one interactive session.
    #[must_use]
    pub fn new(
        cwd: impl Into<PathBuf>,
        model: ModelRef,
        thinking: ThinkingLevel,
        context_window: u32,
        models: Vec<ModelChoice>,
        theme: Theme,
        session: Option<Session>,
        history: Vec<Message>,
        session_dir: PathBuf,
        signal: Arc<std::sync::atomic::AtomicI32>,
        bindings: Bindings,
    ) -> Self {
        let cwd = cwd.into();
        let git_branch = git_branch(&cwd);
        let binding_cwd = cwd.clone();
        let theme_watcher = std::path::Path::new(&theme.name)
            .is_file()
            .then(|| crate::theme::ThemeWatcher::new(&theme.name));
        let session_id = session.as_ref().map(|session| session.id().to_owned());
        let context_tokens = history.iter().map(estimate_message_tokens).sum::<u64>();
        let context_percent = if context_window == 0 {
            0
        } else {
            u8::try_from(
                context_tokens
                    .saturating_mul(100)
                    .saturating_div(u64::from(context_window))
                    .min(100),
            )
            .unwrap_or(100)
        };
        Self {
            transcript: Transcript::new(),
            editor: Editor::new(),
            theme,
            theme_preview: None,
            footer: Footer {
                cwd: cwd.display().to_string(),
                git_branch,
                provider: model.provider.clone(),
                model: model.model.clone(),
                thinking: thinking.as_str().to_owned(),
                session: session_id,
                context_percent: Some(context_percent),
                context_tokens: Some(context_tokens),
                ..Footer::default()
            },
            status: Status::Idle,
            overlay: None,
            selection: None,
            dragging: false,
            copy_pending: false,
            needs_redraw: false,
            notice: None,
            should_quit: false,
            auto_approve: false,
            tick: 0,
            model,
            thinking,
            context_window,
            models,
            models_loader: None,
            providers: Vec::new(),
            switch_provider: None,
            auth_path: None,
            explicit_key_provider: None,
            connect_generation: 0,
            persist_selection: None,
            run_started: None,
            history,
            subagent_base: SubagentConfig::default(),
            subagent_config: SubagentConfig::default(),
            subagent_mode: DelegationMode::Auto,
            subagent_force_once: false,
            subagent_runtime: None,
            forced_run: false,
            run_had_task: false,
            nudge: NudgeState::default(),
            pending_nudge: None,
            session,
            pending_compact: None,
            pending_prompt: None,
            pending_retry: false,
            last_prompt: None,
            overflow_retried: false,
            update_check_requested: false,
            session_dir,
            cwd: binding_cwd,
            permissions: VecDeque::new(),
            questions: VecDeque::new(),
            signal,
            cancel: tokio_util::sync::CancellationToken::new(),
            bindings,
            theme_watcher,
            customizations: Customizations::default(),
            command_prompt: None,
            attachments: Vec::new(),
            next_attachment: 0,
            extension_commands: Vec::new(),
            extension_session_reset: false,
            drop_queued: false,
            hits: Vec::new(),
            tool_views: RefCell::new(HashMap::new()),
            transcript_cache: RefCell::new(None),
            theme_revision: 0,
        }
    }

    /// Handles one terminal key event.
    pub fn handle_key(&mut self, key: KeyEvent) -> Action {
        if key.kind == KeyEventKind::Release {
            return Action::None;
        }
        self.notice = None;
        if self.selection.is_some() {
            self.clear_selection();
        }
        if key.code == KeyCode::BackTab {
            return self.dispatch_action(ActionId::ToggleAutoApprove);
        }
        if self.overlay.is_some() {
            // Session controls must not be swallowed by a modal overlay. Resolve
            // them through the keymap so remapped shortcuts keep working. The
            // connect overlay keeps its own Ctrl+C handling.
            let action = self.bindings.action_for(&key);
            if action == Some(ActionId::Quit) {
                return Action::Quit;
            }
            if action == Some(ActionId::Cancel)
                && !matches!(self.overlay, Some(Overlay::Connect(_)))
            {
                return if self.status == Status::Streaming {
                    self.dispatch_action(ActionId::Cancel)
                } else {
                    Action::Quit
                };
            }
            return self.handle_overlay_key(key);
        }

        if let Some(action) = self.bindings.action_for(&key) {
            return self.dispatch_action(action);
        }

        let control = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        match key.code {
            KeyCode::Char('l') if control => {
                self.editor.clear();
                Action::None
            }
            KeyCode::Char('/') if !control && !alt && self.editor.text().trim().is_empty() => {
                self.open_palette();
                Action::None
            }
            KeyCode::Char(character) if !control && !alt => {
                self.editor.insert_char(character);
                self.attach_completed_path();
                Action::None
            }
            KeyCode::Backspace => {
                self.editor.backspace();
                Action::None
            }
            KeyCode::Delete => {
                self.editor.delete_forward();
                Action::None
            }
            KeyCode::Left => {
                self.editor.move_left();
                Action::None
            }
            KeyCode::Right => {
                self.editor.move_right();
                Action::None
            }
            KeyCode::Up => {
                self.editor.move_up();
                Action::None
            }
            KeyCode::Down => {
                self.editor.move_down();
                Action::None
            }
            KeyCode::Home => {
                self.editor.home();
                Action::None
            }
            KeyCode::End => {
                self.editor.end();
                Action::None
            }
            KeyCode::Esc => {
                if !self.transcript.collapse_thinking() {
                    self.transcript.scroll_to_bottom();
                }
                Action::None
            }
            _ => Action::None,
        }
    }

    /// Starts a mouse selection at a screen position.
    pub fn begin_selection(&mut self, column: u16, row: u16) {
        self.notice = None;
        self.selection = Some(Selection {
            anchor: (row, column),
            cursor: (row, column),
        });
        self.dragging = true;
    }

    /// Extends the active selection to a screen position.
    pub fn drag_selection(&mut self, column: u16, row: u16) {
        if !self.dragging {
            return;
        }
        if let Some(selection) = self.selection.as_mut() {
            selection.cursor = (row, column);
        }
    }

    /// Ends a mouse selection.
    ///
    /// Returns true when the gesture never moved, so the caller can treat it as
    /// a click.
    pub fn end_selection(&mut self, column: u16, row: u16) -> bool {
        if !self.dragging {
            return true;
        }
        self.dragging = false;
        if let Some(selection) = self.selection.as_mut() {
            selection.cursor = (row, column);
        }
        let dragged = self
            .selection
            .is_some_and(|selection| !selection.is_empty());
        if dragged {
            self.copy_pending = true;
            false
        } else {
            self.selection = None;
            true
        }
    }

    /// Clears any selection highlight.
    pub fn clear_selection(&mut self) {
        self.selection = None;
        self.dragging = false;
        self.copy_pending = false;
        self.notice = None;
    }

    /// Handles a left click at a terminal position.
    ///
    /// Returns true when the click toggled a transcript element.
    pub fn handle_click(&mut self, _column: u16, row: u16) -> bool {
        if self.overlay.is_some() {
            return false;
        }
        let Some(target) = self
            .hits
            .iter()
            .find(|hit| hit.row == row)
            .map(|hit| hit.target)
        else {
            return false;
        };
        match target {
            HitTarget::Thinking(index) => self.transcript.toggle_thinking(index),
            HitTarget::Tool(index) => self.transcript.toggle_tool(index),
            HitTarget::Turn(index) => self.transcript.toggle_turn(index),
            HitTarget::Usage => {
                self.overlay = Some(Overlay::Usage);
                true
            }
        }
    }

    fn dispatch_action(&mut self, action: ActionId) -> Action {
        match action {
            ActionId::Submit => {
                let text = self.editor.submit();
                if text.trim().is_empty() {
                    Action::None
                } else {
                    Action::Submit(text)
                }
            }
            ActionId::Newline => {
                self.editor.insert_newline();
                Action::None
            }
            ActionId::FollowUp => Action::FollowUp(self.editor.submit()),
            ActionId::Cancel => {
                if self.status == Status::Streaming {
                    self.status = Status::Waiting;
                    Action::Cancel
                } else if self.editor.is_empty() {
                    Action::Quit
                } else {
                    self.editor.clear();
                    self.clear_attachments();
                    Action::None
                }
            }
            ActionId::Stop => {
                if self.transcript.collapse_thinking() {
                    Action::None
                } else if self.status == Status::Streaming
                    || self.transcript.running_tool().is_some()
                {
                    self.status = Status::Waiting;
                    Action::Cancel
                } else {
                    self.transcript.scroll_to_bottom();
                    Action::None
                }
            }
            ActionId::Quit => Action::Quit,
            ActionId::ExpandTool => {
                self.transcript.toggle_last_tool();
                Action::None
            }
            ActionId::ToggleAllTools => {
                self.transcript.toggle_all_tools();
                Action::None
            }
            ActionId::ToolPicker => {
                self.open_tools();
                Action::None
            }
            ActionId::ToggleAllTurns => {
                self.transcript.toggle_all_turns();
                Action::None
            }
            ActionId::Undo => {
                self.editor.undo();
                Action::None
            }
            ActionId::Redo => {
                self.editor.redo();
                Action::None
            }
            ActionId::KillLine => {
                self.editor.kill_to_end();
                Action::None
            }
            ActionId::Yank => {
                self.editor.yank();
                Action::None
            }
            ActionId::PasteImage => {
                self.paste_clipboard();
                Action::None
            }
            ActionId::HistoryPrevious => {
                self.editor.history_previous();
                Action::None
            }
            ActionId::HistoryNext => {
                self.editor.history_next();
                Action::None
            }
            ActionId::ScrollUp => {
                self.transcript.scroll_up(8);
                Action::None
            }
            ActionId::ScrollDown => {
                self.transcript.scroll_down(8);
                Action::None
            }
            ActionId::Complete => {
                self.complete();
                Action::None
            }
            ActionId::Help => {
                self.overlay = Some(Overlay::Help);
                Action::None
            }
            ActionId::Hotkeys => {
                self.overlay = Some(Overlay::Hotkeys);
                Action::None
            }
            ActionId::ToggleAutoApprove => {
                self.auto_approve = !self.auto_approve;
                Action::ToggleAutoApprove
            }
        }
    }

    fn handle_overlay_key(&mut self, key: KeyEvent) -> Action {
        if matches!(self.overlay, Some(Overlay::List(_))) {
            return self.handle_list_key(key);
        }
        if matches!(self.overlay, Some(Overlay::Picker(_))) {
            return self.handle_picker_key(key);
        }
        if matches!(self.overlay, Some(Overlay::Connect(_))) {
            return self.handle_connect_key(key);
        }
        if let Some(Overlay::Child { scroll, .. }) = self.overlay.as_mut() {
            match key.code {
                KeyCode::Up | KeyCode::Char('k') => *scroll = scroll.saturating_sub(1),
                KeyCode::Down | KeyCode::Char('j') => *scroll = scroll.saturating_add(1),
                KeyCode::PageUp => *scroll = scroll.saturating_sub(10),
                KeyCode::PageDown => *scroll = scroll.saturating_add(10),
                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Enter => self.overlay = None,
                _ => {}
            }
            if self.overlay.is_none() {
                self.show_next_prompt();
            }
            return Action::None;
        }
        if matches!(self.overlay, Some(Overlay::Question(_))) {
            let overlay = self.overlay.take();
            if let Some(Overlay::Question(mut prompt)) = overlay {
                match key.code {
                    KeyCode::Enter => {
                        let answer = prompt.input.trim().to_owned();
                        if let Some(reply) = prompt.reply.take() {
                            let _ = reply.send((!answer.is_empty()).then_some(answer));
                        }
                    }
                    KeyCode::Esc => {
                        if let Some(reply) = prompt.reply.take() {
                            let _ = reply.send(None);
                        }
                    }
                    KeyCode::Backspace => {
                        prompt.input.pop();
                        self.overlay = Some(Overlay::Question(prompt));
                        return Action::None;
                    }
                    KeyCode::Char(character) => {
                        prompt.input.push(character);
                        self.overlay = Some(Overlay::Question(prompt));
                        return Action::None;
                    }
                    _ => {
                        self.overlay = Some(Overlay::Question(prompt));
                        return Action::None;
                    }
                }
            }
            self.show_next_prompt();
            return Action::None;
        }
        let permission = matches!(self.overlay, Some(Overlay::Permission(_)));
        if !permission {
            if matches!(key.code, KeyCode::Esc | KeyCode::Enter)
                || key.code == KeyCode::Char('q')
                || key.code == KeyCode::Char('?')
            {
                self.overlay = None;
                self.show_next_prompt();
            }
            return Action::None;
        }

        let decision = match key.code {
            KeyCode::Char('y' | 'Y' | 'a') => Some(Decision::Allow),
            KeyCode::Char('A') => Some(Decision::Always),
            KeyCode::Char('n' | 'N' | 'd' | 'D') | KeyCode::Esc => Some(Decision::Deny),
            KeyCode::Enter => Some(Decision::Allow),
            KeyCode::Up
            | KeyCode::Down
            | KeyCode::PageUp
            | KeyCode::PageDown
            | KeyCode::Home
            | KeyCode::End
            | KeyCode::Char('o' | 'O') => {
                if let Some(Overlay::Permission(prompt)) = self.overlay.as_mut() {
                    match key.code {
                        KeyCode::Up => prompt.scroll = prompt.scroll.saturating_sub(1),
                        KeyCode::Down => prompt.scroll = prompt.scroll.saturating_add(1),
                        KeyCode::PageUp => prompt.scroll = prompt.scroll.saturating_sub(10),
                        KeyCode::PageDown => prompt.scroll = prompt.scroll.saturating_add(10),
                        KeyCode::Home => prompt.scroll = 0,
                        KeyCode::End => prompt.scroll = u16::MAX,
                        KeyCode::Char('o' | 'O')
                            if key.modifiers.contains(KeyModifiers::CONTROL) =>
                        {
                            prompt.expanded = !prompt.expanded;
                            prompt.scroll = 0;
                        }
                        _ => {}
                    }
                }
                None
            }
            _ => None,
        };
        if let Some(decision) = decision {
            self.resolve_permission(decision);
        }
        Action::None
    }

    fn handle_connect_key(&mut self, key: KeyEvent) -> Action {
        let Some(Overlay::Connect(mut state)) = self.overlay.take() else {
            return Action::None;
        };

        if state.phase == ConnectPhase::Checking {
            if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                return Action::Quit;
            }
            // Esc closes the overlay; the probe result is ignored because the
            // overlay no longer matches its provider and generation.
            if key.code != KeyCode::Esc {
                self.overlay = Some(Overlay::Connect(state));
            }
            return Action::None;
        }

        match key.code {
            KeyCode::Esc => return Action::None,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Action::Quit;
            }
            KeyCode::Enter => {
                if state.input.trim().is_empty() {
                    state.phase = ConnectPhase::Error("API key must not be empty.".to_owned());
                } else {
                    self.connect_generation = self.connect_generation.wrapping_add(1);
                    state.generation = self.connect_generation;
                    state.phase = ConnectPhase::Checking;
                    self.overlay = Some(Overlay::Connect(state));
                    return Action::VerifyKey;
                }
            }
            KeyCode::Backspace => {
                state.input.pop();
                state.phase = ConnectPhase::Editing;
            }
            KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                state.reveal = !state.reveal;
            }
            KeyCode::Char('v') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some(text) = crate::clipboard::read_text() {
                    let text = text.trim().replace(['\r', '\n', ' '], "");
                    state.input.push_str(&text);
                    state.phase = ConnectPhase::Editing;
                }
            }
            KeyCode::Char('s')
                if key.modifiers.contains(KeyModifiers::CONTROL)
                    && matches!(state.phase, ConnectPhase::Error(_))
                    && !state.input.trim().is_empty() =>
            {
                self.overlay = Some(Overlay::Connect(state));
                return Action::SaveKey;
            }
            KeyCode::Char(character)
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT) =>
            {
                state.input.push(character);
                state.phase = ConnectPhase::Editing;
            }
            _ => {}
        }

        self.overlay = Some(Overlay::Connect(state));
        Action::None
    }

    /// Returns the provider, key, and generation for a pending check.
    #[must_use]
    pub fn take_connect_request(&self) -> Option<(String, String, u64)> {
        let Some(Overlay::Connect(state)) = self.overlay.as_ref() else {
            return None;
        };
        if state.phase != ConnectPhase::Checking {
            return None;
        }
        let key = state.input.trim().to_owned();
        (!key.is_empty()).then(|| (state.provider.clone(), key, state.generation))
    }

    /// Applies the result of a probe started for the connect overlay.
    pub fn handle_connect_result(
        &mut self,
        provider: &str,
        generation: u64,
        outcome: ProbeOutcome,
    ) {
        let Some(Overlay::Connect(state)) = self.overlay.as_ref() else {
            return;
        };
        if state.provider != provider || state.generation != generation {
            return;
        }
        let key = state.input.trim().to_owned();
        if key.is_empty() {
            return;
        }

        if !outcome.accepted() {
            if let Some(Overlay::Connect(state)) = self.overlay.as_mut() {
                state.phase = ConnectPhase::Error(outcome.summary());
            }
            return;
        }

        let note = if matches!(outcome, ProbeOutcome::Throttled { .. }) {
            outcome.summary()
        } else {
            "key accepted".to_owned()
        };
        match self.store_connect_key(provider, &key) {
            Ok(()) => self.finish_connect(provider, &note),
            Err(error) => {
                if let Some(Overlay::Connect(state)) = self.overlay.as_mut() {
                    state.phase = ConnectPhase::Error(format!(
                        "the key is valid, but it could not be saved: {error}"
                    ));
                }
            }
        }
    }

    /// Stores the connect overlay's key without contacting the provider.
    pub fn save_connect_key(&mut self) {
        let Some(Overlay::Connect(state)) = self.overlay.as_ref() else {
            return;
        };
        let provider = state.provider.clone();
        let key = state.input.trim().to_owned();
        if key.is_empty() {
            return;
        }
        match self.store_connect_key(&provider, &key) {
            Ok(()) => self.finish_connect(&provider, "saved without checking"),
            Err(error) => {
                if let Some(Overlay::Connect(state)) = self.overlay.as_mut() {
                    state.phase = ConnectPhase::Error(format!("cannot save the key: {error}"));
                }
            }
        }
    }

    fn store_connect_key(&self, provider: &str, key: &str) -> std::result::Result<(), String> {
        let path = self
            .auth_path
            .as_ref()
            .ok_or_else(|| "credential storage is unavailable".to_owned())?;
        AuthStore::new(path)
            .set(provider, key)
            .map_err(|error| error.to_string())
    }

    fn finish_connect(&mut self, provider: &str, note: &str) {
        self.overlay = None;
        let name = self
            .providers
            .iter()
            .find(|choice| choice.id == provider)
            .map_or_else(
                || provider_name(provider).to_owned(),
                |choice| choice.name.clone(),
            );
        self.switch_provider_to(provider);
        self.transcript
            .push_info(format!("Connected to {name} · {note}."));
    }

    /// Returns the credential status for one provider.
    #[must_use]
    pub fn provider_status(&self, provider: &str) -> ProviderAuthStatus {
        if self.explicit_key_provider.as_deref() == Some(provider) {
            return ProviderAuthStatus::configured(provider, CredentialSource::Explicit);
        }
        let store = self
            .auth_path
            .as_ref()
            .map(|path| AuthStore::new(path.clone()));
        auth_status(store.as_ref(), provider)
    }

    fn handle_list_key(&mut self, key: KeyEvent) -> Action {
        let Some(Overlay::List(mut list)) = self.overlay.take() else {
            return Action::None;
        };
        match key.code {
            KeyCode::Up => {
                list.move_by(-1);
                self.preview_selected_theme(&list);
            }
            KeyCode::Down => {
                list.move_by(1);
                self.preview_selected_theme(&list);
            }
            KeyCode::PageUp => {
                list.move_by(-5);
                self.preview_selected_theme(&list);
            }
            KeyCode::PageDown => {
                list.move_by(5);
                self.preview_selected_theme(&list);
            }
            KeyCode::Enter => {
                if list.kind == ChoiceKind::Themes {
                    self.theme_preview = None;
                }
                if let Some(item) = list.current().cloned()
                    && let Some(action) = self.apply_choice(list.kind, &item)
                {
                    return action;
                }
                return Action::None;
            }
            KeyCode::Esc => {
                if list.kind == ChoiceKind::Themes {
                    self.cancel_theme_preview();
                }
                return Action::None;
            }
            KeyCode::Backspace => {
                list.query.pop();
                list.refresh();
                self.preview_selected_theme(&list);
            }
            KeyCode::Char(character)
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT) =>
            {
                list.query.push(character);
                list.refresh();
                self.preview_selected_theme(&list);
            }
            _ => {}
        }
        if list.kind == ChoiceKind::Commands && path_like_query(&list.query) {
            // A pasted or typed absolute path is not a command. Hand the text
            // back to the editor instead of leaving it in the palette filter.
            self.overlay = None;
            self.editor.insert_str(&format!("/{}", list.query));
            return Action::None;
        }
        self.overlay = Some(Overlay::List(list));
        Action::None
    }

    fn apply_choice(&mut self, kind: ChoiceKind, item: &ChoiceItem) -> Option<Action> {
        match kind {
            ChoiceKind::Commands => match item.value.as_str() {
                "model" => self.open_models(),
                "theme" => self.open_themes(),
                "thinking" => self.open_thinking(),
                "quit" => self.should_quit = true,
                "new" => return Some(Action::Submit("/new".to_owned())),
                "name" | "export" | "export html" => {
                    self.editor.insert_str(&format!("/{} ", item.value));
                }
                "skill:" | "template:" => {
                    self.editor.insert_str(&format!("/{}", item.value));
                }
                value
                    if value.starts_with("ext:")
                        || self
                            .extension_commands
                            .iter()
                            .any(|command| command == &format!("/{value}")) =>
                {
                    return Some(Action::Submit(format!("/{value}")));
                }
                _ => {
                    self.handle_command(&format!("/{}", item.value));
                }
            },
            ChoiceKind::Tools => {
                if let Ok(index) = item.value.parse::<usize>() {
                    self.transcript.toggle_tool(index);
                }
            }
            ChoiceKind::Models => {
                if let Some(choice) = self
                    .models
                    .iter()
                    .find(|model| model.id == item.value)
                    .cloned()
                {
                    self.model.provider = choice.provider.clone();
                    self.model.model = choice.id.clone();
                    self.context_window = choice.context_window;
                    self.footer.provider = choice.provider.clone();
                    self.footer.model = choice.id.clone();
                    if let Some(session) = self.session.as_mut() {
                        let _ = session.append_model_change(choice.provider, choice.id.clone());
                    }
                    self.refresh_context();
                    self.sync_subagent_model();
                    self.persist_selection();
                    self.transcript
                        .push_info(format!("Model: {} ({})", choice.name, choice.id));
                }
            }
            ChoiceKind::Providers => {
                if item.value == "__connect__" {
                    self.open_connect();
                } else if self.provider_status(&item.value).usable() {
                    self.switch_provider_to(&item.value);
                } else {
                    self.open_connect_for(&item.value);
                }
            }
            ChoiceKind::ConnectProviders => self.open_connect_for(&item.value),
            ChoiceKind::Themes => match Theme::load(Some(&item.value)) {
                Ok(theme) => {
                    self.theme_watcher = std::path::Path::new(&theme.name)
                        .is_file()
                        .then(|| crate::theme::ThemeWatcher::new(&theme.name));
                    self.theme = theme;
                    self.theme_revision = self.theme_revision.wrapping_add(1);
                    self.transcript.push_info(format!("Theme: {}", item.value));
                }
                Err(error) => self
                    .transcript
                    .push_error(format!("Cannot load theme: {error}")),
            },
            ChoiceKind::Thinking => {
                if let Some(level) = ThinkingLevel::parse(&item.value) {
                    self.thinking = level;
                    self.sync_subagent_thinking();
                    self.footer.thinking = level.as_str().to_owned();
                    if let Some(session) = self.session.as_mut() {
                        let _ = session.append_thinking_level_change(level);
                    }
                    self.transcript
                        .push_info(format!("Reasoning level: {}", level.as_str()));
                }
            }
        }
        None
    }

    fn open_palette(&mut self) {
        let commands = [
            ("help", "Show all commands"),
            ("connect", "Add or replace a provider API key"),
            ("provider", "Choose a provider"),
            ("logout", "Remove a stored API key"),
            ("auth", "Show provider credential status"),
            ("model", "Choose a model"),
            ("theme", "Choose a theme"),
            ("thinking", "Set the reasoning level"),
            ("new", "Start a new session"),
            ("session", "Show the current session"),
            ("tree", "Show the session tree"),
            ("fork", "Branch at an entry"),
            ("clone", "Clone this session"),
            ("name", "Name the session"),
            ("export", "Export the session as JSONL"),
            ("export html", "Export a self-contained HTML transcript"),
            ("copy", "Copy the last assistant message"),
            ("debug", "Print diagnostics"),
            ("compact", "Compact the context"),
            ("usage", "Show token, cost, and context usage"),
            (
                "subagents",
                "Configure subagent delegation (on, off, force, once, types)",
            ),
            ("stop", "Stop the active run"),
            ("update", "Install a newer release"),
            ("reload", "Reload skills, templates, and themes"),
            ("skill:", "Invoke a skill"),
            ("template:", "Expand a prompt template"),
            ("hotkeys", "Show keybindings"),
            ("quit", "Exit codie"),
        ];
        let mut items = commands
            .into_iter()
            .map(|(name, description)| ChoiceItem {
                label: format!("/{name}"),
                detail: description.to_owned(),
                value: name.to_owned(),
            })
            .collect::<Vec<_>>();
        for command in &self.extension_commands {
            let value = command.trim_start_matches('/').to_owned();
            if items.iter().any(|item| item.value == value) {
                continue;
            }
            items.push(ChoiceItem {
                label: command.clone(),
                detail: "Extension command".to_owned(),
                value,
            });
        }
        self.overlay = Some(Overlay::List(ChoiceList::new(
            ChoiceKind::Commands,
            " commands ",
            items,
            "type to filter · ↑/↓ scroll · Enter select · Esc close",
        )));
    }

    fn open_tools(&mut self) {
        let items = self
            .transcript
            .entries()
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| {
                let Entry::Tool(block) = entry else {
                    return None;
                };
                let marker = match block.status {
                    crate::transcript::ToolStatus::Running => "…",
                    crate::transcript::ToolStatus::Success => "✓",
                    crate::transcript::ToolStatus::Error => "✗",
                };
                let state = if block.expanded {
                    "expanded"
                } else {
                    "collapsed"
                };
                Some(ChoiceItem {
                    label: format!("{marker} {}", block.summary()),
                    detail: format!("{state} · {}", block.name),
                    value: index.to_string(),
                })
            })
            .collect::<Vec<_>>();
        if items.is_empty() {
            self.transcript.push_info("No tool calls yet.");
            return;
        }
        self.overlay = Some(Overlay::List(ChoiceList::new(
            ChoiceKind::Tools,
            " tools ",
            items,
            "↑/↓ select · Enter expand/collapse · Esc close",
        )));
    }

    fn open_models(&mut self) {
        if let Some(loader) = &self.models_loader {
            let refreshed = loader(&self.model.provider);
            if !refreshed.is_empty() {
                self.models = refreshed;
                self.sync_subagent_model();
            }
        }
        let current = self.model.model.clone();
        let mut items = self
            .models
            .iter()
            .map(|model| ChoiceItem {
                label: if model.id == current {
                    format!("{} (current)", model.name)
                } else {
                    model.name.clone()
                },
                detail: format!("{} · {}k ctx", model.id, model.context_window / 1000),
                value: model.id.clone(),
            })
            .collect::<Vec<_>>();
        if items.is_empty() {
            items.push(ChoiceItem {
                label: format!("{current} (current)"),
                detail: format!("{}k ctx", self.context_window / 1000),
                value: current,
            });
        }
        self.overlay = Some(Overlay::List(ChoiceList::new(
            ChoiceKind::Models,
            " models ",
            items,
            "type to filter · ↑/↓ scroll · Enter select",
        )));
    }

    fn open_providers(&mut self) {
        let current = self.model.provider.clone();
        let mut items = self
            .providers
            .iter()
            .map(|provider| {
                let status = self.provider_status(&provider.id);
                let marker = if status.usable() { "✓" } else { "○" };
                ChoiceItem {
                    label: if provider.id == current {
                        format!("{marker} {} (current)", provider.name)
                    } else {
                        format!("{marker} {}", provider.name)
                    },
                    detail: format!("{} · {}", provider.id, status.label()),
                    value: provider.id.clone(),
                }
            })
            .collect::<Vec<_>>();
        items.push(ChoiceItem {
            label: "+ Connect or replace a key".to_owned(),
            detail: "Choose any provider and enter an API key".to_owned(),
            value: "__connect__".to_owned(),
        });
        self.overlay = Some(Overlay::List(ChoiceList::new(
            ChoiceKind::Providers,
            " providers ",
            items,
            "Enter switch/connect · type to filter · Esc close",
        )));
    }

    fn open_connect(&mut self) {
        let current = self.model.provider.clone();
        let items = self
            .providers
            .iter()
            .map(|provider| {
                let status = self.provider_status(&provider.id);
                let marker = if status.usable() { "✓" } else { "○" };
                ChoiceItem {
                    label: if provider.id == current {
                        format!("{marker} {} (current)", provider.name)
                    } else {
                        format!("{marker} {}", provider.name)
                    },
                    detail: format!("{} · {}", provider.id, status.label()),
                    value: provider.id.clone(),
                }
            })
            .collect::<Vec<_>>();
        self.overlay = Some(Overlay::List(ChoiceList::new(
            ChoiceKind::ConnectProviders,
            " connect provider ",
            items,
            "Enter enter or replace a key · type to filter · Esc close",
        )));
    }

    fn open_connect_for(&mut self, provider: &str) {
        if self.status == Status::Streaming {
            self.transcript
                .push_info("Finish or cancel the current run before changing providers.");
            return;
        }
        if !codie_ai::provider::requires_key(provider) {
            self.switch_provider_to(provider);
            return;
        }
        self.open_connect_overlay(provider);
    }

    fn open_connect_overlay(&mut self, provider: &str) {
        self.connect_generation = self.connect_generation.wrapping_add(1);
        let mut state = ConnectState::new(provider);
        state.generation = self.connect_generation;
        self.overlay = Some(Overlay::Connect(state));
    }

    fn persist_selection(&self) {
        if let Some(persist) = &self.persist_selection {
            persist(&self.model.provider, &self.model.model);
        }
    }

    fn switch_provider_to(&mut self, id: &str) {
        if self.status == Status::Streaming {
            self.transcript
                .push_info("Finish or cancel the current run before switching providers.");
            return;
        }
        let Some(switch) = self.switch_provider.clone() else {
            self.transcript
                .push_info("Provider switching is unavailable in this mode.");
            return;
        };
        let name = self
            .providers
            .iter()
            .find(|provider| provider.id == id)
            .map(|provider| provider.name.clone())
            .unwrap_or_else(|| id.to_owned());
        match switch(id) {
            Ok(runtime) => {
                self.model.provider = id.to_owned();
                self.model.model = runtime.default_model.clone();
                self.models = runtime.models;
                if let Some(model) = self
                    .models
                    .iter()
                    .find(|model| model.id == self.model.model)
                {
                    self.context_window = model.context_window;
                }
                self.footer.provider = id.to_owned();
                self.footer.model = self.model.model.clone();
                if let Some(session) = self.session.as_mut() {
                    let _ = session.append_model_change(id.to_owned(), self.model.model.clone());
                }
                self.refresh_context();
                self.sync_subagent_model();
                self.persist_selection();
                self.transcript
                    .push_info(format!("Provider: {name} · model: {}", self.model.model));
            }
            Err(error) => self
                .transcript
                .push_error(format!("Cannot switch provider: {error}")),
        }
    }

    fn open_themes(&mut self) {
        self.theme_preview = None;
        let current = self.theme.name.clone();
        let items = crate::theme::BUILTIN_THEMES
            .into_iter()
            .map(|name| ChoiceItem {
                label: if name == current {
                    format!("{name} (current)")
                } else {
                    name.to_owned()
                },
                detail: if matches!(name, "system" | "dark" | "light") {
                    "Built-in theme".to_owned()
                } else {
                    "Dark theme".to_owned()
                },
                value: name.to_owned(),
            })
            .collect();
        let mut list = ChoiceList::new(
            ChoiceKind::Themes,
            " themes ",
            items,
            "filter · ↑/↓ preview · Enter apply · Esc cancel",
        );
        list.selected = list
            .visible
            .iter()
            .position(|index| list.items[*index].value == current)
            .unwrap_or(0);
        self.overlay = Some(Overlay::List(list));
    }

    /// Previews the highlighted theme without committing it.
    fn preview_selected_theme(&mut self, list: &ChoiceList) {
        if list.kind != ChoiceKind::Themes {
            return;
        }
        let Some(item) = list.current() else {
            return;
        };
        if self.theme_preview.is_none() {
            self.theme_preview = Some(self.theme.clone());
        }
        if let Ok(theme) = Theme::load(Some(&item.value))
            && theme != self.theme
        {
            self.theme = theme;
            self.theme_revision = self.theme_revision.wrapping_add(1);
        }
    }

    /// Restores the theme that was active before the `/theme` preview started.
    fn cancel_theme_preview(&mut self) {
        if let Some(theme) = self.theme_preview.take() {
            self.theme = theme;
            self.theme_revision = self.theme_revision.wrapping_add(1);
        }
    }

    fn open_thinking(&mut self) {
        let current = self.thinking;
        let items = [
            ThinkingLevel::Off,
            ThinkingLevel::Minimal,
            ThinkingLevel::Low,
            ThinkingLevel::Medium,
            ThinkingLevel::High,
            ThinkingLevel::XHigh,
            ThinkingLevel::Max,
        ]
        .into_iter()
        .map(|level| ChoiceItem {
            label: if level == current {
                format!("{} (current)", level.as_str())
            } else {
                level.as_str().to_owned()
            },
            detail: if level.is_enabled() {
                "Reasoning enabled".to_owned()
            } else {
                "Reasoning disabled".to_owned()
            },
            value: level.as_str().to_owned(),
        })
        .collect();
        self.overlay = Some(Overlay::List(ChoiceList::new(
            ChoiceKind::Thinking,
            " reasoning level ",
            items,
            "type to filter · ↑/↓ scroll · Enter select",
        )));
    }

    fn refresh_context(&mut self) {
        let tokens = self
            .history
            .iter()
            .map(estimate_message_tokens)
            .sum::<u64>();
        let percent = tokens.saturating_mul(100) / u64::from(self.context_window.max(1));
        self.footer.context_tokens = Some(tokens);
        self.footer.context_percent = Some(u8::try_from(percent.min(100)).unwrap_or(100));
    }

    /// Re-reads an open child transcript once a second while it may still grow.
    fn refresh_child_overlay(&mut self) {
        let Some(Overlay::Child {
            session_id,
            lines,
            updated,
            ..
        }) = self.overlay.as_mut()
        else {
            return;
        };
        if updated.is_some_and(|updated| updated.elapsed() < std::time::Duration::from_secs(1)) {
            return;
        }
        *updated = Some(std::time::Instant::now());
        let path = self.session_dir.join(format!("{session_id}.jsonl"));
        let Ok(session) = Session::open(path) else {
            return;
        };
        let refreshed = child_transcript_lines(&session.projection());
        if !refreshed.is_empty() && *lines != refreshed {
            *lines = refreshed;
        }
    }

    /// Keeps the subagent runtime's inherited model in sync with `/model`,
    /// `/provider`, and catalog refreshes.
    fn sync_subagent_thinking(&self) {
        if let Some(runtime) = &self.subagent_runtime {
            runtime.set_thinking(self.thinking);
        }
    }

    fn sync_subagent_model(&self) {
        let Some(runtime) = &self.subagent_runtime else {
            return;
        };
        let known = self
            .models
            .iter()
            .filter(|model| model.provider == self.model.provider)
            .map(|model| model.id.clone())
            .collect();
        runtime.set_parent_model(self.model.clone(), known);
    }

    fn handle_picker_key(&mut self, key: KeyEvent) -> Action {
        let Some(Overlay::Picker(mut picker)) = self.overlay.take() else {
            return Action::None;
        };

        if let Some(buffer) = picker.rename.as_mut() {
            match key.code {
                KeyCode::Enter => {
                    let name = buffer.trim().to_owned();
                    if let Some(summary) = picker.current() {
                        match Session::open(&summary.path) {
                            Ok(mut session) => {
                                let result = if name.is_empty() {
                                    session.clear_name()
                                } else {
                                    session.set_name(name.clone())
                                };
                                if let Err(error) = result {
                                    self.transcript
                                        .push_error(format!("Cannot rename the session: {error}"));
                                }
                            }
                            Err(error) => self
                                .transcript
                                .push_error(format!("Cannot open the session: {error}")),
                        }
                    }
                    picker.rename = None;
                    self.reload_picker(&mut picker);
                }
                KeyCode::Esc => picker.rename = None,
                KeyCode::Backspace => {
                    buffer.pop();
                }
                KeyCode::Char(character)
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    buffer.push(character);
                }
                _ => {}
            }
            self.overlay = Some(Overlay::Picker(picker));
            return Action::None;
        }

        if picker.confirm_delete {
            match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    if let Some(summary) = picker.current() {
                        let path = summary.path.clone();
                        match std::fs::remove_file(&path) {
                            Ok(()) => self
                                .transcript
                                .push_info(format!("Deleted session {}.", summary.id)),
                            Err(error) => self
                                .transcript
                                .push_error(format!("Cannot delete {}: {error}", path.display())),
                        }
                    }
                    picker.confirm_delete = false;
                    self.reload_picker(&mut picker);
                }
                KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
                    picker.confirm_delete = false;
                }
                _ => {}
            }
            self.overlay = Some(Overlay::Picker(picker));
            return Action::None;
        }

        match key.code {
            KeyCode::Up | KeyCode::Char('k') => picker.move_by(-1),
            KeyCode::Down | KeyCode::Char('j') => picker.move_by(1),
            KeyCode::Enter => {
                if let Some(summary) = picker.current() {
                    match Session::open(&summary.path) {
                        Ok(session) => {
                            let label = session
                                .name()
                                .map_or_else(|| session.id().to_owned(), str::to_owned);
                            self.history = session.projection();
                            self.reset_transcript();
                            self.transcript
                                .push_info(format!("Resumed session {label}."));
                            self.footer.session = Some(session.id().to_owned());
                            self.session = Some(session);
                            self.status = Status::Idle;
                            return Action::None;
                        }
                        Err(error) => self
                            .transcript
                            .push_error(format!("Cannot open the session: {error}")),
                    }
                }
            }
            KeyCode::Esc | KeyCode::Char('q') => return Action::None,
            KeyCode::Char('d') => picker.confirm_delete = true,
            KeyCode::Char('r') => {
                picker.rename = Some(
                    picker
                        .current()
                        .and_then(|summary| summary.name.clone())
                        .unwrap_or_default(),
                );
            }
            KeyCode::Char('s') => {
                picker.sort_by_name = !picker.sort_by_name;
                picker.refresh();
            }
            KeyCode::Backspace => {
                picker.query.pop();
                picker.refresh();
            }
            KeyCode::Char(character)
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT) =>
            {
                picker.query.push(character);
                picker.refresh();
            }
            _ => {}
        }

        self.overlay = Some(Overlay::Picker(picker));
        Action::None
    }

    fn reload_picker(&mut self, picker: &mut Picker) {
        match Session::list_for_cwd(&self.cwd, &self.session_dir) {
            Ok(entries) => {
                picker.entries = entries;
                picker.refresh();
            }
            Err(error) => self
                .transcript
                .push_error(format!("Cannot list sessions: {error}")),
        }
    }

    /// Runs a slash command or returns false when the text is a prompt.
    pub fn handle_command(&mut self, text: &str) -> bool {
        let Some(command) = parse_command(text) else {
            return false;
        };
        match command {
            Command::Help => self.overlay = Some(Overlay::Help),
            Command::Hotkeys => self.overlay = Some(Overlay::Hotkeys),
            Command::Quit => self.should_quit = true,
            Command::Stop => {
                if self.status == Status::Streaming || self.transcript.running_tool().is_some() {
                    self.status = Status::Waiting;
                    self.cancel.cancel();
                    self.drop_queued = true;
                    self.transcript.push_info("Stopping the current run…");
                } else if self.status == Status::Waiting {
                    self.cancel.cancel();
                    self.drop_queued = true;
                    self.transcript.push_info("Already stopping…");
                } else {
                    self.transcript.push_info("Nothing is running.");
                }
            }
            Command::New => {
                self.new_session();
                self.extension_session_reset = true;
            }
            Command::Session => {
                let text = self.session.as_ref().map_or_else(
                    || "No session is being persisted.".to_owned(),
                    |session| {
                        format!(
                            "Session {} · {} · ~{} tokens · {}",
                            session.name().unwrap_or("unnamed"),
                            session.path().display(),
                            session.estimate_tokens(),
                            session.id()
                        )
                    },
                );
                self.transcript.push_info(text);
            }
            Command::Tree => {
                if let Some(session) = self.session.as_ref() {
                    self.overlay = Some(Overlay::Tree(session.tree()));
                } else {
                    self.transcript.push_info("No session is being persisted.");
                }
            }
            Command::Fork(argument) => {
                if let Some(entry) = argument {
                    self.fork_at(&entry);
                } else if let Some(session) = self.session.as_ref() {
                    self.overlay = Some(Overlay::Tree(session.tree()));
                    self.transcript
                        .push_info("Use /fork <entry-id> to branch at an entry.");
                } else {
                    self.transcript.push_info("No session is being persisted.");
                }
            }
            Command::Clone => self.clone_session(),
            Command::Name(argument) => {
                if let Some(session) = self.session.as_mut() {
                    let result = match &argument {
                        Some(name) => session.set_name(name.clone()),
                        None => session.clear_name(),
                    };
                    match result {
                        Ok(()) => {
                            let text = match argument {
                                Some(name) => format!("Session named `{name}`."),
                                None => "Session name cleared.".to_owned(),
                            };
                            self.transcript.push_info(text);
                        }
                        Err(error) => self
                            .transcript
                            .push_error(format!("Cannot name the session: {error}")),
                    }
                } else {
                    self.transcript.push_info("No session is being persisted.");
                }
            }
            Command::Export(argument) => self.export_session(argument),
            Command::ExportHtml(argument) => self.export_html_session(argument),
            Command::Copy => self.copy_last_message(),
            Command::Debug => self.show_debug(),
            Command::Update => self.update_check_requested = true,
            Command::Theme(argument) => {
                self.theme_preview = None;
                if let Some(name) = argument {
                    match Theme::load(Some(&name)) {
                        Ok(theme) => {
                            self.theme_watcher = std::path::Path::new(&theme.name)
                                .is_file()
                                .then(|| crate::theme::ThemeWatcher::new(&theme.name));
                            self.theme = theme;
                            self.theme_revision = self.theme_revision.wrapping_add(1);
                            self.transcript.push_info(format!("Theme: {name}"));
                        }
                        Err(error) => self
                            .transcript
                            .push_error(format!("Cannot load theme: {error}")),
                    }
                } else {
                    self.open_themes();
                }
            }
            Command::Provider(argument) => {
                if let Some(provider) = argument {
                    if self.provider_status(&provider).usable() {
                        self.switch_provider_to(&provider);
                    } else if self.providers.iter().any(|choice| choice.id == provider) {
                        self.open_connect_for(&provider);
                    } else {
                        self.transcript
                            .push_error(format!("Unknown provider `{provider}`."));
                    }
                } else {
                    self.open_providers();
                }
            }
            Command::Connect(argument) => {
                if let Some(provider) = argument {
                    if self.providers.iter().any(|choice| choice.id == provider) {
                        self.open_connect_for(&provider);
                    } else {
                        self.transcript
                            .push_error(format!("Unknown provider `{provider}`."));
                    }
                } else {
                    self.open_connect();
                }
            }
            Command::Logout(argument) => {
                let provider = argument.unwrap_or_else(|| self.model.provider.clone());
                if !self.providers.iter().any(|choice| choice.id == provider) {
                    self.transcript
                        .push_error(format!("Unknown provider `{provider}`."));
                } else {
                    let Some(path) = self.auth_path.clone() else {
                        self.transcript
                            .push_error("Credential storage is unavailable in this mode.");
                        return true;
                    };
                    let store = AuthStore::new(path);
                    match store.remove(&provider) {
                        Ok(true) => self
                            .transcript
                            .push_info(format!("Removed the stored API key for {provider}.")),
                        Ok(false) => {
                            let status = self.provider_status(&provider);
                            if status.source == Some(CredentialSource::Environment) {
                                self.transcript.push_info(format!(
                                    "{provider} uses {}. Unset it to disconnect this provider.",
                                    status.env_var
                                ));
                            } else {
                                self.transcript
                                    .push_info(format!("No stored API key for {provider}."));
                            }
                        }
                        Err(error) => self
                            .transcript
                            .push_error(format!("Cannot remove the API key: {error}")),
                    }
                }
            }
            Command::Auth => {
                if self.providers.is_empty() {
                    self.transcript.push_info("No providers are available.");
                } else {
                    for provider in &self.providers {
                        let status = self.provider_status(&provider.id);
                        self.transcript
                            .push_info(format!("{}: {}", provider.name, status.label()));
                    }
                }
            }
            Command::Model(argument) => {
                if let Some(model) = argument {
                    self.model.model = model.clone();
                    self.footer.model = model.clone();
                    if let Some(choice) = self
                        .models
                        .iter()
                        .find(|choice| choice.id == model)
                        .cloned()
                    {
                        self.model.provider = choice.provider.clone();
                        self.context_window = choice.context_window;
                        self.footer.provider = choice.provider.clone();
                        if let Some(session) = self.session.as_mut() {
                            let _ = session.append_model_change(choice.provider, choice.id.clone());
                        }
                    }
                    self.refresh_context();
                    self.sync_subagent_model();
                    self.persist_selection();
                    self.transcript.push_info("Model changed for the next run.");
                } else {
                    self.open_models();
                }
            }
            Command::Thinking(argument) => {
                if let Some(value) = argument {
                    if let Some(level) = parse_thinking(&value) {
                        self.thinking = level;
                        self.sync_subagent_thinking();
                        self.footer.thinking = level.as_str().to_owned();
                        if let Some(session) = self.session.as_mut() {
                            let _ = session.append_thinking_level_change(level);
                        }
                        self.transcript
                            .push_info(format!("Reasoning level: {}", level.as_str()));
                    } else {
                        self.transcript
                            .push_error(format!("Unknown thinking level `{value}`."));
                    }
                } else {
                    self.open_thinking();
                }
            }
            Command::Compact(argument) => {
                if self.session.is_some() {
                    self.pending_compact = Some(argument.unwrap_or_default());
                } else {
                    self.transcript.push_info("No session is being persisted.");
                }
            }
            Command::Usage => self.overlay = Some(Overlay::Usage),
            Command::Subagents(argument) => {
                let runs = self
                    .subagent_runtime
                    .as_ref()
                    .map(|runtime| runtime.runs())
                    .unwrap_or_default();
                match crate::subagents::parse_command(
                    argument.as_deref(),
                    &self.subagent_config,
                    self.subagent_mode,
                    self.subagent_force_once,
                    &runs,
                ) {
                    crate::subagents::SubagentCommand::Status(text) => {
                        self.transcript.push_info(text)
                    }
                    crate::subagents::SubagentCommand::Mode { mode, message } => {
                        self.subagent_mode = mode;
                        self.subagent_force_once = false;
                        if let Some(runtime) = &self.subagent_runtime {
                            runtime.set_mode(mode);
                        }
                        let directive = Message::system(mode_directive(mode), Vec::new());
                        if let Some(session) = self.session.as_mut() {
                            let _ = session.append_custom(
                                "subagent_settings",
                                serde_json::json!({"mode": mode.as_str(), "force_once": false}),
                            );
                            let _ = session.append_message(&directive);
                        }
                        self.history.push(directive);
                        self.transcript.push_info(message);
                    }
                    crate::subagents::SubagentCommand::Once { message } => {
                        self.subagent_force_once = true;
                        if let Some(runtime) = &self.subagent_runtime {
                            runtime.set_mode(DelegationMode::Force);
                        }
                        self.transcript.push_info(message);
                    }
                    crate::subagents::SubagentCommand::Parallel { limit, message } => {
                        let applied = self
                            .subagent_runtime
                            .as_ref()
                            .is_some_and(|runtime| runtime.set_max_parallel(limit));
                        if applied {
                            self.subagent_config.max_parallel = limit;
                            self.transcript.push_info(message);
                        } else {
                            let current = self
                                .subagent_runtime
                                .as_ref()
                                .map_or(self.subagent_config.max_parallel, |runtime| {
                                    runtime.max_parallel()
                                });
                            self.transcript.push_info(format!(
                                "Subagent parallelism stays at {current}: stop running children before changing it."
                            ));
                        }
                    }
                    crate::subagents::SubagentCommand::Open(index) => {
                        self.open_child_run(index, &runs);
                    }
                    crate::subagents::SubagentCommand::Stop(index) => {
                        if let Some(run) = runs.get(index.saturating_sub(1)) {
                            let cancelled = self
                                .subagent_runtime
                                .as_ref()
                                .is_some_and(|runtime| runtime.cancel_run(&run.call_id));
                            if cancelled {
                                self.transcript.push_info(format!(
                                    "Stopping subagent {} · {}…",
                                    run.agent, run.description
                                ));
                            } else {
                                self.transcript.push_error(format!(
                                    "Subagent {} · {} is not running.",
                                    run.agent, run.description
                                ));
                            }
                        } else {
                            self.transcript.push_error(format!(
                                "No subagent run {index}. Use /subagents status to list runs."
                            ));
                        }
                    }
                    crate::subagents::SubagentCommand::Error(message) => {
                        self.transcript.push_error(message)
                    }
                }
            }
            Command::Skill(name, arguments) => {
                if self.status == Status::Streaming {
                    self.transcript
                        .push_info("Finish or cancel the current run before invoking a skill.");
                } else {
                    let prompt = self
                        .customizations
                        .skill(&name)
                        .map(|skill| skill.prompt(&arguments));
                    match prompt {
                        Some(prompt) => self.command_prompt = Some(prompt),
                        None => self.transcript.push_error(format!(
                            "Unknown skill `{name}`. Add it under .codie/skills/{name}/SKILL.md."
                        )),
                    }
                }
            }
            Command::Template(name, arguments) => {
                if self.status == Status::Streaming {
                    self.transcript
                        .push_info("Finish or cancel the current run before expanding a template.");
                } else {
                    let prompt = self
                        .customizations
                        .template(&name)
                        .map(|template| template.expand(&arguments));
                    match prompt {
                        Some(prompt) => self.command_prompt = Some(prompt),
                        None => self.transcript.push_error(format!(
                            "Unknown template `{name}`. Add it under .codie/templates/{name}.md."
                        )),
                    }
                }
            }
            Command::Reload => {
                self.customizations.refresh();
                for warning in self.customizations.warnings() {
                    self.transcript.push_error(warning.clone());
                }
                let mut config = self.subagent_base.clone();
                config.enabled = self.subagent_config.enabled;
                config.max_parallel = self.subagent_config.max_parallel;
                config.apply_agent_definitions(self.customizations.agents());
                match config.validate() {
                    Ok(()) => {
                        self.subagent_config = config.clone();
                        if let Some(runtime) = &self.subagent_runtime {
                            runtime.set_config(config);
                        }
                    }
                    Err(error) => self
                        .transcript
                        .push_error(format!("Cannot reload agent definitions: {error}")),
                }
                let refreshed = self.reload_project_memory();
                self.transcript.push_info(format!(
                    "Reloaded {} skill(s), {} template(s), {} agent definition(s), and {} project memory.",
                    self.customizations.skills().len(),
                    self.customizations.templates().len(),
                    self.customizations.agents().len(),
                    if refreshed { "refreshed" } else { "unchanged" }
                ));
            }
            Command::Init => {
                if self.status == Status::Streaming {
                    self.transcript
                        .push_info("Finish or cancel the current run before scaffolding.");
                } else {
                    match codie_prompt::scaffold(&self.cwd) {
                        Ok(outcome) => {
                            for path in &outcome.created {
                                let label = if path.file_name().is_some_and(|name| {
                                    name.to_string_lossy() == codie_prompt::PROJECT_MEMORY_FILE
                                }) {
                                    "Created (loaded into every session)"
                                } else {
                                    "Created"
                                };
                                self.transcript
                                    .push_info(format!("{label}: {}", path.display()));
                            }
                            for path in &outcome.existing {
                                self.transcript
                                    .push_info(format!("Kept existing: {}", path.display()));
                            }
                            if outcome.created.iter().any(|path| {
                                path.file_name().is_some_and(|name| {
                                    name.to_string_lossy() == codie_prompt::PROJECT_MEMORY_FILE
                                })
                            }) {
                                self.reload_project_memory();
                            }
                        }
                        Err(error) => self
                            .transcript
                            .push_error(format!("Cannot scaffold project files: {error}")),
                    }
                }
            }
            Command::Unknown(name) => self
                .transcript
                .push_error(format!("Unknown command `/{name}`. Try /help.")),
        }
        true
    }

    /// Handles pasted text.
    ///
    /// A paste that looks like a path closes the command palette first, so an
    /// absolute path never lands in the palette filter.
    pub fn paste(&mut self, text: &str) {
        if let Some(Overlay::Connect(state)) = &mut self.overlay {
            if state.phase != ConnectPhase::Checking {
                let text = text.trim().replace(['\r', '\n', ' ', '\t'], "");
                state.input.push_str(&text);
                state.phase = ConnectPhase::Editing;
            }
            return;
        }
        if let Some(path) = codie_prompt::resolve_path_text(text, &self.cwd) {
            self.overlay = None;
            self.insert_attachment(path);
            return;
        }
        let palette_open = matches!(
            &self.overlay,
            Some(Overlay::List(list)) if list.kind == ChoiceKind::Commands
        );
        if palette_open && (text.contains('/') || text.contains('\\')) {
            self.overlay = None;
            let draft = if text.starts_with('/') {
                text.to_owned()
            } else {
                format!("/{text}")
            };
            self.editor.insert_str(&draft);
            return;
        }
        self.editor.paste(text);
    }

    /// Turns the editor contents into an attachment when they name a file.
    ///
    /// A dragged screenshot arrives one character at a time, so the path must
    /// become an attachment before the temporary file disappears. Doing this on
    /// every keystroke is cheap because the check only runs for path-like first
    /// characters.
    fn attach_completed_path(&mut self) {
        let text = self.editor.text();
        let trimmed = text.trim_start();
        if trimmed.is_empty() || !trimmed.starts_with(['/', '~', '.']) {
            return;
        }
        if let Some(path) = codie_prompt::resolve_path_text(&text, &self.cwd) {
            self.editor.clear();
            self.insert_attachment(path);
        }
    }

    /// Adds one attachment to the editor as a compact label.
    ///
    /// Repeating the same path reuses its label instead of adding a duplicate.
    fn insert_attachment(&mut self, path: PathBuf) {
        if let Some(existing) = self
            .attachments
            .iter()
            .find(|attachment| attachment.path == path)
        {
            let label = existing.label.clone();
            self.editor.insert_str(&format!("{label} "));
            return;
        }
        self.next_attachment += 1;
        let kind = if codie_prompt::is_image_path(&path) {
            "Image"
        } else {
            "File"
        };
        let label = format!("[{kind} {}]", self.next_attachment);
        let notice = format!("Attached {label}.");
        self.attachments.push(Attachment {
            label: label.clone(),
            path,
        });
        self.editor.insert_str(&format!("{label} "));
        self.notice = Some(notice);
    }

    /// Replaces attachment labels with `@path` tokens before submission.
    pub fn expand_attachment_labels(&mut self, text: &str) -> String {
        if self.attachments.is_empty() {
            return text.to_owned();
        }
        let mut expanded = text.to_owned();
        for attachment in &self.attachments {
            expanded = expanded.replace(
                &attachment.label,
                &codie_prompt::quote_attachment_path(&attachment.path),
            );
        }
        expanded
    }

    /// Drops attachment labels; the editor no longer references them.
    pub fn clear_attachments(&mut self) {
        self.attachments.clear();
        self.next_attachment = 0;
    }

    /// Pastes a clipboard image as an attachment, or clipboard text.
    pub fn paste_clipboard(&mut self) {
        if let Some(path) = crate::clipboard::read_image() {
            self.insert_attachment(path);
            return;
        }
        match crate::clipboard::read_text() {
            Some(text) if !text.is_empty() => self.paste(&text),
            _ => self.notice = Some("Clipboard has no image or text.".to_owned()),
        }
    }

    /// Re-reads `CODIE.MD` and refreshes the persisted system prompt.
    ///
    /// Returns true when the prompt changed. The update is recorded as a
    /// context edit so the session file stays the source of truth.
    fn reload_project_memory(&mut self) -> bool {
        let memory = codie_prompt::load_project_memory(&self.cwd);
        let Some(Message::System(system)) = self.history.first_mut() else {
            return false;
        };
        let updated = codie_prompt::replace_project_memory(&system.content, memory.as_ref());
        if updated == system.content {
            return false;
        }
        system.content = updated.clone();

        if let Some(session) = self.session.as_mut() {
            let target = session.entries().iter().find_map(|entry| match entry {
                codie_session::Entry::Message {
                    id,
                    message: Message::System(_),
                    ..
                } => Some(id.clone()),
                _ => None,
            });
            if let Some(id) = target {
                let _ = session.append_context_edit(id, Some(Content::text(updated)));
            }
        }
        true
    }

    /// Adds a permission prompt, showing it when no other prompt is active.
    pub fn push_permission(&mut self, request: PermissionRequest) {
        let preview = permission_preview(&self.cwd, &request.tool, &request.arguments);
        self.permissions.push_back(PermissionPrompt {
            tool: request.tool,
            arguments: request.arguments,
            agent: request.agent,
            depth: request.depth,
            session_id: request.session_id,
            preview,
            scroll: 0,
            expanded: false,
            reply: Some(request.reply),
        });
        self.show_next_prompt();
    }

    /// Adds a question from a delegated run, showing it when the interface is free.
    pub fn push_question(&mut self, request: crate::questions::QuestionRequest) {
        self.questions.push_back(QuestionPrompt {
            question: request.question,
            agent: request.agent,
            session_id: request.session_id,
            input: String::new(),
            reply: Some(request.reply),
        });
        self.show_next_prompt();
    }

    /// Resolves the visible permission prompt.
    pub fn resolve_permission(&mut self, decision: Decision) {
        if let Some(Overlay::Permission(prompt)) = self.overlay.take()
            && let Some(reply) = prompt.reply
        {
            let _ = reply.send(decision);
        }
        self.show_next_prompt();
    }

    fn show_next_prompt(&mut self) {
        if self.overlay.is_some() {
            return;
        }
        if let Some(prompt) = self.permissions.pop_front() {
            self.status = Status::Waiting;
            self.overlay = Some(Overlay::Permission(prompt));
        } else if let Some(prompt) = self.questions.pop_front() {
            self.status = Status::Waiting;
            self.overlay = Some(Overlay::Question(prompt));
        } else if self.status == Status::Waiting {
            self.status = Status::Streaming;
        }
    }

    /// Opens a read-only view of one delegated child session.
    fn open_child_run(&mut self, index: usize, runs: &[RunSnapshot]) {
        let Some(run) = runs.get(index.saturating_sub(1)) else {
            self.transcript.push_error(format!(
                "No subagent run {index}. Use /subagents status to list runs."
            ));
            return;
        };
        let Some(session_id) = run.session_id.as_deref() else {
            self.transcript.push_error(format!(
                "Subagent {} · {} was not persisted, so there is no transcript to open.",
                run.agent, run.description
            ));
            return;
        };
        let path = self.session_dir.join(format!("{session_id}.jsonl"));
        match Session::open(&path) {
            Ok(session) => {
                let lines = child_transcript_lines(&session.projection());
                if lines.is_empty() {
                    self.transcript.push_info(format!(
                        "Subagent {} · {} has no messages yet.",
                        run.agent, run.description
                    ));
                    return;
                }
                self.overlay = Some(Overlay::Child {
                    title: format!(" subagent · {} · {} ", run.agent, run.state.as_str()),
                    session_id: session_id.to_owned(),
                    lines,
                    scroll: 0,
                    updated: Some(std::time::Instant::now()),
                });
            }
            Err(error) => self
                .transcript
                .push_error(format!("Cannot open child session {session_id}: {error}")),
        }
    }

    fn reset_transcript(&mut self) {
        self.transcript.clear();
        self.tool_views.borrow_mut().clear();
    }

    fn new_session(&mut self) {
        match Session::create(self.footer.cwd.clone(), &self.session_dir) {
            Ok(session) => {
                self.session = Some(session);
                self.history.clear();
                self.reset_transcript();
                self.footer.session = self.session.as_ref().map(|session| session.id().to_owned());
                self.transcript.push_info("Started a new session.");
            }
            Err(error) => self
                .transcript
                .push_error(format!("Cannot create a session: {error}")),
        }
    }

    fn fork_at(&mut self, entry_id: &str) {
        let result = self
            .session
            .as_mut()
            .ok_or_else(|| codie_core::Error::session("no session is being persisted"))
            .and_then(|session| {
                session.set_leaf(Some(entry_id))?;
                Ok((session.projection(), session.id().to_owned()))
            });
        match result {
            Ok((history, id)) => {
                self.history = history;
                self.reset_transcript();
                self.footer.session = Some(id);
                self.refresh_context();
                self.transcript
                    .push_info(format!("Forked at entry {entry_id}."));
            }
            Err(error) => self
                .transcript
                .push_error(format!("Cannot fork the session: {error}")),
        }
    }

    fn clone_session(&mut self) {
        let result = self
            .session
            .as_ref()
            .ok_or_else(|| codie_core::Error::session("no session is being persisted"))
            .and_then(|session| session.clone_session(&self.cwd, &self.session_dir));
        match result {
            Ok(session) => {
                let id = session.id().to_owned();
                self.history = session.projection();
                self.session = Some(session);
                self.reset_transcript();
                self.footer.session = Some(id);
                self.transcript.push_info("Cloned the session.");
            }
            Err(error) => self
                .transcript
                .push_error(format!("Cannot clone the session: {error}")),
        }
    }

    fn export_html_session(&mut self, argument: Option<String>) {
        let Some(session) = self.session.as_ref() else {
            self.transcript.push_info("No session is being persisted.");
            return;
        };
        let path = argument.map_or_else(
            || {
                self.cwd
                    .join(format!("codie-session-{}.html", session.id()))
            },
            std::path::PathBuf::from,
        );
        let result = std::fs::File::create(&path)
            .map_err(|error| codie_core::Error::io(&path, error))
            .and_then(|file| session.export_html(file));
        match result {
            Ok(()) => self
                .transcript
                .push_info(format!("Exported HTML to {}.", path.display())),
            Err(error) => self
                .transcript
                .push_error(format!("Cannot export HTML: {error}")),
        }
    }

    fn copy_last_message(&mut self) {
        let text = self.transcript.entries().iter().rev().find_map(|entry| {
            let Entry::Assistant(assistant) = entry else {
                return None;
            };
            (!assistant.text.is_empty()).then(|| assistant.text.clone())
        });
        match text {
            Some(text) => match crate::copy_to_clipboard(&text) {
                Ok(()) => self
                    .transcript
                    .push_info("Copied the last message to the clipboard."),
                Err(error) => self
                    .transcript
                    .push_error(format!("Cannot copy to the clipboard: {error}")),
            },
            None => self.transcript.push_info("No assistant message to copy."),
        }
    }

    fn show_debug(&mut self) {
        let session = self
            .session
            .as_ref()
            .map_or_else(|| "ephemeral".to_owned(), |session| session.id().to_owned());
        let tokens = self.session.as_ref().map_or_else(
            || {
                self.history
                    .iter()
                    .map(estimate_message_tokens)
                    .sum::<u64>()
            },
            codie_session::Session::estimate_tokens,
        );
        let percent = self
            .session
            .as_ref()
            .map_or_else(
                || {
                    let window = u64::from(self.context_window.max(1));
                    tokens.saturating_mul(100) / window
                },
                |session| u64::from(session.context_percent(u64::from(self.context_window))),
            )
            .min(100);
        self.transcript.push_info(format!(
            "codie {} · {} · {} · cwd {} · session {session} · {} messages · ~{tokens} tok · {percent}% ctx · theme {} · {}",
            env!("CARGO_PKG_VERSION"),
            self.model.model,
            self.thinking.as_str(),
            self.cwd.display(),
            self.history.len(),
            self.theme.name,
            if self.auto_approve { "full access" } else { "guarded" },
        ));
    }

    fn export_session(&mut self, argument: Option<String>) {
        let Some(session) = self.session.as_ref() else {
            self.transcript.push_info("No session is being persisted.");
            return;
        };
        let path = argument.map_or_else(
            || {
                self.cwd
                    .join(format!("codie-session-{}.jsonl", session.id()))
            },
            std::path::PathBuf::from,
        );
        let result = std::fs::File::create(&path)
            .map_err(|error| codie_core::Error::io(&path, error))
            .and_then(|file| session.export(file));
        match result {
            Ok(()) => self
                .transcript
                .push_info(format!("Exported session to {}.", path.display())),
            Err(error) => self
                .transcript
                .push_error(format!("Cannot export the session: {error}")),
        }
    }

    fn complete(&mut self) {
        let cursor = self.editor.cursor();
        let line = self
            .editor
            .lines()
            .get(cursor.row)
            .cloned()
            .unwrap_or_default();
        let col = cursor.col.min(line.len());
        let before = &line[..col];
        let start = before.rfind(char::is_whitespace).map_or(0, |index| {
            index + before[index..].chars().next().map_or(1, char::len_utf8)
        });
        let token = &before[start..];

        if let Some(prefix) = token.strip_prefix('@') {
            if let Some(path) = self.complete_file(prefix) {
                self.editor.replace_token(start, &format!("@{path}"));
            }
            return;
        }

        if token.starts_with('/') {
            let extension = self
                .extension_commands
                .iter()
                .find(|command| command.starts_with(token))
                .map(String::as_str);
            if let Some(command) = crate::commands::completions(token).first() {
                self.editor.replace_token(start, command);
            } else if let Some(command) = extension {
                self.editor.replace_token(start, command);
            }
        }
    }

    fn complete_file(&self, prefix: &str) -> Option<String> {
        let (search_root, written_root, stem) = split_completion(prefix, &self.cwd);
        let depth = if written_root.is_empty() { 6 } else { 3 };
        let mut matches = ignore::WalkBuilder::new(&search_root)
            .hidden(true)
            .git_ignore(true)
            .max_depth(Some(depth))
            .build()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_some_and(|kind| kind.is_file()))
            .filter_map(|entry| {
                let relative = entry.path().strip_prefix(&search_root).ok()?;
                let relative = relative.to_string_lossy().replace('\\', "/");
                relative
                    .starts_with(&stem)
                    .then(|| quote_completion(&format!("{written_root}{relative}")))
            })
            .take(200)
            .collect::<Vec<_>>();
        matches.sort();
        matches.into_iter().next()
    }

    /// Appends a finalized message to the active session.
    pub fn persist(&mut self, message: &Message) {
        if let Some(session) = self.session.as_mut()
            && let Err(error) = session.append_message(message)
        {
            self.transcript
                .push_error(format!("Cannot persist the session: {error}"));
        }
    }

    /// Updates footer usage from the latest message.
    pub fn update_usage(&mut self, usage: &Usage, duration_ms: Option<u64>) {
        self.footer.tokens = self.footer.tokens.saturating_add(usage.total_tokens);
        self.footer.input_tokens = self.footer.input_tokens.saturating_add(usage.input);
        self.footer.output_tokens = self.footer.output_tokens.saturating_add(usage.output);
        self.footer.cache_read_tokens = self
            .footer
            .cache_read_tokens
            .saturating_add(usage.cache_read);
        self.footer.cache_write_tokens = self
            .footer
            .cache_write_tokens
            .saturating_add(usage.cache_write);
        self.footer.cost = self.footer.cost.saturating_add(usage.cost.total);
        if usage.output > 0
            && let Some(ms) = duration_ms
            && ms > 0
        {
            self.footer.tokens_per_sec = Some(usage.output as f64 * 1000.0 / ms as f64);
        }
        self.refresh_context();
    }
}

/// Splits an attachment completion prefix into the directory to walk, the text
/// to keep in front of the match, and the stem being completed.
fn split_completion(prefix: &str, cwd: &std::path::Path) -> (PathBuf, String, String) {
    let expanded = expand_tilde(prefix);
    let (written_root, stem) = match expanded.rsplit_once('/') {
        Some((directory, stem)) => (format!("{directory}/"), stem.to_owned()),
        None => (String::new(), expanded.clone()),
    };
    let root = if written_root.is_empty() {
        cwd.to_path_buf()
    } else if std::path::Path::new(&written_root).is_absolute() {
        PathBuf::from(&written_root)
    } else {
        cwd.join(&written_root)
    };
    (root, written_root, stem)
}

fn expand_tilde(prefix: &str) -> String {
    if let Some(rest) = prefix.strip_prefix("~/")
        && let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))
    {
        return format!("{}/{rest}", PathBuf::from(home).display());
    }
    prefix.to_owned()
}

/// Quotes a completed path so it stays one attachment token.
fn quote_completion(display: &str) -> String {
    if display.chars().any(char::is_whitespace) {
        format!("\"{display}\"")
    } else {
        display.to_owned()
    }
}

/// Returns true when a command-palette query is really a filesystem path.
///
/// The leading `/` opened the palette, so the query is everything after it.
/// A colon marks a command argument (`skill:review`), which stays in the
/// palette; a slash or a leading `~` means the user is typing a path.
fn path_like_query(query: &str) -> bool {
    !query.contains(':') && (query.contains('/') || query.starts_with('~'))
}

/// Runs the interactive interface and returns the process exit code.
pub async fn run(options: TuiOptions) -> Result<u8> {
    if options.mode == TuiRunMode::Plain {
        return plain::run(options).await;
    }
    run_full(options).await
}

async fn run_full(mut options: TuiOptions) -> Result<u8> {
    let theme = Theme::load(options.theme.as_deref())?;
    let registry: Arc<dyn Toolbox> = options.registry.clone();
    let (permission_tx, mut permission_rx) = mpsc::unbounded_channel();
    let broker = PermissionBroker::new(permission_tx, options.auto_allowed.clone());
    let permission_broker = broker.clone();
    let (question_tx, mut question_rx) = mpsc::unbounded_channel();
    let questions = crate::questions::QuestionBroker::new(question_tx);
    let registry: Arc<dyn Toolbox> =
        Arc::new(crate::questions::QuestionToolbox::new(registry, questions));
    let session_id = options
        .session
        .as_ref()
        .map(|session| session.id().to_owned());
    let built = crate::subagents::build(crate::subagents::BuildInput {
        config: options.subagents.clone(),
        mode: options.subagent_mode,
        agents: options.customizations.agents().to_vec(),
        registry,
        broker,
        client: Arc::clone(&options.client),
        prompt: crate::subagents::prompt_input(
            &options.cwd,
            options.preamble.clone(),
            options.append.clone(),
            &options.customizations,
        ),
        session_dir: options.session_dir.clone(),
        session_id: session_id.clone(),
        model: options.model.clone(),
        known_models: options
            .models
            .iter()
            .filter(|model| model.provider == options.model.provider)
            .map(|model| model.id.clone())
            .collect(),
        thinking: options.thinking,
    })
    .map_err(anyhow::Error::msg)?;
    let toolbox = built.toolbox;
    let subagents_available = built.runtime.is_some();
    let initial_subagent_mode = if options.subagents.enabled {
        options.subagent_mode
    } else {
        DelegationMode::Off
    };
    let agent = Arc::new(match options.extensions.hooks() {
        Some(hooks) => Agent::with_hooks(Arc::clone(&options.client), Arc::clone(&toolbox), hooks),
        None => Agent::new(Arc::clone(&options.client), Arc::clone(&toolbox)),
    });
    options.extensions.set_session_id(session_id.clone());

    let mut history = options.history.clone();
    let mut app = App::new(
        options.cwd.clone(),
        options.model.clone(),
        options.thinking,
        options.context_window,
        options.models.clone(),
        theme,
        options.session.take(),
        history.clone(),
        options.session_dir.clone(),
        Arc::clone(&options.signal),
        Bindings::load(options.keybindings.as_deref())?,
    );
    app.customizations = options.customizations.clone();
    app.subagent_base = options.subagents.clone();
    app.subagent_config = built.config.clone();
    app.subagent_config.enabled = subagents_available;
    app.subagent_mode = initial_subagent_mode;
    app.subagent_force_once = options.subagent_force_once;
    app.subagent_runtime = built.runtime;
    app.models_loader = options.models_loader.clone();
    app.providers = options.providers.clone();
    app.switch_provider = options.switch_provider.clone();
    app.auth_path = Some(options.auth_path.clone());
    app.explicit_key_provider = options.explicit_key_provider.clone();
    app.persist_selection = options.persist_selection.clone();
    app.extension_commands = options.extensions.command_names();

    if options.startup_connect {
        let provider = app.model.provider.clone();
        app.open_connect_for(&provider);
    }

    if !matches!(history.first(), Some(Message::System(_))) {
        let declarations = toolbox.declarations();
        let input = PromptInput {
            cwd: options.cwd.clone(),
            tools: declarations.clone(),
            preamble: options.preamble.clone(),
            append: options.append.clone(),
            skills: options.customizations.skills().to_vec(),
            project_context: load_project_context(&options.cwd),
            project_memory: load_project_memory(&options.cwd),
        };
        let system = build_system_message(&input, declarations);
        if let Some(session) = app.session.as_mut() {
            session.append_message(&system)?;
        }
        history.insert(0, system);
        app.history = history.clone();
    }

    if options.resume_picker {
        match Session::list_for_cwd(&options.cwd, &options.session_dir) {
            Ok(entries) => app.overlay = Some(Overlay::Picker(Picker::new(entries))),
            Err(error) => app
                .transcript
                .push_error(format!("Cannot list sessions: {error}")),
        }
    }

    let session_info = codie_protocol::ext::ExtSessionInfo {
        id: session_id,
        cwd: app.cwd.display().to_string(),
        model: Some(app.model.model.clone()),
    };
    options.extensions.session_start(&session_info).await;
    let extensions = Arc::clone(&options.extensions);

    let mut terminal = setup_terminal(options.mode)?;
    let result = event_loop(
        &mut terminal,
        &mut app,
        agent,
        toolbox,
        options,
        &mut permission_rx,
        &mut question_rx,
        permission_broker,
    )
    .await;
    restore_terminal();
    extensions.shutdown("tui exit").await;
    result
}

fn setup_terminal(mode: TuiRunMode) -> Result<Terminal<CrosstermBackend<std::io::Stdout>>> {
    install_panic_hook();
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    if mode == TuiRunMode::Full {
        execute!(stdout, EnterAlternateScreen)?;
    }
    execute!(stdout, EnableBracketedPaste, EnableMouseCapture)?;
    let terminal = Terminal::new(CrosstermBackend::new(stdout))?;
    Ok(terminal)
}

fn restore_terminal() {
    let _ = disable_raw_mode();
    let mut stdout = std::io::stdout();
    let _ = execute!(
        stdout,
        DisableBracketedPaste,
        DisableMouseCapture,
        LeaveAlternateScreen,
        ratatui::crossterm::cursor::Show
    );
}

fn install_panic_hook() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            restore_terminal();
            previous(info);
        }));
    });
}

#[derive(Debug)]
enum Input {
    Key(KeyEvent),
    Mouse(MouseEvent),
    Paste(String),
    Signal(i32),
}

/// A finished API-key probe.
enum ConnectEvent {
    /// The probe finished for one overlay attempt.
    Result {
        /// Provider checked.
        provider: String,
        /// Overlay generation that started the probe.
        generation: u64,
        /// Sanitized result.
        outcome: ProbeOutcome,
    },
}

/// Events produced by a manual or background update check.
enum UpdateEvent {
    /// The metadata check finished.
    Checked {
        /// True when the user asked from `/update`.
        manual: bool,
        /// Version comparison or an error.
        result: std::result::Result<codie_ai::update::UpdateStatus, String>,
    },
    /// A newer release was found, with the running and target versions.
    Installing {
        /// Version that is currently running.
        current: String,
        /// Version being installed.
        latest: String,
    },
    /// The verified archive download reported progress.
    Progress(codie_ai::update::UpdateProgress),
    /// Installation finished with the installed version or an error.
    Installed(std::result::Result<String, String>),
}

fn spawn_update(sender: &mpsc::UnboundedSender<UpdateEvent>, manual: bool) {
    let sender = sender.clone();
    tokio::spawn(async move {
        let result = codie_ai::update::check(None)
            .await
            .map_err(|error| error.to_string());

        if manual
            && let Ok(status) = &result
            && status.installable_release()
        {
            let current = status.current.clone();
            let version = status.latest.clone();
            let _ = sender.send(UpdateEvent::Installing {
                current,
                latest: version.clone(),
            });

            // Report only meaningful jumps so the overlay does not redraw for
            // every network chunk.
            let progress_sender = sender.clone();
            let mut last_percent = None;
            let mut last_bytes = 0_u64;
            let installed = codie_ai::update::install_with_progress(&version, move |event| {
                let percent = event.percent();
                let stepped = percent.is_some() && percent != last_percent;
                let advanced = event.downloaded.saturating_sub(last_bytes) >= 256 * 1024;
                if stepped || advanced {
                    last_percent = percent;
                    last_bytes = event.downloaded;
                    let _ = progress_sender.send(UpdateEvent::Progress(event));
                }
            })
            .await
            .map(|()| version)
            .map_err(|error| error.to_string());
            let _ = sender.send(UpdateEvent::Installed(installed));
            return;
        }

        let _ = sender.send(UpdateEvent::Checked { manual, result });
    });
}

/// Consumes a pending `/update` request and shows immediate feedback.
///
/// Returns true when the event loop should start a manual check. The request
/// is ignored while another update is in flight so repeated selections cannot
/// race two installs.
fn take_update_request(app: &mut App, in_flight: bool) -> bool {
    if !std::mem::take(&mut app.update_check_requested) {
        return false;
    }
    if in_flight {
        return false;
    }
    app.overlay = Some(Overlay::Notice {
        title: " update ".to_owned(),
        body: "Checking for updates…".to_owned(),
        ok: false,
    });
    true
}

/// Describes a newer release and how to install it.
fn update_available_message(status: &codie_ai::update::UpdateStatus) -> String {
    if status.installable == Some(false) {
        format!(
            "Update available: codie {} → {}.\n\nNo prebuilt binary is published for this platform yet; reinstall with:\n{}",
            status.current,
            status.latest,
            codie_ai::update::INSTALLER_COMMAND
        )
    } else {
        format!(
            "Update available: codie {} → {}. Run /update to install it.",
            status.current, status.latest
        )
    }
}

/// Renders the download progress shown while `/update` runs.
fn update_progress_message(
    current: &str,
    latest: &str,
    progress: codie_ai::update::UpdateProgress,
) -> String {
    let mut message = format!("Updating codie {current} → {latest}…");
    if let (Some(percent), Some(total)) = (progress.percent(), progress.total) {
        message.push_str(&format!(
            "\n\n{} {percent}%  ·  {} of {}",
            progress_bar(percent),
            format_bytes(progress.downloaded),
            format_bytes(total)
        ));
    } else {
        message.push_str(&format!(
            "\n\nDownloaded {}…",
            format_bytes(progress.downloaded)
        ));
    }
    message.push_str("\n\nVerifying the checksum and replacing the binary.");
    message
}

/// Builds a 24-cell bar such as `████░░░░` for a completion percentage.
fn progress_bar(percent: u8) -> String {
    const WIDTH: u8 = 24;
    let percent = percent.min(100);
    let filled = (u32::from(percent) * u32::from(WIDTH) / 100) as u8;
    let empty = WIDTH.saturating_sub(filled);
    format!(
        "[{}{}]",
        "█".repeat(usize::from(filled)),
        "░".repeat(usize::from(empty))
    )
}

/// Abbreviates byte counts for progress text: `999 B`, `1.2 KB`, `10.4 MB`.
fn format_bytes(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = 1024.0 * 1024.0;
    let value = bytes as f64;
    if value < KIB {
        format!("{bytes} B")
    } else if value < MIB {
        format!("{:.1} KB", value / KIB)
    } else {
        format!("{:.1} MB", value / MIB)
    }
}

fn spawn_input() -> mpsc::UnboundedReceiver<Input> {
    let (sender, receiver) = mpsc::unbounded_channel();
    std::thread::spawn({
        let sender = sender.clone();
        move || {
            loop {
                match ratatui::crossterm::event::read() {
                    Ok(Event::Key(key)) => {
                        if sender.send(Input::Key(key)).is_err() {
                            break;
                        }
                    }
                    Ok(Event::Mouse(mouse)) => {
                        if sender.send(Input::Mouse(mouse)).is_err() {
                            break;
                        }
                    }
                    Ok(Event::Paste(text)) => {
                        if sender.send(Input::Paste(text)).is_err() {
                            break;
                        }
                    }
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
        }
    });
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        for (kind, code) in [
            (SignalKind::interrupt(), 130),
            (SignalKind::terminate(), 143),
            (SignalKind::hangup(), 129),
        ] {
            let sender = sender.clone();
            tokio::spawn(async move {
                if let Ok(mut stream) = signal(kind)
                    && stream.recv().await.is_some()
                {
                    let _ = sender.send(Input::Signal(code));
                }
            });
        }
    }
    receiver
}

async fn submit_user_prompt(
    app: &mut App,
    extensions: &Arc<codie_ext::ExtensionSet>,
    text: String,
) -> Option<String> {
    let session = codie_protocol::ext::ExtSessionInfo {
        id: app.session.as_ref().map(|session| session.id().to_owned()),
        cwd: app.cwd.display().to_string(),
        model: Some(app.model.model.clone()),
    };
    let submitted = extensions.user_submit(&session, &text).await;
    if submitted.block {
        app.transcript.push_error(
            submitted
                .reason
                .unwrap_or_else(|| "prompt blocked by an extension".to_owned()),
        );
        None
    } else {
        Some(submitted.prompt.unwrap_or(text))
    }
}

async fn handle_extension_command(
    app: &mut App,
    extensions: &Arc<codie_ext::ExtensionSet>,
    text: &str,
) -> Result<bool> {
    let Some(invocation) = extensions
        .resolve_command(text)
        .map_err(anyhow::Error::msg)?
    else {
        return Ok(false);
    };
    let session = codie_protocol::ext::ExtSessionInfo {
        id: app.session.as_ref().map(|session| session.id().to_owned()),
        cwd: app.cwd.display().to_string(),
        model: Some(app.model.model.clone()),
    };
    match extensions.invoke_command(&invocation, &session).await {
        Ok(result) => {
            if let Some(prompt) = result.prompt {
                app.command_prompt = Some(prompt);
            } else if let Some(display) = result.display {
                let text = display.title.map_or(display.text.clone(), |title| {
                    format!("{title}\n{}", display.text)
                });
                app.transcript.push_info(text);
            } else {
                app.transcript.push_info("Extension command handled.");
            }
            Ok(true)
        }
        Err(error) => Err(anyhow::anyhow!(error.to_string())),
    }
}

async fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    app: &mut App,
    agent: Arc<Agent>,
    toolbox: Arc<dyn Toolbox>,
    options: TuiOptions,
    permission_rx: &mut mpsc::UnboundedReceiver<PermissionRequest>,
    question_rx: &mut mpsc::UnboundedReceiver<crate::questions::QuestionRequest>,
    broker: PermissionBroker,
) -> Result<u8> {
    let mut input = spawn_input();
    let (event_tx, mut event_rx) = mpsc::unbounded_channel();
    let (compact_tx, mut compact_rx) = mpsc::unbounded_channel();
    let (update_tx, mut update_rx) = mpsc::unbounded_channel::<UpdateEvent>();
    let (connect_tx, mut connect_rx) = mpsc::unbounded_channel::<ConnectEvent>();
    let mut tick = tokio::time::interval(Duration::from_millis(33));
    let mut dirty = true;
    let mut signal_code = 0_i32;

    if let Some(prompt) = options.initial_prompt.clone() {
        submit_prompt(app, &agent, &options, &event_tx, &compact_tx, prompt);
    }

    let mut auto_update_started = false;
    // True between a manual `/update` request and its final Checked/Installed
    // event, so repeated requests cannot run concurrent installs.
    let mut update_in_flight = false;
    // Versions shown on the progress overlay while an install runs.
    let mut updating: Option<(String, String)> = None;

    loop {
        if dirty || app.needs_redraw {
            app.needs_redraw = false;
            terminal.draw(|frame| render::draw(frame, app))?;
            dirty = false;
        }

        tokio::select! {
            Some(input) = input.recv() => {
                dirty = true;
                match input {
                    Input::Key(key) => {
                        let action = app.handle_key(key);
                        match action {
                            Action::None => {}
                            Action::Quit => break,
                            Action::ToggleAutoApprove => {
                                broker.set_auto_approve(app.auto_approve);
                                if app.auto_approve {
                                    if matches!(app.overlay, Some(Overlay::Permission(_))) {
                                        app.resolve_permission(Decision::Allow);
                                    }
                                    app.transcript
                                        .push_info("Full access on: mutating tools run without prompts.");
                                } else {
                                    app.transcript
                                        .push_info("Full access off: mutating tools ask again.");
                                }
                            }
                            Action::VerifyKey => {
                                if let Some((provider, key, generation)) =
                                    app.take_connect_request()
                                {
                                    let sender = connect_tx.clone();
                                    let base_url = options.base_url.clone();
                                    tokio::spawn(async move {
                                        let outcome =
                                            probe_key(&provider, &key, base_url.as_deref()).await;
                                        let _ = sender.send(ConnectEvent::Result {
                                            provider,
                                            generation,
                                            outcome,
                                        });
                                    });
                                }
                            }
                            Action::SaveKey => app.save_connect_key(),
                            Action::Cancel => {
                                app.cancel.cancel();
                                app.transcript.push_info("Stopping…");
                                clear_queued_messages(app, &agent);
                            }
                            Action::Submit(text) => {
                                let text = app.expand_attachment_labels(&text);
                                app.clear_attachments();
                                let text = match codie_prompt::attachment_token_for_path_text(
                                    &text, &app.cwd,
                                ) {
                                    Some(token) => token,
                                    None
                                        if codie_prompt::looks_like_path_text(&text)
                                            && matches!(
                                                parse_command(&text),
                                                Some(Command::Unknown(_))
                                            ) =>
                                    {
                                        if let Some(path) = crate::clipboard::read_image() {
                                            codie_prompt::quote_attachment_path(&path)
                                        } else {
                                            app.transcript.push_error(format!(
                                                "Cannot attach `{}`: the file no longer exists. \
                                                 Copy the screenshot and press Ctrl+V, or drag it again.",
                                                text.trim()
                                            ));
                                            continue;
                                        }
                                    }
                                    None => text,
                                };
                                let extension_candidate = text.trim_start().starts_with("/ext:")
                                    || matches!(
                                        parse_command(&text),
                                        Some(Command::Unknown(_))
                                    );
                                let extension_handled = if extension_candidate {
                                    match handle_extension_command(
                                        app,
                                        &options.extensions,
                                        &text,
                                    )
                                    .await
                                    {
                                        Ok(handled) => handled,
                                        Err(error) => {
                                            app.transcript.push_error(error.to_string());
                                            true
                                        }
                                    }
                                } else {
                                    false
                                };
                                if !extension_handled && app.handle_command(&text) {
                                    if app.extension_session_reset {
                                        app.extension_session_reset = false;
                                        options
                                            .extensions
                                            .session_shutdown("new session")
                                            .await;
                                        let session_info =
                                            codie_protocol::ext::ExtSessionInfo {
                                                id: app.session.as_ref().map(|session| {
                                                    session.id().to_owned()
                                                }),
                                                cwd: app.cwd.display().to_string(),
                                                model: Some(app.model.model.clone()),
                                            };
                                        options.extensions.set_session_id(session_info.id.clone());
                                        options
                                            .extensions
                                            .session_start(&session_info)
                                            .await;
                                    }
                                    if app.should_quit { break; }
                                    if let Some(prompt) = app.command_prompt.take()
                                        && let Some(prompt) = submit_user_prompt(
                                            app,
                                            &options.extensions,
                                            prompt,
                                        )
                                        .await
                                    {
                                        submit_prompt(
                                            app, &agent, &options, &event_tx, &compact_tx, prompt,
                                        );
                                    }
                                } else if extension_handled {
                                    if let Some(prompt) = app.command_prompt.take()
                                        && let Some(prompt) = submit_user_prompt(
                                            app,
                                            &options.extensions,
                                            prompt,
                                        )
                                        .await
                                    {
                                        submit_prompt(
                                            app, &agent, &options, &event_tx, &compact_tx, prompt,
                                        );
                                    }
                                } else if let Some(text) = submit_user_prompt(
                                    app,
                                    &options.extensions,
                                    text,
                                )
                                .await
                                {
                                    if app.status == Status::Streaming {
                                        agent.steer(Message::user(text.clone()));
                                        app.transcript.push_steering(text);
                                    } else {
                                        submit_prompt(
                                            app, &agent, &options, &event_tx, &compact_tx, text,
                                        );
                                    }
                                }
                            }
                            Action::FollowUp(text) => {
                                let text = app.expand_attachment_labels(&text);
                                app.clear_attachments();
                                if !text.trim().is_empty()
                                    && let Some(text) = submit_user_prompt(
                                        app,
                                        &options.extensions,
                                        text,
                                    )
                                    .await
                                {
                                    agent.follow_up(Message::user(text.clone()));
                                    app.transcript.push_steering(text);
                                }
                            }
                        }
                    }
                    Input::Mouse(mouse) => match mouse.kind {
                        MouseEventKind::ScrollUp => {
                            app.clear_selection();
                            app.transcript.scroll_up(3);
                        }
                        MouseEventKind::ScrollDown => {
                            app.clear_selection();
                            app.transcript.scroll_down(3);
                        }
                        MouseEventKind::Down(MouseButton::Left) => {
                            app.begin_selection(mouse.column, mouse.row);
                        }
                        MouseEventKind::Drag(MouseButton::Left) => {
                            app.drag_selection(mouse.column, mouse.row);
                        }
                        MouseEventKind::Up(MouseButton::Left) => {
                            if app.end_selection(mouse.column, mouse.row) {
                                app.handle_click(mouse.column, mouse.row);
                            }
                        }
                        _ => {}
                    },
                    Input::Paste(text) => app.paste(&text),
                    Input::Signal(code) => {
                        signal_code = code;
                        break;
                    }
                }
            }
            Some(request) = permission_rx.recv() => {
                dirty = true;
                app.push_permission(request);
            }
            Some(request) = question_rx.recv() => {
                dirty = true;
                app.push_question(request);
            }
            Some(event) = connect_rx.recv() => {
                dirty = true;
                match event {
                    ConnectEvent::Result {
                        provider,
                        generation,
                        outcome,
                    } => app.handle_connect_result(&provider, generation, outcome),
                }
            }
            Some(event) = event_rx.recv() => {
                dirty = true;
                handle_agent_event(app, event);
                if maybe_recover_overflow(app) {
                    start_compaction(app, &options, &compact_tx, String::new());
                } else if let Some(nudge) = app.pending_nudge.take() {
                    app.run_had_task = false;
                    app.transcript
                        .push_info("No delegation detected; asking once.");
                    start_run(
                        app,
                        &agent,
                        &options,
                        &event_tx,
                        Message::user(nudge.clone()),
                        nudge,
                    );
                }
            }
            Some(update) = update_rx.recv() => {
                dirty = true;
                match update {
                    UpdateEvent::Checked { manual, result } => {
                        if manual {
                            update_in_flight = false;
                        }
                        match result {
                            Ok(status) if status.newer() => {
                                let message = update_available_message(&status);
                                if manual {
                                    app.overlay = Some(Overlay::Notice {
                                        title: " update ".to_owned(),
                                        body: message,
                                        ok: true,
                                    });
                                } else {
                                    app.transcript.push_info(message);
                                }
                            }
                            Ok(status) if manual => {
                                app.overlay = Some(Overlay::Notice {
                                    title: " update ".to_owned(),
                                    body: format!(
                                        "No update found. codie {} is the latest release.",
                                        status.current
                                    ),
                                    ok: true,
                                });
                            }
                            Ok(_) => {}
                            Err(error) if manual => {
                                app.overlay = Some(Overlay::Notice {
                                    title: " update ".to_owned(),
                                    body: format!("Update check failed: {error}"),
                                    ok: true,
                                });
                            }
                            Err(_) => {}
                        }
                    }
                    UpdateEvent::Installing { current, latest } => {
                        updating = Some((current.clone(), latest.clone()));
                        app.overlay = Some(Overlay::Notice {
                            title: " update ".to_owned(),
                            body: format!("Updating codie {current} → {latest}…"),
                            ok: false,
                        });
                    }
                    UpdateEvent::Progress(progress) => {
                        if let Some((current, latest)) = &updating {
                            app.overlay = Some(Overlay::Notice {
                                title: " update ".to_owned(),
                                body: update_progress_message(current, latest, progress),
                                ok: false,
                            });
                        }
                    }
                    UpdateEvent::Installed(result) => {
                        update_in_flight = false;
                        updating = None;
                        app.overlay = Some(Overlay::Notice {
                            title: " update ".to_owned(),
                            body: match result {
                                Ok(version) => format!(
                                    "Updated to codie {version}.\nRestart codie to use the new version."
                                ),
                                Err(error) => format!("Update failed: {error}"),
                            },
                            ok: true,
                        });
                    }
                }
            }
            Some((instructions, result)) = compact_rx.recv() => {
                dirty = true;
                apply_compaction(app, instructions, result);
                if let Some(prompt) = app.pending_prompt.take() {
                    let retry = std::mem::take(&mut app.pending_retry);
                    if retry {
                        start_resume(app, &agent, &options, &event_tx);
                    } else {
                        let message = expand_prompt(app, &options.cwd, &prompt);
                        start_run(app, &agent, &options, &event_tx, message, prompt);
                    }
                }
            }
            _ = tick.tick() => {
                app.tick = app.tick.wrapping_add(1);
                app.refresh_child_overlay();
                if !auto_update_started {
                    auto_update_started = true;
                    // Start after the first frame so startup never waits on the network.
                    if std::env::var_os("CODIE_NO_UPDATE_CHECK").is_none() {
                        spawn_update(&update_tx, false);
                    }
                }
                if app.tick % 30 == 0
                    && app.theme_preview.is_none()
                    && let Some(theme) = app
                        .theme_watcher
                        .as_mut()
                        .and_then(crate::theme::ThemeWatcher::poll)
                {
                    app.theme = theme;
                    app.theme_revision = app.theme_revision.wrapping_add(1);
                    dirty = true;
                }
                if app.status == Status::Streaming
                    || matches!(
                        app.overlay,
                        Some(Overlay::Connect(ConnectState {
                            phase: ConnectPhase::Checking,
                            ..
                        }))
                    )
                {
                    dirty = true;
                }
            }
        }

        // `/update` can arrive from a submitted prompt or from the command
        // palette, which calls `handle_command` directly. Both only set the
        // flag, so it is serviced here for either path.
        if take_update_request(app, update_in_flight) {
            update_in_flight = true;
            spawn_update(&update_tx, true);
        }

        if let Some(instructions) = app.pending_compact.take() {
            if app.status == Status::Streaming {
                app.pending_compact = Some(instructions);
            } else {
                app.transcript.push_info("Compacting the context…");
                app.status = Status::Streaming;
                start_compaction(app, &options, &compact_tx, instructions);
            }
        }

        if std::mem::take(&mut app.drop_queued) {
            clear_queued_messages(app, &agent);
        }

        if app.should_quit {
            break;
        }
    }

    if signal_code == 0 {
        signal_code = app.signal.load(std::sync::atomic::Ordering::SeqCst);
    }
    let _ = toolbox;
    Ok(u8::try_from(signal_code).unwrap_or(0))
}

/// Builds the rich preview shown in a permission prompt.
fn permission_preview(
    cwd: &std::path::Path,
    tool: &str,
    arguments: &Value,
) -> Option<codie_protocol::ext::ExtRenderHint> {
    let path = arguments
        .get("path")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let resolved = if std::path::Path::new(path).is_absolute() {
        std::path::PathBuf::from(path)
    } else {
        cwd.join(path)
    };
    match tool {
        "edit" => {
            if std::fs::metadata(&resolved).is_ok_and(|meta| meta.len() > 2 * 1024 * 1024) {
                return None;
            }
            let old = std::fs::read_to_string(&resolved).ok()?;
            let mut new = old.clone();
            if let Some(edits) = arguments.get("edits").and_then(Value::as_array) {
                for edit in edits {
                    let Some(old_text) = edit.get("oldText").and_then(Value::as_str) else {
                        continue;
                    };
                    let new_text = edit
                        .get("newText")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    if !old_text.is_empty() && new.contains(old_text) {
                        new = new.replacen(old_text, new_text, 1);
                    }
                }
            }
            if new == old {
                return None;
            }
            diff_preview(path, &old, &new, "Edit")
        }
        "write" => {
            let content = arguments.get("content").and_then(Value::as_str)?;
            let can_diff =
                std::fs::metadata(&resolved).is_ok_and(|meta| meta.len() <= 2 * 1024 * 1024);
            if can_diff
                && let Ok(old) = std::fs::read_to_string(&resolved)
                && old != content
                && let Some(preview) = diff_preview(path, &old, content, "Write")
            {
                return Some(preview);
            }
            Some(codie_protocol::ext::ExtRenderHint {
                kind: codie_protocol::ext::ExtRenderKind::Code,
                title: Some(format!("Write {path}")),
                text: content.to_owned(),
            })
        }
        "bash" | "powershell" => {
            let command = arguments.get("command").and_then(Value::as_str)?;
            Some(codie_protocol::ext::ExtRenderHint {
                kind: codie_protocol::ext::ExtRenderKind::Code,
                title: Some("Command".to_owned()),
                text: command.to_owned(),
            })
        }
        "project_memory" => {
            let file = arguments
                .get("file")
                .and_then(Value::as_str)
                .unwrap_or("codie");
            let target = match file {
                "plan" => codie_prompt::PROJECT_PLAN_FILE,
                _ => codie_prompt::PROJECT_MEMORY_FILE,
            };
            let section = arguments
                .get("section")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let text = arguments
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let root = codie_prompt::project_root(cwd);
            let target_path = root.join(target);
            Some(codie_protocol::ext::ExtRenderHint {
                kind: codie_protocol::ext::ExtRenderKind::Code,
                title: Some(format!("Memory · {target} · {section}")),
                text: format!(
                    "{}\n\n{}",
                    target_path.display(),
                    codie_tools::project_memory::redact(text)
                ),
            })
        }
        _ => None,
    }
}

fn diff_preview(
    path: &str,
    old: &str,
    new: &str,
    verb: &str,
) -> Option<codie_protocol::ext::ExtRenderHint> {
    const MAX_LINES: usize = 600;
    if old.lines().count() > 20_000 || new.lines().count() > 20_000 {
        return None;
    }
    let diff = similar::TextDiff::from_lines(old, new);
    let mut unified = diff.unified_diff();
    unified.context_radius(3).header(path, path);
    let text = unified.to_string();
    if text.lines().count() > MAX_LINES {
        return None;
    }
    Some(codie_protocol::ext::ExtRenderHint {
        kind: codie_protocol::ext::ExtRenderKind::Diff,
        title: Some(format!("{verb} {path}")),
        text,
    })
}

/// Drops queued steering and follow-up messages after a stop.
fn clear_queued_messages(app: &mut App, agent: &Agent) {
    let (steering, follow_up) = agent.clear_queues();
    let dropped = steering.len() + follow_up.len();
    if dropped > 0 {
        app.transcript
            .push_info(format!("Dropped {dropped} queued message(s)."));
    }
}

fn submit_prompt(
    app: &mut App,
    agent: &Arc<Agent>,
    options: &TuiOptions,
    sender: &mpsc::UnboundedSender<AgentEvent>,
    compact_sender: &mpsc::UnboundedSender<(
        Option<String>,
        Result<crate::compaction::CompactionOutcome, String>,
    )>,
    prompt: String,
) {
    app.last_prompt = Some(prompt.clone());
    app.overflow_retried = false;

    let intent = detect_intent(&prompt);
    let plan = effective_mode(
        app.subagent_config.enabled,
        app.subagent_mode,
        app.subagent_force_once,
        intent,
    );
    app.subagent_force_once = false;
    app.forced_run = plan.is_forced();
    app.run_had_task = false;
    app.nudge.reset();
    app.pending_nudge = None;
    if let Some(runtime) = &app.subagent_runtime {
        runtime.set_mode(plan.mode);
    }
    if plan.is_forced() {
        let directive = Message::system(FORCED_DIRECTIVE, Vec::new());
        if let Some(session) = app.session.as_mut() {
            let _ = session.append_message(&directive);
        }
        app.history.push(directive);
        app.transcript
            .push_info("Subagents requested for this run.");
    } else if intent != DelegationIntent::None {
        app.transcript
            .push_info(format!("Subagents: {} for this run.", plan.mode.as_str()));
    }

    let prompt_message = expand_prompt(app, &options.cwd, &prompt);
    let window = u64::from(app.context_window);
    let history_has_messages = app
        .history
        .iter()
        .any(|message| !matches!(message, Message::System(_)));
    if app.session.is_some()
        && history_has_messages
        && crate::compaction::needs_compaction(&app.history, &prompt_message, window)
    {
        app.pending_prompt = Some(prompt);
        app.pending_retry = false;
        app.status = Status::Streaming;
        app.transcript
            .push_info("Compacting the context before this request…");
        start_compaction(app, options, compact_sender, String::new());
        return;
    }

    start_run(app, agent, options, sender, prompt_message, prompt);
}

/// Expands `@path` references and reports attachment problems in the transcript.
fn expand_prompt(app: &mut App, cwd: &std::path::Path, prompt: &str) -> Message {
    let expansion = codie_prompt::expand_attachments(prompt, cwd);
    for error in expansion.errors {
        app.transcript.push_error(error);
    }
    if !expansion.attached.is_empty() {
        app.transcript
            .push_info(format!("Attached {} file(s).", expansion.attached.len()));
    }
    Message::user_with_content(expansion.content)
}

/// Applies the state and configuration shared by every run.
fn prepare_run(app: &mut App, options: &TuiOptions) -> Option<AgentConfig> {
    app.status = Status::Streaming;
    app.run_started = Some(std::time::Instant::now());
    app.footer.tokens_per_sec = None;

    let mut config = match AgentConfig::new(app.model.clone(), options.cwd.clone()) {
        Ok(config) => config,
        Err(error) => {
            app.transcript.push_error(error);
            app.status = Status::Error;
            return None;
        }
    };
    config.thinking = app.thinking;
    config.max_turns = options.max_turns;
    config.session_id = app.session.as_ref().map(|session| session.id().to_owned());
    app.cancel = tokio_util::sync::CancellationToken::new();
    config.cancel = app.cancel.clone();
    Some(config)
}

fn start_run(
    app: &mut App,
    agent: &Arc<Agent>,
    options: &TuiOptions,
    sender: &mpsc::UnboundedSender<AgentEvent>,
    message: Message,
    prompt: String,
) {
    app.transcript.push_user(prompt);
    let Some(config) = prepare_run(app, options) else {
        return;
    };
    let history = app.history.clone();
    forward_events(agent.run(message, history, config), sender.clone());
}

fn start_resume(
    app: &mut App,
    agent: &Arc<Agent>,
    options: &TuiOptions,
    sender: &mpsc::UnboundedSender<AgentEvent>,
) {
    let Some(config) = prepare_run(app, options) else {
        return;
    };
    let history = app.history.clone();
    forward_events(agent.resume(history, config), sender.clone());
}

fn forward_events(
    mut stream: codie_core::AgentEventStream,
    sender: mpsc::UnboundedSender<AgentEvent>,
) {
    tokio::spawn(async move {
        while let Some(event) = stream.recv().await {
            let terminal = matches!(event, AgentEvent::AgentEnd { .. });
            if sender.send(event).is_err() || terminal {
                break;
            }
        }
    });
}

fn maybe_recover_overflow(app: &mut App) -> bool {
    if app.status != Status::Error || app.overflow_retried {
        return false;
    }
    let Some(prompt) = app.last_prompt.clone() else {
        return false;
    };
    let overflow = app
        .history
        .last()
        .is_some_and(crate::compaction::is_context_overflow);
    if !overflow {
        return false;
    }

    app.overflow_retried = true;
    if matches!(
        app.history.last(),
        Some(Message::Assistant(assistant))
            if assistant.stop_reason == codie_core::StopReason::Error
    ) {
        app.history.pop();
    }
    if let Some(session) = app.session.as_mut()
        && let Some(leaf) = session.leaf_target().map(str::to_owned)
        && session
            .entries()
            .iter()
            .any(|entry| entry.id() == leaf && entry.message().is_some())
    {
        let _ = session.append_context_edit(&leaf, None);
        app.history = session.projection();
    }

    app.pending_prompt = Some(prompt);
    app.pending_retry = true;
    app.status = Status::Streaming;
    app.transcript
        .push_info("Context overflow: compacting and retrying once…");
    true
}

fn start_compaction(
    app: &mut App,
    options: &TuiOptions,
    sender: &mpsc::UnboundedSender<(
        Option<String>,
        Result<crate::compaction::CompactionOutcome, String>,
    )>,
    instructions: String,
) {
    let client = Arc::clone(&options.client);
    let model = app.model.clone();
    let thinking = app.thinking;
    let messages = app.history.clone();
    let session_id = app.session.as_ref().map(|session| session.id().to_owned());
    let context_window = app.context_window;
    let sender = sender.clone();
    app.cancel = tokio_util::sync::CancellationToken::new();
    let cancel = app.cancel.clone();
    tokio::spawn(async move {
        let custom = (!instructions.trim().is_empty()).then_some(instructions.as_str());
        let input_limit = u64::from(context_window)
            .saturating_sub(crate::compaction::DEFAULT_RESERVE_TOKENS)
            .clamp(4_000, 200_000);
        let result = crate::compaction::summarize_with_cancel(
            client,
            model,
            thinking,
            &messages,
            custom,
            session_id,
            input_limit,
            cancel,
        )
        .await;
        let _ = sender.send((
            (!instructions.trim().is_empty()).then_some(instructions),
            result,
        ));
    });
}

fn apply_compaction(
    app: &mut App,
    instructions: Option<String>,
    result: Result<crate::compaction::CompactionOutcome, String>,
) {
    enum Applied {
        Done(Vec<Message>, u8, String),
        Failed(String),
    }

    let window = app.context_window;
    let stopped = app.cancel.is_cancelled();
    let applied = {
        let Some(session) = app.session.as_mut() else {
            app.status = Status::Error;
            app.transcript
                .push_error("Compaction finished but no session is active.");
            return;
        };
        match result {
            Ok(outcome) => {
                match crate::compaction::apply(session, &outcome, instructions.as_deref()) {
                    Ok(history) => Applied::Done(
                        history,
                        session.context_percent(u64::from(window)),
                        outcome
                            .summary
                            .lines()
                            .next()
                            .unwrap_or("summary")
                            .to_owned(),
                    ),
                    Err(error) => Applied::Failed(error),
                }
            }
            Err(error) => Applied::Failed(error),
        }
    };

    match applied {
        Applied::Done(history, percent, first_line) => {
            app.history = history;
            app.refresh_context();
            app.footer.context_percent = Some(percent);
            app.status = Status::Idle;
            app.transcript
                .push_info(format!("Compacted the context: {first_line}"));
            if stopped {
                app.pending_prompt = None;
                app.pending_retry = false;
                app.transcript.push_info("Stopped.");
            }
        }
        Applied::Failed(error) => {
            app.pending_prompt = None;
            app.pending_retry = false;
            if stopped {
                app.status = Status::Idle;
                app.transcript.push_info("Stopped.");
            } else {
                app.status = Status::Error;
                app.transcript
                    .push_error(format!("Compaction failed: {error}"));
            }
        }
    }
}

fn assistant_auth_failed(message: &codie_core::AssistantMessage) -> bool {
    let Some(error) = &message.error_message else {
        return false;
    };
    let error = error.to_ascii_lowercase();
    error.contains("401")
        || error.contains("403")
        || error.contains("unauthorized")
        || error.contains("invalid api key")
        || error.contains("invalid_api_key")
        || error.contains("authentication")
}

fn handle_agent_event(app: &mut App, event: AgentEvent) {
    match event {
        AgentEvent::AgentStart => app.status = Status::Streaming,
        AgentEvent::MessageStart { message } => match message {
            Message::Assistant(assistant) => {
                app.transcript.start_assistant();
                if !assistant.text().is_empty() {
                    app.transcript.set_text(assistant.text());
                }
            }
            Message::ToolResult(_) => {}
            _ => {}
        },
        AgentEvent::MessageUpdate { message, .. } => {
            if let Message::Assistant(assistant) = message {
                app.transcript.set_text(assistant.text());
                if let Some(started) = app.run_started {
                    let seconds = started.elapsed().as_secs_f64();
                    if seconds > 0.25 {
                        let tokens = assistant.text().chars().count().div_ceil(4) as f64;
                        app.footer.tokens_per_sec = Some(tokens / seconds);
                    }
                }
                let thinking = assistant
                    .content
                    .iter()
                    .filter_map(|content| match content {
                        Content::Thinking { thinking, .. } => Some(thinking.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("");
                app.transcript.set_thinking(thinking);
            }
        }
        AgentEvent::MessageEnd { message } => {
            app.persist(&message);
            match &message {
                Message::Assistant(assistant) => {
                    app.transcript.set_text(assistant.text());
                    app.transcript.finish_assistant();
                    app.update_usage(
                        &assistant.usage,
                        assistant.duration_ms.or_else(|| {
                            app.run_started
                                .map(|started| started.elapsed().as_millis() as u64)
                        }),
                    );
                    if assistant_auth_failed(assistant) && app.overlay.is_none() {
                        let provider = app.model.provider.clone();
                        app.transcript.push_error(
                            "The provider rejected the API key. Enter a new key to continue.",
                        );
                        app.open_connect_overlay(&provider);
                    }
                }
                Message::ToolResult(result) => persist_tool_result(app, result),
                _ => {}
            }
        }
        AgentEvent::ToolExecutionStart {
            tool_call_id,
            tool_name,
            args,
        } => {
            if tool_name == "task" {
                app.run_had_task = true;
            }
            app.transcript.tool_start(tool_call_id, tool_name, args);
        }
        AgentEvent::ToolExecutionEnd {
            tool_call_id,
            result,
            is_error,
            ..
        } => {
            let output = result
                .content
                .iter()
                .filter_map(Content::as_text)
                .collect::<Vec<_>>()
                .join("\n");
            let render = result
                .details
                .as_ref()
                .and_then(|details| details.get("codie_render"))
                .and_then(|value| serde_json::from_value(value.clone()).ok());
            app.transcript
                .tool_end(&tool_call_id, output, is_error, render, result.details);
        }
        AgentEvent::TurnEnd { .. } | AgentEvent::TurnStart => {}
        AgentEvent::AgentEnd { messages, status } => {
            app.history = messages;
            app.status = match status {
                RunStatus::Completed | RunStatus::Aborted => Status::Idle,
                RunStatus::TurnLimit | RunStatus::Error => Status::Error,
            };
            app.transcript.scroll_to_bottom();
            if status == RunStatus::Aborted {
                app.transcript.push_info("Stopped.");
            }
            if status == RunStatus::TurnLimit {
                app.transcript
                    .push_error("Turn limit reached. Send another prompt to continue.");
            }
            let forced = app.forced_run;
            let delegated = app.run_had_task;
            if matches!(status, RunStatus::Completed | RunStatus::TurnLimit)
                && let Some(nudge) = app.nudge.after_turn(forced, delegated)
            {
                app.pending_nudge = Some(nudge.to_owned());
            }
        }
        AgentEvent::ToolExecutionUpdate {
            tool_call_id,
            partial,
            ..
        } => {
            let label = partial
                .content
                .iter()
                .filter_map(Content::as_text)
                .collect::<Vec<_>>()
                .join(" ");
            app.transcript
                .tool_progress(&tool_call_id, label, partial.details);
        }
    }
}

fn persist_tool_result(app: &mut App, result: &ToolResultMessage) {
    app.transcript.tool_end(
        &result.tool_call_id,
        result
            .content
            .iter()
            .filter_map(Content::as_text)
            .collect::<Vec<_>>()
            .join("\n"),
        result.is_error,
        result
            .details
            .as_ref()
            .and_then(|details| details.get("codie_render"))
            .and_then(|value| serde_json::from_value(value.clone()).ok()),
        result.details.clone(),
    );

    // Child usage is not in the parent's assistant messages; carry it into the
    // footer and the session ledger so cost and `/usage` stay truthful.
    if result.tool_name == "task"
        && let Some(usage) = result.details.as_ref().and_then(subagent_usage)
    {
        app.footer.tokens = app.footer.tokens.saturating_add(usage.total_tokens);
        app.footer.input_tokens = app.footer.input_tokens.saturating_add(usage.input);
        app.footer.output_tokens = app.footer.output_tokens.saturating_add(usage.output);
        app.footer.cost = app.footer.cost.saturating_add(usage.cost.total);
        let provider = app.model.provider.clone();
        let model = app.model.model.clone();
        if let Some(session) = app.session.as_mut() {
            let _ = session.append_usage("subagent", provider, model, usage);
        }
    }
}

/// Rebuilds child usage from a `task` result's structured details.
fn subagent_usage(details: &Value) -> Option<Usage> {
    let usage = details.get("usage")?;
    let input = usage.get("input_tokens")?.as_u64()?;
    let output = usage
        .get("output_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let total = usage
        .get("total_tokens")
        .and_then(Value::as_u64)
        .unwrap_or_else(|| input.saturating_add(output));
    let nanos = codie_core::Cost::nanos_from_usd(
        usage
            .get("cost_usd")
            .and_then(Value::as_f64)
            .unwrap_or_default(),
    );
    Some(Usage {
        input,
        output,
        cache_read: 0,
        cache_write: 0,
        reasoning: None,
        total_tokens: total,
        cost: codie_core::Cost {
            total: nanos,
            total_from_provider: true,
            ..codie_core::Cost::default()
        },
    })
}

/// Renders a child session as plain transcript lines for the read-only overlay.
fn child_transcript_lines(messages: &[Message]) -> Vec<String> {
    let mut lines = Vec::new();
    for message in messages {
        match message {
            Message::System(_) => {}
            Message::User(user) => {
                let text = user
                    .content
                    .iter()
                    .filter_map(Content::as_text)
                    .collect::<Vec<_>>()
                    .join("\n");
                push_transcript_block(&mut lines, "task", &text, usize::MAX);
            }
            Message::Assistant(assistant) => {
                push_transcript_block(&mut lines, "child", &assistant.text(), usize::MAX);
                for content in &assistant.content {
                    if let Content::ToolCall {
                        name, arguments, ..
                    } = content
                    {
                        let summary = arguments
                            .get("command")
                            .or_else(|| arguments.get("path"))
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or_default();
                        lines.push(format!("  → {name} {summary}"));
                    }
                }
            }
            Message::ToolResult(result) => {
                let text = result
                    .content
                    .iter()
                    .filter_map(Content::as_text)
                    .collect::<Vec<_>>()
                    .join("\n");
                let marker = if result.is_error { "✗" } else { "✓" };
                let head = text
                    .lines()
                    .filter(|line| !line.trim().is_empty())
                    .take(6)
                    .map(|line| format!("    {line}"))
                    .collect::<Vec<_>>()
                    .join("\n");
                if head.is_empty() {
                    lines.push(format!("  {marker} {}", result.tool_name));
                } else {
                    lines.push(format!("  {marker} {}\n{head}", result.tool_name));
                }
            }
        }
    }
    lines
}

fn push_transcript_block(lines: &mut Vec<String>, role: &str, text: &str, limit: usize) {
    let text = text.trim();
    if text.is_empty() {
        return;
    }
    if !lines.is_empty() {
        lines.push(String::new());
    }
    lines.push(format!("{role}:"));
    let content = text
        .lines()
        .take(limit)
        .map(|line| format!("  {line}"))
        .collect::<Vec<_>>()
        .join("\n");
    lines.push(content);
}

fn git_branch(cwd: &std::path::Path) -> Option<String> {
    let mut directory = Some(cwd);
    while let Some(current) = directory {
        let head = current.join(".git/HEAD");
        if let Ok(contents) = std::fs::read_to_string(&head) {
            let contents = contents.trim();
            return contents
                .strip_prefix("ref: refs/heads/")
                .map(str::to_owned)
                .or_else(|| Some(contents.chars().take(8).collect()));
        }
        directory = current.parent();
    }
    None
}

/// Returns the current directory and branch for tests and the footer.
#[must_use]
pub fn context_label(cwd: &std::path::Path) -> String {
    let mut label = cwd.display().to_string();
    if let Some(branch) = git_branch(cwd) {
        label.push_str(&format!(" ({branch})"));
    }
    label
}

#[cfg(test)]
mod tests {
    use super::*;
    use codie_testkit::spec;

    fn app() -> App {
        App::new(
            ".",
            ModelRef {
                api: "openai-completions".into(),
                provider: "deepseek".into(),
                model: "deepseek-flash".into(),
            },
            ThinkingLevel::Medium,
            128_000,
            Vec::new(),
            Theme::load(Some("dark")).unwrap(),
            None,
            Vec::new(),
            PathBuf::from("."),
            Arc::new(std::sync::atomic::AtomicI32::new(0)),
            Bindings::default(),
        )
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn run(session_id: Option<String>) -> RunSnapshot {
        RunSnapshot {
            call_id: "call_1".to_owned(),
            agent: "explore".to_owned(),
            description: "Map the parser".to_owned(),
            depth: 1,
            session_id,
            turns: 2,
            activity: "completed".to_owned(),
            state: codie_subagents::RunState::Completed,
            elapsed: std::time::Duration::from_secs(5),
            cost_nanos: 0,
            error: None,
            persisted: true,
        }
    }

    #[test]
    #[spec("TUI-041")]
    fn open_child_run_reports_missing_transcripts() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app();
        app.session_dir = dir.path().to_path_buf();

        app.open_child_run(1, &[]);
        assert!(matches!(
            app.transcript.entries().last(),
            Some(Entry::Error(text)) if text.contains("No subagent run 1")
        ));

        app.open_child_run(1, &[run(None)]);
        assert!(matches!(
            app.transcript.entries().last(),
            Some(Entry::Error(text)) if text.contains("was not persisted")
        ));

        app.open_child_run(1, &[run(Some("ses_missing".to_owned()))]);
        assert!(matches!(
            app.transcript.entries().last(),
            Some(Entry::Error(text)) if text.contains("Cannot open child session")
        ));
    }

    #[test]
    #[spec("TUI-041")]
    fn open_child_run_renders_the_child_transcript() {
        let dir = tempfile::tempdir().unwrap();
        let mut child = Session::create_child(dir.path(), dir.path(), "ses_parent").unwrap();
        child
            .append_message(&Message::user("review the header"))
            .unwrap();
        child
            .append_message(&Message::Assistant(codie_core::AssistantMessage::partial(
                "test-api",
                "test-provider",
                "test-model",
            )))
            .unwrap();
        let id = child.id().to_owned();

        let mut app = app();
        app.session_dir = dir.path().to_path_buf();
        app.open_child_run(1, &[run(Some(id))]);
        match &app.overlay {
            Some(Overlay::Child { lines, .. }) => {
                let body = lines.join("\n");
                assert!(body.contains("review the header"), "{body}");
            }
            _ => panic!("expected child overlay"),
        }
    }

    struct NoopChat;

    impl ChatClient for NoopChat {
        fn stream(
            &self,
            _context: &[Message],
            _options: &codie_core::StreamOptions,
        ) -> codie_core::AssistantEventStream {
            codie_core::AssistantEventStream::channel(1).0
        }
    }

    fn subagent_runtime(app: &App, dir: &std::path::Path) -> Arc<SubagentRuntime> {
        Arc::new(
            SubagentRuntime::new(
                SubagentConfig::default(),
                Arc::new(NoopChat),
                Arc::new(codie_tools::Registry::new(Vec::new())),
                PromptInput::new(dir),
                dir,
                None,
                app.model.clone(),
                Vec::new(),
            )
            .expect("runtime"),
        )
    }

    #[test]
    #[spec("TUI-049")]
    fn task_result_usage_reaches_the_footer_and_session() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app();
        app.session = Some(Session::create(dir.path(), dir.path()).unwrap());
        let result = ToolResultMessage {
            tool_call_id: "call_1".to_owned(),
            tool_name: "task".to_owned(),
            content: vec![Content::text("done")],
            details: Some(serde_json::json!({
                "usage": {
                    "input_tokens": 10,
                    "output_tokens": 5,
                    "total_tokens": 15,
                    "cost_usd": 0.25
                }
            })),
            is_error: false,
            duration_ms: None,
            timestamp: 0,
        };
        persist_tool_result(&mut app, &result);
        assert_eq!(app.footer.tokens, 15);
        assert_eq!(app.footer.input_tokens, 10);
        assert_eq!(app.footer.cost, 250_000_000);
        assert!(
            app.session
                .as_ref()
                .unwrap()
                .entries()
                .iter()
                .any(|entry| matches!(
                    entry,
                    codie_session::Entry::Usage { kind, .. } if kind == "subagent"
                ))
        );
    }

    #[test]
    #[spec("AGENT-030")]
    fn forced_run_without_task_queues_one_nudge() {
        let mut app = app();
        app.forced_run = true;
        app.run_had_task = false;
        app.nudge.reset();

        handle_agent_event(
            &mut app,
            AgentEvent::AgentEnd {
                messages: Vec::new(),
                status: RunStatus::Completed,
            },
        );
        assert!(app.pending_nudge.is_some());

        app.pending_nudge = None;
        handle_agent_event(
            &mut app,
            AgentEvent::AgentEnd {
                messages: Vec::new(),
                status: RunStatus::Completed,
            },
        );
        assert!(app.pending_nudge.is_none(), "only one nudge per prompt");
    }

    #[test]
    fn delegated_run_does_not_queue_a_nudge() {
        let mut app = app();
        app.forced_run = true;
        app.run_had_task = true;
        app.nudge.reset();
        handle_agent_event(
            &mut app,
            AgentEvent::AgentEnd {
                messages: Vec::new(),
                status: RunStatus::Completed,
            },
        );
        assert!(app.pending_nudge.is_none());
    }

    #[test]
    #[spec("AGENT-036")]
    fn model_changes_reach_the_subagent_runtime() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app();
        let runtime = subagent_runtime(&app, dir.path());
        app.subagent_runtime = Some(Arc::clone(&runtime));

        app.model.provider = "openrouter".to_owned();
        app.model.model = "tencent/hy4-flash".to_owned();
        app.sync_subagent_model();

        let parent = runtime.parent_model();
        assert_eq!(parent.provider, "openrouter");
        assert_eq!(parent.model, "tencent/hy4-flash");

        app.thinking = ThinkingLevel::High;
        app.sync_subagent_thinking();
        assert_eq!(runtime.thinking(), ThinkingLevel::High);
    }

    #[test]
    #[spec("TUI-046")]
    fn reload_applies_project_agent_definitions() {
        let config = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let path = project.path().join(".codie/agents/reviewer.md");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            "---\ndescription: Reviews code\n---\nReview the change.\n",
        )
        .unwrap();

        let mut app = app();
        app.customizations = Customizations::load(config.path(), Some(project.path()));
        app.subagent_base = SubagentConfig::default();
        app.subagent_config = SubagentConfig::default();

        assert!(app.handle_command("/reload"));
        assert!(
            app.subagent_config.agent_type("reviewer").is_some(),
            "reload must add the persona without a restart"
        );
    }

    #[test]
    #[spec("TUI-047")]
    fn open_child_overlay_refreshes_while_it_grows() {
        let dir = tempfile::tempdir().unwrap();
        let mut child = Session::create_child(dir.path(), dir.path(), "ses_parent").unwrap();
        child.append_message(&Message::user("first line")).unwrap();
        let id = child.id().to_owned();

        let mut app = app();
        app.session_dir = dir.path().to_path_buf();
        app.open_child_run(1, &[run(Some(id))]);
        child.append_message(&Message::user("second line")).unwrap();

        if let Some(Overlay::Child { updated, .. }) = &mut app.overlay {
            *updated = Some(std::time::Instant::now() - std::time::Duration::from_secs(2));
        }
        app.refresh_child_overlay();
        match &app.overlay {
            Some(Overlay::Child { lines, .. }) => {
                let body = lines.join("\n");
                assert!(body.contains("second line"), "{body}");
            }
            _ => panic!("expected child overlay"),
        }
    }

    #[test]
    #[spec("TUI-021")]
    fn scroll_keys_leave_follow_mode_and_esc_returns_to_bottom() {
        let mut app = app();
        app.transcript.push_user("one");
        app.transcript.push_user("two");
        app.transcript.clamp_scroll(100);

        app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::SHIFT));
        assert!(!app.transcript.is_following());
        assert_eq!(app.transcript.scroll(), 92);

        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::SHIFT));
        assert_eq!(app.transcript.scroll(), 100);
        assert!(app.transcript.is_following());

        app.handle_key(KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE));
        assert!(!app.transcript.is_following());
        app.handle_key(key(KeyCode::Esc));
        assert!(app.transcript.is_following());
    }

    #[test]
    fn enter_submits_and_shift_enter_inserts_a_newline() {
        let mut app = app();
        app.editor.insert_str("hello");
        assert_eq!(
            app.handle_key(key(KeyCode::Enter)),
            Action::Submit("hello".to_owned())
        );

        app.editor.insert_str("a");
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT));
        app.editor.insert_str("b");
        assert_eq!(app.editor.text(), "a\nb");
    }

    #[test]
    fn slash_commands_are_handled_before_runs() {
        let mut app = app();
        assert!(app.handle_command("/help"));
        assert!(matches!(app.overlay, Some(Overlay::Help)));
        app.handle_key(key(KeyCode::Esc));
        assert!(app.overlay.is_none());
        assert!(app.handle_command("/thinking high"));
        assert_eq!(app.thinking, ThinkingLevel::High);
        assert!(!app.handle_command("hello"));
    }

    #[test]
    fn ctrl_c_cancels_then_quits() {
        let mut app = app();
        app.status = Status::Streaming;
        assert_eq!(
            app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Action::Cancel
        );
        app.status = Status::Idle;
        assert_eq!(
            app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Action::Quit
        );
    }

    #[test]
    fn ctrl_c_and_ctrl_d_are_not_swallowed_by_overlays() {
        let mut idle = app();
        assert!(idle.handle_command("/help"));
        assert_eq!(
            idle.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Action::Quit
        );

        let mut running = app();
        assert!(running.handle_command("/help"));
        running.status = Status::Streaming;
        assert_eq!(
            running.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Action::Cancel
        );
        assert_eq!(running.status, Status::Waiting);
        assert_eq!(
            running.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Action::Quit
        );

        let mut quitting = app();
        assert!(quitting.handle_command("/help"));
        assert_eq!(
            quitting.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL)),
            Action::Quit
        );

        // The connect overlay owns Ctrl+C and still quits even mid-run.
        let mut connecting = app();
        connecting.overlay = Some(Overlay::Connect(ConnectState::new("openrouter")));
        connecting.status = Status::Streaming;
        assert_eq!(
            connecting.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Action::Quit
        );
    }

    #[test]
    #[spec("TUI-023")]
    fn escape_stops_a_running_turn() {
        let mut app = app();
        app.status = Status::Streaming;
        assert_eq!(app.handle_key(key(KeyCode::Esc)), Action::Cancel);
        assert_eq!(app.status, Status::Waiting);
    }

    #[test]
    #[spec("TUI-023")]
    fn escape_collapses_reasoning_before_stopping() {
        let mut app = app();
        app.status = Status::Streaming;
        app.transcript.start_assistant();
        app.transcript.set_thinking("weighing options".into());
        assert!(app.transcript.toggle_thinking(0));

        assert_eq!(app.handle_key(key(KeyCode::Esc)), Action::None);
        assert_eq!(app.status, Status::Streaming);
        assert!(matches!(
            &app.transcript.entries()[0],
            Entry::Assistant(assistant) if !assistant.thinking_expanded
        ));

        assert_eq!(app.handle_key(key(KeyCode::Esc)), Action::Cancel);
    }

    #[test]
    #[spec("TUI-023")]
    fn slash_stop_cancels_the_active_run() {
        let mut app = app();
        app.status = Status::Streaming;
        assert!(app.handle_command("/stop"));
        assert_eq!(app.status, Status::Waiting);
        assert!(app.cancel.is_cancelled());
        assert!(app.drop_queued);
    }

    #[test]
    #[spec("TUI-023")]
    fn slash_stop_reports_when_nothing_is_running() {
        let mut app = app();
        assert!(app.handle_command("/stop"));
        assert!(!app.cancel.is_cancelled());
        assert!(matches!(
            app.transcript.entries().last(),
            Some(Entry::Info(text)) if text == "Nothing is running."
        ));
    }

    #[test]
    #[spec("TUI-023")]
    fn aborted_runs_return_to_idle() {
        let mut app = app();
        app.status = Status::Streaming;
        handle_agent_event(
            &mut app,
            AgentEvent::AgentEnd {
                messages: Vec::new(),
                status: RunStatus::Aborted,
            },
        );
        assert_eq!(app.status, Status::Idle);
        assert!(matches!(
            app.transcript.entries().last(),
            Some(Entry::Info(text)) if text == "Stopped."
        ));
    }

    #[test]
    #[spec("TUI-003")]
    fn spec_tui_003_file_and_command_completion() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("hello.txt"), "hi").unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "fn main() {}").unwrap();

        let mut app = App::new(
            dir.path(),
            ModelRef {
                api: "openai-completions".into(),
                provider: "deepseek".into(),
                model: "deepseek-flash".into(),
            },
            ThinkingLevel::Medium,
            128_000,
            Vec::new(),
            Theme::load(Some("dark")).unwrap(),
            None,
            Vec::new(),
            dir.path().join("sessions"),
            Arc::new(std::sync::atomic::AtomicI32::new(0)),
            Bindings::default(),
        );

        app.editor.insert_str("@hel");
        app.handle_key(key(KeyCode::Tab));
        assert_eq!(app.editor.text(), "@hello.txt");

        app.editor.clear();
        app.editor.insert_str("/he");
        app.handle_key(key(KeyCode::Tab));
        assert_eq!(app.editor.text(), "/help");

        app.extension_commands = vec!["/ext:todo:todo".to_owned()];
        app.editor.clear();
        app.editor.insert_str("/ext:to");
        app.handle_key(key(KeyCode::Tab));
        assert_eq!(app.editor.text(), "/ext:todo:todo");
    }

    #[test]
    #[spec("TUI-002")]
    fn notice_banner_closes_with_enter() {
        let mut app = app();
        app.overlay = Some(Overlay::Notice {
            title: " update ".to_owned(),
            body: "No update found.".to_owned(),
            ok: true,
        });
        app.handle_key(key(KeyCode::Enter));
        assert!(app.overlay.is_none());
    }

    #[test]
    fn update_command_requests_a_check() {
        let mut app = app();
        assert!(app.handle_command("/update"));
        assert!(app.update_check_requested);
    }

    #[test]
    #[spec("TUI-014")]
    fn spec_tui_014_selecting_update_from_the_palette_requests_a_check() {
        let mut app = app();
        assert_eq!(app.handle_key(key(KeyCode::Char('/'))), Action::None);
        for character in "update".chars() {
            app.handle_key(key(KeyCode::Char(character)));
        }
        let Some(Overlay::List(list)) = &app.overlay else {
            panic!("expected the command palette");
        };
        assert_eq!(
            list.current().map(|item| item.value.as_str()),
            Some("update")
        );

        // Enter applies the palette row directly (no Action::Submit), so the
        // request must survive as a flag for the event loop to service.
        assert_eq!(app.handle_key(key(KeyCode::Enter)), Action::None);
        assert!(app.overlay.is_none());
        assert!(app.update_check_requested);
    }

    #[test]
    fn update_requests_are_serviced_once_with_immediate_feedback() {
        let mut app = app();
        app.handle_key(key(KeyCode::Char('/')));
        for character in "update".chars() {
            app.handle_key(key(KeyCode::Char(character)));
        }
        app.handle_key(key(KeyCode::Enter));

        assert!(take_update_request(&mut app, false));
        assert!(!app.update_check_requested, "the request must be consumed");
        let Some(Overlay::Notice { title, body, ok }) = &app.overlay else {
            panic!("expected the checking notice");
        };
        assert_eq!(title, " update ");
        assert_eq!(body, "Checking for updates…");
        assert!(!ok);

        // A second request is consumed but ignored while the first is in flight.
        app.update_check_requested = true;
        assert!(!take_update_request(&mut app, true));
        assert!(
            !app.update_check_requested,
            "a duplicate request must be dropped, not queued"
        );
    }

    #[test]
    fn update_progress_renders_percentage_and_sizes() {
        let progress = codie_ai::update::UpdateProgress {
            downloaded: 5 * 1024 * 1024,
            total: Some(10 * 1024 * 1024),
        };
        let message = update_progress_message("0.6.1", "0.7.0", progress);
        assert!(
            message.contains("Updating codie 0.6.1 → 0.7.0…"),
            "{message}"
        );
        assert!(message.contains("50%"), "{message}");
        assert!(message.contains("5.0 MB of 10.0 MB"), "{message}");
        assert!(message.contains("████████████░░░░░░░░░░░░"), "{message}");

        let unknown = codie_ai::update::UpdateProgress {
            downloaded: 1536,
            total: None,
        };
        let message = update_progress_message("0.6.1", "0.7.0", unknown);
        assert!(message.contains("Downloaded 1.5 KB…"), "{message}");
        assert!(!message.contains('%'), "{message}");

        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1024), "1.0 KB");
        assert_eq!(format_bytes(10 * 1024 * 1024), "10.0 MB");
        assert_eq!(progress_bar(0), "[░░░░░░░░░░░░░░░░░░░░░░░░]");
        assert_eq!(progress_bar(100), "[████████████████████████]");
        assert_eq!(progress_bar(200), "[████████████████████████]");
    }

    #[test]
    fn spec_tui_002_streaming_updates_replace_text() {
        let mut app = app();
        let mut message = codie_core::AssistantMessage::partial(
            "openai-completions",
            "deepseek",
            "deepseek-flash",
        );
        message.content = vec![Content::text("hel")];
        handle_agent_event(
            &mut app,
            AgentEvent::MessageStart {
                message: Message::Assistant(message.clone()),
            },
        );
        message.content = vec![Content::text("hello")];
        handle_agent_event(
            &mut app,
            AgentEvent::MessageUpdate {
                message: Message::Assistant(message),
                delta: codie_core::AssistantDelta::TextDelta {
                    index: 0,
                    delta: "hello".to_owned(),
                },
            },
        );
        let crate::transcript::Entry::Assistant(assistant) = &app.transcript.entries()[0] else {
            panic!("expected assistant entry");
        };
        assert_eq!(assistant.text, "hello", "updates must replace, not append");
    }

    #[test]
    fn permissions_are_queued_and_resolved() {
        let mut app = app();
        let (reply, answer) = oneshot::channel();
        app.push_permission(PermissionRequest {
            tool: "bash".to_owned(),
            arguments: serde_json::json!({"command": "ls"}),
            agent: None,
            depth: 0,
            session_id: None,
            reply,
        });
        assert!(matches!(app.overlay, Some(Overlay::Permission(_))));
        app.handle_key(key(KeyCode::Char('a')));
        assert_eq!(answer.blocking_recv().unwrap(), Decision::Allow);
        assert!(app.overlay.is_none());
    }

    #[test]
    #[spec("TUI-029")]
    fn spec_tui_029_drag_selects_text_and_click_still_toggles() {
        let mut app = app();
        app.begin_selection(4, 3);
        assert!(app.end_selection(4, 3), "an unmoved drag is a click");
        assert!(app.selection.is_none());
        assert!(!app.copy_pending);

        app.begin_selection(4, 3);
        app.drag_selection(9, 5);
        assert!(!app.end_selection(9, 5), "a moved drag is a selection");
        assert!(app.copy_pending, "the copy happens on the next frame");
        let selection = app.selection.expect("selection");
        assert_eq!(selection.bounds(), ((3, 4), (5, 9)));

        app.handle_key(key(KeyCode::Esc));
        assert!(app.selection.is_none(), "a key press clears the highlight");
        assert!(!app.copy_pending);
    }

    #[test]
    #[spec("TUI-018")]
    fn spec_tui_018_permission_prompt_accepts_y_and_n() {
        let mut app = app();
        let (reply, answer) = oneshot::channel();
        app.push_permission(PermissionRequest {
            tool: "write".to_owned(),
            arguments: serde_json::json!({"path": "a.txt"}),
            agent: None,
            depth: 0,
            session_id: None,
            reply,
        });
        app.handle_key(key(KeyCode::Char('y')));
        assert_eq!(answer.blocking_recv().unwrap(), Decision::Allow);
        assert!(app.overlay.is_none());

        let (reply, answer) = oneshot::channel();
        app.push_permission(PermissionRequest {
            tool: "write".to_owned(),
            arguments: serde_json::json!({"path": "a.txt"}),
            agent: None,
            depth: 0,
            session_id: None,
            reply,
        });
        app.handle_key(key(KeyCode::Char('n')));
        assert_eq!(answer.blocking_recv().unwrap(), Decision::Deny);
        assert!(app.overlay.is_none());
    }

    #[test]
    #[spec("TUI-019")]
    fn spec_tui_019_shift_tab_toggles_full_access() {
        let mut app = app();
        assert_eq!(
            app.handle_key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::NONE)),
            Action::ToggleAutoApprove
        );
        assert!(app.auto_approve);
        assert_eq!(
            app.handle_key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT)),
            Action::ToggleAutoApprove
        );
        assert!(!app.auto_approve);
    }

    #[test]
    #[spec("TUI-011")]
    fn spec_tui_011_tree_fork_and_export_commands() {
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path().join("project");
        std::fs::create_dir_all(&cwd).unwrap();
        let session_dir = dir.path().join("sessions");
        let session = Session::create(&cwd, &session_dir).unwrap();
        let mut app = App::new(
            &cwd,
            ModelRef {
                api: "openai-completions".into(),
                provider: "deepseek".into(),
                model: "deepseek-flash".into(),
            },
            ThinkingLevel::Medium,
            128_000,
            Vec::new(),
            Theme::load(Some("dark")).unwrap(),
            Some(session),
            Vec::new(),
            session_dir,
            Arc::new(std::sync::atomic::AtomicI32::new(0)),
            Bindings::default(),
        );
        app.session
            .as_mut()
            .unwrap()
            .append_message(&Message::user("first"))
            .unwrap();
        app.session
            .as_mut()
            .unwrap()
            .append_message(&Message::user("second"))
            .unwrap();
        let first = app.session.as_ref().unwrap().entries()[0].id().to_owned();

        assert!(app.handle_command("/tree"));
        let Some(Overlay::Tree(entries)) = &app.overlay else {
            panic!("expected the tree overlay");
        };
        assert_eq!(entries.len(), 2);

        app.overlay = None;
        assert!(app.handle_command(&format!("/fork {first}")));
        assert_eq!(app.history.len(), 1);
        assert_eq!(
            app.session.as_ref().unwrap().leaf_target(),
            Some(first.as_str())
        );

        let export = dir.path().join("export.jsonl");
        assert!(app.handle_command(&format!("/export {}", export.display())));
        let contents = std::fs::read_to_string(&export).unwrap();
        assert!(contents.starts_with("{\"type\":\"session\""));

        assert!(app.handle_command("/name renamed"));
        assert_eq!(app.session.as_ref().unwrap().name(), Some("renamed"));
    }

    #[test]
    #[spec("TUI-012")]
    fn spec_tui_012_picker_filters_renames_and_deletes() {
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path().join("project");
        std::fs::create_dir_all(&cwd).unwrap();
        let session_dir = dir.path().join("sessions");

        let mut first = Session::create(&cwd, &session_dir).unwrap();
        first.set_name("alpha").unwrap();
        first.append_message(&Message::user("a")).unwrap();
        let first_path = first.path().to_path_buf();
        drop(first);
        let mut second = Session::create(&cwd, &session_dir).unwrap();
        second.set_name("beta").unwrap();
        second.append_message(&Message::user("b")).unwrap();
        let second_path = second.path().to_path_buf();
        drop(second);

        let mut app = App::new(
            &cwd,
            ModelRef {
                api: "openai-completions".into(),
                provider: "deepseek".into(),
                model: "deepseek-flash".into(),
            },
            ThinkingLevel::Medium,
            128_000,
            Vec::new(),
            Theme::load(Some("dark")).unwrap(),
            None,
            Vec::new(),
            session_dir.clone(),
            Arc::new(std::sync::atomic::AtomicI32::new(0)),
            Bindings::default(),
        );
        let summaries = Session::list_for_cwd(&cwd, &session_dir).unwrap();
        app.overlay = Some(Overlay::Picker(Picker::new(summaries)));

        // Search narrows the list to alpha.
        for character in "alph".chars() {
            app.handle_key(key(KeyCode::Char(character)));
        }
        let Some(Overlay::Picker(picker)) = &app.overlay else {
            panic!("expected the picker");
        };
        assert_eq!(picker.visible.len(), 1);

        // Rename alpha to alphamma (still matching the active filter).
        app.handle_key(key(KeyCode::Char('r')));
        for character in "mma".chars() {
            app.handle_key(key(KeyCode::Char(character)));
        }
        app.handle_key(key(KeyCode::Enter));
        let renamed = Session::open(&first_path).unwrap();
        assert_eq!(renamed.name(), Some("alphamma"));

        // Delete the visible session after confirmation.
        app.handle_key(key(KeyCode::Char('d')));
        app.handle_key(key(KeyCode::Char('y')));
        assert!(!first_path.exists());
        assert!(second_path.exists());
        let remaining = Session::list_for_cwd(&cwd, &session_dir).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].name.as_deref(), Some("beta"));
    }

    fn app_with_named_session() -> (tempfile::TempDir, App) {
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path().join("project");
        std::fs::create_dir_all(&cwd).unwrap();
        let session_dir = dir.path().join("sessions");
        let session = Session::create(&cwd, &session_dir).unwrap();
        let app = App::new(
            &cwd,
            ModelRef {
                api: "openai-completions".into(),
                provider: "deepseek".into(),
                model: "deepseek-flash".into(),
            },
            ThinkingLevel::Medium,
            128_000,
            Vec::new(),
            Theme::load(Some("dark")).unwrap(),
            Some(session),
            Vec::new(),
            session_dir,
            Arc::new(std::sync::atomic::AtomicI32::new(0)),
            Bindings::default(),
        );
        (dir, app)
    }

    #[test]
    #[spec("TUI-013")]
    fn spec_tui_013_compact_command_requests_a_summary() {
        let (_dir, mut app) = app_with_named_session();
        assert!(app.handle_command("/compact keep the test plan"));
        assert_eq!(app.pending_compact.as_deref(), Some("keep the test plan"));
        assert!(app.handle_command("/compact"));
        assert_eq!(app.pending_compact.as_deref(), Some(""));
    }

    #[test]
    #[spec("CTX-007")]
    fn spec_ctx_007_manual_compact_requires_a_session() {
        let (_dir, mut app) = app_with_named_session();
        assert!(app.handle_command("/compact keep the plan"));
        assert_eq!(app.pending_compact.as_deref(), Some("keep the plan"));

        app.pending_compact = None;
        app.session = None;
        assert!(app.handle_command("/compact"));
        assert!(app.pending_compact.is_none());
        assert!(
            app.transcript
                .entries()
                .iter()
                .any(|entry| matches!(entry, crate::transcript::Entry::Info(text) if text.contains("No session")))
        );
    }

    #[test]
    #[spec("CTX-006")]
    fn spec_ctx_006_overflow_recovery_runs_once_and_hides_the_error() {
        let (_dir, mut app) = app_with_named_session();
        let error = Message::Assistant(codie_core::AssistantMessage::error(
            "openai-completions",
            "deepseek",
            "deepseek-flash",
            codie_core::StopReason::Error,
            "This model's maximum context length is 128000 tokens",
        ));
        app.session
            .as_mut()
            .unwrap()
            .append_message(&error)
            .unwrap();
        app.history = app.session.as_ref().unwrap().projection();
        app.status = Status::Error;
        app.last_prompt = Some("keep going".to_owned());

        assert!(maybe_recover_overflow(&mut app));
        assert!(app.overflow_retried);
        assert_eq!(app.pending_prompt.as_deref(), Some("keep going"));
        assert!(app.pending_retry);
        assert!(
            !app.session
                .as_ref()
                .unwrap()
                .projection()
                .iter()
                .any(|message| matches!(message, Message::Assistant(assistant) if assistant.stop_reason == codie_core::StopReason::Error)),
            "the failed message must be hidden by a context edit"
        );

        app.status = Status::Error;
        assert!(
            !maybe_recover_overflow(&mut app),
            "only one recovery attempt is allowed"
        );
    }

    struct NoopClient;

    impl codie_core::ChatClient for NoopClient {
        fn stream(
            &self,
            _context: &[Message],
            _options: &codie_core::StreamOptions,
        ) -> codie_core::AssistantEventStream {
            let (stream, sender) = codie_core::AssistantEventStream::channel(1);
            tokio::spawn(async move {
                let mut message = codie_core::AssistantMessage::partial("api", "provider", "model");
                message.content = vec![Content::text("## Goal\nfinish")];
                message.stop_reason = codie_core::StopReason::Stop;
                let _ = sender
                    .send(codie_core::AssistantDelta::Done {
                        reason: codie_core::StopReason::Stop,
                        message,
                    })
                    .await;
            });
            stream
        }
    }

    #[tokio::test]
    #[spec("CTX-001")]
    async fn spec_ctx_001_threshold_starts_compaction_before_a_request() {
        let (dir, mut app) = app_with_named_session();
        app.context_window = 20_000;
        app.session
            .as_mut()
            .unwrap()
            .append_message(&Message::user("x".repeat(80_000)))
            .unwrap();
        app.history = app.session.as_ref().unwrap().projection();
        let cwd = dir.path().join("project");
        let options = TuiOptions {
            client: Arc::new(NoopClient),
            cwd: cwd.clone(),
            model: app.model.clone(),
            context_window: 20_000,
            models: Vec::new(),
            models_loader: None,
            providers: Vec::new(),
            switch_provider: None,
            thinking: ThinkingLevel::Medium,
            max_turns: 10,
            subagents: SubagentConfig::default(),
            subagent_mode: DelegationMode::Auto,
            subagent_force_once: false,
            session: None,
            history: app.history.clone(),
            initial_prompt: None,
            auto_allowed: Vec::new(),
            theme: None,
            mode: TuiRunMode::Full,
            session_dir: dir.path().join("sessions"),
            resume_picker: false,
            signal: Arc::new(std::sync::atomic::AtomicI32::new(0)),
            registry: Arc::new(Registry::new(Vec::new())),
            keybindings: None,
            preamble: None,
            append: None,
            customizations: Customizations::default(),
            extensions: Arc::new(codie_ext::ExtensionSet::default()),
            auth_path: dir.path().join("auth.json"),
            base_url: None,
            explicit_key_provider: None,
            startup_connect: false,
            persist_selection: None,
        };
        let agent = Arc::new(Agent::new(
            Arc::clone(&options.client),
            Arc::new(Registry::new(Vec::new())),
        ));
        let (events, _event_rx) = mpsc::unbounded_channel();
        let (compact, _compact_rx) = mpsc::unbounded_channel();

        submit_prompt(&mut app, &agent, &options, &events, &compact, "next".into());
        assert_eq!(app.pending_prompt.as_deref(), Some("next"));
        assert!(!app.pending_retry);
        assert_eq!(app.status, Status::Streaming);
    }

    #[test]
    #[spec("TUI-014")]
    fn spec_tui_014_slash_opens_a_filterable_command_palette() {
        let mut app = app();
        assert_eq!(app.handle_key(key(KeyCode::Char('/'))), Action::None);
        let Some(Overlay::List(list)) = &app.overlay else {
            panic!("expected the command palette");
        };
        assert_eq!(list.kind, ChoiceKind::Commands);
        assert!(list.visible.len() >= 10);
        assert!(
            list.items.iter().any(|item| item.value == "update"),
            "the palette must offer /update"
        );
        assert!(
            list.items.iter().any(|item| item.value == "subagents"),
            "the palette must offer /subagents"
        );

        for character in "theme".chars() {
            app.handle_key(key(KeyCode::Char(character)));
        }
        let Some(Overlay::List(list)) = &app.overlay else {
            panic!("expected the command palette");
        };
        assert!(list.items.iter().any(|item| item.value.as_str() == "theme"));
        assert_eq!(
            list.current().map(|item| item.value.as_str()),
            Some("theme")
        );

        // Enter opens the theme picker.
        app.handle_key(key(KeyCode::Enter));
        assert!(matches!(
            &app.overlay,
            Some(Overlay::List(list)) if list.kind == ChoiceKind::Themes
        ));

        // Escape closes without changing anything.
        app.handle_key(key(KeyCode::Esc));
        assert!(app.overlay.is_none());
    }

    #[test]
    #[spec("TUI-032")]
    fn spec_tui_032_paths_close_the_command_palette() {
        let mut app = app();
        app.handle_key(key(KeyCode::Char('/')));
        assert!(matches!(
            &app.overlay,
            Some(Overlay::List(list)) if list.kind == ChoiceKind::Commands
        ));
        for character in "var/folders".chars() {
            app.handle_key(key(KeyCode::Char(character)));
        }
        assert!(app.overlay.is_none(), "a path must leave the palette");
        assert_eq!(app.editor.text(), "/var/folders");

        // A bracketed paste of an absolute path also leaves the palette.
        app.editor.clear();
        app.handle_key(key(KeyCode::Char('/')));
        app.paste("/Users/me/shot.png");
        assert!(app.overlay.is_none());
        assert_eq!(app.editor.text(), "/Users/me/shot.png");
    }

    #[test]
    #[spec("TUI-032")]
    fn spec_tui_032_dragged_paths_become_attachments_while_typing() {
        let mut app = app();
        let directory = tempfile::tempdir().unwrap();
        let shot = directory
            .path()
            .join("Screenshot 2026-11-16 at 3.24.22 PM.png");
        std::fs::write(&shot, b"\x89PNG\r\n\x1a\nfake").unwrap();
        let escaped = shot.to_string_lossy().replace(' ', "\\ ");

        for character in escaped.chars() {
            app.handle_key(key(KeyCode::Char(character)));
        }

        assert_eq!(
            app.editor.text(),
            "[Image 1] ",
            "drag path must become a compact attachment"
        );
        assert_eq!(app.attachments.len(), 1);
    }

    #[test]
    #[spec("TUI-031")]
    fn spec_tui_031_clipboard_paste_inserts_an_attachment_token() {
        let mut app = app();
        assert_eq!(
            Bindings::default()
                .action_for(&KeyEvent::new(KeyCode::Char('v'), KeyModifiers::CONTROL)),
            Some(ActionId::PasteImage)
        );
        app.paste("@\"/tmp/my shot.png\"");
        assert_eq!(app.editor.text(), "@\"/tmp/my shot.png\"");
        assert_eq!(
            codie_prompt::quote_attachment_path(std::path::Path::new("/tmp/my shot.png")),
            "@\"/tmp/my shot.png\""
        );

        // A pasted bare path becomes a compact attachment, and submitting it
        // expands back to the `@path` token.
        let directory = tempfile::tempdir().unwrap();
        let shot = directory.path().join("Screenshot 2026-11-16.png");
        std::fs::write(&shot, b"\x89PNG\r\n\x1a\nfake").unwrap();
        app.editor.clear();
        app.paste(&shot.to_string_lossy());
        assert_eq!(app.editor.text(), "[Image 1] ");
        let expanded = app.expand_attachment_labels("[Image 1] what is this?");
        assert!(expanded.contains("Screenshot 2026-11-16.png"), "{expanded}");
        assert!(expanded.starts_with("@\""), "{expanded}");
        assert!(expanded.ends_with(" what is this?"));
    }

    #[test]
    fn theme_command_accepts_extra_themes() {
        let (_dir, mut app) = app_with_named_session();
        assert!(app.handle_command("/theme nord"));
        assert_eq!(app.theme.name, "nord");
        assert_eq!(app.theme.palette, crate::theme::Palette::nord());

        assert!(app.handle_command("/theme orange"));
        assert_eq!(app.theme.name, "orng");

        assert!(app.handle_command("/theme"));
        let Some(Overlay::List(list)) = &app.overlay else {
            panic!("expected the theme picker");
        };
        assert_eq!(list.items.len(), crate::theme::BUILTIN_THEMES.len());
    }

    #[test]
    #[spec("TUI-015")]
    fn spec_tui_015_model_and_theme_pickers_apply_selection() {
        let (_dir, mut app) = app_with_named_session();
        app.models = vec![
            ModelChoice {
                provider: "deepseek".to_owned(),
                id: "deepseek-flash".to_owned(),
                name: "DeepSeek Flash".to_owned(),
                context_window: 1_000_000,
            },
            ModelChoice {
                provider: "deepseek".to_owned(),
                id: "deepseek-v4-pro".to_owned(),
                name: "DeepSeek V4 Pro".to_owned(),
                context_window: 128_000,
            },
        ];
        assert!(app.handle_command("/model"));
        app.handle_key(key(KeyCode::Down));
        app.handle_key(key(KeyCode::Enter));
        assert_eq!(app.model.model, "deepseek-v4-pro");
        assert_eq!(app.context_window, 128_000);
        assert_eq!(
            app.session.as_ref().unwrap().latest_model(),
            Some(("deepseek", "deepseek-v4-pro"))
        );

        // The cursor starts on the active theme, so Enter keeps it.
        assert!(app.handle_command("/theme"));
        app.handle_key(key(KeyCode::Enter));
        assert_eq!(app.theme.name, "dark");
    }

    #[test]
    #[spec("TUI-015")]
    fn spec_tui_015_theme_picker_previews_and_cancels() {
        let (_dir, mut app) = app_with_named_session();
        assert_eq!(app.theme.name, "dark");

        // The cursor starts on the active theme, and moving previews live.
        assert!(app.handle_command("/theme"));
        app.handle_key(key(KeyCode::Down));
        assert_eq!(app.theme.name, "light");
        assert!(matches!(app.overlay, Some(Overlay::List(_))));
        app.handle_key(key(KeyCode::Down));
        assert_eq!(app.theme.name, "dracula");

        // Esc restores the theme that was active when the picker opened.
        app.handle_key(key(KeyCode::Esc));
        assert_eq!(app.theme.name, "dark");
        assert!(app.overlay.is_none());

        // Filtering previews the highlighted match as well.
        assert!(app.handle_command("/theme"));
        for character in "nord".chars() {
            app.handle_key(key(KeyCode::Char(character)));
        }
        assert_eq!(app.theme.name, "nord");
        app.handle_key(key(KeyCode::Esc));
        assert_eq!(app.theme.name, "dark");

        // Enter keeps the previewed theme and closes the picker.
        assert!(app.handle_command("/theme"));
        app.handle_key(key(KeyCode::Down));
        app.handle_key(key(KeyCode::Enter));
        assert_eq!(app.theme.name, "light");
        assert!(app.overlay.is_none());
    }

    #[test]
    #[spec("TUI-015")]
    fn spec_tui_015_provider_picker_switches_client_and_model() {
        let (_dir, mut app) = app_with_named_session();
        app.model.provider = "deepseek".to_owned();
        app.model.model = "deepseek-flash".to_owned();
        app.providers = vec![
            ProviderChoice {
                id: "deepseek".to_owned(),
                name: "DeepSeek".to_owned(),
            },
            ProviderChoice {
                id: "openrouter".to_owned(),
                name: "OpenRouter".to_owned(),
            },
        ];
        app.explicit_key_provider = Some("openrouter".to_owned());
        app.switch_provider = Some(Arc::new(|id: &str| {
            assert_eq!(id, "openrouter");
            Ok(ProviderRuntime {
                models: vec![ModelChoice {
                    provider: "openrouter".to_owned(),
                    id: "openrouter/auto".to_owned(),
                    name: "OpenRouter Auto".to_owned(),
                    context_window: 128_000,
                }],
                default_model: "openrouter/auto".to_owned(),
            })
        }));

        assert!(app.handle_command("/provider"));
        app.handle_key(key(KeyCode::Down));
        app.handle_key(key(KeyCode::Enter));

        assert_eq!(app.model.provider, "openrouter");
        assert_eq!(app.model.model, "openrouter/auto");
        assert_eq!(app.context_window, 128_000);
        assert_eq!(app.models[0].id, "openrouter/auto");
        assert_eq!(
            app.session.as_ref().unwrap().latest_model(),
            Some(("openrouter", "openrouter/auto"))
        );
    }

    #[test]
    #[spec("TUI-016")]
    fn spec_tui_016_reasoning_picker_sets_and_persists_the_level() {
        let (_dir, mut app) = app_with_named_session();
        assert!(app.handle_command("/thinking"));
        app.handle_key(key(KeyCode::Down));
        app.handle_key(key(KeyCode::Down));
        app.handle_key(key(KeyCode::Enter));
        assert_eq!(app.thinking, ThinkingLevel::Low);
        assert_eq!(app.footer.thinking, "low");
        assert_eq!(
            app.session.as_ref().unwrap().latest_thinking_level(),
            Some(ThinkingLevel::Low)
        );
    }

    #[test]
    #[spec("TUI-017")]
    fn spec_tui_017_footer_reports_tokens_per_second() {
        let mut app = app();
        app.update_usage(&Usage::new(0, 120, 0, 0), Some(2_000));
        assert_eq!(app.footer.tokens_per_sec, Some(60.0));

        app.update_usage(&Usage::new(0, 10, 0, 0), None);
        assert_eq!(
            app.footer.tokens_per_sec,
            Some(60.0),
            "missing duration must keep the previous rate"
        );
    }

    fn connect_app(path: PathBuf) -> App {
        let mut app = app();
        app.auth_path = Some(path);
        app.providers = vec![
            ProviderChoice {
                id: "deepseek".to_owned(),
                name: "DeepSeek".to_owned(),
            },
            ProviderChoice {
                id: "openrouter".to_owned(),
                name: "OpenRouter".to_owned(),
            },
        ];
        app.switch_provider = Some(Arc::new(|id: &str| {
            Ok(ProviderRuntime {
                models: vec![ModelChoice {
                    provider: id.to_owned(),
                    id: "openrouter/auto".to_owned(),
                    name: "OpenRouter Auto".to_owned(),
                    context_window: 128_000,
                }],
                default_model: "openrouter/auto".to_owned(),
            })
        }));
        app.overlay = Some(Overlay::Connect(ConnectState::new("openrouter")));
        app
    }

    #[test]
    fn connect_overlay_masks_the_key_and_requests_a_check() {
        let directory = tempfile::tempdir().unwrap();
        let mut app = connect_app(directory.path().join("auth.json"));

        for character in "sk-or-secret".chars() {
            app.handle_key(key(KeyCode::Char(character)));
        }
        let Some(Overlay::Connect(state)) = &app.overlay else {
            panic!("connect overlay must stay open");
        };
        assert_eq!(state.input, "sk-or-secret");
        assert_eq!(state.display(), "•".repeat(12));
        assert!(!format!("{state:?}").contains("sk-or-secret"));

        assert_eq!(app.handle_key(key(KeyCode::Enter)), Action::VerifyKey);
        assert_eq!(
            app.take_connect_request(),
            Some(("openrouter".to_owned(), "sk-or-secret".to_owned(), 1))
        );
    }

    #[test]
    fn valid_connect_result_stores_the_key_and_switches_provider() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("auth.json");
        let mut app = connect_app(path.clone());

        for character in "sk-or-good".chars() {
            app.handle_key(key(KeyCode::Char(character)));
        }
        app.handle_key(key(KeyCode::Enter));
        let (provider, _, generation) = app.take_connect_request().unwrap();
        app.handle_connect_result(&provider, generation, ProbeOutcome::Valid);

        assert!(app.overlay.is_none());
        assert_eq!(app.model.provider, "openrouter");
        assert_eq!(app.footer.provider, "openrouter");
        assert_eq!(
            AuthStore::new(path).get("openrouter").unwrap().as_deref(),
            Some("sk-or-good")
        );
    }

    #[test]
    #[spec("TUI-035")]
    fn spec_tui_035_rejected_key_does_not_overwrite_the_stored_key() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("auth.json");
        AuthStore::new(&path).set("openrouter", "sk-old").unwrap();
        let mut app = connect_app(path.clone());

        for character in "sk-new-but-bad".chars() {
            app.handle_key(key(KeyCode::Char(character)));
        }
        app.handle_key(key(KeyCode::Enter));
        let (provider, _, generation) = app.take_connect_request().unwrap();
        app.handle_connect_result(
            &provider,
            generation,
            ProbeOutcome::Invalid {
                status: 401,
                message: "key rejected".to_owned(),
            },
        );

        assert_eq!(
            AuthStore::new(path).get("openrouter").unwrap().as_deref(),
            Some("sk-old")
        );
        let Some(Overlay::Connect(state)) = &app.overlay else {
            panic!("failed check must keep the overlay open");
        };
        assert!(matches!(state.phase, ConnectPhase::Error(_)));
        assert_ne!(app.model.provider, "openrouter");
    }

    #[test]
    #[spec("TUI-035")]
    fn spec_tui_035_stale_connect_results_are_ignored() {
        let directory = tempfile::tempdir().unwrap();
        let mut app = connect_app(directory.path().join("auth.json"));

        for character in "sk-or-good".chars() {
            app.handle_key(key(KeyCode::Char(character)));
        }
        app.handle_key(key(KeyCode::Enter));
        let (provider, _, generation) = app.take_connect_request().unwrap();
        app.handle_connect_result(&provider, generation + 1, ProbeOutcome::Valid);

        assert_eq!(app.model.provider, "deepseek");
        assert!(matches!(app.overlay, Some(Overlay::Connect(_))));
    }

    #[test]
    #[spec("TUI-036")]
    fn auth_errors_open_the_connect_overlay() {
        let mut app = app();
        app.model.provider = "openrouter".to_owned();
        app.model.model = "openrouter/auto".to_owned();
        app.providers = vec![ProviderChoice {
            id: "openrouter".to_owned(),
            name: "OpenRouter".to_owned(),
        }];
        let mut assistant =
            codie_core::AssistantMessage::partial("openai-completions", "openrouter", "auto");
        assistant.stop_reason = codie_core::StopReason::Error;
        assistant.error_message = Some("401 Unauthorized: invalid api key".to_owned());

        handle_agent_event(
            &mut app,
            AgentEvent::MessageEnd {
                message: Message::Assistant(assistant),
            },
        );

        assert!(matches!(app.overlay, Some(Overlay::Connect(_))));
        assert_eq!(app.model.provider, "openrouter");
    }

    #[test]
    #[spec("TUI-037")]
    fn provider_switching_is_blocked_while_streaming() {
        let (_dir, mut app) = app_with_named_session();
        app.status = Status::Streaming;
        app.switch_provider = Some(Arc::new(|_| panic!("must not switch mid-run")));

        app.switch_provider_to("openrouter");

        assert_eq!(app.model.provider, "deepseek");
    }

    #[test]
    #[spec("TUI-038")]
    fn model_choices_persist_the_selection() {
        let calls = Arc::new(std::sync::Mutex::new(Vec::<(String, String)>::new()));
        let captured = Arc::clone(&calls);
        let mut app = app();
        app.persist_selection = Some(Arc::new(move |provider: &str, model: &str| {
            captured
                .lock()
                .unwrap()
                .push((provider.to_owned(), model.to_owned()));
        }));
        app.models = vec![ModelChoice {
            provider: "openrouter".to_owned(),
            id: "openrouter/auto".to_owned(),
            name: "OpenRouter Auto".to_owned(),
            context_window: 128_000,
        }];

        app.apply_choice(
            ChoiceKind::Models,
            &ChoiceItem {
                label: "OpenRouter Auto".to_owned(),
                detail: String::new(),
                value: "openrouter/auto".to_owned(),
            },
        );

        assert_eq!(
            calls.lock().unwrap().as_slice(),
            [("openrouter".to_owned(), "openrouter/auto".to_owned())]
        );
    }

    #[test]
    #[spec("TUI-034")]
    fn spec_tui_034_connect_lists_unconfigured_providers_and_offers_key_entry() {
        let directory = tempfile::tempdir().unwrap();
        let mut app = app();
        app.auth_path = Some(directory.path().join("auth.json"));
        app.providers = vec![ProviderChoice {
            id: "openrouter".to_owned(),
            name: "OpenRouter".to_owned(),
        }];

        assert!(app.handle_command("/provider"));
        let Some(Overlay::List(list)) = &app.overlay else {
            panic!("provider picker must open");
        };
        assert!(list.items.iter().any(|item| item.value == "__connect__"));
        assert!(list.items[0].label.starts_with('○'));

        app.handle_key(key(KeyCode::Enter));
        assert!(matches!(app.overlay, Some(Overlay::Connect(_))));
    }
}
