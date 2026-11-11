//! The `codie` run path: resolve config, stream a turn, persist the session.

use std::collections::HashSet;
use std::io::{IsTerminal, Read, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};

use anyhow::{Context, Result, bail};
use clap::{Args, ValueEnum};
use codie_ai::{
    AuthStore, OpenAiProvider, OpenRouterCatalog, fallback_model, find_model, resolve_key,
};
use codie_core::{
    Agent, AgentConfig, AgentEvent, Message, ModelRef, RunStatus, StopReason, ThinkingLevel, Tool,
    Toolbox,
};
use codie_prompt::{PromptInput, build_system_message, load_project_context};
use codie_session::Session;
use codie_tools::{BashTool, EditTool, Policy, PolicyToolbox, ReadTool, Registry, WriteTool};
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

    /// Comma-separated mutating tools permitted in headless mode.
    #[arg(long)]
    pub(crate) allow: Option<String>,

    /// Permit every mutating tool for this run.
    #[arg(long)]
    pub(crate) allow_all: bool,

    /// Maximum assistant requests in this run.
    #[arg(long)]
    pub(crate) max_turns: Option<usize>,

    /// Working directory for tools and project context.
    #[arg(long)]
    pub(crate) cwd: Option<PathBuf>,

    /// Open a specific session file.
    #[arg(long)]
    pub(crate) session: Option<PathBuf>,

    /// Resume the most recent session for this directory.
    #[arg(short = 'c', long = "continue")]
    pub(crate) continue_session: bool,

    /// Do not persist this run.
    #[arg(long)]
    pub(crate) no_session: bool,

    /// Message text. When omitted and stdin is piped, stdin is used.
    #[arg(value_name = "MESSAGE", trailing_var_arg = true)]
    pub(crate) message: Vec<String>,
}

/// Runs the agent once and returns the process exit code.
pub(crate) async fn run(args: RunArgs) -> Result<u8> {
    if args.api_key.is_some() && args.model.is_none() {
        bail!("--api-key requires an explicit --model so the key cannot be paired with a default");
    }

    let config = Config::load()?;
    let cwd = match args.cwd.clone() {
        Some(cwd) => cwd,
        None => std::env::current_dir().context("cannot determine the working directory")?,
    };
    let cwd = cwd.canonicalize().unwrap_or(cwd);

    let provider_id = args
        .provider
        .clone()
        .unwrap_or_else(|| config.default_provider.clone());
    validate_provider(&provider_id)?;

    let model_id = args
        .model
        .clone()
        .or_else(|| config.default_model.clone())
        .unwrap_or_else(|| default_model(&provider_id).to_owned());
    let model = find_model(&provider_id, &model_id)
        .unwrap_or_else(|| fallback_model(&provider_id, &model_id));

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
        "openrouter" => OpenAiProvider::openrouter(key),
        _ => OpenAiProvider::deepseek(key),
    };
    if let Some(base_url) = args.base_url.clone().or_else(|| config.base_url.clone()) {
        provider = provider.with_base_url(base_url);
    }

    let registry = select_tools(args.tools.as_deref(), args.no_tools)?;
    let allowed = allowed_tools(&config, args.allow.as_deref(), args.allow_all);
    let session_dir = config.session_dir_for(&cwd);
    let max_turns = args.max_turns.unwrap_or(config.max_turns);
    if max_turns == 0 {
        bail!("--max-turns must be greater than zero");
    }

    let interactive = !args.print
        && !matches!(args.mode, OutputMode::Json)
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
            client: Arc::new(provider),
            cwd: cwd.clone(),
            model: ModelRef {
                api: model.api.to_string(),
                provider: provider_id.clone(),
                model: model_id.clone(),
            },
            thinking,
            max_turns,
            session,
            history,
            initial_prompt,
            auto_allowed: allowed.clone(),
            theme: args.theme.clone(),
            mode: args.tui_mode.into(),
            session_dir: session_dir.clone(),
            signal: Arc::new(AtomicI32::new(0)),
            registry: Arc::new(registry),
        };
        return codie_tui::run(options).await;
    }

    let toolbox: Arc<dyn Toolbox> = Arc::new(PolicyToolbox::new(
        Arc::new(registry),
        Policy::headless(allowed),
    ));

    let text = read_prompt(&args)?;

    let mut session = open_session(&args, &cwd, &session_dir)?;
    let mut history = session
        .as_ref()
        .map(Session::projection)
        .unwrap_or_default();

    if !matches!(history.first(), Some(Message::System(_))) {
        let declarations = toolbox.declarations();
        let input = PromptInput {
            cwd: cwd.clone(),
            tools: declarations.clone(),
            preamble: None,
            append: None,
            project_context: load_project_context(&cwd),
        };
        let system = build_system_message(&input, declarations);
        if let Some(session) = session.as_mut() {
            session.append_message(&system)?;
        }
        history.insert(0, system);
    }

    let cancel = CancellationToken::new();
    let interrupted = install_signal_handlers(cancel.clone());

    let model_ref = ModelRef {
        api: model.api.to_string(),
        provider: provider_id.clone(),
        model: model_id.clone(),
    };
    let mut agent_config = AgentConfig::new(model_ref, cwd.clone()).map_err(anyhow::Error::msg)?;
    agent_config.thinking = thinking;
    agent_config.max_turns = max_turns;
    agent_config.session_id = session.as_ref().map(|session| session.id().to_owned());
    agent_config.cancel = cancel.clone();

    let catalog_endpoint = format!("{}/models", provider.base_url().trim_end_matches('/'));
    let agent = Agent::new(Arc::new(provider), Arc::clone(&toolbox));

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

    // Refresh the OpenRouter catalog after the first output is written. The
    // task never blocks the run and never fails it (`PROV-021`).
    if provider_id == "openrouter" {
        let catalog = OpenRouterCatalog::new(config.cache_dir.join("openrouter-models.json"))
            .with_endpoint(catalog_endpoint);
        Arc::new(catalog).spawn_refresh();
    }

    let mut events = agent.run(Message::user(text), history, agent_config);
    let mut final_text = String::new();
    let mut final_stop = None;
    let mut status = RunStatus::Error;

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

fn validate_provider(provider: &str) -> Result<()> {
    match provider {
        "deepseek" | "openrouter" => Ok(()),
        other => bail!("unsupported provider `{other}`; expected deepseek or openrouter"),
    }
}

fn default_model(provider: &str) -> &'static str {
    match provider {
        "openrouter" => "deepseek/deepseek-v4.1-flash-1017",
        _ => "deepseek-flash",
    }
}

fn read_prompt(args: &RunArgs) -> Result<String> {
    if !args.message.is_empty() {
        return Ok(args.message.join(" "));
    }
    if std::io::stdin().is_terminal() {
        bail!("provide a message or pipe stdin; interactive mode arrives in Phase 2");
    }
    let mut text = String::new();
    std::io::stdin()
        .read_to_string(&mut text)
        .context("cannot read the prompt from stdin")?;
    let text = text.trim().to_owned();
    if text.is_empty() {
        bail!("prompt is empty");
    }
    Ok(text)
}

fn select_tools(spec: Option<&str>, no_tools: bool) -> Result<Registry> {
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
    for name in ["read", "write", "edit", "bash"] {
        if !names.contains(name) {
            continue;
        }
        let tool = match name {
            "read" => Arc::new(ReadTool) as Arc<dyn Tool>,
            "write" => Arc::new(WriteTool),
            "edit" => Arc::new(EditTool),
            "bash" => Arc::new(BashTool),
            _ => unreachable!(),
        };
        tools.push(tool);
    }

    let unknown = names
        .iter()
        .filter(|name| !matches!(**name, "read" | "write" | "edit" | "bash"))
        .copied()
        .collect::<Vec<_>>();
    if !unknown.is_empty() {
        bail!("unknown tool(s): {}", unknown.join(", "));
    }
    Ok(Registry::new(tools))
}

fn allowed_tools(config: &Config, cli: Option<&str>, allow_all: bool) -> Vec<String> {
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
            ["read", "write", "edit", "bash"]
                .into_iter()
                .map(str::to_owned),
        );
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
    if let Some(path) = args.session.as_ref() {
        return Ok(Some(Session::open(path)?));
    }
    if args.continue_session {
        return Ok(Session::continue_recent(cwd, session_dir)?);
    }
    Ok(Some(Session::create(cwd, session_dir)?))
}
