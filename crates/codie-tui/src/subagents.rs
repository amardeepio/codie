//! Host wiring for subagent delegation in the interactive interfaces.

use std::path::PathBuf;
use std::sync::Arc;

use codie_core::{ChatClient, ModelRef, ThinkingLevel, Toolbox};
use codie_prompt::{PromptInput, load_project_context, load_project_memory};
use codie_subagents::{
    DEFAULT_MAX_DEPTH_LIMIT, DEFAULT_MAX_PARALLEL_LIMIT, DelegationMode, RunSnapshot, RunState,
    SubagentConfig, SubagentRuntime, SubagentToolbox, ToolSelection,
};

use crate::permissions::{PermissionBroker, PermissionToolbox};

/// Result of wiring the tools for one interface.
pub(crate) struct Built {
    pub toolbox: Arc<dyn Toolbox>,
    pub runtime: Option<Arc<SubagentRuntime>>,
    /// Effective configuration, including project agent definitions.
    pub config: SubagentConfig,
}

/// Everything needed to build the subagent-aware toolbox.
pub(crate) struct BuildInput {
    pub config: SubagentConfig,
    pub mode: DelegationMode,
    /// Project personas loaded from `.codie/agents/*.md`.
    pub agents: Vec<codie_prompt::AgentDefinition>,
    pub registry: Arc<dyn Toolbox>,
    pub broker: PermissionBroker,
    pub client: Arc<dyn ChatClient>,
    pub prompt: PromptInput,
    pub session_dir: PathBuf,
    pub session_id: Option<String>,
    pub model: ModelRef,
    pub known_models: Vec<String>,
    pub thinking: ThinkingLevel,
}

/// Builds the permission-wrapped toolbox, adding `task` when enabled.
///
/// The root chain is `PermissionToolbox(SubagentToolbox(Registry))`, so the
/// `task` call prompts like any other tool (`SEC-009`). Children receive the
/// inner `PermissionToolbox(Registry)` and therefore inherit the same policy.
pub(crate) fn build(input: BuildInput) -> Result<Built, String> {
    let registry: Arc<dyn Toolbox> = input.registry.clone();
    let base: Arc<dyn Toolbox> = Arc::new(PermissionToolbox::new(
        Arc::clone(&registry),
        input.broker.clone(),
    ));

    // The interactive interface always declares `task` so `/subagents on|off`
    // works without restarting. `enabled = false` only makes the initial mode
    // `off`; the master switch is the runtime mode, not tool registration.
    let configured = input.config.enabled;
    let mut config = input.config;
    config.enabled = true;
    config.apply_agent_definitions(&input.agents);
    config.validate()?;
    let available = registry
        .declarations()
        .into_iter()
        .map(|declaration| declaration.name)
        .collect::<Vec<_>>();
    config.validate_tools(&available)?;
    let runtime = SubagentRuntime::new(
        config,
        input.client,
        base,
        input.prompt,
        input.session_dir,
        input.session_id,
        input.model,
        input.known_models,
    )?;
    runtime.set_mode(if configured {
        input.mode
    } else {
        DelegationMode::Off
    });
    runtime.set_thinking(input.thinking);
    let runtime = Arc::new(runtime);
    // A nested `task` (max_depth > 0) goes through the same prompt as the
    // root call instead of silently spawning and asking nothing.
    let guard = PermissionToolbox::new(Arc::clone(&registry), input.broker.clone());
    runtime.set_nested_task(
        guard.guard(Arc::new(codie_subagents::TaskTool::new(Arc::clone(
            &runtime,
        )))),
    );
    let subagents = Arc::new(SubagentToolbox::new(
        Arc::clone(&registry),
        Arc::clone(&runtime),
    ));
    let toolbox: Arc<dyn Toolbox> = Arc::new(PermissionToolbox::new(subagents, input.broker));
    let config = (*runtime.config()).clone();
    Ok(Built {
        toolbox,
        runtime: Some(runtime),
        config,
    })
}

/// Builds the prompt inputs shared by the parent and its children.
pub(crate) fn prompt_input(
    cwd: &std::path::Path,
    preamble: Option<String>,
    append: Option<String>,
    customizations: &codie_prompt::Customizations,
) -> PromptInput {
    PromptInput {
        cwd: cwd.to_path_buf(),
        tools: Vec::new(),
        preamble,
        append,
        skills: customizations.skills().to_vec(),
        project_context: load_project_context(cwd),
        project_memory: load_project_memory(cwd),
    }
}

/// One parsed `/subagents` command.
pub(crate) enum SubagentCommand {
    /// Show the current state.
    Status(String),
    /// Set the session mode.
    Mode {
        /// New mode.
        mode: DelegationMode,
        /// Message shown in the transcript.
        message: String,
    },
    /// Force the next prompt only.
    Once {
        /// Message shown in the transcript.
        message: String,
    },
    /// Set the parallelism used by the next session.
    Parallel {
        /// New limit.
        limit: usize,
        /// Message shown in the transcript.
        message: String,
    },
    /// Open one child transcript by its 1-based run index.
    Open(usize),
    /// Cancel one running child by its 1-based run index.
    Stop(usize),
    /// A usage error.
    Error(String),
}

/// Parses `/subagents` arguments into an action.
pub(crate) fn parse_command(
    argument: Option<&str>,
    config: &SubagentConfig,
    mode: DelegationMode,
    force_once: bool,
    runs: &[RunSnapshot],
) -> SubagentCommand {
    let Some(argument) = argument.map(str::trim).filter(|text| !text.is_empty()) else {
        return SubagentCommand::Status(format_status(config, mode, force_once, runs));
    };
    let (name, rest) = argument
        .split_once(char::is_whitespace)
        .map_or((argument, ""), |(name, rest)| (name, rest.trim()));
    match name.to_ascii_lowercase().as_str() {
        "on" | "auto" => SubagentCommand::Mode {
            mode: DelegationMode::Auto,
            message: "Subagents: auto (the model decides).".to_owned(),
        },
        "off" => SubagentCommand::Mode {
            mode: DelegationMode::Off,
            message: "Subagents: off for this session.".to_owned(),
        },
        "force" | "always" => SubagentCommand::Mode {
            mode: DelegationMode::Force,
            message: "Subagents: forced for this session.".to_owned(),
        },
        "once" => SubagentCommand::Once {
            message: "Subagents: forced for the next prompt.".to_owned(),
        },
        "parallel" => match rest.parse::<usize>() {
            Ok(limit) if (1..=DEFAULT_MAX_PARALLEL_LIMIT).contains(&limit) => {
                SubagentCommand::Parallel {
                    limit,
                    message: format!("Subagent parallelism: {limit}."),
                }
            }
            _ => SubagentCommand::Error(format!(
                "Usage: /subagents parallel N (1-{DEFAULT_MAX_PARALLEL_LIMIT})"
            )),
        },
        "types" => SubagentCommand::Status(types_status(config)),
        "status" => SubagentCommand::Status(format_status(config, mode, force_once, runs)),
        "open" => match rest.parse::<usize>() {
            Ok(index) if index > 0 => SubagentCommand::Open(index),
            _ => SubagentCommand::Error(
                "Usage: /subagents open N (see the run numbers in /subagents status)".to_owned(),
            ),
        },
        "stop" => match rest.parse::<usize>() {
            Ok(index) if index > 0 => SubagentCommand::Stop(index),
            _ => SubagentCommand::Error(
                "Usage: /subagents stop N (see the run numbers in /subagents status)".to_owned(),
            ),
        },
        other => SubagentCommand::Error(format!(
            "Unknown /subagents argument `{other}`. Try on, off, force, once, parallel N, types, \
             open N, or stop N."
        )),
    }
}

/// Formats the status block for `/subagents`.
#[must_use]
pub(crate) fn format_status(
    config: &SubagentConfig,
    mode: DelegationMode,
    force_once: bool,
    runs: &[RunSnapshot],
) -> String {
    let once = if force_once { ", once pending" } else { "" };
    let names = config.ordered_type_names().join(", ");
    let mut text = format!(
        "Subagents: {}{once}\n  task tool: {}\n  parallel: {}\n  depth: {} (max {})\n  turns: {}\n  timeout: {}s\n  result cap: {} bytes\n  types: {names}",
        mode.as_str(),
        if config.enabled {
            "available"
        } else {
            "unavailable"
        },
        config.max_parallel,
        config.max_depth,
        DEFAULT_MAX_DEPTH_LIMIT,
        config.max_turns,
        config.timeout.as_secs(),
        config.result_max_bytes,
    );
    if runs.is_empty() {
        text.push_str("\n  runs: none yet");
    } else {
        text.push_str("\n  runs:");
        for (index, run) in runs.iter().enumerate() {
            text.push_str(&format!(
                "\n    {}. {} · {} · {} · {} · {}",
                index + 1,
                run.agent,
                truncate(&run.description, 40),
                format_elapsed(run.elapsed),
                run.state.as_str(),
                run_detail(run),
            ));
        }
        text.push_str("\n  /subagents open N to inspect, stop N to cancel one.");
    }
    text
}

fn run_detail(run: &RunSnapshot) -> String {
    let session = run
        .session_id
        .as_deref()
        .map(|id| format!("session {id}"))
        .unwrap_or_else(|| "not persisted".to_owned());
    if matches!(run.state, RunState::Running | RunState::Queued) {
        format!("turn {} · {}", run.turns, run.activity)
    } else {
        let error = run
            .error
            .as_deref()
            .map(|error| format!(" · {}", truncate(error, 60)))
            .unwrap_or_default();
        format!(
            "{} turn{} · ${:.4} · {session}{error}",
            run.turns,
            if run.turns == 1 { "" } else { "s" },
            run.cost_nanos as f64 / 1_000_000_000.0
        )
    }
}

fn truncate(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_owned();
    }
    let mut output = text
        .chars()
        .take(limit.saturating_sub(1))
        .collect::<String>();
    output.push('…');
    output
}

fn format_elapsed(elapsed: std::time::Duration) -> String {
    let seconds = elapsed.as_secs();
    if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 3600 {
        format!("{}m {:02}s", seconds / 60, seconds % 60)
    } else {
        format!("{}h {:02}m", seconds / 3600, (seconds % 3600) / 60)
    }
}

fn types_status(config: &SubagentConfig) -> String {
    let mut lines = vec!["Subagent types:".to_owned()];
    for name in config.ordered_type_names() {
        let Some(agent) = config.agent_type(&name) else {
            continue;
        };
        let tools = match &agent.tools {
            ToolSelection::Inherit => "inherit".to_owned(),
            ToolSelection::Only(tools) => tools.join(", "),
        };
        let model = agent.model.as_deref().unwrap_or("parent model");
        let turns = agent.max_turns.map_or_else(
            || format!("{} (default)", config.max_turns),
            |turns| turns.to_string(),
        );
        let timeout = agent.timeout.map_or_else(
            || format!("{}s (default)", config.timeout.as_secs()),
            |timeout| format!("{}s", timeout.as_secs()),
        );
        let description = agent.description.as_deref().unwrap_or("no description");
        lines.push(format!(
            "  {name}: {description}\n    model {model}; tools {tools}; turns {turns}; timeout {timeout}"
        ));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use codie_core::{AssistantEventStream, ChatClient, Message, StreamOptions, ThinkingLevel};
    use codie_testkit::spec;

    struct NoopClient;

    impl ChatClient for NoopClient {
        fn stream(&self, _context: &[Message], _options: &StreamOptions) -> AssistantEventStream {
            AssistantEventStream::channel(1).0
        }
    }

    fn input(config: SubagentConfig, dir: &std::path::Path) -> BuildInput {
        BuildInput {
            config,
            mode: DelegationMode::Auto,
            agents: Vec::new(),
            registry: Arc::new(codie_tools::Registry::builtin()),
            broker: PermissionBroker::new(tokio::sync::mpsc::unbounded_channel().0, Vec::new()),
            client: Arc::new(NoopClient),
            prompt: prompt_input(dir, None, None, &codie_prompt::Customizations::default()),
            session_dir: dir.join("sessions"),
            session_id: None,
            model: ModelRef {
                api: "test-api".to_owned(),
                provider: "test-provider".to_owned(),
                model: "test-model".to_owned(),
            },
            known_models: Vec::new(),
            thinking: ThinkingLevel::Medium,
        }
    }

    #[test]
    #[spec("TUI-044")]
    fn status_lists_live_activity_and_open_hint() {
        let config = SubagentConfig::default();
        let runs = vec![
            RunSnapshot {
                call_id: "call_1".to_owned(),
                agent: "explore".to_owned(),
                description: "Map the parser".to_owned(),
                depth: 1,
                session_id: Some("ses_child".to_owned()),
                turns: 2,
                activity: "grep TODO".to_owned(),
                state: RunState::Running,
                elapsed: std::time::Duration::from_secs(12),
                cost_nanos: 0,
                error: None,
                persisted: true,
            },
            RunSnapshot {
                call_id: "call_2".to_owned(),
                agent: "ui-reviewer".to_owned(),
                description: "Review the header".to_owned(),
                depth: 1,
                session_id: Some("ses_done".to_owned()),
                turns: 3,
                activity: "completed".to_owned(),
                state: RunState::Completed,
                elapsed: std::time::Duration::from_secs(34),
                cost_nanos: 12_300_000,
                error: None,
                persisted: true,
            },
        ];

        let text = format_status(&config, DelegationMode::Auto, false, &runs);
        assert!(text.contains("runs:"), "{text}");
        assert!(
            text.contains("1. explore · Map the parser · 12s · running"),
            "{text}"
        );
        assert!(text.contains("turn 2 · grep TODO"), "{text}");
        assert!(text.contains("ui-reviewer"), "{text}");
        assert!(text.contains("$0.0123"), "{text}");
        assert!(text.contains("/subagents open N"), "{text}");
    }

    #[test]
    fn types_status_includes_descriptions_and_budgets() {
        let mut config = SubagentConfig::default();
        if let Some(explore) = config.types.get_mut("explore") {
            explore.max_turns = Some(7);
        }
        let text = types_status(&config);
        assert!(
            text.contains("explore: Read-only codebase search"),
            "{text}"
        );
        assert!(text.contains("turns 7"), "{text}");
    }

    #[test]
    fn disabled_config_still_declares_task_and_starts_off() {
        let dir = tempfile::tempdir().unwrap();
        let config = SubagentConfig {
            enabled: false,
            ..SubagentConfig::default()
        };
        let built = build(input(config, dir.path())).unwrap();
        let runtime = built.runtime.expect("runtime is always built for the TUI");
        assert_eq!(runtime.mode(), DelegationMode::Off);
        assert!(
            built
                .toolbox
                .declarations()
                .iter()
                .any(|declaration| declaration.name == "task")
        );

        runtime.set_mode(DelegationMode::Auto);
        assert_eq!(runtime.mode(), DelegationMode::Auto);
    }
}
