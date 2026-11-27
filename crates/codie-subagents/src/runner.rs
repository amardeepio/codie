//! Child run orchestration and toolbox composition.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::{Duration, Instant};

use codie_core::{
    Agent, AgentConfig, AgentEvent, AgentEventStream, ChatClient, Content, Message, ModelRef,
    RunStatus, ThinkingLevel, Tool, ToolCall, ToolContext, ToolDecl, ToolProgress, Toolbox, Usage,
};
use codie_prompt::{PromptInput, build_system_message};
use codie_session::Session;
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio::sync::{Semaphore, mpsc};
use tokio_util::sync::CancellationToken;

use crate::budget::BudgetMeter;
use crate::config::{AgentTypeConfig, DEFAULT_MAX_PARALLEL_LIMIT, SubagentConfig, ToolSelection};
use crate::intent::DelegationMode;
use crate::tool::{TASK_TOOL_NAME, TaskArgs, TaskOutcome, TaskTool};

/// Terminal state of a delegated run, as reported to a host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunState {
    /// The child is waiting for a concurrency slot.
    Queued,
    /// The child is still executing.
    Running,
    /// The child finished normally.
    Completed,
    /// The child hit its turn budget.
    TurnLimit,
    /// The child was cancelled or timed out.
    Aborted,
    /// The child failed.
    Error,
}

impl RunState {
    /// Returns the lowercase label used by hosts and JSON details.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::TurnLimit => "turn_limit",
            Self::Aborted => "aborted",
            Self::Error => "error",
        }
    }
}

/// One delegated run the host can list, open, or cancel.
#[derive(Debug, Clone)]
pub struct RunSnapshot {
    /// Provider call identifier of the parent `task` call.
    pub call_id: String,
    /// Agent type name.
    pub agent: String,
    /// Short label from the parent call.
    pub description: String,
    /// Delegation depth.
    pub depth: usize,
    /// Persisted child session id, when available.
    pub session_id: Option<String>,
    /// Assistant requests used so far.
    pub turns: u32,
    /// Most recent activity label.
    pub activity: String,
    /// Current state.
    pub state: RunState,
    /// Wall-clock time since the child started.
    pub elapsed: std::time::Duration,
    /// Accumulated cost in nano-USD.
    pub cost_nanos: u64,
    /// Last error reported by the child, when any.
    pub error: Option<String>,
    /// True when a child session file exists.
    pub persisted: bool,
}

#[derive(Debug)]
struct RunEntry {
    call_id: String,
    agent: String,
    description: String,
    depth: usize,
    session_id: Option<String>,
    turns: u32,
    activity: String,
    state: RunState,
    started: Option<Instant>,
    finished: Option<Instant>,
    cost_nanos: u64,
    error: Option<String>,
    persisted: bool,
    cancel: tokio_util::sync::CancellationToken,
}

/// Keeps at most this many finished runs for the host history.
const MAX_FINISHED_RUNS: usize = 50;

#[derive(Debug, Default)]
struct RunRegistry {
    runs: std::sync::Mutex<Vec<RunEntry>>,
}

impl RunRegistry {
    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<RunEntry>> {
        self.runs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn register(&self, entry: RunEntry) {
        let mut runs = self.lock();
        let settled = |state: RunState| {
            matches!(
                state,
                RunState::Completed | RunState::TurnLimit | RunState::Aborted | RunState::Error
            )
        };
        let finished = runs.iter().filter(|run| settled(run.state)).count();
        if finished >= MAX_FINISHED_RUNS {
            let excess = finished - MAX_FINISHED_RUNS + 1;
            let mut removed = 0;
            runs.retain(|run| {
                if removed < excess && settled(run.state) {
                    removed += 1;
                    false
                } else {
                    true
                }
            });
        }
        runs.push(entry);
    }

    fn admit(&self, call_id: &str) {
        if let Some(run) = self.lock().iter_mut().find(|run| run.call_id == call_id)
            && run.state == RunState::Queued
        {
            run.state = RunState::Running;
            run.started = Some(Instant::now());
            run.activity = "starting".to_owned();
        }
    }

    fn attach_session(&self, call_id: &str, session_id: &str) {
        if let Some(run) = self.lock().iter_mut().find(|run| run.call_id == call_id) {
            run.session_id = Some(session_id.to_owned());
            run.persisted = true;
        }
    }

    fn note(&self, call_id: &str, turns: Option<u32>, activity: Option<&str>) {
        if let Some(run) = self.lock().iter_mut().find(|run| run.call_id == call_id) {
            if let Some(turns) = turns {
                run.turns = turns;
            }
            if let Some(activity) = activity {
                run.activity = activity.to_owned();
            }
        }
    }

    fn finish(&self, call_id: &str, state: RunState, turns: u32, usage: &Usage, persisted: bool) {
        if let Some(run) = self.lock().iter_mut().find(|run| run.call_id == call_id) {
            run.state = state;
            run.turns = turns;
            run.cost_nanos = usage.cost.total;
            run.persisted = persisted;
            run.activity = state.as_str().to_owned();
            run.finished = Some(Instant::now());
        }
    }

    fn set_error(&self, call_id: &str, error: impl Into<String>) {
        if let Some(run) = self.lock().iter_mut().find(|run| run.call_id == call_id) {
            run.error = Some(error.into());
        }
    }

    fn cancel(&self, call_id: &str) -> bool {
        let runs = self.lock();
        let Some(run) = runs.iter().find(|run| run.call_id == call_id) else {
            return false;
        };
        if !matches!(run.state, RunState::Queued | RunState::Running) {
            return false;
        }
        run.cancel.cancel();
        true
    }

    fn snapshot(&self) -> Vec<RunSnapshot> {
        self.lock()
            .iter()
            .map(|run| RunSnapshot {
                call_id: run.call_id.clone(),
                agent: run.agent.clone(),
                description: run.description.clone(),
                depth: run.depth,
                session_id: run.session_id.clone(),
                turns: run.turns,
                activity: run.activity.clone(),
                state: run.state,
                elapsed: run.started.map_or(Duration::ZERO, |started| {
                    run.finished
                        .unwrap_or_else(Instant::now)
                        .saturating_duration_since(started)
                }),
                cost_nanos: run.cost_nanos,
                error: run.error.clone(),
                persisted: run.persisted,
            })
            .collect()
    }
}

/// Everything one host needs to run delegated tasks.
pub struct SubagentRuntime {
    /// Resolved configuration.
    pub config: std::sync::RwLock<Arc<SubagentConfig>>,
    /// Parent chat client, reused for children.
    pub client: Arc<dyn ChatClient>,
    /// Parent toolbox, already wrapped by policy or permissions.
    pub parent_toolbox: Arc<dyn Toolbox>,
    /// Prompt inputs for children; `tools` is replaced per child.
    pub prompt: PromptInput,
    /// Directory for child session files.
    pub session_dir: PathBuf,
    /// Parent session identifier, when persisted.
    ///
    /// Mutable so a host that starts a new session keeps linking later
    /// children to the current parent instead of the startup session.
    parent_session_id: std::sync::RwLock<Option<String>>,
    /// Parent model, inherited unless an override is configured.
    ///
    /// Swappable at runtime so `/model` and `/provider` also change the model
    /// children use; a stale copy was the OpenRouter "unknown model" failure.
    parent_model: std::sync::RwLock<ModelRef>,
    /// Model ids known to the active provider.
    known_models: std::sync::RwLock<Vec<String>>,
    /// Thinking level for children.
    thinking: std::sync::RwLock<ThinkingLevel>,
    /// Maximum output tokens for children.
    pub max_output_tokens: Option<u32>,
    /// Sampling temperature for children.
    pub temperature: Option<f32>,
    semaphore: std::sync::Mutex<Arc<Semaphore>>,
    limit: std::sync::atomic::AtomicUsize,
    budget: BudgetMeter,
    mode: AtomicU8,
    runs: RunRegistry,
    nested_task: std::sync::OnceLock<Arc<dyn Tool>>,
}

impl SubagentRuntime {
    /// Creates a runtime, validating the configuration and bounds.
    pub fn new(
        config: SubagentConfig,
        client: Arc<dyn ChatClient>,
        parent_toolbox: Arc<dyn Toolbox>,
        prompt: PromptInput,
        session_dir: impl Into<PathBuf>,
        parent_session_id: Option<String>,
        parent_model: ModelRef,
        known_models: Vec<String>,
    ) -> Result<Self, String> {
        config.validate()?;
        let semaphore = std::sync::Mutex::new(Arc::new(Semaphore::new(config.max_parallel)));
        let limit = std::sync::atomic::AtomicUsize::new(config.max_parallel);
        let budget = BudgetMeter::new(&config);
        Ok(Self {
            config: std::sync::RwLock::new(Arc::new(config)),
            client,
            parent_toolbox,
            prompt,
            session_dir: session_dir.into(),
            parent_session_id: std::sync::RwLock::new(parent_session_id),
            parent_model: std::sync::RwLock::new(parent_model),
            known_models: std::sync::RwLock::new(known_models),
            thinking: std::sync::RwLock::new(ThinkingLevel::default()),
            max_output_tokens: None,
            temperature: None,
            semaphore,
            limit,
            budget,
            mode: AtomicU8::new(mode_to_u8(DelegationMode::Auto)),
            runs: RunRegistry::default(),
            nested_task: std::sync::OnceLock::new(),
        })
    }

    /// Installs the permission-wrapped `task` tool used by nested children.
    ///
    /// Without it a grandchild would be created without the prompt the root
    /// run had to pass; hosts with an interactive broker set this once.
    pub fn set_nested_task(&self, tool: Arc<dyn Tool>) {
        let _ = self.nested_task.set(tool);
    }

    fn nested_task(&self) -> Option<Arc<dyn Tool>> {
        self.nested_task.get().cloned()
    }

    /// Returns the active configuration.
    #[must_use]
    pub fn config(&self) -> Arc<SubagentConfig> {
        self.config
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Changes the inherited model and the ids known to the active provider.
    ///
    /// Hosts call this whenever the parent model or provider changes so
    /// children do not keep routing to the startup model.
    pub fn set_parent_model(&self, model: ModelRef, known_models: Vec<String>) {
        *self
            .parent_model
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = model;
        *self
            .known_models
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = known_models;
    }

    /// Changes the reasoning level children inherit.
    pub fn set_thinking(&self, thinking: ThinkingLevel) {
        *self
            .thinking
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = thinking;
    }

    /// Returns the thinking level children inherit right now.
    #[must_use]
    pub fn thinking(&self) -> ThinkingLevel {
        *self
            .thinking
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Returns the parent session identifier used for new child sessions.
    #[must_use]
    pub fn parent_session_id(&self) -> Option<String> {
        self.parent_session_id
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Replaces the parent session identifier used for new child sessions.
    ///
    /// Hosts call this when `new_session` swaps the parent session so later
    /// children link to the current session instead of the startup one.
    pub fn set_parent_session_id(&self, session_id: Option<String>) {
        *self
            .parent_session_id
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = session_id;
    }

    /// Returns the model children inherit right now.
    #[must_use]
    pub fn parent_model(&self) -> ModelRef {
        self.parent_model
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn known_models(&self) -> Vec<String> {
        self.known_models
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Replaces the configuration, for example after `/reload` adds personas.
    pub fn set_config(&self, config: SubagentConfig) {
        *self
            .config
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Arc::new(config);
    }

    /// Returns every child run known to this process, oldest first.
    #[must_use]
    pub fn runs(&self) -> Vec<RunSnapshot> {
        self.runs.snapshot()
    }

    /// Cancels one running child by its parent call id.
    ///
    /// Returns false when the run is unknown or already settled.
    pub fn cancel_run(&self, call_id: &str) -> bool {
        self.runs.cancel(call_id)
    }

    /// Returns the configured concurrency limit.
    #[must_use]
    pub fn max_parallel(&self) -> usize {
        self.limit.load(Ordering::SeqCst)
    }

    /// Changes the concurrency limit for this session.
    ///
    /// Returns false when the value is invalid or a child is queued or running;
    /// resizing under live permits would temporarily exceed the new bound.
    pub fn set_max_parallel(&self, limit: usize) -> bool {
        if limit == 0 || limit > DEFAULT_MAX_PARALLEL_LIMIT {
            return false;
        }
        if self
            .runs()
            .iter()
            .any(|run| matches!(run.state, RunState::Queued | RunState::Running))
        {
            return false;
        }
        *self
            .semaphore
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Arc::new(Semaphore::new(limit));
        self.limit.store(limit, Ordering::SeqCst);
        true
    }

    fn semaphore(&self) -> Arc<Semaphore> {
        self.semaphore
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Sets the runtime delegation mode used by the `task` tool.
    pub fn set_mode(&self, mode: DelegationMode) {
        self.mode.store(mode_to_u8(mode), Ordering::SeqCst);
    }

    /// Returns the runtime delegation mode.
    #[must_use]
    pub fn mode(&self) -> DelegationMode {
        u8_to_mode(self.mode.load(Ordering::SeqCst))
    }
}

const fn mode_to_u8(mode: DelegationMode) -> u8 {
    match mode {
        DelegationMode::Off => 0,
        DelegationMode::Auto => 1,
        DelegationMode::Force => 2,
    }
}

const fn u8_to_mode(value: u8) -> DelegationMode {
    match value {
        0 => DelegationMode::Off,
        2 => DelegationMode::Force,
        _ => DelegationMode::Auto,
    }
}

impl std::fmt::Debug for SubagentRuntime {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SubagentRuntime")
            .field("config", &self.config())
            .field("parent_session_id", &self.parent_session_id())
            .field("parent_model", &self.parent_model())
            .finish_non_exhaustive()
    }
}

/// Adds the `task` tool to a toolbox.
///
/// This wrapper is innermost: a policy or permission wrapper placed outside it
/// sees `task` like any other tool and gates it (`SEC-009`).
pub struct SubagentToolbox {
    inner: Arc<dyn Toolbox>,
    runtime: Arc<SubagentRuntime>,
}

impl SubagentToolbox {
    /// Wraps `inner`, adding `task` when the configuration enables it.
    #[must_use]
    pub fn new(inner: Arc<dyn Toolbox>, runtime: Arc<SubagentRuntime>) -> Self {
        Self { inner, runtime }
    }
}

impl std::fmt::Debug for SubagentToolbox {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SubagentToolbox")
            .field("enabled", &self.runtime.config().enabled)
            .finish_non_exhaustive()
    }
}

impl Toolbox for SubagentToolbox {
    fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        if name == TASK_TOOL_NAME && self.runtime.config().enabled {
            return Some(Arc::new(TaskTool::new(Arc::clone(&self.runtime))));
        }
        self.inner.get(name)
    }

    fn declarations(&self) -> Vec<ToolDecl> {
        let config = self.runtime.config();
        let mut declarations = self.inner.declarations();
        if config.enabled && !declarations.iter().any(|decl| decl.name == TASK_TOOL_NAME) {
            declarations.push(TaskTool::declaration(&config));
        }
        declarations
    }

    fn denial(&self, name: &str) -> Option<String> {
        self.inner.denial(name)
    }
}

/// Restricts a child to a parent-sandboxed tool subset.
struct ChildToolbox {
    parent: Arc<dyn Toolbox>,
    runtime: Arc<SubagentRuntime>,
    selection: ToolSelection,
    allow_task: bool,
}

impl ChildToolbox {
    fn allows(&self, name: &str) -> bool {
        if name == TASK_TOOL_NAME {
            return self.allow_task;
        }
        self.selection.allows(name)
    }
}

impl Toolbox for ChildToolbox {
    fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        if !self.allows(name) {
            return None;
        }
        if name == TASK_TOOL_NAME {
            return Some(
                self.runtime
                    .nested_task()
                    .unwrap_or_else(|| Arc::new(TaskTool::new(Arc::clone(&self.runtime)))),
            );
        }
        self.parent.get(name)
    }

    fn declarations(&self) -> Vec<ToolDecl> {
        let mut declarations = self
            .parent
            .declarations()
            .into_iter()
            .filter(|declaration| declaration.name != TASK_TOOL_NAME)
            .filter(|declaration| self.allows(&declaration.name))
            .collect::<Vec<_>>();
        if self.allow_task && self.parent.denial(TASK_TOOL_NAME).is_none() {
            declarations.push(TaskTool::declaration(&self.runtime.config()));
        }
        declarations
    }

    fn denial(&self, name: &str) -> Option<String> {
        if !self.allows(name) {
            // Hidden tools are absent from the declarations, so the loop's
            // normal unknown-tool path applies (`AGENT-019`, `AGENT-022`).
            return None;
        }
        self.parent.denial(name)
    }
}

pub(crate) async fn execute_task(
    runtime: &Arc<SubagentRuntime>,
    call: &ToolCall,
    context: &ToolContext,
    updates: Option<mpsc::UnboundedSender<ToolProgress>>,
) -> Result<TaskOutcome, String> {
    let config = runtime.config();
    let config = &*config;
    if !config.enabled {
        return Err("Subagents are disabled for this session.".to_owned());
    }
    if context.delegation_depth > config.max_depth {
        return Err(format!(
            "Subagent delegation depth {} exceeds the limit of {}.",
            context.delegation_depth, config.max_depth
        ));
    }

    let args = TaskArgs::parse(&call.arguments)?;
    let agent_config = config
        .agent_type(&args.agent)
        .ok_or_else(|| {
            let known = config.types.keys().cloned().collect::<Vec<_>>().join(", ");
            format!(
                "Unknown subagent type `{}`. Available types: {known}.",
                args.agent
            )
        })?
        .clone();
    let model = resolve_model(runtime, &agent_config, args.model.as_deref())?;

    runtime.budget.check_start()?;

    let cancel = context.cancel.child_token();
    let child_depth = context.delegation_depth + 1;
    let mut child_config = AgentConfig::new(model.clone(), context.cwd.clone())
        .map_err(|error| format!("invalid subagent configuration: {error}"))?;
    // Register before waiting so `/subagents` can show queued children and
    // cancel one without waiting for a slot.
    runtime.runs.register(RunEntry {
        call_id: call.id.clone(),
        agent: args.agent.clone(),
        description: args.description.clone(),
        depth: child_depth,
        session_id: None,
        turns: 0,
        activity: "waiting for a slot".to_owned(),
        state: RunState::Queued,
        started: None,
        finished: None,
        cost_nanos: 0,
        error: None,
        persisted: false,
        cancel: cancel.clone(),
    });

    let permit = tokio::select! {
        biased;
        () = cancel.cancelled() => {
            runtime.runs.finish(
                &call.id,
                RunState::Aborted,
                0,
                &Usage::default(),
                false,
            );
            return Err(format!(
                "Subagent `{}` was cancelled before it started.",
                args.description
            ));
        }
        permit = Arc::clone(&runtime.semaphore()) .acquire_owned() => permit,
    }
    .map_err(|_| {
        runtime
            .runs
            .finish(&call.id, RunState::Error, 0, &Usage::default(), false);
        "Subagent concurrency limiter is closed.".to_owned()
    })?;
    runtime.runs.admit(&call.id);
    let allow_task = child_depth <= config.max_depth;
    let child_toolbox = ChildToolbox {
        parent: Arc::clone(&runtime.parent_toolbox),
        runtime: Arc::clone(runtime),
        selection: agent_config.tools.clone(),
        allow_task,
    };
    let declarations = child_toolbox.declarations();

    let mut prompt_input = runtime.prompt.clone();
    prompt_input.tools = declarations.clone();
    prompt_input.append = Some(child_instructions(
        prompt_input.append.as_deref(),
        &agent_config,
    ));
    let system = build_system_message(&prompt_input, declarations);

    let (child_session, session_id) =
        match create_child_session(runtime, call, &args, child_depth, &model) {
            Ok(pair) => pair,
            Err(error) => {
                tracing::warn!(%error, "cannot create a child session; continuing in memory");
                (None, None)
            }
        };
    if let Some(id) = session_id.as_deref() {
        runtime.runs.attach_session(&call.id, id);
    }
    let mut child_session = child_session;
    if let Some(session) = child_session.as_mut() {
        if let Err(error) = session
            .append_message(&system)
            .and_then(|()| session.append_message(&Message::user(args.prompt.clone())))
        {
            tracing::warn!(%error, "cannot persist the child prompt");
        }
    }

    child_config.max_turns = agent_config.max_turns.unwrap_or(config.max_turns);
    child_config.thinking = runtime.thinking();
    child_config.session_id = session_id.clone();
    child_config.cancel = cancel.clone();
    child_config.delegation_depth = child_depth;
    child_config.agent = Some(args.agent.clone());
    child_config.max_output_tokens = runtime.max_output_tokens;
    child_config.temperature = runtime.temperature;

    // Children never receive extension loop hooks (`AGENT-026`).
    let agent = Agent::new(Arc::clone(&runtime.client), Arc::new(child_toolbox));
    let mut events = agent.run(
        Message::user(args.prompt.clone()),
        vec![system],
        child_config,
    );

    let timeout_limit = agent_config.timeout.unwrap_or(config.timeout);
    let outcome = drain_child(
        &mut events,
        child_session.as_mut(),
        &updates,
        session_id.clone(),
        &args.agent,
        child_depth,
        runtime,
        &call.id,
        timeout_limit,
        &cancel,
    )
    .await;
    drop(permit);

    runtime.budget.record(&outcome.usage);
    runtime.runs.finish(
        &call.id,
        state_from_status(outcome.status),
        outcome.turns,
        &outcome.usage,
        outcome.persisted,
    );
    if let Some(error) = outcome.error.as_deref() {
        runtime.runs.set_error(&call.id, error);
    }
    Ok(outcome)
}

/// Adds the delegated-run framing and the type persona to the parent append.
fn child_instructions(parent: Option<&str>, agent: &AgentTypeConfig) -> String {
    let mut text = String::new();
    if let Some(parent) = parent.map(str::trim).filter(|text| !text.is_empty()) {
        text.push_str(parent);
        text.push_str("\n\n");
    }
    text.push_str(
        "You are a subagent spawned by another session. You see only this prompt, not the parent \
         conversation. Complete the task, then reply with a concise summary the parent can act on.",
    );
    if let Some(prompt) = agent
        .prompt
        .as_deref()
        .map(str::trim)
        .filter(|prompt| !prompt.is_empty())
    {
        text.push_str("\n\n");
        text.push_str(prompt);
    }
    text
}

const fn state_from_status(status: RunStatus) -> RunState {
    match status {
        RunStatus::Completed => RunState::Completed,
        RunStatus::TurnLimit => RunState::TurnLimit,
        RunStatus::Aborted => RunState::Aborted,
        RunStatus::Error => RunState::Error,
    }
}

fn create_child_session(
    runtime: &SubagentRuntime,
    call: &ToolCall,
    args: &TaskArgs,
    depth: usize,
    model: &ModelRef,
) -> Result<(Option<Session>, Option<String>), String> {
    if !runtime.config().persist_sessions {
        return Ok((None, None));
    }
    let parent = runtime
        .parent_session_id()
        .unwrap_or_else(|| "ephemeral".to_owned());
    let mut session = Session::create_child(&runtime.prompt.cwd, &runtime.session_dir, &parent)
        .map_err(|error| error.to_string())?;
    let digest = digest_hex(args.prompt.as_bytes());
    session
        .append_custom(
            "subagent",
            json!({
                "call_id": call.id,
                "parent_session": parent,
                "agent": args.agent,
                "depth": depth,
                "model": model.model,
                "prompt_sha256": digest,
            }),
        )
        .map_err(|error| error.to_string())?;
    let id = session.id().to_owned();
    Ok((Some(session), Some(id)))
}

async fn drain_child(
    events: &mut AgentEventStream,
    mut session: Option<&mut Session>,
    updates: &Option<mpsc::UnboundedSender<ToolProgress>>,
    session_id: Option<String>,
    agent: &str,
    depth: usize,
    runtime: &SubagentRuntime,
    call_id: &str,
    timeout_limit: Duration,
    cancel: &CancellationToken,
) -> TaskOutcome {
    let mut status = RunStatus::Error;
    let mut saw_end = false;
    let mut summary = String::new();
    let mut error = None;
    let mut usage = Usage::default();
    let mut turns = 0u32;
    let mut last_update = Instant::now()
        .checked_sub(Duration::from_secs(2))
        .unwrap_or_else(Instant::now);
    let mut last_activity = String::new();

    let deadline = tokio::time::sleep(timeout_limit);
    tokio::pin!(deadline);
    let mut timed_out = false;
    loop {
        let event = tokio::select! {
            biased;
            () = &mut deadline => {
                timed_out = true;
                cancel.cancel();
                break;
            }
            event = events.recv() => event,
        };
        let Some(event) = event else {
            break;
        };
        match &event {
            AgentEvent::TurnStart => {
                turns = turns.saturating_add(1);
                send_progress(
                    updates,
                    &mut last_update,
                    &mut last_activity,
                    agent,
                    depth,
                    session_id.as_deref(),
                    turns,
                    "thinking",
                    runtime,
                    call_id,
                );
            }
            AgentEvent::ToolExecutionStart {
                tool_name, args, ..
            } => {
                let activity = activity_label(tool_name, args);
                send_progress(
                    updates,
                    &mut last_update,
                    &mut last_activity,
                    agent,
                    depth,
                    session_id.as_deref(),
                    turns,
                    &activity,
                    runtime,
                    call_id,
                );
            }
            AgentEvent::MessageEnd { message } => {
                if let Some(session) = session.as_deref_mut()
                    && let Err(error) = session.append_message(message)
                {
                    tracing::warn!(%error, "cannot persist a child message");
                }
                if let Message::Assistant(assistant) = message {
                    usage = add_usage(&usage, &assistant.usage);
                    if let Some(reason) = assistant.error_message.as_deref() {
                        error = Some(reason.to_owned());
                    }
                    let text = assistant.text();
                    if !text.trim().is_empty() {
                        summary = text;
                    }
                }
            }
            AgentEvent::AgentEnd { status: ended, .. } => {
                status = *ended;
                saw_end = true;
                break;
            }
            _ => {}
        }
    }

    if timed_out {
        status = RunStatus::Aborted;
        error = Some(format!(
            "timed out after {} seconds",
            timeout_limit.as_secs()
        ));
    } else if !saw_end {
        status = RunStatus::Error;
    }

    TaskOutcome {
        status,
        summary,
        session_id,
        agent: agent.to_owned(),
        depth,
        turns,
        usage,
        error,
        persisted: session.is_some(),
    }
}

/// Describes the child's current tool call in one short line.
fn activity_label(tool: &str, args: &serde_json::Value) -> String {
    let target = ["path", "command", "pattern", "query", "url"]
        .iter()
        .find_map(|key| args.get(*key).and_then(serde_json::Value::as_str))
        .map(str::trim)
        .filter(|target| !target.is_empty());
    let Some(target) = target else {
        return tool.to_owned();
    };
    let target = target.replace(['\n', '\r'], " ");
    let mut chars = target.chars();
    let short = chars.by_ref().take(48).collect::<String>();
    if chars.next().is_some() {
        format!("{tool} {short}…")
    } else {
        format!("{tool} {short}")
    }
}

#[allow(clippy::too_many_arguments)]
fn send_progress(
    updates: &Option<mpsc::UnboundedSender<ToolProgress>>,
    last_update: &mut Instant,
    last_activity: &mut String,
    agent: &str,
    depth: usize,
    session_id: Option<&str>,
    turns: u32,
    activity: &str,
    runtime: &SubagentRuntime,
    call_id: &str,
) {
    runtime.runs.note(call_id, Some(turns), Some(activity));
    let Some(sender) = updates else {
        return;
    };
    let changed = last_activity != activity;
    if !changed && last_update.elapsed() < Duration::from_secs(1) {
        return;
    }
    *last_update = Instant::now();
    *last_activity = activity.to_owned();
    let label = format!("turn {turns} · {activity}");
    let details = json!({
        "subagent": {
            "agent": agent,
            "depth": depth,
            "session_id": session_id,
            "turns": turns,
            "activity": activity,
        }
    });
    let _ = sender.send(ToolProgress {
        content: vec![Content::text(label)],
        details: Some(details),
    });
}

fn resolve_model(
    runtime: &SubagentRuntime,
    agent: &AgentTypeConfig,
    requested: Option<&str>,
) -> Result<ModelRef, String> {
    let parent = runtime.parent_model();
    let Some(requested) = requested.map(str::to_owned).or_else(|| agent.model.clone()) else {
        return Ok(parent);
    };
    let (provider, model) = match requested.split_once('/') {
        Some((provider, model)) => (provider, model),
        None => (parent.provider.as_str(), requested.as_str()),
    };
    if provider != parent.provider {
        return Err(format!(
            "Subagent model `{requested}` must use the active provider `{}`.",
            parent.provider
        ));
    }
    let known = runtime.known_models();
    if !known.is_empty() && !known.iter().any(|candidate| candidate == model) {
        return Err(format!(
            "Unknown subagent model `{model}` for provider `{}`.",
            parent.provider
        ));
    }
    Ok(ModelRef {
        api: parent.api.clone(),
        provider: provider.to_owned(),
        model: model.to_owned(),
    })
}

fn add_usage(total: &Usage, next: &Usage) -> Usage {
    let mut usage = Usage {
        input: total.input.saturating_add(next.input),
        output: total.output.saturating_add(next.output),
        cache_read: total.cache_read.saturating_add(next.cache_read),
        cache_write: total.cache_write.saturating_add(next.cache_write),
        reasoning: None,
        total_tokens: total.total_tokens.saturating_add(next.total_tokens),
        cost: codie_core::Cost {
            input: total.cost.input.saturating_add(next.cost.input),
            output: total.cost.output.saturating_add(next.cost.output),
            cache_read: total.cost.cache_read.saturating_add(next.cost.cache_read),
            cache_write: total.cost.cache_write.saturating_add(next.cost.cache_write),
            total: total.cost.total.saturating_add(next.cost.total),
            total_from_provider: total.cost.total_from_provider || next.cost.total_from_provider,
        },
    };
    usage.reasoning = match (total.reasoning, next.reasoning) {
        (Some(a), Some(b)) => Some(a.saturating_add(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    };
    usage
}

fn digest_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(output, "{byte:02x}");
    }
    output
}
