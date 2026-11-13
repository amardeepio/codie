//! Append-only plain mode (`TUI-010`).
//!
//! No alternate screen, no cursor movement, no raw mode. Prompts and output are
//! line-oriented so screen readers and log captures see a stable transcript.

use std::io::Write;
use std::sync::Arc;

use anyhow::Result;
use codie_core::{Agent, AgentConfig, AgentEvent, Content, Message, RunStatus, Toolbox};
use codie_prompt::{PromptInput, build_system_message, load_project_context};
use codie_session::Session;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::app::TuiOptions;
use crate::commands::{Command, parse as parse_command};
use crate::permissions::{Decision, PermissionBroker, PermissionRequest, PermissionToolbox};

/// Runs the append-only interface and returns the process exit code.
pub(crate) async fn run(mut options: TuiOptions) -> Result<u8> {
    let registry = Arc::clone(&options.registry);
    let (permission_tx, mut permission_rx) = mpsc::unbounded_channel();
    let broker = PermissionBroker::new(permission_tx, options.auto_allowed.clone());
    let toolbox: Arc<dyn Toolbox> = Arc::new(PermissionToolbox::new(registry.clone(), broker));
    let agent = Arc::new(match options.extensions.hooks() {
        Some(hooks) => Agent::with_hooks(Arc::clone(&options.client), toolbox.clone(), hooks),
        None => Agent::new(Arc::clone(&options.client), toolbox.clone()),
    });

    let mut history = options.history.clone();
    let mut last_assistant = String::new();
    let mut session = options.session.take();
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
        if let Some(session) = session.as_mut() {
            session.append_message(&system)?;
        }
        history.insert(0, system);
    }

    println!(
        "codie {} · {} · plain mode. /help for commands.",
        env!("CARGO_PKG_VERSION"),
        options.model.model
    );

    let mut session_info = codie_protocol::ext::ExtSessionInfo {
        id: session.as_ref().map(|session| session.id().to_owned()),
        cwd: options.cwd.display().to_string(),
        model: Some(options.model.model.clone()),
    };
    options.extensions.set_session_id(session_info.id.clone());
    options.extensions.session_start(&session_info).await;

    let cancel = CancellationToken::new();
    let signal = cancel.clone();
    #[cfg(unix)]
    tokio::spawn(async move {
        let mut term =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).ok();
        tokio::select! {
            _ = tokio::signal::ctrl_c() => signal.cancel(),
            _ = async {
                match term.as_mut() {
                    Some(stream) => { stream.recv().await; }
                    None => std::future::pending::<()>().await,
                }
            } => signal.cancel(),
        }
    });

    if let Some(prompt) = options.initial_prompt.take() {
        run_prompt(
            &agent,
            &mut history,
            &mut session,
            &options,
            &cancel,
            &mut permission_rx,
            &mut last_assistant,
            prompt,
        )
        .await?;
    }

    loop {
        if cancel.is_cancelled() {
            return Ok(130);
        }
        print!("> ");
        std::io::stdout().flush()?;
        let Some(line) = read_line().await else {
            break;
        };
        let line = line.trim().to_owned();
        if line.is_empty() {
            continue;
        }
        if let Some(command) = parse_command(&line) {
            if matches!(command, Command::Unknown(_)) {
                match handle_extension_command(&options, &session, &line).await {
                    Ok(Some(prompt)) => {
                        run_prompt(
                            &agent,
                            &mut history,
                            &mut session,
                            &options,
                            &cancel,
                            &mut permission_rx,
                            &mut last_assistant,
                            prompt,
                        )
                        .await?;
                        continue;
                    }
                    Ok(None) if line.trim_start().starts_with("/ext:") => continue,
                    Ok(None) => {}
                    Err(error) => {
                        println!("{error}");
                        continue;
                    }
                }
            }
            match command {
                Command::Quit => break,
                Command::Help | Command::Hotkeys => {
                    println!(
                        "/help /quit /new /session /tree /fork /clone /name /export [path] /export html [path] /copy /debug /update /theme /provider [id] /model [id] /thinking [level] /compact /skill:<name> /template:<name> /reload"
                    )
                }
                Command::Theme(argument) => match argument {
                    Some(name) => println!("Theme: {name} (full mode only)"),
                    None => println!("Themes: system, dark, light"),
                },
                Command::Provider(argument) => match argument {
                    Some(provider) => {
                        let Some(switch) = options.switch_provider.clone() else {
                            println!("Provider switching is unavailable in this mode.");
                            continue;
                        };
                        match switch(&provider) {
                            Ok(runtime) => {
                                options.model.provider = provider.clone();
                                options.model.model = runtime.default_model.clone();
                                options.models = runtime.models;
                                if let Some(session) = session.as_mut() {
                                    session.append_model_change(
                                        provider.clone(),
                                        runtime.default_model.clone(),
                                    )?;
                                }
                                session_info.model = Some(runtime.default_model.clone());
                                println!("Provider: {provider} · model: {}", runtime.default_model);
                            }
                            Err(error) => println!("Cannot switch provider: {error}"),
                        }
                    }
                    None => {
                        let current = &options.model.provider;
                        for provider in &options.providers {
                            let marker = if &provider.id == current { "*" } else { " " };
                            println!("{marker} {} ({})", provider.name, provider.id);
                        }
                    }
                },
                Command::Model(argument) => {
                    if let Some(model) = argument {
                        options.model.model = model;
                    }
                    println!("Model: {}", options.model.model);
                }
                Command::Thinking(argument) => {
                    if let Some(value) = argument {
                        if let Some(level) = crate::commands::parse_thinking(&value) {
                            options.thinking = level;
                        }
                    }
                    println!("Thinking: {}", options.thinking.as_str());
                }
                Command::Session => {
                    println!(
                        "{}",
                        session.as_ref().map_or_else(
                            || "no session".to_owned(),
                            |session| session.id().to_owned()
                        )
                    );
                }
                Command::Tree => match session.as_ref() {
                    Some(session) => {
                        for entry in session.tree() {
                            let marker = if entry.on_branch { "●" } else { "○" };
                            println!(
                                "{}{marker} {} {:<16} {}",
                                "  ".repeat(entry.depth.min(8)),
                                entry.id,
                                entry.kind,
                                entry.description
                            );
                        }
                    }
                    None => println!("no session"),
                },
                Command::Fork(argument) => {
                    if let Some(session) = session.as_mut() {
                        if let Some(entry) = argument {
                            match session.set_leaf(Some(&entry)) {
                                Ok(()) => {
                                    history = session.projection();
                                    println!("Forked at entry {entry}.");
                                }
                                Err(error) => println!("Cannot fork: {error}"),
                            }
                        } else {
                            println!("Use /fork <entry-id>; /tree lists entries.");
                        }
                    } else {
                        println!("no session");
                    }
                }
                Command::Clone => match session.as_ref() {
                    Some(current) => {
                        match current.clone_session(&options.cwd, &options.session_dir) {
                            Ok(cloned) => {
                                history = cloned.projection();
                                println!("Cloned session {}.", cloned.id());
                                session = Some(cloned);
                            }
                            Err(error) => println!("Cannot clone: {error}"),
                        }
                    }
                    None => println!("no session"),
                },
                Command::Name(argument) => match session.as_mut() {
                    Some(session) => {
                        let result = match argument {
                            Some(name) => session.set_name(name),
                            None => session.clear_name(),
                        };
                        match result {
                            Ok(()) => println!("Session name updated."),
                            Err(error) => println!("Cannot name the session: {error}"),
                        }
                    }
                    None => println!("no session"),
                },
                Command::Export(argument) => match session.as_ref() {
                    Some(session) => {
                        let path = argument.map_or_else(
                            || {
                                options
                                    .cwd
                                    .join(format!("codie-session-{}.jsonl", session.id()))
                            },
                            std::path::PathBuf::from,
                        );
                        match std::fs::File::create(&path)
                            .map_err(|error| codie_core::Error::io(&path, error))
                            .and_then(|file| session.export(file))
                        {
                            Ok(()) => println!("Exported to {}", path.display()),
                            Err(error) => println!("Cannot export: {error}"),
                        }
                    }
                    None => println!("no session"),
                },
                Command::ExportHtml(argument) => match session.as_ref() {
                    Some(current) => {
                        let path = argument.map_or_else(
                            || {
                                options
                                    .cwd
                                    .join(format!("codie-session-{}.html", current.id()))
                            },
                            std::path::PathBuf::from,
                        );
                        let result = std::fs::File::create(&path)
                            .map_err(|error| codie_core::Error::io(&path, error))
                            .and_then(|file| current.export_html(file));
                        match result {
                            Ok(()) => println!("Exported HTML to {}.", path.display()),
                            Err(error) => println!("Cannot export HTML: {error}"),
                        }
                    }
                    None => println!("No session is being persisted."),
                },
                Command::Copy => {
                    if last_assistant.is_empty() {
                        println!("No assistant message to copy.");
                    } else {
                        match crate::copy_to_clipboard(&last_assistant) {
                            Ok(()) => println!("Copied the last message to the clipboard."),
                            Err(error) => println!("Cannot copy to the clipboard: {error}"),
                        }
                    }
                }
                Command::Debug => {
                    let session_id = session
                        .as_ref()
                        .map_or_else(|| "ephemeral".to_owned(), |current| current.id().to_owned());
                    let entries = session
                        .as_ref()
                        .map_or(0, |current| current.entries().len());
                    let tokens = session.as_ref().map_or_else(
                        || {
                            history
                                .iter()
                                .map(|message| match message {
                                    Message::System(message) => {
                                        message.content.chars().count() as u64
                                    }
                                    Message::User(message) => message
                                        .content
                                        .iter()
                                        .filter_map(Content::as_text)
                                        .map(|text| text.chars().count() as u64)
                                        .sum(),
                                    Message::Assistant(message) => {
                                        message.text().chars().count() as u64
                                    }
                                    Message::ToolResult(message) => message
                                        .content
                                        .iter()
                                        .filter_map(Content::as_text)
                                        .map(|text| text.chars().count() as u64)
                                        .sum(),
                                })
                                .sum::<u64>()
                                / 4
                        },
                        codie_session::Session::estimate_tokens,
                    );
                    println!(
                        "codie {} · {} · {} · cwd {} · session {session_id} · {} messages · {entries} entries · ~{tokens} tok",
                        env!("CARGO_PKG_VERSION"),
                        options.model.model,
                        options.thinking.as_str(),
                        options.cwd.display(),
                        history.len(),
                    );
                }
                Command::Update => match codie_ai::update::check(None).await {
                    Ok(status) if status.newer() => {
                        println!(
                            "Update available: codie {} → {}.",
                            status.current, status.latest
                        );
                        if status.installable == Some(false) {
                            println!(
                                "No prebuilt binary is published for this platform yet; reinstall with: {}",
                                codie_ai::update::INSTALLER_COMMAND
                            );
                        } else {
                            println!("Installing…");
                            match codie_ai::update::install(&status.latest).await {
                                Ok(()) => println!(
                                    "Installed codie {}. Restart codie to use the new version.",
                                    status.latest
                                ),
                                Err(error) => println!("Update failed: {error}"),
                            }
                        }
                    }
                    Ok(status) => println!(
                        "codie {} — up to date (latest {}).",
                        status.current, status.latest
                    ),
                    Err(error) => println!("Update check failed: {error}"),
                },
                Command::New => {
                    options.extensions.session_shutdown("new session").await;
                    session = Some(Session::create(&options.cwd, &options.session_dir)?);
                    history.clear();
                    session_info.id = session.as_ref().map(|session| session.id().to_owned());
                    options.extensions.set_session_id(session_info.id.clone());
                    options.extensions.session_start(&session_info).await;
                    println!("Started a new session.");
                }
                Command::Compact(instructions) => {
                    if let Some(current) = session.as_ref() {
                        let custom = instructions.as_deref();
                        match crate::compaction::summarize(
                            Arc::clone(&options.client),
                            options.model.clone(),
                            options.thinking,
                            &history,
                            custom,
                            Some(current.id().to_owned()),
                            u64::from(options.context_window)
                                .saturating_sub(crate::compaction::DEFAULT_RESERVE_TOKENS)
                                .clamp(4_000, 200_000),
                        )
                        .await
                        {
                            Ok(outcome) => {
                                if let Some(current) = session.as_mut() {
                                    match crate::compaction::apply(current, &outcome, custom) {
                                        Ok(projected) => {
                                            history = projected;
                                            println!("Compacted the context.");
                                        }
                                        Err(error) => println!("Cannot compact: {error}"),
                                    }
                                }
                            }
                            Err(error) => println!("Compaction failed: {error}"),
                        }
                    } else {
                        println!("no session");
                    }
                }
                Command::Skill(name, arguments) => match options.customizations.skill(&name) {
                    Some(skill) => {
                        let prompt = skill.prompt(&arguments);
                        run_prompt(
                            &agent,
                            &mut history,
                            &mut session,
                            &options,
                            &cancel,
                            &mut permission_rx,
                            &mut last_assistant,
                            prompt,
                        )
                        .await?;
                    }
                    None => println!("Unknown skill `{name}`."),
                },
                Command::Template(name, arguments) => {
                    match options.customizations.template(&name) {
                        Some(template) => {
                            let prompt = template.expand(&arguments);
                            run_prompt(
                                &agent,
                                &mut history,
                                &mut session,
                                &options,
                                &cancel,
                                &mut permission_rx,
                                &mut last_assistant,
                                prompt,
                            )
                            .await?;
                        }
                        None => println!("Unknown template `{name}`."),
                    }
                }
                Command::Reload => {
                    options.customizations.refresh();
                    println!(
                        "Reloaded {} skill(s) and {} template(s).",
                        options.customizations.skills().len(),
                        options.customizations.templates().len()
                    );
                }
                Command::Unknown(name) => println!("Unknown command `/{name}`."),
            }
            continue;
        }

        let submitted = options.extensions.user_submit(&session_info, &line).await;
        if submitted.block {
            println!(
                "{}",
                submitted
                    .reason
                    .unwrap_or_else(|| "prompt blocked by an extension".to_owned())
            );
            continue;
        }
        let line = submitted.prompt.unwrap_or(line);
        run_prompt(
            &agent,
            &mut history,
            &mut session,
            &options,
            &cancel,
            &mut permission_rx,
            &mut last_assistant,
            line,
        )
        .await?;
    }

    options.extensions.shutdown("plain exit").await;
    Ok(if cancel.is_cancelled() { 130 } else { 0 })
}

async fn handle_extension_command(
    options: &TuiOptions,
    session: &Option<Session>,
    text: &str,
) -> Result<Option<String>> {
    let Some(invocation) = options
        .extensions
        .resolve_command(text)
        .map_err(anyhow::Error::msg)?
    else {
        return Ok(None);
    };
    let session_info = codie_protocol::ext::ExtSessionInfo {
        id: session.as_ref().map(|session| session.id().to_owned()),
        cwd: options.cwd.display().to_string(),
        model: Some(options.model.model.clone()),
    };
    match options
        .extensions
        .invoke_command(&invocation, &session_info)
        .await
    {
        Ok(result) => {
            if let Some(prompt) = result.prompt {
                Ok(Some(prompt))
            } else {
                if let Some(display) = result.display {
                    if let Some(title) = display.title {
                        println!("{title}");
                    }
                    println!("{}", display.text);
                } else {
                    println!("Extension command handled.");
                }
                Ok(None)
            }
        }
        Err(error) => Err(anyhow::anyhow!(error.to_string())),
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_prompt(
    agent: &Arc<Agent>,
    history: &mut Vec<Message>,
    session: &mut Option<Session>,
    options: &TuiOptions,
    cancel: &CancellationToken,
    permission_rx: &mut mpsc::UnboundedReceiver<PermissionRequest>,
    last_assistant: &mut String,
    prompt: String,
) -> Result<()> {
    let mut config =
        AgentConfig::new(options.model.clone(), options.cwd.clone()).map_err(anyhow::Error::msg)?;
    config.thinking = options.thinking;
    config.max_turns = options.max_turns;
    config.session_id = session.as_ref().map(|session| session.id().to_owned());
    config.cancel = cancel.clone();

    println!("\n› {prompt}");
    let expansion = codie_prompt::expand_attachments(&prompt, &options.cwd);
    for error in &expansion.errors {
        println!("warning: {error}");
    }
    let mut stream = agent.run(
        Message::user_with_content(expansion.content),
        history.clone(),
        config,
    );
    let mut printed = 0_usize;
    let mut in_text = false;

    loop {
        tokio::select! {
            Some(request) = permission_rx.recv() => {
                resolve_permission(request).await;
            }
            event = stream.recv() => {
                let Some(event) = event else { break };
                match event {
                    AgentEvent::MessageStart { message } => {
                        if matches!(message, Message::Assistant(_)) {
                            printed = 0;
                            in_text = true;
                        }
                    }
                    AgentEvent::MessageUpdate {
                        message: Message::Assistant(assistant),
                        ..
                    } => {
                        let text = assistant.text();
                        if text.len() > printed {
                            print!("{}", &text[printed..]);
                            let _ = std::io::stdout().flush();
                            printed = text.len();
                        }
                    }
                    AgentEvent::MessageUpdate { .. } => {}
                    AgentEvent::MessageEnd { message } => {
                        if let Message::Assistant(assistant) = &message {
                            let text = assistant.text();
                            if text.len() > printed {
                                print!("{}", &text[printed..]);
                            }
                            *last_assistant = text.clone();
                            if in_text {
                                println!();
                                in_text = false;
                            }
                        } else if let Message::ToolResult(result) = &message {
                            if result.is_error {
                                let text = result
                                    .content
                                    .iter()
                                    .filter_map(Content::as_text)
                                    .collect::<Vec<_>>()
                                    .join("\n");
                                println!("! {text}");
                            }
                        }
                        if let Some(session) = session.as_mut() {
                            session.append_message(&message)?;
                        }
                    }
                    AgentEvent::ToolExecutionStart { tool_name, args, .. } => {
                        let summary = args
                            .get("path")
                            .or_else(|| args.get("command"))
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or_default();
                        println!("→ {tool_name} {summary}");
                    }
                    AgentEvent::ToolExecutionEnd { tool_name, is_error, result, .. } => {
                        if is_error {
                            let text = result
                                .content
                                .iter()
                                .filter_map(Content::as_text)
                                .collect::<Vec<_>>()
                                .join("\n");
                            println!("✗ {tool_name}: {text}");
                        } else {
                            println!("✓ {tool_name}");
                        }
                    }
                    AgentEvent::AgentEnd { messages, status } => {
                        *history = messages;
                        if status != RunStatus::Completed {
                            println!("run ended with status: {status:?}");
                        }
                        break;
                    }
                    _ => {}
                }
            }
        }
        if cancel.is_cancelled() {
            break;
        }
    }
    Ok(())
}

async fn resolve_permission(request: PermissionRequest) {
    println!(
        "\npermission: allow `{}`?\n{}\n[y] yes  [n] no  [A] always allow",
        request.tool,
        serde_json::to_string_pretty(&request.arguments).unwrap_or_default()
    );
    let decision = tokio::task::spawn_blocking(|| {
        let mut line = String::new();
        let _ = std::io::stdin().read_line(&mut line);
        match line.trim() {
            "y" | "Y" | "a" => Decision::Allow,
            "A" => Decision::Always,
            _ => Decision::Deny,
        }
    })
    .await
    .unwrap_or(Decision::Deny);
    let _ = request.reply.send(decision);
}

async fn read_line() -> Option<String> {
    tokio::task::spawn_blocking(|| {
        let mut line = String::new();
        match std::io::stdin().read_line(&mut line) {
            Ok(0) => None,
            Ok(_) => Some(line),
            Err(_) => None,
        }
    })
    .await
    .ok()
    .flatten()
}
