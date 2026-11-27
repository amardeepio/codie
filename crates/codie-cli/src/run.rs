//! The `codie` run path: resolve config, stream a turn, persist the session.

use std::collections::{BTreeMap, HashSet};
use std::io::{IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};

use anyhow::{Context, Result, bail};
use clap::{Args, ValueEnum};
use codie_ai::provider::{self, CatalogKind};
use codie_ai::{
    AuthStore, Model, OpenAiCatalog, OpenRouterCatalog, canonical_provider, fallback_model,
    merge_models, provider_env_var, resolve_key,
};
use codie_core::{
    Agent, AgentConfig, AgentEvent, AssistantDelta, AssistantEventStream, AssistantMessage,
    ChatClient, Message, ModelRef, RunStatus, StopReason, StreamOptions, ThinkingLevel, Tool,
    Toolbox,
};
use codie_ext::{ExtensionSet, LoadOptions};
use codie_prompt::{
    Customizations, PromptInput, TrustStore, build_system_message, expand_attachments,
    load_instructions, load_project_context, load_project_memory,
};
use codie_protocol::ext::ExtSessionInfo;
use codie_session::Session;
use codie_subagents::{
    DelegationMode, FORCED_DIRECTIVE, FORCED_NUDGE, SubagentRuntime, SubagentToolbox,
    detect_intent, effective_mode,
};
use codie_tools::{
    BashTool, EditTool, FindTool, GrepTool, LsTool, Policy, PolicyToolbox, PowerShellTool,
    ProjectMemoryTool, ReadTool, Registry, ToolName, WebFetchTool, WriteTool,
};
use codie_tui::app::{ProviderChoice, ProviderRuntime};
use codie_tui::{TuiOptions, TuiRunMode};
use tokio_util::sync::CancellationToken;

use crate::config::{self, Config};

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

    /// Theme name (`system`, `dark`, `light`, `dracula`, `material`, `gruvbox`, `nord`, `nightowl`,
    /// `orng`, `tokyonight`) or a TOML theme path.
    #[arg(long, value_name = "NAME|PATH")]
    pub(crate) theme: Option<String>,

    /// Provider id (`deepseek`, `openrouter`, or `gemini`).
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

    /// Subagent delegation mode: off, auto, or force.
    #[arg(long, value_name = "MODE")]
    pub(crate) subagents: Option<String>,

    /// Maximum concurrent subagent runs (1-8).
    #[arg(long, value_name = "N")]
    pub(crate) subagents_parallel: Option<usize>,

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

impl RunArgs {
    /// Returns true when this run enters the interactive terminal.
    ///
    /// Mirrors the conditions used by [`run`], so logging can be configured
    /// correctly before the terminal is set up.
    pub(crate) fn interactive(&self) -> bool {
        !self.print
            && !matches!(self.mode, OutputMode::Json | OutputMode::Rpc)
            && std::io::stdin().is_terminal()
            && (std::io::stdout().is_terminal() || matches!(self.tui_mode, TuiModeArg::Plain))
    }
}

/// Runs the agent once and returns the process exit code.
pub(crate) async fn run(args: RunArgs, deferred_flags: BTreeMap<String, String>) -> Result<u8> {
    if args.api_key.is_some() && args.model.is_none() {
        bail!("--api-key requires an explicit --model so the key cannot be paired with a default");
    }

    let mut config = Config::load()?;
    let cwd = match args.cwd.clone() {
        Some(cwd) => cwd,
        None => std::env::current_dir().context("cannot determine the working directory")?,
    };
    let cwd = config::normalize_cwd(&cwd);

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
    // Project personas are the most specific source: apply them after
    // `config.toml` so `.codie/agents/<name>.md` can override a configured type.
    if !customizations.agents().is_empty() {
        config
            .subagents
            .apply_agent_definitions(customizations.agents());
        config.subagents.validate().map_err(anyhow::Error::msg)?;
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

    let interactive = args.interactive();

    let saved = config.load_selection();
    let requested_provider = args.provider.clone().or_else(|| {
        if config.provider_configured {
            None
        } else {
            saved
                .provider
                .clone()
                .filter(|provider| provider::find(provider).is_some())
        }
    });
    let provider_id = requested_provider.unwrap_or_else(|| config.default_provider.clone());
    let mut provider_id = validate_provider(&provider_id)?.to_owned();

    let thinking = match args.thinking.as_deref() {
        Some(value) => ThinkingLevel::parse(value)
            .with_context(|| format!("invalid thinking level `{value}`"))?,
        None => config.thinking,
    };

    let base_url_override = args.base_url.clone().or_else(|| config.base_url.clone());
    let explicit_key_value = args
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|key| !key.is_empty());
    let explicit_key = explicit_key_value.is_some();
    let store = AuthStore::new(config.auth_path());
    let mut key = resolve_key(
        explicit_key_value,
        &store,
        &provider_id,
        provider_env_var(&provider_id),
    );

    // When the configured provider has no key but a different provider does,
    // prefer the connected one so the TUI can start usefully. Explicit
    // `--provider`, `--model`, and `--api-key` choices are always respected.
    if interactive
        && key.is_err()
        && args.provider.is_none()
        && args.model.is_none()
        && !explicit_key
        && base_url_override.is_none()
        && let Some((next_provider, next_key)) = first_configured_key(&store, &provider_id)
    {
        provider_id = next_provider;
        key = Ok(next_key);
    }

    let requires_key = provider::requires_key(&provider_id);
    // The RPC server must start even without a provider key: a host such as the
    // desktop app stores the key through `auth_set_key` after connecting, and
    // sends prompts only once one exists. Prompts surface the missing-key error
    // instead of the process dying before the handshake.
    let start_without_key = interactive || matches!(args.mode, OutputMode::Rpc);
    let key = match key {
        Ok(key) => Some(key),
        Err(_) if !requires_key => Some(String::new()),
        Err(error) if !start_without_key => {
            return Err(anyhow::Error::new(error)).with_context(|| {
                format!(
                    "no API key for {provider_id}; run `codie auth login {provider_id}` or set {}",
                    provider_env_var(&provider_id)
                )
            });
        }
        Err(_) => None,
    };
    let connected = key.is_some();
    let startup_connect = interactive && !connected;

    let spec = provider::find(&provider_id).context("validated provider is not registered")?;
    let mut provider = provider::build_client(&provider_id, key.clone().unwrap_or_default())
        .context("validated provider is not buildable")?;
    if let Some(base_url) = base_url_override.clone() {
        provider = provider.with_base_url(base_url);
    }

    let catalog_models = match spec.catalog {
        CatalogKind::OpenRouter => {
            let catalog = Arc::new(
                OpenRouterCatalog::new(config.cache_dir.join("openrouter-models.json"))
                    .with_endpoint(format!(
                        "{}/models",
                        provider.base_url().trim_end_matches('/')
                    )),
            );
            let models = catalog.cached_coding_models();
            Arc::clone(&catalog).spawn_refresh();
            models
        }
        CatalogKind::OpenAiModels => {
            let mut catalog = OpenAiCatalog::new(
                *spec,
                config.cache_dir.join(format!("{}-models.json", spec.id)),
            )
            .with_endpoint(format!(
                "{}/models",
                provider.base_url().trim_end_matches('/')
            ));
            if let Some(key) = &key {
                catalog = catalog.with_api_key(key.clone());
            }
            let models = catalog.cached_models();
            if connected {
                Arc::new(catalog).spawn_refresh();
            }
            models
        }
        CatalogKind::Bundled => Vec::new(),
    };
    let provider_models = merge_models((spec.models)(), catalog_models);
    let provider = provider.with_models(provider_models.clone());

    let initial_client: Arc<dyn ChatClient> = if connected {
        Arc::new(provider)
    } else {
        Arc::new(DisconnectedClient::new(provider_id.clone()))
    };
    let switchable = Arc::new(SwitchableClient::new(initial_client));

    let configured_model = config
        .default_model
        .clone()
        .filter(|id| provider_models.iter().any(|model| &model.id == id));
    let saved_model = if config.model_configured {
        None
    } else {
        saved
            .model
            .clone()
            .filter(|id| provider_models.iter().any(|model| &model.id == id))
    };
    let model_id = args
        .model
        .clone()
        .or(configured_model)
        .or(saved_model)
        .unwrap_or_else(|| spec.default_model.to_owned());
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
        let base_url = base_url_override.clone();
        let loader = move |provider: &str| -> Vec<codie_tui::app::ModelChoice> {
            let Some(spec) = provider::find(provider) else {
                return Vec::new();
            };
            let curated = (spec.models)();
            let endpoint_base = base_url.clone().unwrap_or_else(|| spec.base_url.to_owned());
            let endpoint = format!("{}/models", endpoint_base.trim_end_matches('/'));
            match spec.catalog {
                CatalogKind::OpenRouter => {
                    let catalog = OpenRouterCatalog::new(cache_dir.join("openrouter-models.json"))
                        .with_endpoint(endpoint);
                    model_choices(&merge_models(curated, catalog.cached_coding_models()))
                }
                CatalogKind::OpenAiModels => {
                    let catalog = OpenAiCatalog::new(
                        *spec,
                        cache_dir.join(format!("{}-models.json", spec.id)),
                    )
                    .with_endpoint(endpoint);
                    model_choices(&merge_models(curated, catalog.cached_models()))
                }
                CatalogKind::Bundled => model_choices(&curated),
            }
        };
        Some(Arc::new(loader)
            as Arc<
                dyn Fn(&str) -> Vec<codie_tui::app::ModelChoice> + Send + Sync,
            >)
    };

    let providers = provider::all()
        .iter()
        .map(|spec| ProviderChoice {
            id: spec.id.to_owned(),
            name: spec.name.to_owned(),
        })
        .collect::<Vec<_>>();
    let switch_provider = {
        let switchable = Arc::clone(&switchable);
        let auth_path = config.auth_path();
        let cache_dir = config.cache_dir.clone();
        let api_key = args.api_key.clone();
        let explicit_key_provider = explicit_key.then(|| provider_id.clone());
        let base_url = base_url_override.clone();
        let switch = move |provider: &str| -> Result<ProviderRuntime, String> {
            let provider = validate_provider(provider).map_err(|error| format!("{error:#}"))?;
            let spec = provider::find(provider)
                .ok_or_else(|| format!("provider `{provider}` is not registered"))?;
            let store = AuthStore::new(&auth_path);
            let explicit = explicit_key_provider
                .as_deref()
                .filter(|configured| *configured == provider)
                .and(api_key.as_deref());
            let key = match resolve_key(explicit, &store, provider, spec.env_var) {
                Ok(key) => key,
                Err(_) if !spec.requires_key => String::new(),
                Err(error) => return Err(format!("{error:#}")),
            };
            let mut client = provider::build_client(provider, key.clone())
                .ok_or_else(|| format!("provider `{provider}` is not buildable"))?;
            if let Some(base_url) = &base_url {
                client = client.with_base_url(base_url.clone());
            }
            let mut models = (spec.models)();
            match spec.catalog {
                CatalogKind::OpenRouter => {
                    let catalog = OpenRouterCatalog::new(cache_dir.join("openrouter-models.json"))
                        .with_endpoint(format!(
                            "{}/models",
                            client.base_url().trim_end_matches('/')
                        ));
                    let cached = catalog.cached_coding_models();
                    Arc::new(catalog).spawn_refresh();
                    models = merge_models(models, cached);
                }
                CatalogKind::OpenAiModels => {
                    let catalog = OpenAiCatalog::new(
                        *spec,
                        cache_dir.join(format!("{}-models.json", spec.id)),
                    )
                    .with_endpoint(format!(
                        "{}/models",
                        client.base_url().trim_end_matches('/')
                    ))
                    .with_api_key(key);
                    let cached = catalog.cached_models();
                    Arc::new(catalog).spawn_refresh();
                    models = merge_models(models, cached);
                }
                CatalogKind::Bundled => {}
            }
            switchable.set(Arc::new(client.with_models(models.clone())));
            Ok(ProviderRuntime {
                default_model: spec.default_model.to_owned(),
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

    let subagent_mode = match args.subagents.as_deref() {
        Some(value) => DelegationMode::parse(value)
            .with_context(|| format!("invalid --subagents value `{value}`"))?,
        None => DelegationMode::Auto,
    };
    let mut subagent_config = config.subagents.clone();
    // An explicit flag is authoritative: `--subagents off` disables delegation
    // even when config.toml enabled it, while `auto`/`force` opt in. A parallel
    // override alone also opts in.
    if args.subagents.is_some() {
        subagent_config.enabled = subagent_mode != DelegationMode::Off;
    } else if args.subagents_parallel.is_some() {
        subagent_config.enabled = true;
    }
    if let Some(limit) = args.subagents_parallel {
        if limit == 0 || limit > codie_subagents::DEFAULT_MAX_PARALLEL_LIMIT {
            bail!(
                "--subagents-parallel must be between 1 and {}",
                codie_subagents::DEFAULT_MAX_PARALLEL_LIMIT
            );
        }
        subagent_config.max_parallel = limit;
    }
    let available_tools = registry
        .declarations()
        .into_iter()
        .map(|declaration| declaration.name)
        .collect::<Vec<_>>();
    subagent_config
        .validate_tools(&available_tools)
        .map_err(anyhow::Error::msg)?;

    if interactive {
        let session = open_session(&args, &cwd, &session_dir)?;
        let history = session
            .as_ref()
            .map(Session::projection)
            .unwrap_or_default();
        let initial_prompt = (!args.message.is_empty()).then(|| args.message.join(" "));
        let persist_selection = {
            let config = config.clone();
            Some(Arc::new(move |provider: &str, model: &str| {
                let _ = config.save_selection(provider, model);
            }) as Arc<dyn Fn(&str, &str) + Send + Sync>)
        };
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
            subagents: subagent_config.clone(),
            subagent_mode,
            subagent_force_once: false,
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
            auth_path: config.auth_path(),
            base_url: base_url_override,
            explicit_key_provider: explicit_key.then(|| provider_id.clone()),
            startup_connect,
            persist_selection,
        };
        return codie_tui::run(options).await;
    }

    let registry: Arc<dyn Toolbox> = Arc::new(registry);
    let policy = Policy::headless(allowed.clone());
    let inner_toolbox: Arc<dyn Toolbox> =
        Arc::new(PolicyToolbox::new(Arc::clone(&registry), policy.clone()));
    let mut toolbox: Arc<dyn Toolbox> = Arc::clone(&inner_toolbox);

    if matches!(args.mode, OutputMode::Rpc) {
        // The RPC server builds its system message after it has assembled the
        // subagent-aware toolbox; building it here from the raw registry would
        // omit `task` from the model-visible declarations.
        let session = open_session(&args, &cwd, &session_dir)?;
        let history = session
            .as_ref()
            .map(Session::projection)
            .unwrap_or_default();
        let model_ref = ModelRef {
            api: model.api.to_string(),
            provider: provider_id.clone(),
            model: model_id.clone(),
        };
        return crate::rpc::run(crate::rpc::RpcOptions {
            client: provider,
            toolbox: registry,
            allowed,
            config: config.clone(),
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
            subagents: subagent_config,
            subagent_mode,
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

    let intent = detect_intent(&text);
    let delegation_plan = effective_mode(subagent_config.enabled, subagent_mode, false, intent);
    if subagent_config.enabled && delegation_plan.mode != DelegationMode::Off {
        let prompt_input = PromptInput {
            cwd: cwd.clone(),
            tools: Vec::new(),
            preamble: preamble.clone(),
            append: append.clone(),
            skills: customizations.skills().to_vec(),
            project_context: load_project_context(&cwd),
            project_memory: load_project_memory(&cwd),
        };
        let runtime = SubagentRuntime::new(
            subagent_config.clone(),
            Arc::clone(&provider),
            Arc::clone(&inner_toolbox),
            prompt_input,
            session_dir.clone(),
            session_id.clone(),
            model_ref.clone(),
            provider_models
                .iter()
                .map(|model| model.id.clone())
                .collect(),
        )
        .map_err(anyhow::Error::msg)?;
        runtime.set_mode(delegation_plan.mode);
        runtime.set_thinking(thinking);
        let subagents = Arc::new(SubagentToolbox::new(
            Arc::clone(&registry),
            Arc::new(runtime),
        ));
        toolbox = Arc::new(PolicyToolbox::new(subagents, policy.clone()));
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
    if delegation_plan.is_forced() {
        let directive = Message::system(FORCED_DIRECTIVE, Vec::new());
        if let Some(session) = session.as_mut() {
            session.append_message(&directive)?;
        }
        history.push(directive);
    }
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
    let mut nudged = false;
    let mut follow_up: Option<Message> = None;

    loop {
        let mut had_task = false;
        let mut events = if recovered {
            agent.resume(history.clone(), agent_config.clone())
        } else if let Some(message) = follow_up.take() {
            agent.run(message, history.clone(), agent_config.clone())
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
                AgentEvent::ToolExecutionStart { tool_name, .. } if tool_name == "task" => {
                    had_task = true;
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
        let recovering = status == RunStatus::Error && !recovered && overflow;
        if !recovering {
            // `AGENT-030`: one nudge when a forced run never delegated.
            if delegation_plan.is_forced()
                && !had_task
                && !nudged
                && matches!(status, RunStatus::Completed | RunStatus::TurnLimit)
            {
                nudged = true;
                let nudge = Message::user(FORCED_NUDGE);
                if let Some(session) = session.as_mut() {
                    session.append_message(&nudge)?;
                }
                history.push(nudge.clone());
                follow_up = Some(nudge);
                if !matches!(args.mode, OutputMode::Json) {
                    eprintln!("No delegation detected; asking once.");
                }
                continue;
            }
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
        project_memory: load_project_memory(cwd),
    };
    let system = build_system_message(&input, declarations);
    if let Some(session) = session.as_mut() {
        session.append_message(&system)?;
    }
    history.insert(0, system);
    Ok(())
}

fn validate_provider(provider: &str) -> Result<&'static str> {
    canonical_provider(provider).ok_or_else(|| {
        anyhow::anyhow!(
            "unsupported provider `{provider}`; expected {}",
            provider::ids().join(", ")
        )
    })
}

/// Finds the first other provider with a usable stored or environment key.
fn first_configured_key(store: &AuthStore, preferred: &str) -> Option<(String, String)> {
    provider::all()
        .iter()
        .filter(|spec| spec.requires_key && !spec.id.eq_ignore_ascii_case(preferred))
        .find_map(|spec| {
            resolve_key(None, store, spec.id, spec.env_var)
                .ok()
                .filter(|key| !key.is_empty())
                .map(|key| (spec.id.to_owned(), key))
        })
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

/// A client used before the TUI has a usable provider credential.
///
/// A submitted prompt fails immediately with an actionable message instead of
/// sending an empty bearer token. Connecting a provider replaces this client
/// through [`SwitchableClient`].
struct DisconnectedClient {
    provider: String,
}

impl DisconnectedClient {
    fn new(provider: String) -> Self {
        Self { provider }
    }
}

impl ChatClient for DisconnectedClient {
    fn stream(&self, _context: &[Message], options: &StreamOptions) -> AssistantEventStream {
        let (stream, sender) = AssistantEventStream::channel(1);
        let mut message = AssistantMessage::partial(
            options.model.api.clone(),
            options.model.provider.clone(),
            options.model.model.clone(),
        );
        message.stop_reason = StopReason::Error;
        message.error_message = Some(format!(
            "No provider is connected. Run /connect to add an API key for {}.",
            provider::find(&self.provider).map_or("the provider", |spec| spec.name)
        ));
        let _ = sender.try_send(AssistantDelta::Error {
            reason: StopReason::Error,
            message,
        });
        stream
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
        ToolName::ProjectMemory => Arc::new(ProjectMemoryTool::new()),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[codie_testkit::spec("CLI-013")]
    fn spec_cli_013_subagent_flags() {
        assert_eq!(DelegationMode::parse("off"), Some(DelegationMode::Off));
        assert_eq!(DelegationMode::parse("auto"), Some(DelegationMode::Auto));
        assert_eq!(DelegationMode::parse("force"), Some(DelegationMode::Force));
        assert_eq!(DelegationMode::parse("nope"), None);
        assert_eq!(codie_subagents::DEFAULT_MAX_PARALLEL_LIMIT, 8);
    }
}
