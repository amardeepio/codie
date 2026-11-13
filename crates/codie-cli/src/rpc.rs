//! RPC mode: line-delimited JSON commands on stdin, records on stdout.
//!
//! Implements `PROTO-003` and `PROTO-006`. Responses echo the client `id`, agent
//! events are forwarded unchanged, and every run ends with `agent_settled`.

use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};

use anyhow::{Context, Result};
use codie_ai::{Model, fallback_model, find_model};
use codie_core::{
    Agent, AgentConfig, AgentEvent, AgentEventStream, ChatClient, Message, ModelRef, RunStatus,
    ThinkingLevel, Toolbox,
};
use codie_ext::ExtensionSet;
use codie_prompt::{
    Customizations, PromptInput, build_system_message, expand_attachments, load_project_context,
};
use codie_protocol::ext::ExtSessionInfo;
use codie_protocol::{
    RpcCommand, RpcReady, RpcRequest, RpcResponse, RpcSessionHeader, RpcSettled, SessionType,
};
use codie_session::Session;
use serde::Serialize;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// Everything the RPC server needs from the run setup.
pub(crate) struct RpcOptions {
    pub(crate) client: Arc<dyn ChatClient>,
    pub(crate) toolbox: Arc<dyn Toolbox>,
    pub(crate) cwd: PathBuf,
    pub(crate) session_dir: PathBuf,
    pub(crate) session: Option<Session>,
    pub(crate) history: Vec<Message>,
    pub(crate) model: ModelRef,
    /// Selectable models, including cached OpenRouter catalog entries.
    pub(crate) models: Vec<Model>,
    pub(crate) context_window: u64,
    pub(crate) thinking: ThinkingLevel,
    pub(crate) max_turns: usize,
    pub(crate) preamble: Option<String>,
    pub(crate) append: Option<String>,
    pub(crate) customizations: Customizations,
    pub(crate) extensions: Arc<ExtensionSet>,
}

struct ActiveRun {
    stream: AgentEventStream,
    cancel: CancellationToken,
}

struct State {
    options: RpcOptions,
    agent: Arc<Agent>,
    session: Option<Session>,
    history: Vec<Message>,
    model: ModelRef,
    context_window: u64,
    thinking: ThinkingLevel,
    active: Option<ActiveRun>,
    shutdown_requested: bool,
    exit_code: Option<u8>,
}

/// Runs the RPC server until stdin closes, `shutdown`, or a signal.
pub(crate) async fn run(options: RpcOptions) -> Result<u8> {
    let agent = Arc::new(match options.extensions.hooks() {
        Some(hooks) => Agent::with_hooks(
            Arc::clone(&options.client),
            Arc::clone(&options.toolbox),
            hooks,
        ),
        None => Agent::new(Arc::clone(&options.client), Arc::clone(&options.toolbox)),
    });
    let mut state = State {
        options,
        agent,
        session: None,
        history: Vec::new(),
        model: ModelRef {
            api: String::new(),
            provider: String::new(),
            model: String::new(),
        },
        context_window: 0,
        thinking: ThinkingLevel::Off,
        active: None,
        shutdown_requested: false,
        exit_code: None,
    };
    state.take_options();
    let session_id = state
        .session
        .as_ref()
        .map(|session| session.id().to_owned());
    state.options.extensions.set_session_id(session_id.clone());
    let session_info = ExtSessionInfo {
        id: session_id,
        cwd: state.options.cwd.display().to_string(),
        model: Some(state.model.model.clone()),
    };
    state.options.extensions.session_start(&session_info).await;
    state.emit(&RpcReady::new())?;
    state.emit_session_header()?;

    let (line_tx, mut line_rx) = mpsc::unbounded_channel::<String>();
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        for line in std::io::BufRead::lines(stdin.lock()) {
            match line {
                Ok(line) => {
                    if line_tx.send(line).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    let (signal_tx, mut signal_rx) = mpsc::unbounded_channel::<u8>();
    install_signal_handlers(signal_tx);

    loop {
        if state.shutdown_requested && state.active.is_none() {
            break;
        }

        tokio::select! {
            line = line_rx.recv() => {
                let Some(line) = line else {
                    state.shutdown_requested = true;
                    if let Some(active) = state.active.as_ref() {
                        active.cancel.cancel();
                    }
                    continue;
                };
                if line.trim().is_empty() {
                    continue;
                }
                state.handle_line(&line).await?;
            }
            event = next_event(&mut state.active) => {
                if let Some(event) = event {
                    state.handle_event(event)?;
                } else {
                    state.finish_run(RunStatus::Error)?;
                }
            }
            Some(code) = signal_rx.recv() => {
                state.exit_code = Some(code);
                state.shutdown_requested = true;
                if let Some(active) = state.active.as_ref() {
                    active.cancel.cancel();
                } else {
                    break;
                }
            }
        }
    }

    if state.active.is_some() {
        state.finish_run(RunStatus::Aborted)?;
    }
    state.options.extensions.shutdown("rpc exit").await;

    Ok(state.exit_code.unwrap_or(0))
}

impl State {
    fn take_options(&mut self) {
        self.model = self.options.model.clone();
        self.context_window = self.options.context_window;
        self.thinking = self.options.thinking;
        self.history = std::mem::take(&mut self.options.history);
        self.session = self.options.session.take();
    }

    fn emit<T: Serialize>(&self, record: &T) -> Result<()> {
        let mut stdout = std::io::stdout().lock();
        writeln!(stdout, "{}", serde_json::to_string(record)?)?;
        stdout.flush()?;
        Ok(())
    }

    fn respond(&self, response: RpcResponse) -> Result<()> {
        self.emit(&response)
    }

    fn emit_session_header(&self) -> Result<()> {
        match self.session.as_ref() {
            Some(session) => self.emit(&RpcSessionHeader {
                record_type: SessionType::Session,
                version: session.header().version,
                id: session.header().id.clone(),
                timestamp: session.header().timestamp.clone(),
                cwd: session.header().cwd.clone(),
                parent_session: session.header().parent_session.clone(),
            })?,
            None => self.emit(&RpcSessionHeader {
                record_type: SessionType::Session,
                version: 1,
                id: "ephemeral".to_owned(),
                timestamp: String::new(),
                cwd: self.options.cwd.display().to_string(),
                parent_session: None,
            })?,
        }
        Ok(())
    }

    async fn handle_line(&mut self, line: &str) -> Result<()> {
        let value: Value = match serde_json::from_str(line) {
            Ok(value) => value,
            Err(error) => {
                return self.respond(RpcResponse::failure(
                    None,
                    "unknown",
                    "bad_request",
                    format!("invalid JSON: {error}"),
                ));
            }
        };
        let id = value.get("id").and_then(Value::as_str).map(str::to_owned);
        let fallback_command = value
            .get("command")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_owned();

        let request: RpcRequest = match serde_json::from_value(value) {
            Ok(request) => request,
            Err(error) => {
                return self.respond(RpcResponse::failure(
                    id,
                    fallback_command,
                    "bad_request",
                    error.to_string(),
                ));
            }
        };

        let command = request.command_name();
        let id = request.id.clone();
        match request.command {
            RpcCommand::Prompt { message } => self.prompt(id, command, &message).await,
            RpcCommand::Steer { message } => self.queue(id, command, &message, true),
            RpcCommand::FollowUp { message } => self.queue(id, command, &message, false),
            RpcCommand::Abort => {
                if let Some(active) = self.active.as_ref() {
                    active.cancel.cancel();
                    self.respond(RpcResponse::success(id, command, json!({"aborted": true})))
                } else {
                    self.respond(RpcResponse::failure(
                        id,
                        command,
                        "no_active_run",
                        "no run is active",
                    ))
                }
            }
            RpcCommand::GetState => self.get_state(id, command),
            RpcCommand::GetMessages => self.get_messages(id, command),
            RpcCommand::GetEntries => self.get_entries(id, command),
            RpcCommand::GetSessionStats => self.get_session_stats(id, command),
            RpcCommand::NewSession => self.new_session(id, command).await,
            RpcCommand::SetModel { model, provider } => {
                self.set_model(id, command, &model, provider.as_deref())
            }
            RpcCommand::SetThinking { level } => self.set_thinking(id, command, &level),
            RpcCommand::Compact { instructions } => {
                self.compact(id, command, instructions.as_deref()).await
            }
            RpcCommand::ExportHtml { path } => self.export_html(id, command, path.as_deref()),
            RpcCommand::Shutdown => {
                self.shutdown_requested = true;
                if let Some(active) = self.active.as_ref() {
                    active.cancel.cancel();
                }
                self.respond(RpcResponse::success(id, command, json!({"shutdown": true})))
            }
        }
    }

    async fn prompt(&mut self, id: Option<String>, command: &str, message: &str) -> Result<()> {
        let session_info = ExtSessionInfo {
            id: self.session.as_ref().map(|session| session.id().to_owned()),
            cwd: self.options.cwd.display().to_string(),
            model: Some(self.model.model.clone()),
        };
        let mut message = message.to_owned();
        if let Some(invocation) = self
            .options
            .extensions
            .resolve_command(&message)
            .map_err(anyhow::Error::msg)?
        {
            match self
                .options
                .extensions
                .invoke_command(&invocation, &session_info)
                .await
            {
                Ok(result) => {
                    if let Some(prompt) = result.prompt {
                        message = prompt;
                    } else {
                        return self.respond(RpcResponse::success(
                            id,
                            command,
                            json!({"handled": true, "display": result.display}),
                        ));
                    }
                }
                Err(error) => {
                    return self.respond(RpcResponse::failure(
                        id,
                        command,
                        "extension_failed",
                        error.to_string(),
                    ));
                }
            }
        }
        let submitted = self
            .options
            .extensions
            .user_submit(&session_info, &message)
            .await;
        if submitted.block {
            return self.respond(RpcResponse::failure(
                id,
                command,
                "blocked",
                submitted
                    .reason
                    .unwrap_or_else(|| "prompt blocked by an extension".to_owned()),
            ));
        }
        if let Some(prompt) = submitted.prompt {
            message = prompt;
        }
        let expansion = expand_attachments(&message, &self.options.cwd);
        let prompt = Message::user_with_content(expansion.content);
        let warnings = expansion.errors;

        if self.active.is_some() {
            self.agent.follow_up(prompt);
            return self.respond(RpcResponse::success(
                id,
                command,
                json!({"queued": true, "mode": "follow_up", "warnings": warnings}),
            ));
        }

        let cancel = CancellationToken::new();
        let config = self.agent_config(cancel.clone())?;
        let stream = self.agent.run(prompt, self.history.clone(), config);
        self.active = Some(ActiveRun { stream, cancel });
        self.respond(RpcResponse::success(
            id,
            command,
            json!({
                "started": true,
                "session": self.session.as_ref().map(Session::id),
                "warnings": warnings,
            }),
        ))
    }

    fn queue(
        &mut self,
        id: Option<String>,
        command: &str,
        message: &str,
        steer: bool,
    ) -> Result<()> {
        if self.active.is_none() {
            return self.respond(RpcResponse::failure(
                id,
                command,
                "no_active_run",
                "no run is active",
            ));
        }
        let message = Message::user(message);
        if steer {
            self.agent.steer(message);
        } else {
            self.agent.follow_up(message);
        }
        self.respond(RpcResponse::success(id, command, json!({"queued": true})))
    }

    fn get_state(&self, id: Option<String>, command: &str) -> Result<()> {
        self.respond(RpcResponse::success(
            id,
            command,
            json!({
                "session": self.session.as_ref().map(Session::id),
                "cwd": self.options.cwd.display().to_string(),
                "model": &self.model,
                "context_window": self.context_window,
                "thinking": self.thinking.as_str(),
                "active": self.active.is_some(),
                "messages": self.history.len(),
                "estimated_tokens": self.estimated_tokens(),
                "context_percent": self.context_percent(),
            }),
        ))
    }

    fn get_messages(&self, id: Option<String>, command: &str) -> Result<()> {
        let messages = self
            .history
            .iter()
            .filter(|message| !matches!(message, Message::System(_)))
            .collect::<Vec<_>>();
        self.respond(RpcResponse::success(
            id,
            command,
            json!({"messages": messages}),
        ))
    }

    fn get_entries(&self, id: Option<String>, command: &str) -> Result<()> {
        let entries = self
            .session
            .as_ref()
            .map(|session| session.entries())
            .unwrap_or_default();
        self.respond(RpcResponse::success(
            id,
            command,
            json!({
                "session": self.session.as_ref().map(Session::id),
                "entries": entries,
            }),
        ))
    }

    fn get_session_stats(&self, id: Option<String>, command: &str) -> Result<()> {
        self.respond(RpcResponse::success(
            id,
            command,
            json!({
                "session": self.session.as_ref().map(Session::id),
                "messages": self.history.len(),
                "entries": self.session.as_ref().map_or(0, |session| session.entries().len()),
                "estimated_tokens": self.estimated_tokens(),
                "context_window": self.context_window,
                "context_percent": self.context_percent(),
                "usage": self.session.as_ref().and_then(Session::last_usage),
            }),
        ))
    }

    async fn new_session(&mut self, id: Option<String>, command: &str) -> Result<()> {
        if self.active.is_some() {
            return self.respond(RpcResponse::failure(
                id,
                command,
                "run_active",
                "abort the active run before starting a new session",
            ));
        }
        self.options
            .extensions
            .session_shutdown("new session")
            .await;
        let mut session = Session::create(&self.options.cwd, &self.options.session_dir)?;
        let system = self.system_message();
        session.append_message(&system)?;
        self.history = vec![system];
        let session_id = session.id().to_owned();
        self.session = Some(session);
        self.options
            .extensions
            .set_session_id(Some(session_id.clone()));
        let session_info = ExtSessionInfo {
            id: Some(session_id.clone()),
            cwd: self.options.cwd.display().to_string(),
            model: Some(self.model.model.clone()),
        };
        self.options.extensions.session_start(&session_info).await;
        self.emit_session_header()?;
        self.respond(RpcResponse::success(
            id,
            command,
            json!({"session": session_id}),
        ))
    }

    fn set_model(
        &mut self,
        id: Option<String>,
        command: &str,
        model_id: &str,
        provider: Option<&str>,
    ) -> Result<()> {
        let provider = provider.unwrap_or(&self.model.provider).to_owned();
        if provider != self.model.provider {
            return self.respond(RpcResponse::failure(
                id,
                command,
                "unsupported",
                "changing provider over RPC is not supported; restart codie instead",
            ));
        }

        let model = self
            .options
            .models
            .iter()
            .find(|candidate| candidate.provider == provider && candidate.id == model_id)
            .cloned()
            .or_else(|| find_model(&provider, model_id))
            .unwrap_or_else(|| fallback_model(&provider, model_id));
        self.model.model = model_id.to_owned();
        self.context_window = u64::from(model.context_window);
        if let Some(session) = self.session.as_mut() {
            session.append_model_change(&provider, model_id)?;
        }
        self.respond(RpcResponse::success(
            id,
            command,
            json!({"model": &self.model, "context_window": self.context_window}),
        ))
    }

    fn set_thinking(&mut self, id: Option<String>, command: &str, level: &str) -> Result<()> {
        let Some(thinking) = ThinkingLevel::parse(level) else {
            return self.respond(RpcResponse::failure(
                id,
                command,
                "invalid_argument",
                format!("unknown thinking level `{level}`"),
            ));
        };
        self.thinking = thinking;
        if let Some(session) = self.session.as_mut() {
            session.append_thinking_level_change(thinking)?;
        }
        self.respond(RpcResponse::success(
            id,
            command,
            json!({"thinking": thinking.as_str()}),
        ))
    }

    async fn compact(
        &mut self,
        id: Option<String>,
        command: &str,
        instructions: Option<&str>,
    ) -> Result<()> {
        if self.active.is_some() {
            return self.respond(RpcResponse::failure(
                id,
                command,
                "run_active",
                "abort the active run before compacting",
            ));
        }
        let input_limit = self
            .context_window
            .saturating_sub(codie_tui::compaction::DEFAULT_RESERVE_TOKENS)
            .clamp(4_000, 200_000);
        let session_id = self.session.as_ref().map(|session| session.id().to_owned());

        let outcome = codie_tui::compaction::summarize(
            Arc::clone(&self.options.client),
            self.model.clone(),
            self.thinking,
            &self.history,
            instructions,
            session_id,
            input_limit,
        )
        .await;

        let outcome = match outcome {
            Ok(outcome) => outcome,
            Err(error) => {
                return self.respond(RpcResponse::failure(
                    id,
                    command,
                    "compaction_failed",
                    error.to_string(),
                ));
            }
        };

        match self.session.as_mut() {
            Some(session) => {
                self.history = codie_tui::compaction::apply(session, &outcome, instructions)
                    .map_err(anyhow::Error::msg)?;
            }
            None => {
                // Without a session there is nowhere to persist the summary.
                return self.respond(RpcResponse::failure(
                    id,
                    command,
                    "no_session",
                    "compaction requires a session",
                ));
            }
        }

        self.respond(RpcResponse::success(
            id,
            command,
            json!({
                "compacted": true,
                "messages": self.history.len(),
                "estimated_tokens": self.estimated_tokens(),
            }),
        ))
    }

    fn export_html(&self, id: Option<String>, command: &str, path: Option<&str>) -> Result<()> {
        let Some(session) = self.session.as_ref() else {
            return self.respond(RpcResponse::failure(
                id,
                command,
                "no_session",
                "there is no session to export",
            ));
        };
        let path = path.map_or_else(
            || {
                self.options
                    .cwd
                    .join(format!("codie-session-{}.html", session.id()))
            },
            PathBuf::from,
        );
        let file = std::fs::File::create(&path)
            .with_context(|| format!("cannot create {}", path.display()))?;
        session.export_html(file)?;
        self.respond(RpcResponse::success(
            id,
            command,
            json!({"path": path.display().to_string()}),
        ))
    }

    fn handle_event(&mut self, event: AgentEvent) -> Result<()> {
        if let AgentEvent::MessageEnd { message } = &event
            && let Some(session) = self.session.as_mut()
        {
            session.append_message(message)?;
        }

        match &event {
            AgentEvent::AgentEnd { messages, status } => {
                self.history = messages.clone();
                let aborted = *status == RunStatus::Aborted;
                self.emit(&event)?;
                self.active = None;
                self.emit(&RpcSettled::new(aborted))?;
            }
            _ => self.emit(&event)?,
        }
        Ok(())
    }

    fn finish_run(&mut self, status: RunStatus) -> Result<()> {
        if self.active.take().is_some() {
            self.emit(&RpcSettled::new(status == RunStatus::Aborted))?;
        }
        Ok(())
    }

    fn agent_config(&self, cancel: CancellationToken) -> Result<AgentConfig> {
        let mut config = AgentConfig::new(self.model.clone(), self.options.cwd.clone())
            .map_err(anyhow::Error::msg)?;
        config.thinking = self.thinking;
        config.max_turns = self.options.max_turns;
        config.session_id = self.session.as_ref().map(|session| session.id().to_owned());
        config.cancel = cancel;
        Ok(config)
    }

    fn system_message(&self) -> Message {
        let declarations = self.options.toolbox.declarations();
        let input = PromptInput {
            cwd: self.options.cwd.clone(),
            tools: declarations.clone(),
            preamble: self.options.preamble.clone(),
            append: self.options.append.clone(),
            skills: self.options.customizations.skills().to_vec(),
            project_context: load_project_context(&self.options.cwd),
        };
        build_system_message(&input, declarations)
    }

    fn estimated_tokens(&self) -> u64 {
        self.session
            .as_ref()
            .map_or_else(|| history_tokens(&self.history), Session::estimate_tokens)
    }

    fn context_percent(&self) -> u8 {
        match self.session.as_ref() {
            Some(session) => session.context_percent(self.context_window),
            None => {
                let tokens = history_tokens(&self.history);
                let percent = tokens.saturating_mul(100) / self.context_window.max(1);
                u8::try_from(percent.min(100)).unwrap_or(100)
            }
        }
    }
}

fn history_tokens(history: &[Message]) -> u64 {
    history
        .iter()
        .map(|message| {
            let text = match message {
                Message::System(message) => message.content.clone(),
                Message::User(message) => message
                    .content
                    .iter()
                    .filter_map(codie_core::Content::as_text)
                    .collect::<String>(),
                Message::Assistant(message) => message.text(),
                Message::ToolResult(message) => message
                    .content
                    .iter()
                    .filter_map(codie_core::Content::as_text)
                    .collect::<String>(),
            };
            text.chars().count() as u64 / 4
        })
        .sum()
}

async fn next_event(active: &mut Option<ActiveRun>) -> Option<AgentEvent> {
    match active.as_mut() {
        Some(run) => run.stream.recv().await,
        None => std::future::pending().await,
    }
}

/// Installs signal handlers. The first signal cancels the run and records the
/// exit code; a second signal exits immediately (`PROTO-006`).
fn install_signal_handlers(sender: mpsc::UnboundedSender<u8>) {
    let seen = Arc::new(AtomicI32::new(0));
    let tx = sender;

    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};

        for (kind, code) in [
            (SignalKind::interrupt(), 130),
            (SignalKind::terminate(), 143),
            (SignalKind::hangup(), 129),
        ] {
            let seen = Arc::clone(&seen);
            let tx = tx.clone();
            tokio::spawn(async move {
                let Ok(mut stream) = signal(kind) else {
                    return;
                };
                while stream.recv().await.is_some() {
                    if seen.swap(1, Ordering::SeqCst) == 0 {
                        let _ = tx.send(code);
                    } else {
                        std::process::exit(i32::from(code));
                    }
                }
            });
        }
    }

    #[cfg(not(unix))]
    {
        tokio::spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                let _ = tx.send(130);
            }
        });
    }
}
