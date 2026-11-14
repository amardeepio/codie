//! The `codie` run path: resolve config, stream a turn, persist the session.

use std::collections::{BTreeMap, HashSet};
use std::io::{IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};

use anyhow::{Context, Result, bail};
use clap::{Args, ValueEnum};
use codie_ai::{
    AuthStore, Model, OpenAiProvider, OpenRouterCatalog, PROVIDER_OPENROUTER, builtin_models,
    fallback_model, merge_models, provider_base_url, resolve_key,
};
use codie_core::{
    Agent, AgentConfig, AgentEvent, ChatClient, Message, ModelRef, RunStatus, StopReason,
    ThinkingLevel, Tool, Toolbox,
};
use codie_ext::{ExtensionSet, LoadOptions};
use codie_prompt::{
    Customizations, PromptInput, TrustStore, build_system_message, expand_attachments,
    load_instructions, load_project_context,
};
use codie_protocol::ext::ExtSessionInfo;
use codie_session::Session;
use codie_tools::{
    BashTool, EditTool, FindTool, GrepTool, LsTool, Policy, PolicyToolbox, PowerShellTool,
    ReadTool, Registry, ToolName, WebFetchTool, WriteTool,
};
use codie_tui::app::{ProviderChoice, ProviderRuntime};
use codie_tui::{TuiOptions, TuiRunMode};
use tokio_util::sync::CancellationToken;

use crate::auth_cmd::env_var;
use crate::config::Config;

/// Interactive rendering mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum TuiModeArg {
    /// Alternate screen with a fixed editor dock.
    Full,
    /// Normal scrollback with a fixed editor dock.
    Inline,
    /// Append-only output for screen readers.
    Plain,
}

impl From<TuiModeArg> for TuiRunMode {
    fn from(value: TuiModeArg) -> Self {
        match value {
            TuiModeArg::Full => Self::Full,
            TuiModeArg::Inline => Self::Inline,
            TuiModeArg::Plain => Self::Plain,
        }
    }
}

/// Output interface for a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum OutputMode {
    /// Print the final assistant text.
    Text,
    /// Emit a JSONL event stream.
    Json,
    /// Serve line-delimited JSON commands for editor integrations.
    Rpc,
}

/// Arguments for a one-shot run.
#[derive(Debug, Clone, Args)]
pub(crate) struct RunArgs {
    /// One-shot mode. Required when stdin or stdout is a TTY.
    #[arg(short = 'p', long)]
    pub(crate) print: bool,

    /// Output interface.
    #[arg(long, value_enum, default_value = "text")]
    pub(crate) mode: OutputMode,

    /// Interactive rendering mode: full, inline, or plain.
    #[arg(long, value_enum, default_value = "full")]
    pub(crate) tui_mode: TuiModeArg,

    /// Theme name (`system`, `dark`, `light`) or a TOML theme path.
    #[arg(long, value_name = "NAME|PATH")]
    pub(crate) theme: Option<String>,

    /// Provider id (`deepseek` or `openrouter`).
    #[arg(long)]
    pub(crate) provider: Option<String>,

    /// Model id, used verbatim.
    #[arg(long)]
    pub(crate) model: Option<String>,

    /// Ephemeral API key. Never persisted.
    #[arg(long)]
    pub(crate) api_key: Option<String>,

    /// Provider base URL override (proxy or local test endpoint).
    #[arg(long, value_name = "URL")]
    pub(crate) base_url: Option<String>,

    /// Thinking level: off, minimal, low, medium, high, xhigh, max.
    #[arg(long)]
    pub(crate) thinking: Option<String>,

    /// Comma-separated tool allowlist.
    #[arg(long)]
    pub(crate) tools: Option<String>,

    /// Disable every tool.
    #[arg(long)]
    pub(crate) no_tools: bool,

    /// Load an extension manifest or directory. Repeatable.
    #[arg(long = "extension", value_name = "PATH", action = clap::ArgAction::Append)]
    pub(crate) extension: Vec<PathBuf>,

    /// Disable every extension.
    #[arg(long)]
    pub(crate) no_extensions: bool,

    /// Comma-separated mutating tools permitted in headless mode.
    #[arg(long)]
    pub(crate) allow: Option<String>,

    /// Permit every mutating tool for this run.
    #[arg(long)]
    pub(crate) allow_all: bool,

    /// Trust this project and record it in `trust.toml`.
    #[arg(long)]
    pub(crate) approve: bool,

    /// Replace the system preamble (overrides `SYSTEM.md`).
    #[arg(long, value_name = "TEXT")]
    pub(crate) system_prompt: Option<String>,

    /// Append to the system preamble (after `APPEND_SYSTEM.md`).
    #[arg(long, value_name = "TEXT")]
    pub(crate) append_system_prompt: Option<String>,

    /// Maximum assistant requests in this run.
    #[arg(long)]
    pub(crate) max_turns: Option<usize>,

    /// Working directory for tools and project context.
    #[arg(long)]
    pub(crate) cwd: Option<PathBuf>,

    /// Open a specific session by file path or identifier.
    #[arg(long, value_name = "ID|PATH")]
    pub(crate) session: Option<String>,

    /// Resume the most recent session for this directory.
    #[arg(short = 'c', long = "continue", conflicts_with_all = ["resume", "fork", "clone", "no_session"])]
    pub(crate) continue_session: bool,

    /// Pick a session to resume. With an identifier, resume that session.
    #[arg(
        short = 'r',
        long,
        value_name = "ID",
        num_args = 0..=1,
        conflicts_with_all = ["continue_session", "fork", "clone", "session", "no_session"]
    )]
    pub(crate) resume: Option<Option<String>>,

    /// Fork a session's current branch into a new session in this directory.
    #[arg(long, value_name = "ID|PATH", conflicts_with_all = ["continue_session", "clone", "session", "no_session"])]
    pub(crate) fork: Option<String>,

    /// Clone every entry of a session into a new session in this directory.
    #[arg(long, value_name = "ID|PATH", conflicts_with_all = ["continue_session", "fork", "session", "no_session"])]
    pub(crate) clone: Option<String>,

    /// Do not persist this run.
    #[arg(long, conflicts_with_all = ["continue_session", "resume", "fork", "clone", "session"])]
    pub(crate) no_session: bool,

    /// Message text. When omitted and stdin is piped, stdin is used.
    #[arg(value_name = "MESSAGE", trailing_var_arg = true)]
    pub(crate) message: Vec<String>,
}

/// Runs the agent once and returns the process exit code.
pub(crate) async fn run(args: RunArgs, deferred_flags: BTreeMap<String, String>) -> Result<u8> {
    if args.api_key.is_some() && args.model.is_none() {
        bail!("--api-key requires an explicit --model so the key cannot be paired with a default");
    }

    let config = Config::load()?;
    let cwd = match args.cwd.clone() {
        Some(cwd) => cwd,
        None => std::env::current_dir().context("cannot determine the working directory")?,
    };
    let cwd = cwd.canonicalize().unwrap_or(cwd);

    // Project trust decides whether `.codie/` resources are visible. The prompt
    // runs before any terminal setup, so it cannot corrupt the TUI state.
    let mut trust = TrustStore::load(&config.config_dir);
    if args.approve {
        trust
            .approve(&cwd)
            .with_context(|| format!("cannot record trust for {}", cwd.display()))?;
    }
    let can_prompt = std::io::stdin().is_terminal()
        && std::io::stdout().is_terminal()
        && !args.print
        && !matches!(args.mode, OutputMode::Json | OutputMode::Rpc);
    let project_root = if trust.is_trusted(&cwd) {
        Some(cwd.clone())
    } else if TrustStore::has_project_resources(&cwd) && can_prompt && prompt_trust(&cwd)? {
        trust
            .approve(&cwd)
            .with_context(|| format!("cannot record trust for {}", cwd.display()))?;
        Some(cwd.clone())
    } else {
        if TrustStore::has_project_resources(&cwd) {
            eprintln!(
                "warning: ignoring .codie resources in {} (run with --approve to trust this project)",
                cwd.display()
            );
        }
        None
    };

    let instructions = load_instructions(&cwd);
    let preamble = instructions.preamble(args.system_prompt.as_deref());
    let append = instructions.combined_append(args.append_system_prompt.as_deref());
    let customizations = Customizations::load(&config.config_dir, project_root.as_deref());
    for warning in customizations.warnings() {
        eprintln!("warning: {warning}");
    }
    if let Some(theme) = args.theme.as_deref() {
        // Declarative resources are validated before any extension process starts.
        let _ = codie_tui::Theme::load(Some(theme))?;
    }

    let extensions = if args.no_extensions {
        ExtensionSet::default()
    } else {
        ExtensionSet::load(LoadOptions {
            config_dir: &config.config_dir,
            project_root: project_root.as_deref(),
            explicit: &args.extension,
            disabled: &config.extensions_disabled,
            cwd: &cwd,
            session_id: None,
            deferred_flags: &deferred_flags,
        })
        .await
    };
    for warning in extensions.warnings() {
        eprintln!("warning: {warning}");
    }
    extensions
        .validate_flags(&deferred_flags)
        .map_err(anyhow::Error::msg)?;
    let extensions = Arc::new(extensions);

    let provider_id = args
        .provider
        .clone()
        .unwrap_or_else(|| config.default_provider.clone());
    validate_provider(&provider_id)?;

    let thinking = match args.thinking.as_deref() {
        Some(value) => ThinkingLevel::parse(value)
            .with_context(|| format!("invalid thinking level `{value}`"))?,
        None => config.thinking,
    };

    let store = AuthStore::new(config.auth_path());
    let key = resolve_key(
        args.api_key.as_deref(),
        &store,
        &provider_id,
        env_var(&provider_id),
    )
    .with_context(|| {
        format!(
            "no API key for {provider_id}; run `codie auth login {provider_id}` or set {}",
            env_var(&provider_id)
        )
    })?;

    let mut provider = match provider_id.as_str() {
        PROVIDER_OPENROUTER => OpenAiProvider::openrouter(key),
        _ => OpenAiProvider::deepseek(key),
    };
    if let Some(base_url) = args.base_url.clone().or_else(|| config.base_url.clone()) {
        provider = provider.with_base_url(base_url);
    }

    let catalog = (provider_id == PROVIDER_OPENROUTER).then(|| {
        Arc::new(
            OpenRouterCatalog::new(config.cache_dir.join("openrouter-models.json")).with_endpoint(
                format!("{}/models", provider.base_url().trim_end_matches('/')),
            ),
        )
    });
    let catalog_models = catalog
        .as_ref()
        .map(|catalog| catalog.cached_coding_models())
        .unwrap_or_default();
    let provider_models = merge_models(
        builtin_models()
            .into_iter()
            .filter(|candidate| candidate.provider.as_ref() == provider_id)
            .collect(),
        catalog_models,
    );
    let provider = provider.with_models(provider_models.clone());

    let switchable = Arc::new(SwitchableClient::new(Arc::new(provider)));

    let configured_model = config
        .default_model
        .clone()
        .filter(|id| provider_models.iter().any(|model| &model.id == id));
    let model_id = args
        .model
        .clone()
        .or(configured_model)
        .unwrap_or_else(|| default_model(&provider_id).to_owned());
    let model = provider_models
        .iter()
        .find(|candidate| candidate.id == model_id)
        .cloned()
        .unwrap_or_else(|| fallback_model(&provider_id, &model_id));

    let provider: Arc<dyn ChatClient> = switchable.clone();

    let extension_tool_names = extensions
        .extensions()
        .iter()
        .flat_map(|extension| extension.tools.iter().map(|tool| tool.name.clone()))
        .collect::<Vec<_>>();
    let mut registry = select_tools(args.tools.as_deref(), args.no_tools, &extension_tool_names)?;
    if !args.no_tools {
        let requested = args.tools.as_deref().map(|spec| {
            spec.split(',')
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .collect::<HashSet<_>>()
        });
        for tool in extensions.tools() {
            if requested
                .as_ref()
                .is_none_or(|names| names.contains(tool.name()))
            {
                registry.register(tool);
            }
        }
    }
    let allowed = allowed_tools(
        &config,
        args.allow.as_deref(),
        args.allow_all,
        &extension_tool_names,
    );
    let session_dir = config.session_dir_for(&cwd);
    let max_turns = args.max_turns.unwrap_or(config.max_turns);
    let models = model_choices(&provider_models);
    let models_loader = {
        let cache_dir = config.cache_dir.clone();
        let base_url = args.base_url.clone().or_else(|| config.base_url.clone());
        let loader = move |provider: &str| -> Vec<codie_tui::app::ModelChoice> {
            let curated = builtin_models()
                .into_iter()
                .filter(|candidate| candidate.provider.as_ref() == provider)
                .collect::<Vec<_>>();
            if provider == PROVIDER_OPENROUTER {
                let base_url = base_url
                    .clone()
                    .unwrap_or_else(|| provider_base_url(PROVIDER_OPENROUTER).to_owned());
                let catalog = OpenRouterCatalog::new(cache_dir.join("openrouter-models.json"))
                    .with_endpoint(format!("{}/models", base_url.trim_end_matches('/')));
                model_choices(&merge_models(curated, catalog.cached_coding_models()))
            } else {
                model_choices(&curated)
            }
        };
        Some(Arc::new(loader)
            as Arc<
                dyn Fn(&str) -> Vec<codie_tui::app::ModelChoice> + Send + Sync,
            >)
    };

    let providers = vec![
        ProviderChoice {
            id: "deepseek".to_owned(),
            name: "DeepSeek".to_owned(),
        },
        ProviderChoice {
            id: PROVIDER_OPENROUTER.to_owned(),
            name: "OpenRouter".to_owned(),
        },
    ];
    let switch_provider = {
        let switchable = Arc::clone(&switchable);
        let auth_path = config.auth_path();
        let cache_dir = config.cache_dir.clone();
        let api_key = args.api_key.clone();
        let base_url = args.base_url.clone().or_else(|| config.base_url.clone());
        let switch = move |provider: &str| -> Result<ProviderRuntime, String> {
            validate_provider(provider).map_err(|error| format!("{error:#}"))?;
            let store = AuthStore::new(&auth_path);
            let key = resolve_key(api_key.as_deref(), &store, provider, env_var(provider))
                .map_err(|error| format!("{error:#}"))?;
            let mut client = match provider {
                PROVIDER_OPENROUTER => OpenAiProvider::openrouter(key),
                _ => OpenAiProvider::deepseek(key),
            };
            if let Some(base_url) = &base_url {
                client = client.with_base_url(base_url.clone());
            }
            let mut models = builtin_models()
                .into_iter()
                .filter(|candidate| candidate.provider.as_ref() == provider)
                .collect::<Vec<_>>();
            if provider == PROVIDER_OPENROUTER {
                let catalog =
                    OpenRouterCatalog::new(cache_dir.join("openrouter-models.json")).with_endpoint(
                        format!("{}/models", client.base_url().trim_end_matches('/')),
                    );
                let cached = catalog.cached_coding_models();
                Arc::new(catalog).spawn_refresh();
                models = merge_models(models, cached);
            }
            switchable.set(Arc::new(client.with_models(models.clone())));
            Ok(ProviderRuntime {
                default_model: default_model(provider).to_owned(),
                models: model_choices(&models),
            })
        };
        Some(Arc::new(switch)
            as Arc<
                dyn Fn(&str) -> Result<ProviderRuntime, String> + Send + Sync,
            >)
    };
    if max_turns == 0 {
        bail!("--max-turns must be greater than zero");
    }

    // Refresh the catalog for every mode. The task never blocks the run and
    // never fails it (`PROV-021`).
    if let Some(catalog) = &catalog {
        Arc::clone(catalog).spawn_refresh();
    }

    let interactive = !args.print
        && !matches!(args.mode, OutputMode::Json | OutputMode::Rpc)
        && std::io::stdin().is_terminal()
        && (std::io::stdout().is_terminal() || matches!(args.tui_mode, TuiModeArg::Plain));

    if interactive {
        let session = open_session(&args, &cwd, &session_dir)?;
        let history = session
            .as_ref()
            .map(Session::projection)
            .unwrap_or_default();
        let initial_prompt = (!args.message.is_empty()).then(|| args.message.join(" "));
        let options = TuiOptions {
            client: Arc::clone(&provider),
            cwd: cwd.clone(),
            model: ModelRef {
                api: model.api.to_string(),
                provider: provider_id.clone(),
                model: model_id.clone(),
            },
            context_window: model.context_window,
            models,
            models_loader,
            providers,
            switch_provider,
            thinking,
            max_turns,
            session,
            history,
            initial_prompt,
            auto_allowed: allowed.clone(),
            theme: args.theme.clone(),
            mode: args.tui_mode.into(),
            session_dir: session_dir.clone(),
            resume_picker: matches!(args.resume, Some(None)),
            signal: Arc::new(AtomicI32::new(0)),
            registry: Arc::new(registry),
            keybindings: Some(config.config_dir.join("keybindings.toml")),
            preamble: preamble.clone(),
            append: append.clone(),
            customizations: customizations.clone(),
            extensions: Arc::clone(&extensions),
        };
        return codie_tui::run(options).await;
    }

    let toolbox: Arc<dyn Toolbox> = Arc::new(PolicyToolbox::new(
        Arc::new(registry),
        Policy::headless(allowed),
    ));

    if matches!(args.mode, OutputMode::Rpc) {
        let mut session = open_session(&args, &cwd, &session_dir)?;
        let mut history = session
            .as_ref()
            .map(Session::projection)
            .unwrap_or_default();
        ensure_system_message(
            &mut history,
            &mut session,
            toolbox.as_ref(),
            &cwd,
            preamble.as_deref(),
            append.as_deref(),
            &customizations,
        )?;
        let model_ref = ModelRef {
            api: model.api.to_string(),
            provider: provider_id.clone(),
            model: model_id.clone(),
        };
        return crate::rpc::run(crate::rpc::RpcOptions {
            client: provider,
            toolbox,
            cwd,
            session_dir,
            session,
            history,
            model: model_ref,
            models: provider_models,
            context_window: u64::from(model.context_window),
            thinking,
            max_turns,
            preamble,
            append,
            customizations,
            extensions: Arc::clone(&extensions),
        })
        .await;
    }

    let mut text = read_prompt(&args)?;
    let mut session = open_session(&args, &cwd, &session_dir)?;
    let session_id = session.as_ref().map(|session| session.id().to_owned());
    extensions.set_session_id(session_id.clone());
    let model_ref = ModelRef {
        api: model.api.to_string(),
        provider: provider_id.clone(),
        model: model_id.clone(),
    };
    let session_info = ExtSessionInfo {
        id: session_id.clone(),
        cwd: cwd.display().to_string(),
        model: Some(model_id.clone()),
    };

    if let Some(invocation) = extensions
        .resolve_command(&text)
        .map_err(anyhow::Error::msg)?
    {
        let result = extensions
            .invoke_command(&invocation, &session_info)
            .await
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        if let Some(prompt) = result.prompt {
            text = prompt;
        } else {
            if let Some(display) = result.display {
                if !matches!(args.mode, OutputMode::Json) {
                    if let Some(title) = display.title {
                        println!("{title}");
                    }
                    println!("{}", display.text);
                }
            }
            extensions.shutdown("command complete").await;
            return Ok(0);
        }
    }

    let submitted = extensions.user_submit(&session_info, &text).await;
    if submitted.block {
        bail!(
            "{}",
            submitted
                .reason
                .unwrap_or_else(|| "prompt blocked by an extension".to_owned())
        );
    }
    if let Some(prompt) = submitted.prompt {
        text = prompt;
    }

    let expansion = expand_attachments(&text, &cwd);
    for error in &expansion.errors {
        eprintln!("warning: {error}");
    }
    let prompt_message = Message::user_with_content(expansion.content);

    let mut history = session
        .as_ref()
        .map(Session::projection)
        .unwrap_or_default();

    ensure_system_message(
        &mut history,
        &mut session,
        toolbox.as_ref(),
        &cwd,
        preamble.as_deref(),
        append.as_deref(),
        &customizations,
    )?;
    extensions.session_start(&session_info).await;

    let cancel = CancellationToken::new();
    let interrupted = install_signal_handlers(cancel.clone());

    let mut agent_config =
        AgentConfig::new(model_ref.clone(), cwd.clone()).map_err(anyhow::Error::msg)?;
    agent_config.thinking = thinking;
    agent_config.max_turns = max_turns;
    agent_config.session_id = session.as_ref().map(|session| session.id().to_owned());
    agent_config.cancel = cancel.clone();

    let agent = match extensions.hooks() {
        Some(hooks) => Agent::with_hooks(Arc::clone(&provider), Arc::clone(&toolbox), hooks),
        None => Agent::new(Arc::clone(&provider), Arc::clone(&toolbox)),
    };

    if matches!(args.mode, OutputMode::Json) {
        match session.as_ref() {
            Some(session) => println!("{}", serde_json::to_string(session.header())?),
            None => println!(
                "{}",
                serde_json::json!({
                    "type": "session",
                    "version": 1,
                    "id": "ephemeral",
                    "cwd": cwd.display().to_string(),
                })
            ),
        }
    }

    let context_window = u64::from(model.context_window);
    let input_limit = context_window
        .saturating_sub(codie_tui::compaction::DEFAULT_RESERVE_TOKENS)
        .clamp(4_000, 200_000);
    if let Some(current) = session.as_mut()
        && history
            .iter()
            .any(|message| !matches!(message, Message::System(_)))
        && codie_tui::compaction::needs_compaction(&history, &prompt_message, context_window)
    {
        match codie_tui::compaction::summarize(
            Arc::clone(&provider),
            model_ref.clone(),
            thinking,
            &history,
            None,
            Some(current.id().to_owned()),
            input_limit,
        )
        .await
        {
            Ok(outcome) => {
                history = codie_tui::compaction::apply(current, &outcome, None)
                    .map_err(anyhow::Error::msg)?;
                if !matches!(args.mode, OutputMode::Json) {
                    eprintln!("Compacted the context before the request.");
                }
            }
            Err(error) if !matches!(args.mode, OutputMode::Json) => {
                eprintln!("warning: automatic compaction failed: {error}");
            }
            Err(_) => {}
        }
    }

    let mut final_text = String::new();
    let mut final_stop;
    let mut status;
    let mut recovered = false;

    loop {
        let mut events = if recovered {
            agent.resume(history.clone(), agent_config.clone())
        } else {
            agent.run(
                prompt_message.clone(),
                history.clone(),
                agent_config.clone(),
            )
        };
        final_text.clear();
        final_stop = None;
        status = RunStatus::Error;
        let mut last_assistant = None;

        while let Some(event) = events.recv().await {
            if matches!(args.mode, OutputMode::Json) {
                let line = serde_json::to_string(&event)?;
                let mut stdout = std::io::stdout().lock();
                writeln!(stdout, "{line}")?;
                stdout.flush()?;
            }

            match &event {
                AgentEvent::MessageEnd { message } => {
                    if let Some(session) = session.as_mut() {
                        session.append_message(message)?;
                    }
                    if let Some(assistant) = message.as_assistant() {
                        final_stop = Some(assistant.stop_reason);
                        last_assistant = Some(assistant.clone());
                        let text = assistant.text();
                        if !text.is_empty() {
                            final_text = text;
                        }
                    }
                }
                AgentEvent::AgentEnd { status: ended, .. } => {
                    status = *ended;
                    break;
                }
                _ => {}
            }
        }

        let overflow = last_assistant.as_ref().is_some_and(|assistant| {
            codie_tui::compaction::is_context_overflow(&Message::Assistant(assistant.clone()))
        });
        if status != RunStatus::Error || recovered || !overflow {
            break;
        }

        // CTX-006: one compaction and one retry per invocation.
        let Some(current) = session.as_mut() else {
            break;
        };
        if let Some(leaf) = current.leaf_target().map(str::to_owned)
            && current
                .entries()
                .iter()
                .any(|entry| entry.id() == leaf && entry.message().is_some())
        {
            current.append_context_edit(&leaf, None)?;
        }
        history = current.projection();
        let session_id = current.id().to_owned();
        let outcome = match codie_tui::compaction::summarize(
            Arc::clone(&provider),
            model_ref.clone(),
            thinking,
            &history,
            None,
            Some(session_id),
            input_limit,
        )
        .await
        {
            Ok(outcome) => outcome,
            Err(error) => {
                eprintln!("context recovery failed: {error}");
                break;
            }
        };
        history =
            codie_tui::compaction::apply(current, &outcome, None).map_err(anyhow::Error::msg)?;
        if !matches!(args.mode, OutputMode::Json) {
            eprintln!("Context overflow recovered; retrying once.");
        }
        recovered = true;
    }

    if matches!(args.mode, OutputMode::Json) {
        let settled = serde_json::json!({
            "type": "agent_settled",
            "aborted": status == RunStatus::Aborted,
        });
        println!("{settled}");
    } else {
        if !final_text.is_empty() {
            println!("{final_text}");
        }
        if final_stop == Some(StopReason::Length) {
            eprintln!("Response was truncated: the model reached its output token limit.");
        }
    }

    if status != RunStatus::Completed && !matches!(args.mode, OutputMode::Json) {
        eprintln!("run ended with status: {status:?}");
    }

    extensions.shutdown("run complete").await;
    Ok(exit_code(
        status,
        final_stop,
        args.mode,
        interrupted.load(Ordering::SeqCst),
    ))
}

/// Maps a finished run to the documented process exit code (`CLI-003`).
///
/// A received signal always wins. JSON mode reports failures through the event
/// stream, so it exits zero unless the invocation itself failed or was
/// interrupted.
fn exit_code(status: RunStatus, stop: Option<StopReason>, mode: OutputMode, signal: i32) -> u8 {
    if signal != 0 {
        return u8::try_from(signal).unwrap_or(1);
    }
    if matches!(mode, OutputMode::Json) {
        return 0;
    }
    if status != RunStatus::Completed {
        return 1;
    }
    match stop {
        Some(
            StopReason::Length
            | StopReason::Error
            | StopReason::Aborted
            | StopReason::Pending
            | StopReason::ToolUse,
        ) => 1,
        Some(StopReason::Stop | StopReason::Deferred) | None => 0,
    }
}

/// Installs SIGINT/SIGTERM/SIGHUP handlers and returns the observed exit code.
///
/// The first signal cancels the run; the code is stored so the run path can
/// return 130, 143, or 129 instead of a generic failure.
fn install_signal_handlers(cancel: CancellationToken) -> Arc<AtomicI32> {
    let code = Arc::new(AtomicI32::new(0));

    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};

        for (kind, value) in [
            (SignalKind::interrupt(), 130),
            (SignalKind::terminate(), 143),
            (SignalKind::hangup(), 129),
        ] {
            let code = Arc::clone(&code);
            let cancel = cancel.clone();
            tokio::spawn(async move {
                let Ok(mut stream) = signal(kind) else {
                    return;
                };
                if stream.recv().await.is_some() {
                    let _ = code.compare_exchange(0, value, Ordering::SeqCst, Ordering::SeqCst);
                    cancel.cancel();
                }
            });
        }
    }

    #[cfg(not(unix))]
    {
        let code = Arc::clone(&code);
        tokio::spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                let _ = code.compare_exchange(0, 130, Ordering::SeqCst, Ordering::SeqCst);
                cancel.cancel();
            }
        });
    }

    code
}

#[allow(clippy::too_many_arguments)]
fn ensure_system_message(
    history: &mut Vec<Message>,
    session: &mut Option<Session>,
    toolbox: &dyn Toolbox,
    cwd: &Path,
    preamble: Option<&str>,
    append: Option<&str>,
    customizations: &Customizations,
) -> Result<()> {
    if matches!(history.first(), Some(Message::System(_))) {
        return Ok(());
    }
    let declarations = toolbox.declarations();
    let input = PromptInput {
        cwd: cwd.to_path_buf(),
        tools: declarations.clone(),
        preamble: preamble.map(str::to_owned),
        append: append.map(str::to_owned),
        skills: customizations.skills().to_vec(),
        project_context: load_project_context(cwd),
    };
    let system = build_system_message(&input, declarations);
    if let Some(session) = session.as_mut() {
        session.append_message(&system)?;
    }
    history.insert(0, system);
    Ok(())
}

fn validate_provider(provider: &str) -> Result<()> {
    match provider {
        "deepseek" | "openrouter" => Ok(()),
        other => bail!("unsupported provider `{other}`; expected deepseek or openrouter"),
    }
}

fn default_model(provider: &str) -> &'static str {
    match provider {
        PROVIDER_OPENROUTER => "openrouter/auto",
        _ => "deepseek-flash",
    }
}

/// A chat client whose inner provider can be swapped at runtime.
///
/// The agent and tool registry keep one stable `Arc<dyn ChatClient>` for the
/// whole session; `/provider` replaces the inner client instead of rebuilding
/// them.
struct SwitchableClient {
    current: std::sync::RwLock<Arc<dyn ChatClient>>,
}

impl SwitchableClient {
    fn new(client: Arc<dyn ChatClient>) -> Self {
        Self {
            current: std::sync::RwLock::new(client),
        }
    }

    fn set(&self, client: Arc<dyn ChatClient>) {
        *self
            .current
            .write()
            .unwrap_or_else(|error| error.into_inner()) = client;
    }
}

impl ChatClient for SwitchableClient {
    fn stream(
        &self,
        context: &[Message],
        options: &codie_core::StreamOptions,
    ) -> codie_core::AssistantEventStream {
        let client = self
            .current
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        client.stream(context, options)
    }
}

/// Converts adapter models into the picker entries the TUI consumes.
fn model_choices(models: &[Model]) -> Vec<codie_tui::app::ModelChoice> {
    models
        .iter()
        .map(|model| codie_tui::app::ModelChoice {
            provider: model.provider.to_string(),
            id: model.id.clone(),
            name: model.name.clone(),
            context_window: model.context_window,
        })
        .collect()
}

/// Asks the user whether to trust the project's `.codie/` resources.
///
/// Only called when stdin and stdout are terminals and the run is interactive.
fn prompt_trust(cwd: &Path) -> Result<bool> {
    eprint!(
        "Project {} contains .codie resources (skills, templates, or system instructions).\n\
         Trust this project? [y/N] ",
        cwd.display()
    );
    std::io::stderr().flush().ok();
    let mut line = String::new();
    if std::io::stdin()
        .read_line(&mut line)
        .context("cannot read the trust answer")?
        == 0
    {
        return Ok(false);
    }
    Ok(matches!(
        line.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

fn read_prompt(args: &RunArgs) -> Result<String> {
    if !args.message.is_empty() {
        return Ok(args.message.join(" "));
    }
    if std::io::stdin().is_terminal() {
        bail!("provide a message, pipe stdin, or run codie with a terminal for interactive mode");
    }
    let mut text = String::new();
    std::io::stdin()
        .read_to_string(&mut text)
        .context("cannot read the prompt from stdin")?;
    let text = text.trim().to_owned();
    if text.is_empty() {
        bail!("prompt is empty; provide a message or pipe stdin");
    }
    Ok(text)
}

fn select_tools(
    spec: Option<&str>,
    no_tools: bool,
    extension_tools: &[String],
) -> Result<Registry> {
    if no_tools {
        return Ok(Registry::new(Vec::new()));
    }
    let Some(spec) = spec else {
        return Ok(Registry::builtin());
    };
    let names = spec
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .collect::<HashSet<_>>();

    let mut tools: Vec<Arc<dyn Tool>> = Vec::new();
    for tool_name in ToolName::all() {
        if names.contains(tool_name.as_str()) {
            tools.push(tool_for(tool_name));
        }
    }

    let unknown = names
        .iter()
        .filter(|name| {
            name.parse::<ToolName>().is_err()
                && !extension_tools.iter().any(|extension| extension == **name)
        })
        .copied()
        .collect::<Vec<_>>();
    if !unknown.is_empty() {
        bail!("unknown tool(s): {}", unknown.join(", "));
    }
    Ok(Registry::new(tools))
}

/// Returns the implementation for one built-in tool name.
fn tool_for(name: ToolName) -> Arc<dyn Tool> {
    match name {
        ToolName::Read => Arc::new(ReadTool),
        ToolName::Write => Arc::new(WriteTool),
        ToolName::Edit => Arc::new(EditTool),
        ToolName::Bash => Arc::new(BashTool),
        ToolName::PowerShell => Arc::new(PowerShellTool),
        ToolName::Grep => Arc::new(GrepTool),
        ToolName::Find => Arc::new(FindTool),
        ToolName::Ls => Arc::new(LsTool),
        ToolName::WebFetch => Arc::new(WebFetchTool::new()),
    }
}

fn allowed_tools(
    config: &Config,
    cli: Option<&str>,
    allow_all: bool,
    extension_tools: &[String],
) -> Vec<String> {
    let mut allowed = config.allow.clone();
    if let Some(cli) = cli {
        allowed.extend(
            cli.split(',')
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_owned),
        );
    }
    if allow_all {
        allowed.extend(
            ToolName::all()
                .into_iter()
                .map(|tool| tool.as_str().to_owned()),
        );
        allowed.extend(extension_tools.iter().cloned());
    }
    allowed
}

fn open_session(
    args: &RunArgs,
    cwd: &std::path::Path,
    session_dir: &std::path::Path,
) -> Result<Option<Session>> {
    if args.no_session {
        return Ok(None);
    }
    if let Some(needle) = args.session.as_deref() {
        return Ok(Some(Session::resolve(session_dir, needle)?));
    }
    if let Some(needle) = args.fork.as_deref() {
        let source = Session::resolve(session_dir, needle)?;
        return Ok(Some(source.fork_at(None, cwd, session_dir)?));
    }
    if let Some(needle) = args.clone.as_deref() {
        let source = Session::resolve(session_dir, needle)?;
        return Ok(Some(source.clone_session(cwd, session_dir)?));
    }
    if let Some(Some(needle)) = args.resume.as_ref() {
        return Ok(Some(Session::resolve(session_dir, needle)?));
    }
    if args.continue_session || args.resume.is_some() {
        return Ok(Session::continue_recent(cwd, session_dir)?);
    }
    Ok(Some(Session::create(cwd, session_dir)?))
}
