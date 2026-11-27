//! RPC mode: line-delimited JSON commands on stdin, records on stdout.
//!
//! Implements `PROTO-003` and `PROTO-006`. Responses echo the client `id`, agent
//! events are forwarded unchanged, and every run ends with `agent_settled`.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};

use anyhow::{Context, Result};
use codie_ai::{AuthStore, Model, auth_status, fallback_model, find_model, provider};
use codie_core::{
    Agent, AgentConfig, AgentEvent, AgentEventStream, ChatClient, Cost, Message, ModelRef,
    RunStatus, ThinkingLevel, ToolDecl, Toolbox, Usage,
};
use codie_ext::ExtensionSet;
use codie_prompt::{
    Customizations, MAX_ATTACHMENTS, PromptInput, build_system_message, expand_attachments,
    inline_image, load_project_context, load_project_memory, quote_attachment_path,
};
use codie_protocol::ext::ExtSessionInfo;
use codie_protocol::{
    PermissionDecision, PermissionMode, RpcAttachment, RpcCommand, RpcPermissionRequest, RpcReady,
    RpcRequest, RpcResponse, RpcSessionHeader, RpcSettled, RpcSubagentMode, SessionType,
};
use codie_session::{Entry, Session};
use codie_subagents::{
    DelegationMode, FORCED_DIRECTIVE, SubagentConfig, SubagentRuntime, SubagentToolbox, TaskTool,
    ToolSelection, detect_intent, effective_mode, mode_directive,
};
use serde::Serialize;
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use crate::permissions::{PermissionGate, PermissionPrompt, RpcToolbox};

/// Everything the RPC server needs from the run setup.
pub(crate) struct RpcOptions {
    pub(crate) client: Arc<dyn ChatClient>,
    /// Raw tool registry; the interactive policy is layered on top per mode.
    pub(crate) toolbox: Arc<dyn Toolbox>,
    /// Mutating tools pre-approved with `--allow` or configuration.
    pub(crate) allowed: Vec<String>,
    /// Effective configuration, used by `get_config`/`set_config`.
    pub(crate) config: crate::config::Config,
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
    /// Resolved subagent configuration, including project personas.
    pub(crate) subagents: SubagentConfig,
    /// Initial delegation mode for the session.
    pub(crate) subagent_mode: DelegationMode,
}

struct ActiveRun {
    stream: AgentEventStream,
    cancel: CancellationToken,
}

struct State {
    options: RpcOptions,
    agent: Arc<Agent>,
    /// The root toolbox, including `task` when delegation is enabled. System
    /// messages must declare exactly this set.
    toolbox: Arc<dyn Toolbox>,
    session: Option<Session>,
    history: Vec<Message>,
    model: ModelRef,
    context_window: u64,
    thinking: ThinkingLevel,
    active: Option<ActiveRun>,
    shutdown_requested: bool,
    exit_code: Option<u8>,
    gate: PermissionGate,
    pending_permissions: HashMap<String, oneshot::Sender<PermissionDecision>>,
    permission_sequence: u64,
    subagent_runtime: Option<Arc<SubagentRuntime>>,
    subagent_config: SubagentConfig,
    subagent_mode: DelegationMode,
    subagent_force_once: bool,
}

/// Runs the RPC server until stdin closes, `shutdown`, or a signal.
pub(crate) async fn run(options: RpcOptions) -> Result<u8> {
    let (permission_tx, mut permission_rx) = mpsc::unbounded_channel();
    let gate = PermissionGate::new(permission_tx, options.allowed.clone());
    // The permission wrapper is the parent toolbox for children and the outer
    // layer for the root, so `task` itself is gated like any mutating tool
    // (`SEC-009`) while children inherit the same policy (`AGENT-022`).
    let permission = Arc::new(RpcToolbox::new(
        Arc::clone(&options.toolbox),
        options.allowed.clone(),
        gate.clone(),
    ));
    let parent_toolbox: Arc<dyn Toolbox> = permission.clone();

    let subagent_config = options.subagents.clone();
    let subagent_mode = if subagent_config.enabled {
        options.subagent_mode
    } else {
        DelegationMode::Off
    };
    let subagent_runtime = if subagent_config.enabled {
        let prompt_input = PromptInput {
            cwd: options.cwd.clone(),
            tools: Vec::new(),
            preamble: options.preamble.clone(),
            append: options.append.clone(),
            skills: options.customizations.skills().to_vec(),
            project_context: load_project_context(&options.cwd),
            project_memory: load_project_memory(&options.cwd),
        };
        let runtime = Arc::new(
            SubagentRuntime::new(
                subagent_config.clone(),
                Arc::clone(&options.client),
                Arc::clone(&parent_toolbox),
                prompt_input,
                options.session_dir.clone(),
                options
                    .session
                    .as_ref()
                    .map(|session| session.id().to_owned()),
                options.model.clone(),
                options
                    .models
                    .iter()
                    .map(|model| model.id.clone())
                    .collect(),
            )
            .map_err(anyhow::Error::msg)?,
        );
        runtime.set_mode(subagent_mode);
        runtime.set_thinking(options.thinking);
        // A nested `task` (max_depth > 0) must pass the same permission gate as
        // the root call instead of spawning unguarded (`AGENT-038`).
        runtime.set_nested_task(permission.guard(Arc::new(TaskTool::new(Arc::clone(&runtime)))));
        Some(runtime)
    } else {
        None
    };

    let toolbox: Arc<dyn Toolbox> = match &subagent_runtime {
        Some(runtime) => {
            let subagents = Arc::new(SubagentToolbox::new(
                Arc::clone(&options.toolbox),
                Arc::clone(runtime),
            ));
            Arc::new(RpcToolbox::new(
                subagents,
                options.allowed.clone(),
                gate.clone(),
            ))
        }
        None => Arc::clone(&parent_toolbox),
    };
    let agent = Arc::new(match options.extensions.hooks() {
        Some(hooks) => Agent::with_hooks(Arc::clone(&options.client), Arc::clone(&toolbox), hooks),
        None => Agent::new(Arc::clone(&options.client), Arc::clone(&toolbox)),
    });
    let mut state = State {
        options,
        agent,
        toolbox,
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
        gate,
        pending_permissions: HashMap::new(),
        permission_sequence: 0,
        subagent_runtime,
        subagent_config,
        subagent_mode,
        subagent_force_once: false,
    };
    state.take_options();
    // The system prompt must be built from the assembled toolbox (including
    // `task`) rather than the raw registry. A resumed session may carry a
    // stale prompt from before delegation was enabled, so repair it in memory
    // before the first request.
    state.ensure_system_message()?;
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
                    state.deny_pending_permissions();
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
            Some(prompt) = permission_rx.recv() => {
                state.permission_requested(prompt)?;
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
                state.deny_pending_permissions();
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
                persisted: session.is_persisted(),
            })?,
            None => self.emit(&RpcSessionHeader {
                record_type: SessionType::Session,
                version: 1,
                id: "ephemeral".to_owned(),
                timestamp: String::new(),
                cwd: self.options.cwd.display().to_string(),
                parent_session: None,
                persisted: false,
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
            RpcCommand::Prompt {
                message,
                attachments,
            } => self.prompt(id, command, &message, &attachments).await,
            RpcCommand::Steer { message } => self.queue(id, command, &message, true),
            RpcCommand::FollowUp { message } => self.queue(id, command, &message, false),
            RpcCommand::Abort => {
                if let Some(active) = self.active.as_ref() {
                    active.cancel.cancel();
                    self.deny_pending_permissions();
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
            RpcCommand::SetPermissionMode { mode } => self.set_permission_mode(id, command, mode),
            RpcCommand::PermissionResponse {
                request_id,
                decision,
            } => self.permission_response(id, command, &request_id, decision),
            RpcCommand::GetState => self.get_state(id, command),
            RpcCommand::GetCatalog => self.get_catalog(id, command),
            RpcCommand::GetThemes => self.get_themes(id, command),
            RpcCommand::GetConfig => self.get_config(id, command),
            RpcCommand::SetConfig { key, value } => self.set_config(id, command, &key, &value),
            RpcCommand::AuthStatus => self.auth_status(id, command),
            RpcCommand::AuthSetKey {
                provider,
                key,
                verify,
            } => {
                self.auth_set_key(id, command, &provider, &key, verify)
                    .await
            }
            RpcCommand::AuthLogout { provider } => self.auth_logout(id, command, &provider),
            RpcCommand::GetSubagents => self.get_subagents(id, command),
            RpcCommand::SetSubagents { mode, parallel } => {
                self.set_subagents(id, command, mode, parallel)
            }
            RpcCommand::CancelSubagent { call_id } => self.cancel_subagent(id, command, &call_id),
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
                self.deny_pending_permissions();
                if let Some(active) = self.active.as_ref() {
                    active.cancel.cancel();
                }
                self.respond(RpcResponse::success(id, command, json!({"shutdown": true})))
            }
        }
    }

    async fn prompt(
        &mut self,
        id: Option<String>,
        command: &str,
        message: &str,
        attachments: &[RpcAttachment],
    ) -> Result<()> {
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

        // Path attachments reuse the `@path` expansion so text files and
        // images behave exactly like an inline reference.
        let mut text = message;
        for attachment in attachments {
            if let RpcAttachment::Path { path } = attachment {
                text.push(' ');
                text.push_str(&quote_attachment_path(Path::new(path)));
            }
        }

        let expansion = expand_attachments(&text, &self.options.cwd);
        let mut content = expansion.content;
        let mut warnings = expansion.errors;
        let mut attached = expansion.attached.len();
        for attachment in attachments {
            let RpcAttachment::Image {
                data,
                mime_type,
                name,
            } = attachment
            else {
                continue;
            };
            attached += 1;
            if attached > MAX_ATTACHMENTS {
                warnings.push(format!(
                    "Attachment limit reached: at most {MAX_ATTACHMENTS} files per message."
                ));
                continue;
            }
            match inline_image(data, mime_type) {
                Ok(block) => content.push(block),
                Err(error) => warnings.push(format!(
                    "Cannot attach `{}`: {error}",
                    name.as_deref().unwrap_or("image")
                )),
            }
        }
        let prompt = Message::user_with_content(content);

        if self.active.is_some() {
            self.agent.follow_up(prompt);
            return self.respond(RpcResponse::success(
                id,
                command,
                json!({"queued": true, "mode": "follow_up", "warnings": warnings}),
            ));
        }

        // Delegation mode is resolved per prompt, matching the interactive
        // interface. A prompt that asks for (or suppresses) delegation wins over
        // the session mode, and a force run records the directive the model saw.
        let intent = detect_intent(&text);
        let plan = effective_mode(
            self.subagent_config.enabled,
            self.subagent_mode,
            self.subagent_force_once,
            intent,
        );
        self.subagent_force_once = false;
        if let Some(runtime) = &self.subagent_runtime {
            runtime.set_mode(plan.mode);
        }
        if plan.is_forced() {
            let directive = Message::system(FORCED_DIRECTIVE, Vec::new());
            if let Some(session) = self.session.as_mut() {
                session.append_message(&directive)?;
            }
            self.history.push(directive);
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
                "subagents": plan.mode.as_str(),
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

    fn set_permission_mode(
        &self,
        id: Option<String>,
        command: &str,
        mode: PermissionMode,
    ) -> Result<()> {
        self.gate.set_mode(mode);
        self.respond(RpcResponse::success(
            id,
            command,
            json!({"mode": mode.as_str()}),
        ))
    }

    /// Emits a permission request and parks the reply channel until the client
    /// answers, aborts, or disconnects.
    fn permission_requested(&mut self, prompt: PermissionPrompt) -> Result<()> {
        self.permission_sequence += 1;
        let request = RpcPermissionRequest::new(
            format!("perm-{}", self.permission_sequence),
            prompt.tool_call_id,
            prompt.tool,
            prompt.arguments,
        )
        .with_origin(prompt.agent, prompt.depth);
        self.emit(&request)?;
        self.pending_permissions.insert(request.id, prompt.reply);
        Ok(())
    }

    fn permission_response(
        &mut self,
        id: Option<String>,
        command: &str,
        request_id: &str,
        decision: PermissionDecision,
    ) -> Result<()> {
        let Some(reply) = self.pending_permissions.remove(request_id) else {
            return self.respond(RpcResponse::failure(
                id,
                command,
                "unknown_permission",
                format!("no pending permission request `{request_id}`"),
            ));
        };
        let applied = reply.send(decision).is_ok();
        self.respond(RpcResponse::success(
            id,
            command,
            json!({
                "request": request_id,
                "decision": decision.as_str(),
                "applied": applied,
            }),
        ))
    }

    /// Drops every pending request, which resolves it as a denial.
    fn deny_pending_permissions(&mut self) {
        for (_, reply) in self.pending_permissions.drain() {
            let _ = reply.send(PermissionDecision::Deny);
        }
    }

    fn get_catalog(&self, id: Option<String>, command: &str) -> Result<()> {
        let models = self
            .options
            .models
            .iter()
            .map(|model| {
                json!({
                    "id": model.id,
                    "name": model.name,
                    "provider": model.provider,
                    "api": model.api,
                    "context_window": model.context_window,
                    "max_output_tokens": model.max_output_tokens,
                    "reasoning": model.reasoning,
                    "cost": {
                        "input": model.cost.input,
                        "output": model.cost.output,
                        "cache_read": model.cost.cache_read,
                        "cache_write": model.cost.cache_write,
                    },
                })
            })
            .collect::<Vec<_>>();
        let providers = provider::all()
            .iter()
            .map(|spec| {
                json!({
                    "id": spec.id,
                    "name": spec.name,
                    "requires_key": spec.requires_key,
                    "env_var": spec.env_var,
                    "default_model": spec.default_model,
                })
            })
            .collect::<Vec<_>>();
        self.respond(RpcResponse::success(
            id,
            command,
            json!({
                "current": &self.model,
                "context_window": self.context_window,
                "models": models,
                "providers": providers,
            }),
        ))
    }

    fn get_themes(&self, id: Option<String>, command: &str) -> Result<()> {
        let themes = codie_tui::theme::BUILTIN_THEMES
            .iter()
            .filter(|name| **name != "system")
            .filter_map(|name| {
                let theme = codie_tui::theme::Theme::load(Some(name)).ok()?;
                Some(json!({
                    "name": theme.name,
                    "colors": theme.palette.to_hex(),
                }))
            })
            .collect::<Vec<_>>();
        self.respond(RpcResponse::success(
            id,
            command,
            json!({ "themes": themes }),
        ))
    }

    fn get_config(&self, id: Option<String>, command: &str) -> Result<()> {
        let config = &self.options.config;
        self.respond(RpcResponse::success(
            id,
            command,
            json!({
                "provider": &self.model.provider,
                "model": &self.model.model,
                "thinking": self.thinking.as_str(),
                "max_turns": self.options.max_turns,
                "allow": &self.options.allowed,
                "base_url": &config.base_url,
                "extensions_disabled": &config.extensions_disabled,
                "session_dir": config.session_dir.display().to_string(),
                "config_path": config.config_dir.join("config.toml").display().to_string(),
                "auth_path": config.auth_path().display().to_string(),
            }),
        ))
    }

    fn set_config(
        &mut self,
        id: Option<String>,
        command: &str,
        key: &str,
        value: &str,
    ) -> Result<()> {
        let thinking = if key == "thinking" {
            match ThinkingLevel::parse(value) {
                Some(level) => Some(level),
                None => {
                    return self.respond(RpcResponse::failure(
                        id,
                        command,
                        "invalid_argument",
                        format!("unknown thinking level `{value}`"),
                    ));
                }
            }
        } else {
            None
        };

        let path = self.options.config.config_dir.join("config.toml");
        if let Err(error) = crate::config_cmd::apply_value(&path, key, value) {
            return self.respond(RpcResponse::failure(
                id,
                command,
                "invalid_argument",
                error.to_string(),
            ));
        }

        let mut restart_required = false;
        match key {
            "thinking" => {
                if let Some(level) = thinking {
                    self.thinking = level;
                    self.options.thinking = level;
                    if let Some(runtime) = &self.subagent_runtime {
                        runtime.set_thinking(level);
                    }
                    if let Some(session) = self.session.as_mut() {
                        session.append_thinking_level_change(level)?;
                    }
                }
            }
            "max-turns" => match value.parse::<usize>() {
                Ok(turns) if turns > 0 => self.options.max_turns = turns,
                _ => {
                    return self.respond(RpcResponse::failure(
                        id,
                        command,
                        "invalid_argument",
                        format!("max-turns must be a positive integer, got `{value}`"),
                    ));
                }
            },
            "allow" => {
                let allowed = value
                    .split(',')
                    .map(str::trim)
                    .filter(|entry| !entry.is_empty())
                    .map(str::to_owned)
                    .collect::<Vec<_>>();
                self.gate.set_configured(allowed.clone());
                self.options.allowed = allowed;
            }
            "provider" | "model" | "base-url" | "extensions-disabled" => {
                restart_required = true;
            }
            _ => {}
        }

        self.respond(RpcResponse::success(
            id,
            command,
            json!({
                "key": key,
                "value": value,
                "restart_required": restart_required,
            }),
        ))
    }

    fn auth_status(&self, id: Option<String>, command: &str) -> Result<()> {
        let store = AuthStore::new(self.options.config.auth_path());
        let providers = provider::all()
            .iter()
            .map(|spec| {
                let status = auth_status(Some(&store), spec.id);
                json!({
                    "provider": spec.id,
                    "name": spec.name,
                    "configured": status.configured,
                    "usable": status.usable(),
                    "source": status.source.map(|source| source.as_str()),
                    "env_var": status.env_var,
                    "error": status.error,
                })
            })
            .collect::<Vec<_>>();
        self.respond(RpcResponse::success(
            id,
            command,
            json!({
                "providers": providers,
                "auth_path": store.path().display().to_string(),
            }),
        ))
    }

    async fn auth_set_key(
        &mut self,
        id: Option<String>,
        command: &str,
        provider: &str,
        key: &str,
        verify: bool,
    ) -> Result<()> {
        let Some(spec) = provider::find(provider) else {
            return self.respond(RpcResponse::failure(
                id,
                command,
                "unknown_provider",
                format!(
                    "unsupported provider `{provider}`; expected {}",
                    provider::ids().join(", ")
                ),
            ));
        };
        if !spec.requires_key {
            return self.respond(RpcResponse::failure(
                id,
                command,
                "no_key_required",
                format!("provider `{}` does not use an API key", spec.id),
            ));
        }
        let key = key.trim();
        if key.is_empty() {
            return self.respond(RpcResponse::failure(
                id,
                command,
                "invalid_argument",
                "API key must not be empty",
            ));
        }

        if verify {
            let outcome =
                codie_ai::probe_key(spec.id, key, self.options.config.base_url.as_deref()).await;
            if !outcome.accepted() {
                return self.respond(RpcResponse::failure(
                    id,
                    command,
                    "invalid_key",
                    outcome.summary(),
                ));
            }
        }

        let store = AuthStore::new(self.options.config.auth_path());
        if let Err(error) = store.set(spec.id, key) {
            return self.respond(RpcResponse::failure(
                id,
                command,
                "auth_failed",
                error.to_string(),
            ));
        }
        self.respond(RpcResponse::success(
            id,
            command,
            json!({
                "provider": spec.id,
                "configured": true,
                "verified": verify,
                "restart_required": spec.id == self.model.provider,
            }),
        ))
    }

    fn auth_logout(&mut self, id: Option<String>, command: &str, provider: &str) -> Result<()> {
        let Some(spec) = provider::find(provider) else {
            return self.respond(RpcResponse::failure(
                id,
                command,
                "unknown_provider",
                format!(
                    "unsupported provider `{provider}`; expected {}",
                    provider::ids().join(", ")
                ),
            ));
        };
        let store = AuthStore::new(self.options.config.auth_path());
        match store.remove(spec.id) {
            Ok(removed) => self.respond(RpcResponse::success(
                id,
                command,
                json!({
                    "provider": spec.id,
                    "removed": removed,
                    "restart_required": spec.id == self.model.provider,
                }),
            )),
            Err(error) => self.respond(RpcResponse::failure(
                id,
                command,
                "auth_failed",
                error.to_string(),
            )),
        }
    }

    /// Reports delegation configuration and every child run known to the host.
    fn get_subagents(&self, id: Option<String>, command: &str) -> Result<()> {
        self.respond(RpcResponse::success(id, command, self.subagents_status()))
    }

    fn subagents_status(&self) -> Value {
        let runs = self
            .subagent_runtime
            .as_ref()
            .map(|runtime| runtime.runs())
            .unwrap_or_default();
        let runs = runs
            .iter()
            .map(|run| {
                json!({
                    "call_id": run.call_id,
                    "agent": run.agent,
                    "description": run.description,
                    "depth": run.depth,
                    "session_id": run.session_id,
                    "turns": run.turns,
                    "activity": run.activity,
                    "state": run.state.as_str(),
                    "elapsed_ms": u64::try_from(run.elapsed.as_millis()).unwrap_or(u64::MAX),
                    "cost_usd": run.cost_nanos as f64 / 1_000_000_000.0,
                    "error": run.error,
                    "persisted": run.persisted,
                })
            })
            .collect::<Vec<_>>();
        let types = self
            .subagent_config
            .ordered_type_names()
            .into_iter()
            .filter_map(|name| {
                let agent = self.subagent_config.agent_type(&name)?;
                Some(json!({
                    "name": name,
                    "description": agent.description,
                    "model": agent.model,
                    "tools": match &agent.tools {
                        ToolSelection::Inherit => json!("inherit"),
                        ToolSelection::Only(tools) => json!(tools),
                    },
                    "max_turns": agent.max_turns,
                    "timeout_secs": agent.timeout.map(|timeout| timeout.as_secs()),
                }))
            })
            .collect::<Vec<_>>();
        let active = self.subagent_runtime.is_some();
        json!({
            "active": active,
            "enabled": self.subagent_config.enabled,
            "mode": self.subagent_mode.as_str(),
            "force_once": self.subagent_force_once,
            "max_parallel": self
                .subagent_runtime
                .as_ref()
                .map_or(self.subagent_config.max_parallel, |runtime| {
                    runtime.max_parallel()
                }),
            "max_depth": self.subagent_config.max_depth,
            "max_turns": self.subagent_config.max_turns,
            "timeout_secs": self.subagent_config.timeout.as_secs(),
            "result_max_bytes": self.subagent_config.result_max_bytes,
            "types": types,
            "runs": runs,
        })
    }

    fn set_subagents(
        &mut self,
        id: Option<String>,
        command: &str,
        mode: Option<RpcSubagentMode>,
        parallel: Option<usize>,
    ) -> Result<()> {
        let Some(runtime) = self.subagent_runtime.clone() else {
            return self.respond(RpcResponse::failure(
                id,
                command,
                "subagents_disabled",
                "Subagent delegation is disabled for this session. Enable it in settings and restart.",
            ));
        };

        if let Some(limit) = parallel {
            if !runtime.set_max_parallel(limit) {
                return self.respond(RpcResponse::failure(
                    id,
                    command,
                    "subagents_busy",
                    "Stop queued or running subagents before changing parallelism.",
                ));
            }
            self.subagent_config.max_parallel = limit;
        }

        if let Some(mode) = mode {
            match mode {
                RpcSubagentMode::Once => {
                    self.subagent_force_once = true;
                    runtime.set_mode(DelegationMode::Force);
                }
                RpcSubagentMode::Off | RpcSubagentMode::Auto | RpcSubagentMode::Force => {
                    let next = match mode {
                        RpcSubagentMode::Off => DelegationMode::Off,
                        RpcSubagentMode::Force => DelegationMode::Force,
                        _ => DelegationMode::Auto,
                    };
                    self.subagent_mode = next;
                    self.subagent_force_once = false;
                    runtime.set_mode(next);
                    self.record_subagent_mode(next)?;
                }
            }
        }

        self.respond(RpcResponse::success(id, command, self.subagents_status()))
    }

    /// Persists a session mode change the same way the interactive interface
    /// does, so a replay sees the instruction the model received.
    fn record_subagent_mode(&mut self, mode: DelegationMode) -> Result<()> {
        let directive = Message::system(mode_directive(mode), Vec::new());
        if let Some(session) = self.session.as_mut() {
            session.append_custom(
                "subagent_settings",
                json!({"mode": mode.as_str(), "force_once": false}),
            )?;
            session.append_message(&directive)?;
        }
        self.history.push(directive);
        Ok(())
    }

    fn cancel_subagent(&mut self, id: Option<String>, command: &str, call_id: &str) -> Result<()> {
        let Some(runtime) = self.subagent_runtime.as_ref() else {
            return self.respond(RpcResponse::failure(
                id,
                command,
                "subagents_disabled",
                "Subagent delegation is disabled for this session.",
            ));
        };
        if runtime.cancel_run(call_id) {
            self.respond(RpcResponse::success(
                id,
                command,
                json!({"call_id": call_id, "cancelled": true}),
            ))
        } else {
            self.respond(RpcResponse::failure(
                id,
                command,
                "unknown_subagent",
                format!("no queued or running subagent for call `{call_id}`"),
            ))
        }
    }

    fn get_state(&self, id: Option<String>, command: &str) -> Result<()> {
        let mut total_tokens = 0_u64;
        let mut cost = 0_u64;
        for message in &self.history {
            if let Message::Assistant(assistant) = message {
                total_tokens = total_tokens.saturating_add(assistant.usage.total_tokens);
                cost = cost.saturating_add(assistant.usage.cost.total);
            }
        }
        // Delegated work is recorded as usage entries, not assistant messages.
        // Include it so the desktop's totals (and `/usage`) stay truthful.
        if let Some(session) = &self.session {
            for entry in session.entries() {
                if let Entry::Usage { usage, .. } = entry {
                    total_tokens = total_tokens.saturating_add(usage.total_tokens);
                    cost = cost.saturating_add(usage.cost.total);
                }
            }
        }
        self.respond(RpcResponse::success(
            id,
            command,
            json!({
                "session": self.session.as_ref().map(Session::id),
                "persisted": self.session.as_ref().is_some_and(Session::is_persisted),
                "cwd": self.options.cwd.display().to_string(),
                "model": &self.model,
                "context_window": self.context_window,
                "thinking": self.thinking.as_str(),
                "active": self.active.is_some(),
                "messages": self.history.len(),
                "estimated_tokens": self.estimated_tokens(),
                "context_percent": self.context_percent(),
                "total_tokens": total_tokens,
                "cost": cost,
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
        self.deny_pending_permissions();
        let mut session = Session::create(&self.options.cwd, &self.options.session_dir)?;
        let system = self.system_message();
        session.append_message(&system)?;
        self.history = vec![system];
        let session_id = session.id().to_owned();
        self.session = Some(session);
        if let Some(runtime) = &self.subagent_runtime {
            runtime.set_parent_session_id(Some(session_id.clone()));
        }
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
        if let Some(runtime) = &self.subagent_runtime {
            runtime.set_parent_model(
                self.model.clone(),
                self.options
                    .models
                    .iter()
                    .map(|model| model.id.clone())
                    .collect(),
            );
        }
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
        if let Some(runtime) = &self.subagent_runtime {
            runtime.set_thinking(thinking);
        }
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
        if let AgentEvent::MessageEnd { message } = &event {
            if let Some(session) = self.session.as_mut() {
                session.append_message(message)?;
            }
            // Child usage is not in the parent's assistant messages; record it
            // in the session ledger so cost and token totals include it.
            if let Message::ToolResult(result) = message
                && result.tool_name == "task"
                && let Some(usage) = result.details.as_ref().and_then(subagent_usage)
                && let Some(session) = self.session.as_mut()
                && let Err(error) = session.append_usage(
                    "subagent",
                    self.model.provider.clone(),
                    self.model.model.clone(),
                    usage,
                )
            {
                eprintln!("warning: cannot record subagent usage: {error}");
            }
        }

        match &event {
            AgentEvent::AgentEnd { messages, status } => {
                self.history = messages.clone();
                let aborted = *status == RunStatus::Aborted;
                self.emit(&event)?;
                self.active = None;
                self.deny_pending_permissions();
                self.emit(&RpcSettled::new(aborted))?;
            }
            _ => self.emit(&event)?,
        }
        Ok(())
    }

    fn finish_run(&mut self, status: RunStatus) -> Result<()> {
        if self.active.take().is_some() {
            self.deny_pending_permissions();
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
        self.render_system_message(self.toolbox.declarations())
    }

    fn render_system_message(&self, declarations: Vec<ToolDecl>) -> Message {
        let input = PromptInput {
            cwd: self.options.cwd.clone(),
            tools: declarations.clone(),
            preamble: self.options.preamble.clone(),
            append: self.options.append.clone(),
            skills: self.options.customizations.skills().to_vec(),
            project_context: load_project_context(&self.options.cwd),
            project_memory: load_project_memory(&self.options.cwd),
        };
        build_system_message(&input, declarations)
    }

    /// Ensures the model-visible system message declares the current toolbox.
    ///
    /// Fresh sessions persist the message. A resumed session whose persisted
    /// prompt pre-dates a tool-set change is repaired in memory, because the
    /// session format is append-only and the model must still see `task`.
    fn ensure_system_message(&mut self) -> Result<()> {
        let system = self.system_message();
        let stale = match (self.history.first(), &system) {
            (Some(Message::System(existing)), Message::System(next)) => {
                existing.tools != next.tools || existing.content != next.content
            }
            _ => true,
        };
        if !stale {
            return Ok(());
        }

        if self.history.is_empty() {
            if let Some(session) = self.session.as_mut() {
                session.append_message(&system)?;
            }
            self.history.insert(0, system);
            return Ok(());
        }
        if let Some(Message::System(existing)) = self.history.first_mut() {
            if let Message::System(next) = system {
                *existing = next;
            }
        } else {
            self.history.insert(0, system);
        }
        Ok(())
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
    let nanos = Cost::nanos_from_usd(
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
        cost: Cost {
            total: nanos,
            total_from_provider: true,
            ..Cost::default()
        },
    })
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
