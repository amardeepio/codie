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
use codie_prompt::{PromptInput, build_system_message, load_project_context};
use codie_session::Session;
use codie_tools::Registry;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{DisableBracketedPaste, EnableBracketedPaste, Event};
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
use crate::transcript::Transcript;

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

/// Everything the interactive app needs from the CLI.
pub struct TuiOptions {
    /// Provider client.
    pub client: Arc<dyn ChatClient>,
    /// Working directory.
    pub cwd: PathBuf,
    /// Model identity.
    pub model: ModelRef,
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
    /// Exit code observed from a signal, if any.
    pub signal: Arc<std::sync::atomic::AtomicI32>,
    /// Tool registry selected by the CLI.
    pub registry: Arc<Registry>,
    /// Optional keybindings file.
    pub keybindings: Option<PathBuf>,
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
    /// Animation tick.
    pub tick: u64,
    /// Current model identity.
    pub model: ModelRef,
    /// Current thinking level.
    pub thinking: ThinkingLevel,
    /// Conversation history.
    pub history: Vec<Message>,
    /// Active session, when persisted.
    pub session: Option<Session>,
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
}

impl App {
    /// Creates an app for one interactive session.
    #[must_use]
    pub fn new(
        cwd: impl Into<PathBuf>,
        model: ModelRef,
        thinking: ThinkingLevel,
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
            tick: 0,
            model,
            thinking,
            history,
            session,
            session_dir,
            cwd: binding_cwd,
            permissions: VecDeque::new(),
            signal,
            cancel: tokio_util::sync::CancellationToken::new(),
            bindings,
            theme_watcher,
        }
    }

    /// Handles one terminal key event.
    pub fn handle_key(&mut self, key: KeyEvent) -> Action {
        if key.kind == KeyEventKind::Release {
            return Action::None;
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
        }
    }

    fn handle_overlay_key(&mut self, key: KeyEvent) -> Action {
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
            KeyCode::Char('a') => Some(Decision::Allow),
            KeyCode::Char('A') => Some(Decision::Always),
            KeyCode::Char('d') | KeyCode::Esc => Some(Decision::Deny),
            _ => None,
        };
        if let Some(decision) = decision {
            self.resolve_permission(decision);
        }
        Action::None
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
            Command::New => self.new_session(),
            Command::Session => {
                let text = self.session.as_ref().map_or_else(
                    || "No session is being persisted.".to_owned(),
                    |session| format!("Session {} · {}", session.id(), session.path().display()),
                );
                self.transcript.push_info(text);
            }
            Command::Model(argument) => {
                if let Some(model) = argument {
                    self.model.model = model.clone();
                    self.footer.model = model;
                    self.transcript.push_info("Model changed for the next run.");
                } else {
                    self.transcript
                        .push_info(format!("Model: {}", self.model.model));
                }
            }
            Command::Thinking(argument) => {
                if let Some(value) = argument {
                    if let Some(level) = parse_thinking(&value) {
                        self.thinking = level;
                        self.footer.thinking = level.as_str().to_owned();
                        self.transcript
                            .push_info(format!("Thinking level: {}", level.as_str()));
                    } else {
                        self.transcript
                            .push_error(format!("Unknown thinking level `{value}`."));
                    }
                } else {
                    self.transcript
                        .push_info(format!("Thinking level: {}", self.thinking.as_str()));
                }
            }
            Command::Compact(_) => self
                .transcript
                .push_info("Manual compaction arrives in Phase 3."),
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
            if let Some(command) = crate::commands::completions(token).first() {
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
    pub fn update_usage(&mut self, usage: &Usage) {
        self.footer.tokens = self.footer.tokens.saturating_add(usage.total_tokens);
        self.footer.cost = self.footer.cost.saturating_add(usage.cost.total);
        let context = self.history.iter().map(message_chars).sum::<u64>() / 4;
        let percent = context.saturating_mul(100) / 200_000;
        self.footer.context_percent = Some(u8::try_from(percent.min(100)).unwrap_or(100));
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
    let toolbox: Arc<dyn Toolbox> = Arc::new(PermissionToolbox::new(registry.clone(), broker));
    let agent = Arc::new(Agent::new(
        Arc::clone(&options.client),
        Arc::clone(&toolbox),
    ));

    let mut history = options.history.clone();
    let mut app = App::new(
        options.cwd.clone(),
        options.model.clone(),
        options.thinking,
        theme,
        options.session.take(),
        history.clone(),
        options.session_dir.clone(),
        Arc::clone(&options.signal),
        Bindings::load(options.keybindings.as_deref())?,
    );

    if !matches!(history.first(), Some(Message::System(_))) {
        let declarations = toolbox.declarations();
        let input = PromptInput {
            cwd: options.cwd.clone(),
            tools: declarations.clone(),
            preamble: None,
            append: None,
            project_context: load_project_context(&options.cwd),
        };
        let system = build_system_message(&input, declarations);
        if let Some(session) = app.session.as_mut() {
            session.append_message(&system)?;
        }
        history.insert(0, system);
        app.history = history.clone();
    }

    let mut terminal = setup_terminal(options.mode)?;
    let result = event_loop(
        &mut terminal,
        &mut app,
        agent,
        toolbox,
        options,
        &mut permission_rx,
    )
    .await;
    restore_terminal();
    result
}

fn setup_terminal(mode: TuiRunMode) -> Result<Terminal<CrosstermBackend<std::io::Stdout>>> {
    install_panic_hook();
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    if mode == TuiRunMode::Full {
        execute!(stdout, EnterAlternateScreen)?;
    }
    execute!(stdout, EnableBracketedPaste)?;
    let terminal = Terminal::new(CrosstermBackend::new(stdout))?;
    Ok(terminal)
}

fn restore_terminal() {
    let _ = disable_raw_mode();
    let mut stdout = std::io::stdout();
    let _ = execute!(
        stdout,
        DisableBracketedPaste,
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
    Paste(String),
    Signal(i32),
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

async fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    app: &mut App,
    agent: Arc<Agent>,
    toolbox: Arc<dyn Toolbox>,
    options: TuiOptions,
    permission_rx: &mut mpsc::UnboundedReceiver<PermissionRequest>,
) -> Result<u8> {
    let mut input = spawn_input();
    let (event_tx, mut event_rx) = mpsc::unbounded_channel();
    let mut tick = tokio::time::interval(Duration::from_millis(33));
    let mut dirty = true;
    let mut signal_code = 0_i32;

    if let Some(prompt) = options.initial_prompt.clone() {
        start_run(app, &agent, &options, &event_tx, prompt);
    }

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
                            Action::Cancel => {
                                app.cancel.cancel();
                                app.transcript.push_info("Cancelling…");
                            }
                            Action::Submit(text) => {
                                if app.handle_command(&text) {
                                    if app.should_quit { break; }
                                } else if app.status == Status::Streaming {
                                    agent.steer(Message::user(text.clone()));
                                    app.transcript.push_user(text);
                                } else {
                                    start_run(app, &agent, &options, &event_tx, text);
                                }
                            }
                            Action::FollowUp(text) => {
                                if !text.trim().is_empty() {
                                    agent.follow_up(Message::user(text.clone()));
                                    app.transcript.push_user(text);
                                }
                            }
                        }
                    }
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
            }
            _ = tick.tick() => {
                app.tick = app.tick.wrapping_add(1);
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

fn start_run(
    app: &mut App,
    agent: &Arc<Agent>,
    options: &TuiOptions,
    sender: &mpsc::UnboundedSender<AgentEvent>,
    prompt: String,
) {
    app.transcript.push_user(prompt.clone());
    app.status = Status::Streaming;

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
    let mut stream = agent.run(Message::user(prompt), history, config);
    let sender = sender.clone();
    tokio::spawn(async move {
        while let Some(event) = stream.recv().await {
            let terminal = matches!(event, AgentEvent::AgentEnd { .. });
            if sender.send(event).is_err() || terminal {
                break;
            }
        }
    });
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
                    app.update_usage(&assistant.usage);
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
            app.transcript.tool_end(&tool_call_id, output, is_error);
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
    }

    #[test]
    #[spec("TUI-002")]
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
}
