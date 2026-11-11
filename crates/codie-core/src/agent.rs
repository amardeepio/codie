//! The agent turn loop: stream an assistant message, run tools, repeat.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use futures_util::StreamExt;
use futures_util::stream::FuturesUnordered;
use tokio_util::sync::CancellationToken;

use crate::content::{Content, StopReason, ThinkingLevel, ToolCall};
use crate::event::{AgentEvent, AgentEventSender, AgentEventStream, AssistantDelta, RunStatus};
use crate::message::{AssistantMessage, Message, ToolResultMessage, now_ms};
use crate::tool::{ExecutionMode, Tool, ToolContext, ToolResult, Toolbox};

/// Model identity needed by the loop and by assistant messages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelRef {
    /// Wire API identifier.
    pub api: String,
    /// Provider identifier.
    pub provider: String,
    /// Model identifier.
    pub model: String,
}

/// Per-request options handed to a chat client.
#[derive(Debug, Clone)]
pub struct StreamOptions {
    /// Model identity.
    pub model: ModelRef,
    /// Requested thinking level.
    pub thinking: ThinkingLevel,
    /// Maximum output tokens.
    pub max_output_tokens: Option<u32>,
    /// Sampling temperature.
    pub temperature: Option<f32>,
    /// Session identifier for provider affinity.
    pub session_id: Option<String>,
    /// Cancellation signal.
    pub cancel: CancellationToken,
}

/// A streaming chat client.
pub trait ChatClient: Send + Sync {
    /// Streams one assistant response for the given transcript.
    fn stream(
        &self,
        context: &[Message],
        options: &StreamOptions,
    ) -> crate::event::AssistantEventStream;
}

/// The mutable context a hook receives.
#[derive(Debug, Clone, PartialEq)]
pub struct HookContext {
    /// Messages that would be sent to the model.
    pub messages: Vec<Message>,
    /// Model that would be used for the next request.
    pub model: ModelRef,
}

/// Optional loop hooks (`AGENT-010`).
///
/// Every method is fallible. An error from `transform_context`,
/// `prepare_request`, `convert_to_llm`, `prepare_next_turn`, or `finish_turn`
/// fails the run with a synthesized assistant error message. `before_tool`
/// blocks one call, and `after_tool` can replace a result. Hook failures never
/// panic the loop.
#[async_trait::async_trait]
pub trait Hooks: Send + Sync {
    /// Rewrites the initial transcript before the first request.
    async fn transform_context(&self, context: HookContext) -> Result<HookContext, String> {
        Ok(context)
    }

    /// Runs before each model request and may replace context or model.
    async fn prepare_request(&self, context: HookContext) -> Result<HookContext, String> {
        Ok(context)
    }

    /// Converts the transcript into the provider-visible form.
    async fn convert_to_llm(&self, context: HookContext) -> Result<HookContext, String> {
        Ok(context)
    }

    /// Runs before a new turn starts, after tool results are appended.
    async fn prepare_next_turn(&self, context: HookContext) -> Result<HookContext, String> {
        Ok(context)
    }

    /// Runs after a turn's `TurnEnd` event.
    async fn finish_turn(&self, context: HookContext) -> Result<HookContext, String> {
        Ok(context)
    }

    /// Runs before a tool executes. An error blocks the call.
    async fn before_tool(&self, _call: &ToolCall) -> Result<(), String> {
        Ok(())
    }

    /// Runs after a tool executes and may replace the result.
    async fn after_tool(&self, _call: &ToolCall, result: ToolResult) -> Result<ToolResult, String> {
        Ok(result)
    }
}

/// How queued messages are delivered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum QueueMode {
    /// Deliver every queued message at once.
    All,
    /// Deliver one message per turn.
    #[default]
    OneAtATime,
}

/// Configuration for one agent run.
#[derive(Debug, Clone)]
pub struct AgentConfig {
    /// Model identity.
    pub model: ModelRef,
    /// Working directory for tools.
    pub cwd: PathBuf,
    /// Thinking level.
    pub thinking: ThinkingLevel,
    /// Maximum assistant requests in one run.
    pub max_turns: usize,
    /// Batch scheduling mode.
    pub tool_execution: ExecutionMode,
    /// Session identifier.
    pub session_id: Option<String>,
    /// Maximum output tokens per request.
    pub max_output_tokens: Option<u32>,
    /// Sampling temperature.
    pub temperature: Option<f32>,
    /// Steering queue delivery mode.
    pub steering_mode: QueueMode,
    /// Follow-up queue delivery mode.
    pub follow_up_mode: QueueMode,
    /// Cancellation signal.
    pub cancel: CancellationToken,
}

impl AgentConfig {
    /// Creates a validated configuration.
    ///
    /// # Errors
    ///
    /// Returns an error when `max_turns` is zero.
    pub fn new(model: ModelRef, cwd: PathBuf) -> Result<Self, String> {
        if cwd.as_os_str().is_empty() {
            return Err("working directory must not be empty".to_owned());
        }
        Ok(Self {
            model,
            cwd,
            thinking: ThinkingLevel::default(),
            max_turns: 200,
            tool_execution: ExecutionMode::Parallel,
            session_id: None,
            max_output_tokens: None,
            temperature: None,
            steering_mode: QueueMode::OneAtATime,
            follow_up_mode: QueueMode::OneAtATime,
            cancel: CancellationToken::new(),
        })
    }
}

#[derive(Debug, Default)]
struct Queues {
    steering: VecDeque<Message>,
    follow_up: VecDeque<Message>,
}

/// A reusable agent with steering and follow-up queues.
pub struct Agent {
    client: Arc<dyn ChatClient>,
    toolbox: Arc<dyn Toolbox>,
    hooks: Option<Arc<dyn Hooks>>,
    queues: Arc<Mutex<Queues>>,
}

impl Agent {
    /// Creates an agent without hooks.
    #[must_use]
    pub fn new(client: Arc<dyn ChatClient>, toolbox: Arc<dyn Toolbox>) -> Self {
        Self {
            client,
            toolbox,
            hooks: None,
            queues: Arc::new(Mutex::new(Queues::default())),
        }
    }

    /// Creates an agent with loop hooks.
    #[must_use]
    pub fn with_hooks(
        client: Arc<dyn ChatClient>,
        toolbox: Arc<dyn Toolbox>,
        hooks: Arc<dyn Hooks>,
    ) -> Self {
        Self {
            client,
            toolbox,
            hooks: Some(hooks),
            queues: Arc::new(Mutex::new(Queues::default())),
        }
    }

    /// Queues a message for delivery during the current run.
    pub fn steer(&self, message: Message) {
        if let Ok(mut queues) = self.queues.lock() {
            queues.steering.push_back(message);
        }
    }

    /// Queues a message for after the current run would otherwise stop.
    pub fn follow_up(&self, message: Message) {
        if let Ok(mut queues) = self.queues.lock() {
            queues.follow_up.push_back(message);
        }
    }

    /// Drops all queued messages.
    pub fn clear_queues(&self) -> (Vec<Message>, Vec<Message>) {
        self.queues.lock().map_or_else(
            |_| (Vec::new(), Vec::new()),
            |mut queues| {
                (
                    queues.steering.drain(..).collect(),
                    queues.follow_up.drain(..).collect(),
                )
            },
        )
    }

    /// Starts a run and returns its event stream.
    pub fn run(
        &self,
        prompt: Message,
        history: Vec<Message>,
        config: AgentConfig,
    ) -> AgentEventStream {
        let (stream, sender) = AgentEventStream::channel(256);
        let client = Arc::clone(&self.client);
        let toolbox = Arc::clone(&self.toolbox);
        let hooks = self.hooks.clone();
        let queues = Arc::clone(&self.queues);
        let steering = take_shared(&queues, QueueKind::Steering, config.steering_mode);

        tokio::spawn(async move {
            run_inner(
                client, toolbox, hooks, prompt, history, config, steering, queues, sender,
            )
            .await;
        });

        stream
    }
}

async fn run_inner(
    client: Arc<dyn ChatClient>,
    toolbox: Arc<dyn Toolbox>,
    hooks: Option<Arc<dyn Hooks>>,
    prompt: Message,
    history: Vec<Message>,
    mut config: AgentConfig,
    initial_steering: Vec<Message>,
    queues: Arc<Mutex<Queues>>,
    sender: AgentEventSender,
) {
    let mut context = history;
    context.push(prompt.clone());
    context.extend(initial_steering.clone());
    let hooks = hooks.as_deref();

    if config.max_turns == 0 {
        finish(&sender, Vec::new(), RunStatus::TurnLimit).await;
        return;
    }

    if let Some(hooks) = hooks {
        match hooks
            .transform_context(HookContext {
                messages: context.clone(),
                model: config.model.clone(),
            })
            .await
        {
            Ok(transformed) => {
                context = transformed.messages;
                config.model = transformed.model;
            }
            Err(error) => {
                fail_hook(&sender, context, &config, error).await;
                return;
            }
        }
    }

    emit(&sender, AgentEvent::AgentStart).await;
    emit(&sender, AgentEvent::TurnStart).await;
    for message in std::iter::once(prompt).chain(initial_steering) {
        emit(
            &sender,
            AgentEvent::MessageStart {
                message: message.clone(),
            },
        )
        .await;
        emit(&sender, AgentEvent::MessageEnd { message }).await;
    }

    let mut turns = 0_usize;
    loop {
        if config.cancel.is_cancelled() {
            finish(&sender, context, RunStatus::Aborted).await;
            return;
        }
        if turns >= config.max_turns {
            finish(&sender, context, RunStatus::TurnLimit).await;
            return;
        }
        turns += 1;

        if turns > 1 {
            if let Some(hooks) = hooks {
                match hooks
                    .prepare_next_turn(HookContext {
                        messages: context.clone(),
                        model: config.model.clone(),
                    })
                    .await
                {
                    Ok(prepared) => {
                        context = prepared.messages;
                        config.model = prepared.model;
                    }
                    Err(error) => {
                        fail_hook(&sender, context, &config, error).await;
                        return;
                    }
                }
            }
            emit(&sender, AgentEvent::TurnStart).await;
        }

        if let Some(hooks) = hooks {
            let prepared = hooks.prepare_request(HookContext {
                messages: context.clone(),
                model: config.model.clone(),
            });
            let prepared = match prepared.await {
                Ok(prepared) => prepared,
                Err(error) => {
                    fail_hook(&sender, context, &config, error).await;
                    return;
                }
            };
            let converted = match hooks.convert_to_llm(prepared).await {
                Ok(converted) => converted,
                Err(error) => {
                    fail_hook(&sender, context, &config, error).await;
                    return;
                }
            };
            context = converted.messages;
            config.model = converted.model;
        }

        let options = StreamOptions {
            model: config.model.clone(),
            thinking: config.thinking,
            max_output_tokens: config.max_output_tokens,
            temperature: config.temperature,
            session_id: config.session_id.clone(),
            cancel: config.cancel.clone(),
        };

        let message = match stream_assistant(&client, &context, &options, &sender).await {
            Some(message) => message,
            None => {
                finish(&sender, context, RunStatus::Error).await;
                return;
            }
        };

        let assistant_message = Message::Assistant(message.clone());
        context.push(assistant_message.clone());
        emit(
            &sender,
            AgentEvent::MessageEnd {
                message: assistant_message,
            },
        )
        .await;

        if message.stop_reason.is_terminal_failure() {
            let status = if message.stop_reason == StopReason::Aborted {
                RunStatus::Aborted
            } else {
                RunStatus::Error
            };
            emit(
                &sender,
                AgentEvent::TurnEnd {
                    message: Message::Assistant(message),
                    tool_results: Vec::new(),
                },
            )
            .await;
            if let Some(hooks) = hooks {
                match hooks
                    .finish_turn(HookContext {
                        messages: context.clone(),
                        model: config.model.clone(),
                    })
                    .await
                {
                    Ok(finished) => {
                        context = finished.messages;
                        config.model = finished.model;
                    }
                    Err(error) => {
                        fail_hook(&sender, context, &config, error).await;
                        return;
                    }
                }
            }
            finish(&sender, context, status).await;
            return;
        }

        let calls = message.tool_calls();
        let results = if calls.is_empty() {
            Vec::new()
        } else if message.stop_reason == StopReason::Length {
            truncated_results(&calls)
        } else {
            execute_tools(&calls, &config, toolbox.as_ref(), hooks, &sender).await
        };

        for result in &results {
            let message = Message::ToolResult(result.clone());
            context.push(message.clone());
            emit(
                &sender,
                AgentEvent::MessageStart {
                    message: message.clone(),
                },
            )
            .await;
            emit(&sender, AgentEvent::MessageEnd { message }).await;
        }

        emit(
            &sender,
            AgentEvent::TurnEnd {
                message: Message::Assistant(message.clone()),
                tool_results: results.clone(),
            },
        )
        .await;

        if let Some(hooks) = hooks {
            match hooks
                .finish_turn(HookContext {
                    messages: context.clone(),
                    model: config.model.clone(),
                })
                .await
            {
                Ok(finished) => {
                    context = finished.messages;
                    config.model = finished.model;
                }
                Err(error) => {
                    fail_hook(&sender, context, &config, error).await;
                    return;
                }
            }
        }

        if config.cancel.is_cancelled() {
            finish(&sender, context, RunStatus::Aborted).await;
            return;
        }

        let steering = take_shared(&queues, QueueKind::Steering, config.steering_mode);
        if !steering.is_empty() {
            for message in steering {
                context.push(message.clone());
                emit(
                    &sender,
                    AgentEvent::MessageStart {
                        message: message.clone(),
                    },
                )
                .await;
                emit(&sender, AgentEvent::MessageEnd { message }).await;
            }
            continue;
        }

        if !calls.is_empty() {
            continue;
        }

        let follow_up = take_shared(&queues, QueueKind::FollowUp, config.follow_up_mode);
        if !follow_up.is_empty() {
            for message in follow_up {
                context.push(message.clone());
                emit(
                    &sender,
                    AgentEvent::MessageStart {
                        message: message.clone(),
                    },
                )
                .await;
                emit(&sender, AgentEvent::MessageEnd { message }).await;
            }
            continue;
        }

        finish(&sender, context, RunStatus::Completed).await;
        return;
    }
}

async fn stream_assistant(
    client: &Arc<dyn ChatClient>,
    context: &[Message],
    options: &StreamOptions,
    sender: &AgentEventSender,
) -> Option<AssistantMessage> {
    let mut stream = client.stream(context, options);
    let mut partial: Option<AssistantMessage> = None;

    while let Some(delta) = stream.recv().await {
        match &delta {
            AssistantDelta::Start { partial: started } => {
                partial = Some(started.clone());
                emit(
                    sender,
                    AgentEvent::MessageStart {
                        message: Message::Assistant(started.clone()),
                    },
                )
                .await;
            }
            AssistantDelta::Done { message, .. } | AssistantDelta::Error { message, .. } => {
                return Some(message.clone());
            }
            _ => {
                if let Some(current) = partial.as_mut() {
                    apply_delta(current, &delta);
                    emit(
                        sender,
                        AgentEvent::MessageUpdate {
                            message: Message::Assistant(current.clone()),
                            delta,
                        },
                    )
                    .await;
                }
            }
        }
    }

    if let Some(mut current) = partial {
        if current.stop_reason == StopReason::Pending {
            current.stop_reason = StopReason::Error;
            current.error_message =
                Some("provider stream ended without a terminal event".to_owned());
        }
        Some(current)
    } else {
        Some(AssistantMessage::error(
            options.model.api.clone(),
            options.model.provider.clone(),
            options.model.model.clone(),
            StopReason::Error,
            "provider produced no response",
        ))
    }
}

fn apply_delta(message: &mut AssistantMessage, delta: &AssistantDelta) {
    match delta {
        AssistantDelta::TextStart { index } => {
            ensure_slot(message, *index);
            message.content[*index] = Content::Text {
                text: String::new(),
                signature: None,
            };
        }
        AssistantDelta::TextDelta { index, delta } => {
            ensure_slot(message, *index);
            match &mut message.content[*index] {
                Content::Text { text, .. } => text.push_str(delta),
                _ => {
                    message.content[*index] = Content::text(delta.clone());
                }
            }
        }
        AssistantDelta::TextEnd { index, content } => {
            ensure_slot(message, *index);
            message.content[*index] = Content::Text {
                text: content.clone(),
                signature: None,
            };
        }
        AssistantDelta::ThinkingStart { index } => {
            ensure_slot(message, *index);
            message.content[*index] = Content::Thinking {
                thinking: String::new(),
                signature: None,
                redacted: false,
            };
        }
        AssistantDelta::ThinkingDelta { index, delta } => {
            ensure_slot(message, *index);
            match &mut message.content[*index] {
                Content::Thinking { thinking, .. } => thinking.push_str(delta),
                _ => {
                    message.content[*index] = Content::thinking(delta.clone());
                }
            }
        }
        AssistantDelta::ThinkingEnd { index, content } => {
            ensure_slot(message, *index);
            message.content[*index] = Content::Thinking {
                thinking: content.clone(),
                signature: None,
                redacted: false,
            };
        }
        AssistantDelta::ToolCallStart { index } => {
            ensure_slot(message, *index);
        }
        AssistantDelta::ToolCallDelta { .. } => {}
        AssistantDelta::ToolCallEnd { index, call } => {
            ensure_slot(message, *index);
            message.content[*index] = Content::ToolCall {
                id: call.id.clone(),
                name: call.name.clone(),
                arguments: call.arguments.clone(),
                thought_signature: None,
            };
        }
        AssistantDelta::Start { .. }
        | AssistantDelta::Done { .. }
        | AssistantDelta::Error { .. } => {}
    }
}

fn ensure_slot(message: &mut AssistantMessage, index: usize) {
    while message.content.len() <= index {
        message.content.push(Content::text(""));
    }
}

fn truncated_results(calls: &[ToolCall]) -> Vec<ToolResultMessage> {
    calls
        .iter()
        .map(|call| ToolResultMessage {
            tool_call_id: call.id.clone(),
            tool_name: call.name.clone(),
            content: vec![Content::text(format!(
                "Tool call `{}` was not executed because the assistant response hit the output token limit. Re-issue the call with complete arguments.",
                call.name
            ))],
            details: None,
            is_error: true,
            duration_ms: Some(0),
            timestamp: now_ms(),
        })
        .collect()
}

async fn execute_tools(
    calls: &[ToolCall],
    config: &AgentConfig,
    toolbox: &dyn Toolbox,
    hooks: Option<&dyn Hooks>,
    sender: &AgentEventSender,
) -> Vec<ToolResultMessage> {
    let sequential = config.tool_execution == ExecutionMode::Sequential
        || calls.iter().any(|call| {
            toolbox
                .get(&call.name)
                .is_some_and(|tool| tool.execution_mode() == ExecutionMode::Sequential)
        });

    if sequential {
        let mut results = Vec::with_capacity(calls.len());
        for call in calls {
            let result = execute_one(call, config, toolbox, hooks, sender, true).await;
            results.push(result_message(call, &result));
        }
        return results;
    }

    let mut handles = Vec::with_capacity(calls.len());
    let mut results: Vec<Option<ToolResultMessage>> = vec![None; calls.len()];

    for (index, call) in calls.iter().enumerate() {
        emit(
            sender,
            AgentEvent::ToolExecutionStart {
                tool_call_id: call.id.clone(),
                tool_name: call.name.clone(),
                args: call.arguments.clone(),
            },
        )
        .await;

        if let Some(reason) = toolbox.denial(&call.name) {
            let result = ToolResult::error(format!("Tool `{}` was denied: {reason}", call.name))
                .with_duration(0);
            emit_tool_end(sender, call, &result).await;
            results[index] = Some(result_message(call, &result));
            continue;
        }

        let Some(tool) = toolbox.get(&call.name) else {
            let result =
                ToolResult::error(format!("Tool `{}` not found", call.name)).with_duration(0);
            emit_tool_end(sender, call, &result).await;
            results[index] = Some(result_message(call, &result));
            continue;
        };

        if let Some(hooks) = hooks
            && let Err(reason) = hooks.before_tool(call).await
        {
            let result = blocked_by_hook(&call.name, &reason);
            emit_tool_end(sender, call, &result).await;
            results[index] = Some(result_message(call, &result));
            continue;
        }

        let call_clone = call.clone();
        let context = ToolContext {
            cwd: config.cwd.clone(),
            cancel: config.cancel.clone(),
            session_id: config.session_id.clone(),
        };
        let sender_clone = sender.clone();
        handles.push(async move {
            let started = Instant::now();
            let result = run_tool_with_hooks(hooks, tool, &call_clone, &context).await;
            let duration = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
            let result = result.with_duration(duration);
            emit_tool_end(&sender_clone, &call_clone, &result).await;
            (index, result)
        });
    }

    let mut pending = handles.into_iter().collect::<FuturesUnordered<_>>();
    while let Some((index, result)) = pending.next().await {
        results[index] = Some(result_message(&calls[index], &result));
    }

    results.into_iter().flatten().collect()
}

async fn execute_one(
    call: &ToolCall,
    config: &AgentConfig,
    toolbox: &dyn Toolbox,
    hooks: Option<&dyn Hooks>,
    sender: &AgentEventSender,
    emit_events: bool,
) -> ToolResult {
    if emit_events {
        emit(
            sender,
            AgentEvent::ToolExecutionStart {
                tool_call_id: call.id.clone(),
                tool_name: call.name.clone(),
                args: call.arguments.clone(),
            },
        )
        .await;
    }

    let result = if let Some(reason) = toolbox.denial(&call.name) {
        ToolResult::error(format!("Tool `{}` was denied: {reason}", call.name))
    } else if let Some(hooks) = hooks
        && let Err(reason) = hooks.before_tool(call).await
    {
        blocked_by_hook(&call.name, &reason)
    } else if let Some(tool) = toolbox.get(&call.name) {
        let context = ToolContext {
            cwd: config.cwd.clone(),
            cancel: config.cancel.clone(),
            session_id: config.session_id.clone(),
        };
        run_tool_with_hooks(hooks, tool, call, &context).await
    } else {
        ToolResult::error(format!("Tool `{}` not found", call.name))
    }
    .with_duration(0);

    if emit_events {
        emit_tool_end(sender, call, &result).await;
    }
    result
}

async fn execute_tool(tool: Arc<dyn Tool>, call: &ToolCall, context: &ToolContext) -> ToolResult {
    use futures_util::FutureExt as _;
    use std::panic::AssertUnwindSafe;

    match AssertUnwindSafe(tool.execute(call, context, None))
        .catch_unwind()
        .await
    {
        Ok(result) => result,
        Err(_) => ToolResult::error("Tool panicked while executing; the call was aborted")
            .with_duration(0),
    }
}

async fn run_tool_with_hooks(
    hooks: Option<&dyn Hooks>,
    tool: Arc<dyn Tool>,
    call: &ToolCall,
    context: &ToolContext,
) -> ToolResult {
    let result = execute_tool(tool, call, context).await;
    let Some(hooks) = hooks else {
        return result;
    };
    match hooks.after_tool(call, result).await {
        Ok(result) => result,
        Err(reason) => ToolResult::error(format!(
            "Tool `{}` failed after execution: {reason}",
            call.name
        ))
        .with_duration(0),
    }
}

fn blocked_by_hook(name: &str, reason: &str) -> ToolResult {
    ToolResult::error(format!("Tool `{name}` was blocked by a hook: {reason}")).with_duration(0)
}

/// Emits a synthesized assistant error and ends the run after a hook failure.
async fn fail_hook(
    sender: &AgentEventSender,
    context: Vec<Message>,
    config: &AgentConfig,
    error: String,
) {
    let message = Message::Assistant(AssistantMessage::error(
        config.model.api.clone(),
        config.model.provider.clone(),
        config.model.model.clone(),
        StopReason::Error,
        format!("hook failed: {error}"),
    ));
    let mut messages = context;
    emit(
        sender,
        AgentEvent::MessageStart {
            message: message.clone(),
        },
    )
    .await;
    messages.push(message.clone());
    emit(sender, AgentEvent::MessageEnd { message }).await;
    finish(sender, messages, RunStatus::Error).await;
}

async fn emit_tool_end(sender: &AgentEventSender, call: &ToolCall, result: &ToolResult) {
    emit(
        sender,
        AgentEvent::ToolExecutionEnd {
            tool_call_id: call.id.clone(),
            tool_name: call.name.clone(),
            result: result.clone(),
            is_error: result.is_error,
            duration_ms: result.duration_ms,
        },
    )
    .await;
}

fn result_message(call: &ToolCall, result: &ToolResult) -> ToolResultMessage {
    ToolResultMessage {
        tool_call_id: call.id.clone(),
        tool_name: call.name.clone(),
        content: result.content.clone(),
        details: result.details.clone(),
        is_error: result.is_error,
        duration_ms: result.duration_ms,
        timestamp: now_ms(),
    }
}

async fn emit(sender: &AgentEventSender, event: AgentEvent) {
    let _ = sender.send(event).await;
}

async fn finish(sender: &AgentEventSender, messages: Vec<Message>, status: RunStatus) {
    emit(sender, AgentEvent::AgentEnd { messages, status }).await;
}

fn take_queued(queue: &mut VecDeque<Message>, mode: QueueMode) -> Vec<Message> {
    match mode {
        QueueMode::All => queue.drain(..).collect(),
        QueueMode::OneAtATime => queue.pop_front().into_iter().collect(),
    }
}

#[derive(Debug, Clone, Copy)]
enum QueueKind {
    Steering,
    FollowUp,
}

fn take_shared(queues: &Arc<Mutex<Queues>>, kind: QueueKind, mode: QueueMode) -> Vec<Message> {
    let Ok(mut guard) = queues.lock() else {
        return Vec::new();
    };
    let queue = match kind {
        QueueKind::Steering => &mut guard.steering,
        QueueKind::FollowUp => &mut guard.follow_up,
    };
    take_queued(queue, mode)
}
