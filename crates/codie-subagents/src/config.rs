//! Subagent configuration and validation.

use std::collections::BTreeMap;
use std::time::Duration;

use codie_prompt::AgentDefinition;

/// Default number of child runs that may execute at once.
pub const DEFAULT_MAX_PARALLEL: usize = 4;
/// Hard upper bound for `max_parallel`.
pub const DEFAULT_MAX_PARALLEL_LIMIT: usize = 8;
/// Default turn budget for one child run.
pub const DEFAULT_MAX_TURNS: usize = 50;
/// Default wall-clock limit for one child run.
pub const DEFAULT_TIMEOUT_SECS: u64 = 600;
/// Default model-visible result cap in bytes.
pub const DEFAULT_RESULT_MAX_BYTES: usize = 20_000;
/// Hard upper bound for `max_depth`.
pub const DEFAULT_MAX_DEPTH_LIMIT: usize = 4;
/// Smallest accepted result cap.
pub const MIN_RESULT_MAX_BYTES: usize = 1_024;

/// Which tools a child of a given type may use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolSelection {
    /// Inherit the parent's permitted tools.
    Inherit,
    /// Only these tool names, intersected with the parent's permitted tools.
    Only(Vec<String>),
}

impl ToolSelection {
    /// Returns true when `name` is selected.
    #[must_use]
    pub fn allows(&self, name: &str) -> bool {
        match self {
            Self::Inherit => true,
            Self::Only(names) => names.iter().any(|candidate| candidate == name),
        }
    }
}

/// One agent type, such as `general`, `explore`, or a project-defined persona.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentTypeConfig {
    /// Model identifier. `None` inherits the parent model.
    pub model: Option<String>,
    /// Tool subset for this type.
    pub tools: ToolSelection,
    /// Short description of when to use the type, shown to the model.
    pub description: Option<String>,
    /// Extra system instructions for children of this type.
    pub prompt: Option<String>,
    /// Per-run turn budget. `None` uses the global `max_turns`.
    pub max_turns: Option<usize>,
    /// Per-run wall-clock limit. `None` uses the global `timeout`.
    pub timeout: Option<Duration>,
}

impl Default for AgentTypeConfig {
    fn default() -> Self {
        Self {
            model: None,
            tools: ToolSelection::Inherit,
            description: None,
            prompt: None,
            max_turns: None,
            timeout: None,
        }
    }
}

/// Resolved subagent configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct SubagentConfig {
    /// Whether the host registers the `task` tool.
    pub enabled: bool,
    /// Maximum concurrent child runs.
    pub max_parallel: usize,
    /// Maximum delegation depth. `0` allows the root to delegate only.
    pub max_depth: usize,
    /// Turn budget for one child run.
    pub max_turns: usize,
    /// Wall-clock limit for one child run.
    pub timeout: Duration,
    /// Whether child sessions are written to disk.
    pub persist_sessions: bool,
    /// Maximum model-visible result size in bytes.
    pub result_max_bytes: usize,
    /// Aggregate token cap across all children in the process.
    pub max_total_tokens: Option<u64>,
    /// Aggregate cost cap in USD across all children in the process.
    pub max_total_cost_usd: Option<f64>,
    /// Agent types by name.
    pub types: BTreeMap<String, AgentTypeConfig>,
}

impl Default for SubagentConfig {
    fn default() -> Self {
        let mut types = BTreeMap::new();
        types.insert("general".to_owned(), AgentTypeConfig::default());
        types.insert(
            "explore".to_owned(),
            AgentTypeConfig {
                model: None,
                tools: ToolSelection::Only(
                    ["read", "grep", "find", "ls", "web_fetch"]
                        .into_iter()
                        .map(str::to_owned)
                        .collect(),
                ),
                description: Some("Read-only codebase search and reading.".to_owned()),
                ..AgentTypeConfig::default()
            },
        );
        if let Some(general) = types.get_mut("general") {
            general.description = Some(
                "Independent implementation or review work with the parent's permitted tools."
                    .to_owned(),
            );
        }
        Self {
            enabled: false,
            max_parallel: DEFAULT_MAX_PARALLEL,
            max_depth: 0,
            max_turns: DEFAULT_MAX_TURNS,
            timeout: Duration::from_secs(DEFAULT_TIMEOUT_SECS),
            persist_sessions: true,
            result_max_bytes: DEFAULT_RESULT_MAX_BYTES,
            max_total_tokens: None,
            max_total_cost_usd: None,
            types,
        }
    }
}

impl SubagentConfig {
    /// Returns the configuration for `name`, when it exists.
    #[must_use]
    pub fn agent_type(&self, name: &str) -> Option<&AgentTypeConfig> {
        self.types.get(name)
    }

    /// Returns type names with `general` first and the rest sorted.
    ///
    /// The order is stable so the `task` schema and the model-visible list do
    /// not churn between runs.
    #[must_use]
    pub fn ordered_type_names(&self) -> Vec<String> {
        let mut names = self.types.keys().cloned().collect::<Vec<_>>();
        names.sort();
        if let Some(index) = names.iter().position(|name| name == "general") {
            let general = names.remove(index);
            names.insert(0, general);
        }
        names
    }

    /// Adds definitions loaded from `.codie/agents/*.md`.
    ///
    /// A definition overrides the matching built-in or configured type, which
    /// makes the project the most specific source. Fields omitted from the
    /// file keep the existing value, so overriding `explore` without a
    /// `tools:` line stays read-only.
    pub fn apply_agent_definitions(&mut self, definitions: &[AgentDefinition]) {
        for definition in definitions {
            let base = self
                .types
                .get(&definition.name)
                .cloned()
                .unwrap_or_default();
            let description = if definition.description.trim().is_empty() {
                base.description
            } else {
                Some(definition.description.trim().to_owned())
            };
            self.types.insert(
                definition.name.clone(),
                AgentTypeConfig {
                    model: definition.model.clone().or(base.model),
                    tools: definition
                        .tools
                        .as_ref()
                        .map(|tools| ToolSelection::Only(tools.clone()))
                        .unwrap_or(base.tools),
                    description,
                    prompt: Some(definition.prompt.trim().to_owned())
                        .filter(|prompt| !prompt.is_empty())
                        .or(base.prompt),
                    max_turns: definition.max_turns.or(base.max_turns),
                    timeout: definition
                        .timeout_secs
                        .map(Duration::from_secs)
                        .or(base.timeout),
                },
            );
        }
    }

    /// Validates numeric bounds and required agent types.
    pub fn validate(&self) -> Result<(), String> {
        if self.max_parallel == 0 || self.max_parallel > DEFAULT_MAX_PARALLEL_LIMIT {
            return Err(format!(
                "subagents.max_parallel must be between 1 and {DEFAULT_MAX_PARALLEL_LIMIT}"
            ));
        }
        if self.max_depth > DEFAULT_MAX_DEPTH_LIMIT {
            return Err(format!(
                "subagents.max_depth must be at most {DEFAULT_MAX_DEPTH_LIMIT}"
            ));
        }
        if self.max_turns == 0 {
            return Err("subagents.max_turns must be greater than zero".to_owned());
        }
        if self.timeout.is_zero() {
            return Err("subagents.timeout_secs must be greater than zero".to_owned());
        }
        if self.result_max_bytes < MIN_RESULT_MAX_BYTES {
            return Err(format!(
                "subagents.result_max_bytes must be at least {MIN_RESULT_MAX_BYTES}"
            ));
        }
        if self.max_total_tokens == Some(0) {
            return Err("subagents.max_total_tokens must be greater than zero".to_owned());
        }
        if self
            .max_total_cost_usd
            .is_some_and(|value| !value.is_finite() || value <= 0.0)
        {
            return Err("subagents.max_total_cost_usd must be a positive number".to_owned());
        }
        if !self.types.contains_key("general") {
            return Err("subagents.types must define `general`".to_owned());
        }
        for (name, agent) in &self.types {
            if name.trim().is_empty() || name.contains(char::is_whitespace) {
                return Err(
                    "subagent type names must be non-empty and contain no spaces".to_owned(),
                );
            }
            if agent.max_turns == Some(0) {
                return Err(format!(
                    "subagents.types.{name}.max_turns must be greater than zero"
                ));
            }
            if agent.timeout.is_some_and(|timeout| timeout.is_zero()) {
                return Err(format!(
                    "subagents.types.{name}.timeout_secs must be greater than zero"
                ));
            }
        }
        Ok(())
    }

    /// Validates that every explicit tool name exists in `available`.
    ///
    /// Host tools are allowed even when the current interface does not expose
    /// them: `question` exists in the interactive app but not headless, and
    /// `task` is added by depth rather than by the registry.
    pub fn validate_tools(&self, available: &[String]) -> Result<(), String> {
        const HOST_TOOLS: [&str; 2] = ["question", "task"];
        for (name, agent) in &self.types {
            if let ToolSelection::Only(tools) = &agent.tools {
                for tool in tools {
                    if !HOST_TOOLS.contains(&tool.as_str())
                        && !available.iter().any(|candidate| candidate == tool)
                    {
                        return Err(format!(
                            "subagents.types.{name}.tools contains unknown tool `{tool}`"
                        ));
                    }
                }
            }
        }
        Ok(())
    }
}
