//! Application state, key handling, and the interactive run loop.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use codie_core::{
    Agent, AgentConfig, AgentEvent, ChatClient, Content, Message, ModelRef, RunStatus,
    ThinkingLevel, ToolResultMessage, Toolbox, Usage,
};
use codie_prompt::{Customizations, PromptInput, build_system_message, load_project_context};
use codie_session::{Session, SessionSummary, TreeEntry};
use codie_tools::Registry;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{
    DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture, Event,
    MouseEventKind,
};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};

use crate::commands::{Command, parse as parse_command, parse_thinking};
use crate::editor::Editor;
use crate::keybindings::{ActionId, Bindings};
use crate::permissions::{Decision, PermissionBroker, PermissionRequest, PermissionToolbox};
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
    /// Built-in themes.
    Themes,
    /// Reasoning levels.
    Thinking,
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

/// One overlay on top of the transcript.
pub enum Overlay {
    /// Slash-command help.
    Help,
    /// Keybinding reference.
    Hotkeys,
    /// A pending permission prompt.
    Permission(PermissionPrompt),
    /// The current session tree.
    Tree(Vec<TreeEntry>),
    /// The session picker.
    Picker(Picker),
    /// A command, model, theme, or reasoning list.
    List(ChoiceList),
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
    /// Reply channel.
    reply: Option<oneshot::Sender<Decision>>,
}

/// Footer metadata.
#[derive(Debug, Clone, Default)]
pub struct Footer {
    /// Working directory.
    pub cwd: String,
    /// Git branch, when inside a repository.
    pub git_branch: Option<String>,
    /// Model identifier.
    pub model: String,
    /// Thinking level.
    pub thinking: String,
    /// Total tokens in the current run.
    pub tokens: u64,
    /// Total cost in nano-USD.
    pub cost: u64,
    /// Most recent or live tokens per second.
    pub tokens_per_sec: Option<f64>,
    /// Session identifier.
    pub session: Option<String>,
    /// Estimated context usage percentage.
    pub context_percent: Option<u8>,
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
    /// Quit the app.
    Quit,
}

/// The interactive application state.
pub struct App {
    /// Scrolling transcript.
    pub transcript: Transcript,
    /// Multiline editor.
    pub editor: Editor,
    /// Active theme.
    pub theme: Theme,
    /// Footer metadata.
    pub footer: Footer,
    /// Current status.
    pub status: Status,
    /// Active overlay.
    pub overlay: Option<Overlay>,
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
    /// When the current run started, for live tokens/sec.
    pub run_started: Option<std::time::Instant>,
    /// Conversation history.
    pub history: Vec<Message>,
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
    /// Slash commands declared by extensions.
    pub extension_commands: Vec<String>,
    /// True after `/new` starts a fresh session.
    pub extension_session_reset: bool,
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
        Self {
            transcript: Transcript::new(),
            editor: Editor::new(),
            theme,
            footer: Footer {
                cwd: cwd.display().to_string(),
                git_branch,
                model: model.model.clone(),
                thinking: thinking.as_str().to_owned(),
                session: session_id,
                ..Footer::default()
            },
            status: Status::Idle,
            overlay: None,
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
            run_started: None,
            history,
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
            signal,
            cancel: tokio_util::sync::CancellationToken::new(),
            bindings,
            theme_watcher,
            customizations: Customizations::default(),
            command_prompt: None,
            extension_commands: Vec::new(),
            extension_session_reset: false,
        }
    }

    /// Handles one terminal key event.
    pub fn handle_key(&mut self, key: KeyEvent) -> Action {
        if key.kind == KeyEventKind::Release {
            return Action::None;
        }
        if key.code == KeyCode::BackTab {
            return self.dispatch_action(ActionId::ToggleAutoApprove);
        }
        if self.overlay.is_some() {
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
                self.transcript.scroll_to_bottom();
                Action::None
            }
            _ => Action::None,
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
                    Action::None
                }
            }
            ActionId::Quit => Action::Quit,
            ActionId::ExpandTool => {
                self.transcript.toggle_last_tool();
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
        let permission = matches!(self.overlay, Some(Overlay::Permission(_)));
        if !permission {
            if matches!(key.code, KeyCode::Esc | KeyCode::Enter)
                || key.code == KeyCode::Char('q')
                || key.code == KeyCode::Char('?')
            {
                self.overlay = None;
            }
            return Action::None;
        }

        let decision = match key.code {
            KeyCode::Char('y' | 'Y' | 'a') => Some(Decision::Allow),
            KeyCode::Char('A') => Some(Decision::Always),
            KeyCode::Char('n' | 'N' | 'd' | 'D') | KeyCode::Esc => Some(Decision::Deny),
            KeyCode::Enter => Some(Decision::Allow),
            _ => None,
        };
        if let Some(decision) = decision {
            self.resolve_permission(decision);
        }
        Action::None
    }

    fn handle_list_key(&mut self, key: KeyEvent) -> Action {
        let Some(Overlay::List(mut list)) = self.overlay.take() else {
            return Action::None;
        };
        match key.code {
            KeyCode::Up => list.move_by(-1),
            KeyCode::Down => list.move_by(1),
            KeyCode::PageUp => list.move_by(-5),
            KeyCode::PageDown => list.move_by(5),
            KeyCode::Enter => {
                if let Some(item) = list.current().cloned()
                    && let Some(action) = self.apply_choice(list.kind, &item)
                {
                    return action;
                }
                return Action::None;
            }
            KeyCode::Esc => return Action::None,
            KeyCode::Backspace => {
                list.query.pop();
                list.refresh();
            }
            KeyCode::Char(character)
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT) =>
            {
                list.query.push(character);
                list.refresh();
            }
            _ => {}
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
                    self.footer.model = choice.id.clone();
                    if let Some(session) = self.session.as_mut() {
                        let _ = session.append_model_change(choice.provider, choice.id.clone());
                    }
                    self.refresh_context();
                    self.transcript
                        .push_info(format!("Model: {} ({})", choice.name, choice.id));
                }
            }
            ChoiceKind::Providers => self.switch_provider_to(&item.value),
            ChoiceKind::Themes => match Theme::load(Some(&item.value)) {
                Ok(theme) => {
                    self.theme_watcher = std::path::Path::new(&theme.name)
                        .is_file()
                        .then(|| crate::theme::ThemeWatcher::new(&theme.name));
                    self.theme = theme;
                    self.transcript.push_info(format!("Theme: {}", item.value));
                }
                Err(error) => self
                    .transcript
                    .push_error(format!("Cannot load theme: {error}")),
            },
            ChoiceKind::Thinking => {
                if let Some(level) = ThinkingLevel::parse(&item.value) {
                    self.thinking = level;
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
            ("provider", "Choose a provider"),
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

    fn open_models(&mut self) {
        if let Some(loader) = &self.models_loader {
            let refreshed = loader(&self.model.provider);
            if !refreshed.is_empty() {
                self.models = refreshed;
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
            .map(|provider| ChoiceItem {
                label: if provider.id == current {
                    format!("{} (current)", provider.name)
                } else {
                    provider.name.clone()
                },
                detail: provider.id.clone(),
                value: provider.id.clone(),
            })
            .collect::<Vec<_>>();
        if items.is_empty() {
            items.push(ChoiceItem {
                label: format!("{current} (current)"),
                detail: "No other providers are configured".to_owned(),
                value: current,
            });
        }
        self.overlay = Some(Overlay::List(ChoiceList::new(
            ChoiceKind::Providers,
            " providers ",
            items,
            "type to filter · ↑/↓ scroll · Enter select",
        )));
    }

    fn switch_provider_to(&mut self, id: &str) {
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
                self.footer.model = self.model.model.clone();
                if let Some(session) = self.session.as_mut() {
                    let _ = session.append_model_change(id.to_owned(), self.model.model.clone());
                }
                self.refresh_context();
                self.transcript
                    .push_info(format!("Provider: {name} · model: {}", self.model.model));
            }
            Err(error) => self
                .transcript
                .push_error(format!("Cannot switch provider: {error}")),
        }
    }

    fn open_themes(&mut self) {
        let current = self.theme.name.clone();
        let items = ["system", "dark", "light"]
            .into_iter()
            .map(|name| ChoiceItem {
                label: if name == current {
                    format!("{name} (current)")
                } else {
                    name.to_owned()
                },
                detail: "Built-in theme".to_owned(),
                value: name.to_owned(),
            })
            .collect();
        self.overlay = Some(Overlay::List(ChoiceList::new(
            ChoiceKind::Themes,
            " themes ",
            items,
            "type to filter · ↑/↓ scroll · Enter select",
        )));
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
        let tokens = self.history.iter().map(message_chars).sum::<u64>() / 4;
        let percent = tokens.saturating_mul(100) / u64::from(self.context_window.max(1));
        self.footer.context_percent = Some(u8::try_from(percent.min(100)).unwrap_or(100));
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
                            self.transcript.clear();
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
                if let Some(name) = argument {
                    match Theme::load(Some(&name)) {
                        Ok(theme) => {
                            self.theme_watcher = std::path::Path::new(&theme.name)
                                .is_file()
                                .then(|| crate::theme::ThemeWatcher::new(&theme.name));
                            self.theme = theme;
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
                    self.switch_provider_to(&provider);
                } else {
                    self.open_providers();
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
                        if let Some(session) = self.session.as_mut() {
                            let _ = session.append_model_change(choice.provider, choice.id.clone());
                        }
                    }
                    self.refresh_context();
                    self.transcript.push_info("Model changed for the next run.");
                } else {
                    self.open_models();
                }
            }
            Command::Thinking(argument) => {
                if let Some(value) = argument {
                    if let Some(level) = parse_thinking(&value) {
                        self.thinking = level;
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
                self.transcript.push_info(format!(
                    "Reloaded {} skill(s) and {} template(s).",
                    self.customizations.skills().len(),
                    self.customizations.templates().len()
                ));
            }
            Command::Unknown(name) => self
                .transcript
                .push_error(format!("Unknown command `/{name}`. Try /help.")),
        }
        true
    }

    /// Adds a permission prompt, showing it when no other prompt is active.
    pub fn push_permission(&mut self, request: PermissionRequest) {
        self.permissions.push_back(PermissionPrompt {
            tool: request.tool,
            arguments: request.arguments,
            reply: Some(request.reply),
        });
        self.show_next_permission();
    }

    /// Resolves the visible permission prompt.
    pub fn resolve_permission(&mut self, decision: Decision) {
        if let Some(Overlay::Permission(prompt)) = self.overlay.take()
            && let Some(reply) = prompt.reply
        {
            let _ = reply.send(decision);
        }
        self.show_next_permission();
    }

    fn show_next_permission(&mut self) {
        if self.overlay.is_some() {
            return;
        }
        if let Some(prompt) = self.permissions.pop_front() {
            self.status = Status::Waiting;
            self.overlay = Some(Overlay::Permission(prompt));
        } else if self.status == Status::Waiting {
            self.status = Status::Streaming;
        }
    }

    fn new_session(&mut self) {
        match Session::create(self.footer.cwd.clone(), &self.session_dir) {
            Ok(session) => {
                self.session = Some(session);
                self.history.clear();
                self.transcript.clear();
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
                self.transcript.clear();
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
                self.transcript.clear();
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
            || self.history.iter().map(message_chars).sum::<u64>() / 4,
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
        let mut matches = ignore::WalkBuilder::new(&self.cwd)
            .hidden(true)
            .git_ignore(true)
            .build()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_some_and(|kind| kind.is_file()))
            .filter_map(|entry| {
                let path = entry.path().strip_prefix(&self.cwd).ok()?;
                let display = path.to_string_lossy().replace('\\', "/");
                display.starts_with(prefix).then_some(display)
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

fn message_chars(message: &Message) -> u64 {
    match message {
        Message::System(system) => system.content.chars().count() as u64,
        Message::User(user) => user
            .content
            .iter()
            .map(|content| content.as_text().map_or(0, |text| text.len() as u64))
            .sum(),
        Message::Assistant(assistant) => assistant
            .content
            .iter()
            .map(|content| match content {
                Content::Text { text, .. } => text.chars().count() as u64,
                Content::Thinking { thinking, .. } => thinking.chars().count() as u64,
                Content::ToolCall { arguments, .. } => arguments.to_string().chars().count() as u64,
                Content::Image { .. } => 4_800,
            })
            .sum(),
        Message::ToolResult(result) => result
            .content
            .iter()
            .map(|content| content.as_text().map_or(0, |text| text.len() as u64))
            .sum(),
    }
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
    let registry = Arc::clone(&options.registry);
    let (permission_tx, mut permission_rx) = mpsc::unbounded_channel();
    let broker = PermissionBroker::new(permission_tx, options.auto_allowed.clone());
    let permission_broker = broker.clone();
    let toolbox: Arc<dyn Toolbox> = Arc::new(PermissionToolbox::new(registry.clone(), broker));
    let agent = Arc::new(match options.extensions.hooks() {
        Some(hooks) => Agent::with_hooks(Arc::clone(&options.client), Arc::clone(&toolbox), hooks),
        None => Agent::new(Arc::clone(&options.client), Arc::clone(&toolbox)),
    });
    let session_id = options
        .session
        .as_ref()
        .map(|session| session.id().to_owned());
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
    app.models_loader = options.models_loader.clone();
    app.providers = options.providers.clone();
    app.switch_provider = options.switch_provider.clone();
    app.extension_commands = options.extensions.command_names();

    if !matches!(history.first(), Some(Message::System(_))) {
        let declarations = toolbox.declarations();
        let input = PromptInput {
            cwd: options.cwd.clone(),
            tools: declarations.clone(),
            preamble: options.preamble.clone(),
            append: options.append.clone(),
            skills: options.customizations.skills().to_vec(),
            project_context: load_project_context(&options.cwd),
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
    Mouse(MouseEventKind),
    Paste(String),
    Signal(i32),
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
    /// A verified download is in progress.
    Installing(String),
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
            let version = status.latest.clone();
            let _ = sender.send(UpdateEvent::Installing(version.clone()));
            let installed = codie_ai::update::install(&version)
                .await
                .map(|()| version)
                .map_err(|error| error.to_string());
            let _ = sender.send(UpdateEvent::Installed(installed));
            return;
        }

        let _ = sender.send(UpdateEvent::Checked { manual, result });
    });
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
                        if sender.send(Input::Mouse(mouse.kind)).is_err() {
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
    broker: PermissionBroker,
) -> Result<u8> {
    let mut input = spawn_input();
    let (event_tx, mut event_rx) = mpsc::unbounded_channel();
    let (compact_tx, mut compact_rx) = mpsc::unbounded_channel();
    let (update_tx, mut update_rx) = mpsc::unbounded_channel::<UpdateEvent>();
    let mut tick = tokio::time::interval(Duration::from_millis(33));
    let mut dirty = true;
    let mut signal_code = 0_i32;

    if let Some(prompt) = options.initial_prompt.clone() {
        submit_prompt(app, &agent, &options, &event_tx, &compact_tx, prompt);
    }

    let mut auto_update_started = false;

    loop {
        if dirty {
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
                            Action::Cancel => {
                                app.cancel.cancel();
                                app.transcript.push_info("Cancelling…");
                            }
                            Action::Submit(text) => {
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
                                    if app.update_check_requested {
                                        app.update_check_requested = false;
                                        app.overlay = Some(Overlay::Notice {
                                            title: " update ".to_owned(),
                                            body: "Checking for updates…".to_owned(),
                                            ok: false,
                                        });
                                        spawn_update(&update_tx, true);
                                    }
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
                                        app.transcript.push_user(text);
                                    } else {
                                        submit_prompt(
                                            app, &agent, &options, &event_tx, &compact_tx, text,
                                        );
                                    }
                                }
                            }
                            Action::FollowUp(text) => {
                                if !text.trim().is_empty()
                                    && let Some(text) = submit_user_prompt(
                                        app,
                                        &options.extensions,
                                        text,
                                    )
                                    .await
                                {
                                    agent.follow_up(Message::user(text.clone()));
                                    app.transcript.push_user(text);
                                }
                            }
                        }
                    }
                    Input::Mouse(MouseEventKind::ScrollUp) => {
                        app.transcript.scroll_up(3);
                    }
                    Input::Mouse(MouseEventKind::ScrollDown) => {
                        app.transcript.scroll_down(3);
                    }
                    Input::Mouse(_) => {}
                    Input::Paste(text) => app.editor.paste(&text),
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
            Some(event) = event_rx.recv() => {
                dirty = true;
                handle_agent_event(app, event);
                if maybe_recover_overflow(app) {
                    start_compaction(app, &options, &compact_tx, String::new());
                }
            }
            Some(update) = update_rx.recv() => {
                dirty = true;
                match update {
                    UpdateEvent::Checked { manual, result } => match result {
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
                    },
                    UpdateEvent::Installing(version) => {
                        app.overlay = Some(Overlay::Notice {
                            title: " update ".to_owned(),
                            body: format!("Installing codie {version}…"),
                            ok: false,
                        });
                    }
                    UpdateEvent::Installed(result) => {
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
                if !auto_update_started {
                    auto_update_started = true;
                    // Start after the first frame so startup never waits on the network.
                    if std::env::var_os("CODIE_NO_UPDATE_CHECK").is_none() {
                        spawn_update(&update_tx, false);
                    }
                }
                if app.tick % 30 == 0
                    && let Some(theme) = app
                        .theme_watcher
                        .as_mut()
                        .and_then(crate::theme::ThemeWatcher::poll)
                {
                    app.theme = theme;
                    dirty = true;
                }
                if app.status == Status::Streaming {
                    dirty = true;
                }
            }
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

fn start_run(
    app: &mut App,
    agent: &Arc<Agent>,
    options: &TuiOptions,
    sender: &mpsc::UnboundedSender<AgentEvent>,
    message: Message,
    prompt: String,
) {
    app.transcript.push_user(prompt);
    app.status = Status::Streaming;
    app.run_started = Some(std::time::Instant::now());
    app.footer.tokens_per_sec = None;

    let mut config = match AgentConfig::new(app.model.clone(), options.cwd.clone()) {
        Ok(config) => config,
        Err(error) => {
            app.transcript.push_error(error);
            app.status = Status::Error;
            return;
        }
    };
    config.thinking = app.thinking;
    config.max_turns = options.max_turns;
    config.session_id = app.session.as_ref().map(|session| session.id().to_owned());
    app.cancel = tokio_util::sync::CancellationToken::new();
    config.cancel = app.cancel.clone();

    let history = app.history.clone();
    let stream = agent.run(message, history, config);
    forward_events(stream, sender.clone());
}

fn start_resume(
    app: &mut App,
    agent: &Arc<Agent>,
    options: &TuiOptions,
    sender: &mpsc::UnboundedSender<AgentEvent>,
) {
    app.status = Status::Streaming;
    app.run_started = Some(std::time::Instant::now());
    app.footer.tokens_per_sec = None;

    let mut config = match AgentConfig::new(app.model.clone(), options.cwd.clone()) {
        Ok(config) => config,
        Err(error) => {
            app.transcript.push_error(error);
            app.status = Status::Error;
            return;
        }
    };
    config.thinking = app.thinking;
    config.max_turns = options.max_turns;
    config.session_id = app.session.as_ref().map(|session| session.id().to_owned());
    app.cancel = tokio_util::sync::CancellationToken::new();
    config.cancel = app.cancel.clone();

    let history = app.history.clone();
    let stream = agent.resume(history, config);
    forward_events(stream, sender.clone());
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
    app: &App,
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
    tokio::spawn(async move {
        let custom = (!instructions.trim().is_empty()).then_some(instructions.as_str());
        let input_limit = u64::from(context_window)
            .saturating_sub(crate::compaction::DEFAULT_RESERVE_TOKENS)
            .clamp(4_000, 200_000);
        let result = crate::compaction::summarize(
            client,
            model,
            thinking,
            &messages,
            custom,
            session_id,
            input_limit,
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
            app.footer.context_percent = Some(percent);
            app.status = Status::Idle;
            app.transcript
                .push_info(format!("Compacted the context: {first_line}"));
        }
        Applied::Failed(error) => {
            app.pending_prompt = None;
            app.pending_retry = false;
            app.status = Status::Error;
            app.transcript
                .push_error(format!("Compaction failed: {error}"));
        }
    }
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
                }
                Message::ToolResult(result) => persist_tool_result(app, result),
                _ => {}
            }
        }
        AgentEvent::ToolExecutionStart {
            tool_call_id,
            tool_name,
            args,
        } => app.transcript.tool_start(tool_call_id, tool_name, args),
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
                .tool_end(&tool_call_id, output, is_error, render);
        }
        AgentEvent::TurnEnd { .. } | AgentEvent::TurnStart => {}
        AgentEvent::AgentEnd { messages, status } => {
            app.history = messages;
            app.status = if status == RunStatus::Completed {
                Status::Idle
            } else {
                Status::Error
            };
            app.transcript.scroll_to_bottom();
            if status == RunStatus::TurnLimit {
                app.transcript
                    .push_error("Turn limit reached. Send another prompt to continue.");
            }
        }
        AgentEvent::ToolExecutionUpdate { .. } => {}
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
    );
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
            reply,
        });
        assert!(matches!(app.overlay, Some(Overlay::Permission(_))));
        app.handle_key(key(KeyCode::Char('a')));
        assert_eq!(answer.blocking_recv().unwrap(), Decision::Allow);
        assert!(app.overlay.is_none());
    }

    #[test]
    #[spec("TUI-018")]
    fn spec_tui_018_permission_prompt_accepts_y_and_n() {
        let mut app = app();
        let (reply, answer) = oneshot::channel();
        app.push_permission(PermissionRequest {
            tool: "write".to_owned(),
            arguments: serde_json::json!({"path": "a.txt"}),
            reply,
        });
        app.handle_key(key(KeyCode::Char('y')));
        assert_eq!(answer.blocking_recv().unwrap(), Decision::Allow);
        assert!(app.overlay.is_none());

        let (reply, answer) = oneshot::channel();
        app.push_permission(PermissionRequest {
            tool: "write".to_owned(),
            arguments: serde_json::json!({"path": "a.txt"}),
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

        assert!(app.handle_command("/theme"));
        app.handle_key(key(KeyCode::Enter));
        assert_eq!(app.theme.name, "system");
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
}
