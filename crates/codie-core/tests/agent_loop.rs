//! Conformance tests for the Phase 1 agent turn loop (AGENT-001..AGENT-009).
//!
//! Every test uses a scripted [`FakeClient`] and fake [`Toolbox`] so the loop
//! runs without a provider, network, or filesystem access.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use codie_core::{
    Agent, AgentConfig, AgentEvent, AgentEventStream, AssistantDelta, AssistantEventStream,
    AssistantMessage, AssistantStreamSender, ChatClient, Content, ExecutionMode, HookContext,
    Hooks, Message, ModelRef, QueueMode, RunStatus, StopReason, StreamOptions, Tool, ToolCall,
    ToolContext, ToolDecl, ToolProgress, ToolResult, ToolResultMessage, Toolbox, Usage,
};
use codie_testkit::spec;
use serde_json::{Value, json};
use tokio::sync::{Notify, mpsc};

const API: &str = "test-api";
const PROVIDER: &str = "test-provider";
const MODEL: &str = "test-model";

// ---------------------------------------------------------------------------
// Scripted chat client
// ---------------------------------------------------------------------------

/// One provider response scripted for a single request.
#[derive(Debug, Clone)]
enum Script {
    /// Plain text response that stops normally.
    Text(String),
    /// A tool-use turn with the given calls, in source order.
    ToolCalls(Vec<ToolCall>),
    /// A truncated turn (`stop_reason = length`) that carries tool calls.
    LengthToolCalls(Vec<ToolCall>),
    /// A failed request: terminal `error` without a preceding `start`.
    Error(String),
    /// Starts streaming, then aborts when the run's cancellation token fires.
    AbortOnCancel,
}

struct FakeClient {
    scripts: Mutex<VecDeque<Script>>,
    requests: Mutex<Vec<Vec<Message>>>,
    started: Arc<Notify>,
}

impl FakeClient {
    fn new(scripts: Vec<Script>) -> Self {
        Self {
            scripts: Mutex::new(scripts.into()),
            requests: Mutex::new(Vec::new()),
            started: Arc::new(Notify::new()),
        }
    }

    fn requests(&self) -> Vec<Vec<Message>> {
        self.requests.lock().expect("request log poisoned").clone()
    }

    fn request_count(&self) -> usize {
        self.requests.lock().expect("request log poisoned").len()
    }
}

impl ChatClient for FakeClient {
    fn stream(&self, context: &[Message], options: &StreamOptions) -> AssistantEventStream {
        self.requests
            .lock()
            .expect("request log poisoned")
            .push(context.to_vec());

        let script = self
            .scripts
            .lock()
            .expect("script queue poisoned")
            .pop_front()
            .unwrap_or_else(|| Script::Error("unexpected request without a script".to_owned()));

        let (stream, sender) = AssistantEventStream::channel(64);
        let started = Arc::clone(&self.started);
        let cancel = options.cancel.clone();

        tokio::spawn(async move {
            match script {
                Script::Text(text) => emit_text(&sender, &text, Usage::new(1, 2, 0, 0)).await,
                Script::ToolCalls(calls) => {
                    emit_tool_calls(&sender, calls, StopReason::ToolUse).await;
                }
                Script::LengthToolCalls(calls) => {
                    emit_tool_calls(&sender, calls, StopReason::Length).await;
                }
                Script::Error(message) => {
                    let message =
                        AssistantMessage::error(API, PROVIDER, MODEL, StopReason::Error, message);
                    sender
                        .send(AssistantDelta::Error {
                            reason: StopReason::Error,
                            message,
                        })
                        .await;
                }
                Script::AbortOnCancel => {
                    sender
                        .send(AssistantDelta::Start {
                            partial: AssistantMessage::partial(API, PROVIDER, MODEL),
                        })
                        .await;
                    sender.send(AssistantDelta::TextStart { index: 0 }).await;
                    sender
                        .send(AssistantDelta::TextDelta {
                            index: 0,
                            delta: "partial".to_owned(),
                        })
                        .await;
                    started.notify_one();
                    cancel.cancelled().await;
                    sender
                        .send(AssistantDelta::TextEnd {
                            index: 0,
                            content: "partial".to_owned(),
                        })
                        .await;
                    let mut message = AssistantMessage::partial(API, PROVIDER, MODEL);
                    message.content = vec![Content::text("partial")];
                    message.stop_reason = StopReason::Aborted;
                    sender
                        .send(AssistantDelta::Error {
                            reason: StopReason::Aborted,
                            message,
                        })
                        .await;
                }
            }
        });

        stream
    }
}

async fn emit_text(sender: &AssistantStreamSender, text: &str, usage: Usage) {
    sender
        .send(AssistantDelta::Start {
            partial: AssistantMessage::partial(API, PROVIDER, MODEL),
        })
        .await;
    sender.send(AssistantDelta::TextStart { index: 0 }).await;
    if !text.is_empty() {
        sender
            .send(AssistantDelta::TextDelta {
                index: 0,
                delta: text.to_owned(),
            })
            .await;
        sender
            .send(AssistantDelta::TextEnd {
                index: 0,
                content: text.to_owned(),
            })
            .await;
    }
    let mut message = AssistantMessage::partial(API, PROVIDER, MODEL);
    message.content = vec![Content::text(text)];
    message.usage = usage;
    message.stop_reason = StopReason::Stop;
    sender
        .send(AssistantDelta::Done {
            reason: StopReason::Stop,
            message,
        })
        .await;
}

async fn emit_tool_calls(sender: &AssistantStreamSender, calls: Vec<ToolCall>, reason: StopReason) {
    sender
        .send(AssistantDelta::Start {
            partial: AssistantMessage::partial(API, PROVIDER, MODEL),
        })
        .await;

    let mut content = Vec::with_capacity(calls.len());
    for (index, call) in calls.iter().enumerate() {
        sender.send(AssistantDelta::ToolCallStart { index }).await;
        sender
            .send(AssistantDelta::ToolCallDelta {
                index,
                delta: call.arguments.to_string(),
            })
            .await;
        sender
            .send(AssistantDelta::ToolCallEnd {
                index,
                call: call.clone(),
            })
            .await;
        content.push(Content::ToolCall {
            id: call.id.clone(),
            name: call.name.clone(),
            arguments: call.arguments.clone(),
            thought_signature: None,
        });
    }

    let mut message = AssistantMessage::partial(API, PROVIDER, MODEL);
    message.content = content;
    message.usage = Usage::new(1, 1, 0, 0);
    message.stop_reason = reason;
    sender.send(AssistantDelta::Done { reason, message }).await;
}

// ---------------------------------------------------------------------------
// Fake tools
// ---------------------------------------------------------------------------

enum ToolOutcome {
    Text(String),
    Error(String),
    WaitForCancel,
}

struct FakeTool {
    name: &'static str,
    description: &'static str,
    outcome: ToolOutcome,
    delay: Duration,
    mode: ExecutionMode,
    gate_in: Option<Arc<AtomicBool>>,
    gate_out: Option<Arc<AtomicBool>>,
    on_execute: Option<Arc<dyn Fn() + Send + Sync>>,
    calls: Arc<AtomicUsize>,
}

impl FakeTool {
    fn new(name: &'static str) -> Self {
        Self {
            name,
            description: "fake tool",
            outcome: ToolOutcome::Text(format!("{name} ok")),
            delay: Duration::ZERO,
            mode: ExecutionMode::Parallel,
            gate_in: None,
            gate_out: None,
            on_execute: None,
            calls: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn error(name: &'static str, message: &str) -> Self {
        Self {
            outcome: ToolOutcome::Error(message.to_owned()),
            ..Self::new(name)
        }
    }

    fn wait_for_cancel(name: &'static str) -> Self {
        Self {
            outcome: ToolOutcome::WaitForCancel,
            ..Self::new(name)
        }
    }

    fn with_delay(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }

    fn sequential(mut self) -> Self {
        self.mode = ExecutionMode::Sequential;
        self
    }

    fn waits_for(mut self, gate: Arc<AtomicBool>) -> Self {
        self.gate_in = Some(gate);
        self
    }

    fn opens(mut self, gate: Arc<AtomicBool>) -> Self {
        self.gate_out = Some(gate);
        self
    }

    fn on_execute(mut self, callback: impl Fn() + Send + Sync + 'static) -> Self {
        self.on_execute = Some(Arc::new(callback));
        self
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl Tool for FakeTool {
    fn name(&self) -> &str {
        self.name
    }

    fn description(&self) -> &str {
        self.description
    }

    fn schema(&self) -> Value {
        json!({"type": "object", "additionalProperties": true})
    }

    fn execution_mode(&self) -> ExecutionMode {
        self.mode
    }

    async fn execute(
        &self,
        _call: &ToolCall,
        context: &ToolContext,
        _updates: Option<mpsc::UnboundedSender<ToolProgress>>,
    ) -> ToolResult {
        self.calls.fetch_add(1, Ordering::SeqCst);

        if let Some(callback) = &self.on_execute {
            callback();
        }

        if let Some(gate) = &self.gate_in {
            for _ in 0..1_000_000 {
                if gate.load(Ordering::SeqCst) {
                    break;
                }
                tokio::task::yield_now().await;
            }
        }

        if !self.delay.is_zero() {
            tokio::time::sleep(self.delay).await;
        }

        if let Some(gate) = &self.gate_out {
            gate.store(true, Ordering::SeqCst);
        }

        match &self.outcome {
            ToolOutcome::Text(text) => ToolResult::text(text.clone()),
            ToolOutcome::Error(text) => ToolResult::error(text.clone()),
            ToolOutcome::WaitForCancel => {
                context.cancel.cancelled().await;
                ToolResult::error("aborted by cancellation")
            }
        }
    }
}

struct FakeToolbox {
    tools: HashMap<String, Arc<dyn Tool>>,
    denials: HashMap<String, String>,
}

impl FakeToolbox {
    fn new(tools: Vec<Arc<dyn Tool>>) -> Self {
        let tools = tools
            .into_iter()
            .map(|tool| (tool.name().to_owned(), tool))
            .collect();
        Self {
            tools,
            denials: HashMap::new(),
        }
    }

    fn denying(mut self, name: &str, reason: &str) -> Self {
        self.denials.insert(name.to_owned(), reason.to_owned());
        self
    }
}

impl Toolbox for FakeToolbox {
    fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.get(name).cloned()
    }

    fn declarations(&self) -> Vec<ToolDecl> {
        let mut declarations: Vec<ToolDecl> = self
            .tools
            .values()
            .map(|tool| ToolDecl {
                name: tool.name().to_owned(),
                description: tool.description().to_owned(),
                parameters: tool.schema(),
            })
            .collect();
        declarations.sort_by(|left, right| left.name.cmp(&right.name));
        declarations
    }

    fn denial(&self, name: &str) -> Option<String> {
        self.denials.get(name).cloned()
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn model_ref() -> ModelRef {
    ModelRef {
        api: API.to_owned(),
        provider: PROVIDER.to_owned(),
        model: MODEL.to_owned(),
    }
}

fn run_config() -> AgentConfig {
    let mut config = AgentConfig::new(model_ref(), PathBuf::from(".")).expect("valid config");
    config.max_turns = 10;
    config
}

fn text(text: &str) -> Script {
    Script::Text(text.to_owned())
}

fn call(id: &str, name: &str) -> ToolCall {
    ToolCall {
        id: id.to_owned(),
        name: name.to_owned(),
        arguments: json!({}),
    }
}

fn toolbox(tools: Vec<Arc<dyn Tool>>) -> Arc<FakeToolbox> {
    Arc::new(FakeToolbox::new(tools))
}

/// Coerces a concrete fake client to the trait object accepted by `Agent::new`.
fn as_chat_client(client: &Arc<FakeClient>) -> Arc<dyn ChatClient> {
    let client: Arc<dyn ChatClient> = client.clone();
    client
}

fn user_texts(messages: &[Message]) -> Vec<String> {
    messages
        .iter()
        .filter_map(|message| match message {
            Message::User(user) => Some(
                user.content
                    .iter()
                    .filter_map(Content::as_text)
                    .collect::<Vec<_>>()
                    .join(""),
            ),
            _ => None,
        })
        .collect()
}

fn assistant_messages(messages: &[Message]) -> Vec<&AssistantMessage> {
    messages.iter().filter_map(Message::as_assistant).collect()
}

fn first_turn_results(events: &[AgentEvent]) -> Vec<ToolResultMessage> {
    events
        .iter()
        .find_map(|event| match event {
            AgentEvent::TurnEnd { tool_results, .. } => Some(tool_results.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

fn tool_end_ids(events: &[AgentEvent]) -> Vec<String> {
    events
        .iter()
        .filter_map(|event| match event {
            AgentEvent::ToolExecutionEnd { tool_call_id, .. } => Some(tool_call_id.clone()),
            _ => None,
        })
        .collect()
}

fn tool_start_count(events: &[AgentEvent]) -> usize {
    events
        .iter()
        .filter(|event| matches!(event, AgentEvent::ToolExecutionStart { .. }))
        .count()
}

fn turn_end_count(events: &[AgentEvent]) -> usize {
    events
        .iter()
        .filter(|event| matches!(event, AgentEvent::TurnEnd { .. }))
        .count()
}

fn tool_result_message_ids(events: &[AgentEvent]) -> Vec<String> {
    events
        .iter()
        .filter_map(|event| match event {
            AgentEvent::MessageStart {
                message: Message::ToolResult(result),
            } => Some(result.tool_call_id.clone()),
            _ => None,
        })
        .collect()
}

fn agent_end(events: &[AgentEvent]) -> (Vec<Message>, RunStatus) {
    match events.last() {
        Some(AgentEvent::AgentEnd { messages, status }) => (messages.clone(), *status),
        other => panic!("expected AgentEnd as the last event, got {other:?}"),
    }
}

/// Drains a run until `AgentEnd` and verifies nothing follows it.
async fn collect_run(mut stream: AgentEventStream) -> Vec<AgentEvent> {
    let mut events = Vec::new();
    while let Some(event) = stream.recv().await {
        let is_end = matches!(event, AgentEvent::AgentEnd { .. });
        events.push(event);
        if is_end {
            assert!(
                stream.recv().await.is_none(),
                "an agent event was emitted after AgentEnd"
            );
            break;
        }
    }
    assert!(
        matches!(events.last(), Some(AgentEvent::AgentEnd { .. })),
        "run did not end with AgentEnd: {events:?}"
    );
    events
}

fn position<F>(events: &[AgentEvent], predicate: F) -> Option<usize>
where
    F: Fn(&AgentEvent) -> bool,
{
    events.iter().position(predicate)
}

// ---------------------------------------------------------------------------
// AGENT-001: AgentEnd is always last
// ---------------------------------------------------------------------------

#[tokio::test]
#[spec("AGENT-001")]
async fn spec_agent_001_agent_end_is_last() {
    // Completed run.
    let client = Arc::new(FakeClient::new(vec![text("done")]));
    let agent = Agent::new(client, toolbox(Vec::new()));
    let completed = collect_run(agent.run(Message::user("hi"), Vec::new(), run_config())).await;
    let (_, status) = agent_end(&completed);
    assert_eq!(status, RunStatus::Completed);

    // Provider error run.
    let client = Arc::new(FakeClient::new(vec![Script::Error("boom".to_owned())]));
    let agent = Agent::new(client, toolbox(Vec::new()));
    let failed = collect_run(agent.run(Message::user("hi"), Vec::new(), run_config())).await;
    let (messages, status) = agent_end(&failed);
    assert_eq!(status, RunStatus::Error);
    let assistants = assistant_messages(&messages);
    assert_eq!(assistants.len(), 1);
    assert_eq!(assistants[0].stop_reason, StopReason::Error);
    assert_eq!(assistants[0].error_message.as_deref(), Some("boom"));

    // Aborted run.
    let client = Arc::new(FakeClient::new(vec![Script::AbortOnCancel]));
    let agent = Agent::new(as_chat_client(&client), toolbox(Vec::new()));
    let config = run_config();
    let cancel = config.cancel.clone();
    let stream = agent.run(Message::user("hi"), Vec::new(), config);
    client.started.notified().await;
    cancel.cancel();
    let aborted = collect_run(stream).await;
    let (_, status) = agent_end(&aborted);
    assert_eq!(status, RunStatus::Aborted);
}

// ---------------------------------------------------------------------------
// AGENT-013: continuation retries without duplicating the prompt
// ---------------------------------------------------------------------------

#[tokio::test]
#[spec("AGENT-013")]
async fn spec_agent_013_resume_continues_without_adding_a_prompt() {
    let client = Arc::new(FakeClient::new(vec![text("done")]));
    let agent = Agent::new(client.clone(), toolbox(Vec::new()));
    let history = vec![
        Message::system("system prompt", Vec::new()),
        Message::user("hi"),
    ];

    let events = collect_run(agent.resume(history, run_config())).await;
    let (messages, status) = agent_end(&events);
    assert_eq!(status, RunStatus::Completed);
    assert_eq!(user_texts(&messages), vec!["hi".to_owned()]);
    assert_eq!(client.request_count(), 1);
    let request = &client.requests()[0];
    assert_eq!(request.len(), 2, "resume must not append a new prompt");
    assert!(matches!(request[1], Message::User(_)));
}

// ---------------------------------------------------------------------------
// AGENT-002: assistant finalized before any tool starts
// ---------------------------------------------------------------------------

#[tokio::test]
#[spec("AGENT-002")]
async fn spec_agent_002_assistant_finalized_before_tools() {
    let read = Arc::new(FakeTool::new("read"));
    let client = Arc::new(FakeClient::new(vec![
        Script::ToolCalls(vec![call("call_1", "read")]),
        text("done"),
    ]));
    let agent = Agent::new(client, toolbox(vec![read]));

    let events = collect_run(agent.run(Message::user("hi"), Vec::new(), run_config())).await;

    let assistant_end = position(&events, |event| {
        matches!(
            event,
            AgentEvent::MessageEnd {
                message: Message::Assistant(_)
            }
        )
    })
    .expect("assistant MessageEnd");
    let first_tool_start = position(&events, |event| {
        matches!(event, AgentEvent::ToolExecutionStart { .. })
    })
    .expect("ToolExecutionStart");
    let first_result_start = position(&events, |event| {
        matches!(
            event,
            AgentEvent::MessageStart {
                message: Message::ToolResult(_)
            }
        )
    })
    .expect("tool result MessageStart");

    assert!(
        assistant_end < first_tool_start,
        "tool started before the assistant message was finalized"
    );
    assert!(
        assistant_end < first_result_start,
        "tool result appended before the assistant message it answers"
    );
}

// ---------------------------------------------------------------------------
// AGENT-003: one result per call, source order, IDs copied verbatim
// ---------------------------------------------------------------------------

#[tokio::test]
#[spec("AGENT-003")]
async fn spec_agent_003_results_in_source_order() {
    let client = Arc::new(FakeClient::new(vec![
        Script::ToolCalls(vec![
            call("call_a", "alpha"),
            call("call_b", "beta"),
            call("call_c", "gamma"),
        ]),
        text("done"),
    ]));
    let agent = Agent::new(
        client,
        toolbox(vec![
            Arc::new(FakeTool::new("alpha")),
            Arc::new(FakeTool::new("beta")),
            Arc::new(FakeTool::new("gamma")),
        ]),
    );

    let events = collect_run(agent.run(Message::user("hi"), Vec::new(), run_config())).await;

    let results = first_turn_results(&events);
    assert_eq!(results.len(), 3);
    let ids: Vec<&str> = results
        .iter()
        .map(|result| result.tool_call_id.as_str())
        .collect();
    let names: Vec<&str> = results
        .iter()
        .map(|result| result.tool_name.as_str())
        .collect();
    assert_eq!(ids, ["call_a", "call_b", "call_c"]);
    assert_eq!(names, ["alpha", "beta", "gamma"]);
    assert!(results.iter().all(|result| !result.is_error));

    assert_eq!(
        tool_result_message_ids(&events),
        ["call_a", "call_b", "call_c"],
        "transcript tool results must be appended in source order"
    );

    let (messages, _) = agent_end(&events);
    let transcript_ids: Vec<&str> = messages
        .iter()
        .filter_map(|message| match message {
            Message::ToolResult(result) => Some(result.tool_call_id.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(transcript_ids, ["call_a", "call_b", "call_c"]);
}

// ---------------------------------------------------------------------------
// AGENT-004: parallel completion order vs source order transcript
// ---------------------------------------------------------------------------

#[tokio::test]
#[spec("AGENT-004")]
async fn spec_agent_004_parallel_completion_order() {
    // Completion is gated so the order is deterministic rather than timing
    // based: `b` finishes first, then `c`, then `a`.
    let gate_b = Arc::new(AtomicBool::new(false));
    let gate_c = Arc::new(AtomicBool::new(false));

    let tool_a = Arc::new(FakeTool::new("a").waits_for(Arc::clone(&gate_c)));
    let tool_b = Arc::new(FakeTool::new("b").opens(Arc::clone(&gate_b)));
    let tool_c = Arc::new(
        FakeTool::new("c")
            .waits_for(Arc::clone(&gate_b))
            .opens(Arc::clone(&gate_c)),
    );

    let client = Arc::new(FakeClient::new(vec![
        Script::ToolCalls(vec![
            call("call_a", "a"),
            call("call_b", "b"),
            call("call_c", "c"),
        ]),
        text("done"),
    ]));
    let agent = Agent::new(client, toolbox(vec![tool_a, tool_b, tool_c]));

    let events = collect_run(agent.run(Message::user("hi"), Vec::new(), run_config())).await;

    assert_eq!(
        tool_end_ids(&events),
        ["call_b", "call_c", "call_a"],
        "parallel ToolExecutionEnd must be completion ordered"
    );

    let results = first_turn_results(&events);
    let ids: Vec<&str> = results
        .iter()
        .map(|result| result.tool_call_id.as_str())
        .collect();
    assert_eq!(
        ids,
        ["call_a", "call_b", "call_c"],
        "tool results must stay in source order"
    );

    let last_end = event_index_from_end(&events, |event| {
        matches!(event, AgentEvent::ToolExecutionEnd { .. })
    });
    let turn_end = position(&events, |event| matches!(event, AgentEvent::TurnEnd { .. }));
    assert!(last_end < turn_end.expect("TurnEnd"));
}

#[tokio::test]
#[spec("AGENT-004")]
async fn spec_agent_004_sequential_order_is_source_order() {
    let client = Arc::new(FakeClient::new(vec![
        Script::ToolCalls(vec![
            call("call_a", "a"),
            call("call_b", "b"),
            call("call_c", "c"),
        ]),
        text("done"),
    ]));
    let agent = Agent::new(
        client,
        toolbox(vec![
            Arc::new(FakeTool::new("a").with_delay(Duration::from_millis(5))),
            Arc::new(FakeTool::new("b").with_delay(Duration::from_millis(1))),
            Arc::new(FakeTool::new("c").sequential()),
        ]),
    );

    let mut config = run_config();
    config.tool_execution = ExecutionMode::Sequential;

    let events = collect_run(agent.run(Message::user("hi"), Vec::new(), config)).await;

    assert_eq!(tool_end_ids(&events), ["call_a", "call_b", "call_c"]);
    let results = first_turn_results(&events);
    let ids: Vec<&str> = results
        .iter()
        .map(|result| result.tool_call_id.as_str())
        .collect();
    assert_eq!(ids, ["call_a", "call_b", "call_c"]);
}

fn event_index_from_end<F>(events: &[AgentEvent], predicate: F) -> usize
where
    F: Fn(&AgentEvent) -> bool,
{
    events.iter().rposition(predicate).expect("matching event")
}

// ---------------------------------------------------------------------------
// AGENT-005: tool failures become error results and the run continues
// ---------------------------------------------------------------------------

#[tokio::test]
#[spec("AGENT-005")]
#[spec("TOOL-007")]
async fn spec_agent_005_tool_failures_become_results() {
    let boom = Arc::new(FakeTool::error("boom", "kaboom"));
    let client = Arc::new(FakeClient::new(vec![
        Script::ToolCalls(vec![
            call("call_unknown", "ghost"),
            call("call_denied", "blocked"),
            call("call_error", "boom"),
        ]),
        text("recovered"),
    ]));
    let toolbox = Arc::new(FakeToolbox::new(vec![boom]).denying("blocked", "policy"));
    let agent = Agent::new(client, toolbox);

    let events = collect_run(agent.run(Message::user("hi"), Vec::new(), run_config())).await;

    let results = first_turn_results(&events);
    assert_eq!(
        results.len(),
        3,
        "every call must produce exactly one result"
    );
    assert!(
        results.iter().all(|result| result.is_error),
        "unknown, denied, and failing tools must all be error results"
    );
    let ids: Vec<&str> = results
        .iter()
        .map(|result| result.tool_call_id.as_str())
        .collect();
    assert_eq!(ids, ["call_unknown", "call_denied", "call_error"]);

    let end_errors: Vec<bool> = events
        .iter()
        .filter_map(|event| match event {
            AgentEvent::ToolExecutionEnd { is_error, .. } => Some(*is_error),
            _ => None,
        })
        .collect();
    assert_eq!(end_errors, [true, true, true]);

    let (messages, status) = agent_end(&events);
    assert_eq!(
        status,
        RunStatus::Completed,
        "tool failure must not end the run"
    );
    let assistants = assistant_messages(&messages);
    assert_eq!(assistants.len(), 2);
    assert_eq!(assistants[1].text(), "recovered");
}

// ---------------------------------------------------------------------------
// AGENT-006: stop_reason length never executes tools
// ---------------------------------------------------------------------------

#[tokio::test]
#[spec("AGENT-006")]
async fn spec_agent_006_length_never_executes_tools() {
    let side = Arc::new(FakeTool::new("side"));
    let client = Arc::new(FakeClient::new(vec![
        Script::LengthToolCalls(vec![call("call_side", "side")]),
        text("after"),
    ]));
    let agent = Agent::new(
        client.clone(),
        toolbox(vec![Arc::clone(&side) as Arc<dyn Tool>]),
    );

    let events = collect_run(agent.run(Message::user("hi"), Vec::new(), run_config())).await;

    assert_eq!(side.calls(), 0, "truncated tool calls must not execute");
    assert_eq!(
        tool_start_count(&events),
        0,
        "no ToolExecutionStart for length"
    );
    assert_eq!(client.request_count(), 2, "the run must continue");

    let results = first_turn_results(&events);
    assert_eq!(results.len(), 1);
    assert!(results[0].is_error);
    assert_eq!(results[0].tool_call_id, "call_side");
    let text = results[0]
        .content
        .iter()
        .filter_map(Content::as_text)
        .collect::<String>();
    assert!(
        text.contains("not executed"),
        "unexpected result text: {text}"
    );
    assert!(text.contains("Re-issue"), "unexpected result text: {text}");

    let (messages, status) = agent_end(&events);
    assert_eq!(status, RunStatus::Completed);
    let assistants = assistant_messages(&messages);
    assert_eq!(assistants.len(), 2);
    assert_eq!(assistants[1].text(), "after");
}

#[tokio::test]
#[spec("AGENT-006")]
async fn spec_agent_006_length_without_calls_completes() {
    let client = Arc::new(FakeClient::new(vec![Script::LengthToolCalls(Vec::new())]));
    let agent = Agent::new(client.clone(), toolbox(Vec::new()));

    let events = collect_run(agent.run(Message::user("hi"), Vec::new(), run_config())).await;

    assert_eq!(client.request_count(), 1);
    assert_eq!(tool_start_count(&events), 0);
    assert!(first_turn_results(&events).is_empty());
    let (_, status) = agent_end(&events);
    assert_eq!(status, RunStatus::Completed);
}

// ---------------------------------------------------------------------------
// AGENT-007: steering and follow-up polling
// ---------------------------------------------------------------------------

fn steering_agent(scripts: Vec<Script>, queued: Vec<Message>) -> (Arc<FakeClient>, Arc<Agent>) {
    let slot: Arc<Mutex<Option<Arc<Agent>>>> = Arc::new(Mutex::new(None));
    let slot_for_tool = Arc::clone(&slot);
    let queued_for_tool = queued.clone();
    let tool = FakeTool::new("steer").on_execute(move || {
        let agent = slot_for_tool.lock().expect("agent slot poisoned").clone();
        if let Some(agent) = agent {
            for message in &queued_for_tool {
                agent.steer(message.clone());
            }
        }
    });

    let client = Arc::new(FakeClient::new(scripts));
    let agent = Arc::new(Agent::new(
        as_chat_client(&client),
        Arc::new(FakeToolbox::new(vec![Arc::new(tool)])),
    ));
    *slot.lock().expect("agent slot poisoned") = Some(Arc::clone(&agent));
    (client, agent)
}

#[tokio::test]
#[spec("AGENT-007")]
async fn spec_agent_007_steering_before_first_request_is_delivered() {
    let client = Arc::new(FakeClient::new(vec![text("done")]));
    let agent = Agent::new(as_chat_client(&client), toolbox(Vec::new()));
    agent.steer(Message::user("steer-1"));

    let events = collect_run(agent.run(Message::user("prompt"), Vec::new(), run_config())).await;

    let requests = client.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(user_texts(&requests[0]), ["prompt", "steer-1"]);

    let steering_start = position(&events, |event| {
        matches!(
            event,
            AgentEvent::MessageStart {
                message: Message::User(user)
            } if user.content.iter().filter_map(Content::as_text).collect::<String>() == "steer-1"
        )
    })
    .expect("steering user message");
    let assistant_start = position(&events, |event| {
        matches!(
            event,
            AgentEvent::MessageStart {
                message: Message::Assistant(_)
            }
        )
    })
    .expect("assistant MessageStart");
    assert!(steering_start < assistant_start);
}

#[tokio::test]
#[spec("AGENT-007")]
async fn spec_agent_007_steering_one_at_a_time_between_turns() {
    let (client, agent) = steering_agent(
        vec![
            Script::ToolCalls(vec![call("call_steer", "steer")]),
            text("second"),
            text("third"),
        ],
        vec![Message::user("steer-1"), Message::user("steer-2")],
    );

    let events = collect_run(agent.run(Message::user("prompt"), Vec::new(), run_config())).await;

    let requests = client.requests();
    assert_eq!(
        requests.len(),
        3,
        "one-at-a-time delivers one item per turn"
    );
    assert_eq!(user_texts(&requests[0]), ["prompt"]);
    assert_eq!(user_texts(&requests[1]), ["prompt", "steer-1"]);
    assert_eq!(user_texts(&requests[2]), ["prompt", "steer-1", "steer-2"]);

    let first_turn_end = position(&events, |event| matches!(event, AgentEvent::TurnEnd { .. }))
        .expect("first TurnEnd");
    let steer_start = events
        .iter()
        .skip(first_turn_end)
        .position(|event| {
            matches!(
                event,
                AgentEvent::MessageStart {
                    message: Message::User(_)
                }
            )
        })
        .map(|offset| offset + first_turn_end)
        .expect("steering message after TurnEnd");
    assert!(
        first_turn_end < steer_start,
        "steering must be delivered between turns"
    );
}

#[tokio::test]
#[spec("AGENT-007")]
async fn spec_agent_007_steering_all_mode_drains_queue() {
    let (client, agent) = steering_agent(
        vec![
            Script::ToolCalls(vec![call("call_steer", "steer")]),
            text("second"),
        ],
        vec![Message::user("steer-1"), Message::user("steer-2")],
    );

    let mut config = run_config();
    config.steering_mode = QueueMode::All;

    let events = collect_run(agent.run(Message::user("prompt"), Vec::new(), config)).await;

    let requests = client.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        user_texts(&requests[1]),
        ["prompt", "steer-1", "steer-2"],
        "all mode drains the queue in FIFO order"
    );
    assert_eq!(turn_end_count(&events), 2);
}

#[tokio::test]
#[spec("AGENT-007")]
async fn spec_agent_007_follow_up_only_when_run_would_stop() {
    let client = Arc::new(FakeClient::new(vec![
        Script::ToolCalls(vec![call("call_read", "read")]),
        text("second"),
        text("third"),
    ]));
    let agent = Agent::new(
        as_chat_client(&client),
        toolbox(vec![Arc::new(FakeTool::new("read"))]),
    );
    agent.follow_up(Message::user("follow-up"));

    let events = collect_run(agent.run(Message::user("prompt"), Vec::new(), run_config())).await;

    let requests = client.requests();
    assert_eq!(requests.len(), 3);
    assert_eq!(
        user_texts(&requests[0]),
        ["prompt"],
        "follow-up must not be delivered while tools continue the run"
    );
    assert_eq!(
        user_texts(&requests[1]),
        ["prompt"],
        "follow-up must wait until the run would otherwise stop"
    );
    assert_eq!(user_texts(&requests[2]), ["prompt", "follow-up"]);

    let (_, status) = agent_end(&events);
    assert_eq!(status, RunStatus::Completed);
}

#[tokio::test]
#[spec("AGENT-007")]
async fn spec_agent_007_follow_up_one_at_a_time() {
    let client = Arc::new(FakeClient::new(vec![
        text("one"),
        text("two"),
        text("three"),
    ]));
    let agent = Agent::new(as_chat_client(&client), toolbox(Vec::new()));
    agent.follow_up(Message::user("follow-1"));
    agent.follow_up(Message::user("follow-2"));

    let events = collect_run(agent.run(Message::user("prompt"), Vec::new(), run_config())).await;

    let requests = client.requests();
    assert_eq!(requests.len(), 3);
    assert_eq!(user_texts(&requests[1]), ["prompt", "follow-1"]);
    assert_eq!(user_texts(&requests[2]), ["prompt", "follow-1", "follow-2"]);
    assert_eq!(turn_end_count(&events), 3);
}

#[tokio::test]
#[spec("AGENT-007")]
async fn spec_agent_007_follow_up_all_mode() {
    let client = Arc::new(FakeClient::new(vec![text("one"), text("two")]));
    let agent = Agent::new(as_chat_client(&client), toolbox(Vec::new()));
    agent.follow_up(Message::user("follow-1"));
    agent.follow_up(Message::user("follow-2"));

    let mut config = run_config();
    config.follow_up_mode = QueueMode::All;

    let events = collect_run(agent.run(Message::user("prompt"), Vec::new(), config)).await;

    let requests = client.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(user_texts(&requests[1]), ["prompt", "follow-1", "follow-2"]);
    assert_eq!(turn_end_count(&events), 2);
}

// ---------------------------------------------------------------------------
// AGENT-008: cooperative abort
// ---------------------------------------------------------------------------

#[tokio::test]
#[spec("AGENT-008")]
async fn spec_agent_008_abort_is_cooperative() {
    let client = Arc::new(FakeClient::new(vec![Script::AbortOnCancel]));
    let agent = Agent::new(as_chat_client(&client), toolbox(Vec::new()));
    let config = run_config();
    let cancel = config.cancel.clone();

    let stream = agent.run(Message::user("prompt"), Vec::new(), config);
    client.started.notified().await;
    cancel.cancel();

    let events = collect_run(stream).await;
    let (messages, status) = agent_end(&events);
    assert_eq!(status, RunStatus::Aborted);
    assert_ne!(status, RunStatus::Error, "abort is not a provider error");

    let assistants = assistant_messages(&messages);
    assert_eq!(assistants.len(), 1);
    assert_eq!(assistants[0].stop_reason, StopReason::Aborted);
}

#[tokio::test]
#[spec("AGENT-008")]
async fn spec_agent_008_abort_during_tools_keeps_finalized_message() {
    let config = run_config();
    let cancel = config.cancel.clone();
    let cancel_for_tool = cancel.clone();
    let slow = FakeTool::wait_for_cancel("slow").on_execute(move || cancel_for_tool.cancel());

    let client = Arc::new(FakeClient::new(vec![
        Script::ToolCalls(vec![call("call_slow", "slow")]),
        text("unreachable"),
    ]));
    let agent = Agent::new(as_chat_client(&client), toolbox(vec![Arc::new(slow)]));

    let events = collect_run(agent.run(Message::user("prompt"), Vec::new(), config)).await;
    let (messages, status) = agent_end(&events);

    assert_eq!(status, RunStatus::Aborted);
    assert_eq!(
        client.request_count(),
        1,
        "no second assistant message is synthesized on abort"
    );

    let assistants = assistant_messages(&messages);
    assert_eq!(assistants.len(), 1);
    assert_eq!(
        assistants[0].stop_reason,
        StopReason::ToolUse,
        "the finalized assistant message is retained"
    );

    let results = first_turn_results(&events);
    assert_eq!(results.len(), 1);
    assert!(results[0].is_error);
    assert_eq!(results[0].tool_call_id, "call_slow");
}

// ---------------------------------------------------------------------------
// AGENT-009: max_turns bounds assistant requests
// ---------------------------------------------------------------------------

#[tokio::test]
#[spec("AGENT-009")]
async fn spec_agent_009_max_turns_enforced() {
    let read = Arc::new(FakeTool::new("read"));
    let client = Arc::new(FakeClient::new(vec![
        Script::ToolCalls(vec![call("call_1", "read")]),
        Script::ToolCalls(vec![call("call_2", "read")]),
    ]));
    let agent = Agent::new(
        as_chat_client(&client),
        toolbox(vec![read as Arc<dyn Tool>]),
    );

    let mut config = run_config();
    config.max_turns = 2;

    let events = collect_run(agent.run(Message::user("prompt"), Vec::new(), config)).await;

    assert_eq!(client.request_count(), 2, "at most max_turns requests");
    let (messages, status) = agent_end(&events);
    assert_eq!(status, RunStatus::TurnLimit);
    assert_eq!(assistant_messages(&messages).len(), 2);
    assert_eq!(turn_end_count(&events), 2);
}

// ---------------------------------------------------------------------------
// SEC-003: a denied tool is a recoverable error result
// ---------------------------------------------------------------------------

#[tokio::test]
#[spec("SEC-003")]
async fn spec_sec_003_denial_is_an_error_result() {
    let tool = Arc::new(FakeTool::new("write"));
    let client = Arc::new(FakeClient::new(vec![
        Script::ToolCalls(vec![call("call_1", "write")]),
        text("understood"),
    ]));
    let toolbox = Arc::new(
        FakeToolbox::new(vec![tool.clone() as Arc<dyn Tool>])
            .denying("write", "not allowed by policy"),
    );
    let agent = Agent::new(as_chat_client(&client), toolbox);

    let events = collect_run(agent.run(Message::user("prompt"), Vec::new(), run_config())).await;

    assert_eq!(tool.calls(), 0, "a denied tool must not execute");
    let results = first_turn_results(&events);
    assert_eq!(results.len(), 1);
    assert!(results[0].is_error);
    assert!(
        results[0].content[0]
            .as_text()
            .is_some_and(|text| text.contains("denied"))
    );
    let (messages, status) = agent_end(&events);
    assert_eq!(status, RunStatus::Completed);
    assert_eq!(
        assistant_messages(&messages)
            .last()
            .map(|message| message.text()),
        Some("understood".to_owned())
    );
}

// ---------------------------------------------------------------------------
// AGENT-010: hooks are fallible and may block tools
// ---------------------------------------------------------------------------

struct RejectingHooks {
    fail_prepare: bool,
    block_tool: bool,
}

#[async_trait]
impl Hooks for RejectingHooks {
    async fn prepare_request(&self, context: HookContext) -> Result<HookContext, String> {
        if self.fail_prepare {
            return Err("prepare exploded".to_owned());
        }
        Ok(context)
    }

    async fn before_tool(&self, call: &ToolCall) -> Result<(), String> {
        if self.block_tool {
            return Err(format!("`{}` is not permitted by the hook", call.name));
        }
        Ok(())
    }
}

struct PanickingTool;

#[async_trait]
impl Tool for PanickingTool {
    fn name(&self) -> &str {
        "panic"
    }

    fn description(&self) -> &str {
        "panics on purpose"
    }

    fn schema(&self) -> Value {
        json!({"type": "object"})
    }

    async fn execute(
        &self,
        _call: &ToolCall,
        _context: &ToolContext,
        _updates: Option<mpsc::UnboundedSender<ToolProgress>>,
    ) -> ToolResult {
        panic!("tool panicked for the test");
    }
}

#[tokio::test]
#[spec("AGENT-010")]
async fn spec_agent_010_hook_errors_fail_run() {
    let tool = Arc::new(FakeTool::new("read"));
    let client = Arc::new(FakeClient::new(vec![text("should not run")]));
    let hooks = Arc::new(RejectingHooks {
        fail_prepare: true,
        block_tool: false,
    });
    let agent = Agent::with_hooks(
        as_chat_client(&client),
        toolbox(vec![tool.clone() as Arc<dyn Tool>]),
        hooks,
    );

    let events = collect_run(agent.run(Message::user("prompt"), Vec::new(), run_config())).await;

    assert_eq!(client.request_count(), 0, "the request hook fails first");
    let (messages, status) = agent_end(&events);
    assert_eq!(status, RunStatus::Error);
    let last = messages.last().expect("a synthesized error message");
    let assistant = last.as_assistant().expect("assistant message");
    assert_eq!(assistant.stop_reason, StopReason::Error);
    assert!(
        assistant
            .error_message
            .as_deref()
            .is_some_and(|message| message.contains("hook failed: prepare exploded"))
    );
}

#[tokio::test]
#[spec("AGENT-010")]
async fn spec_agent_010_before_tool_block_is_an_error_result() {
    let tool = Arc::new(FakeTool::new("read"));
    let client = Arc::new(FakeClient::new(vec![
        Script::ToolCalls(vec![call("call_1", "read")]),
        text("recovered"),
    ]));
    let hooks = Arc::new(RejectingHooks {
        fail_prepare: false,
        block_tool: true,
    });
    let agent = Agent::with_hooks(
        as_chat_client(&client),
        toolbox(vec![tool.clone() as Arc<dyn Tool>]),
        hooks,
    );

    let events = collect_run(agent.run(Message::user("prompt"), Vec::new(), run_config())).await;

    assert_eq!(tool.calls(), 0, "a blocked tool must not execute");
    let results = first_turn_results(&events);
    assert_eq!(results.len(), 1);
    assert!(results[0].is_error);
    assert!(
        results[0].content[0]
            .as_text()
            .is_some_and(|text| text.contains("blocked by a hook"))
    );
    let (messages, status) = agent_end(&events);
    assert_eq!(status, RunStatus::Completed);
    assert_eq!(
        assistant_messages(&messages)
            .last()
            .map(|message| message.text()),
        Some("recovered".to_owned())
    );
}

// ---------------------------------------------------------------------------
// AGENT-011: persistence is event-driven
// ---------------------------------------------------------------------------

#[tokio::test]
#[spec("AGENT-011")]
async fn spec_agent_011_persistence_is_event_driven() {
    let tool = Arc::new(FakeTool::new("read"));
    let client = Arc::new(FakeClient::new(vec![
        Script::ToolCalls(vec![call("call_1", "read")]),
        text("done"),
    ]));
    let agent = Agent::new(
        as_chat_client(&client),
        toolbox(vec![tool as Arc<dyn Tool>]),
    );

    let events = collect_run(agent.run(Message::user("prompt"), Vec::new(), run_config())).await;

    // A session writer appends exactly what MessageEnd carries, nothing else.
    let persisted = events
        .iter()
        .filter_map(|event| match event {
            AgentEvent::MessageEnd { message } => Some(message.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();

    let (final_messages, status) = agent_end(&events);
    assert_eq!(status, RunStatus::Completed);
    assert_eq!(persisted, final_messages, "the projection is event-driven");
    assert!(
        persisted.iter().all(|message| {
            message
                .as_assistant()
                .is_none_or(|assistant| assistant.stop_reason != StopReason::Pending)
        }),
        "no partial assistant message is ever persisted"
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, AgentEvent::MessageUpdate { .. })),
        "streaming updates are separate from persistence events"
    );
}

// ---------------------------------------------------------------------------
// AGENT-012: a panicking tool is caught
// ---------------------------------------------------------------------------

#[tokio::test]
#[spec("AGENT-012")]
async fn spec_agent_012_tool_panic_is_caught() {
    let client = Arc::new(FakeClient::new(vec![
        Script::ToolCalls(vec![call("call_1", "panic")]),
        text("recovered"),
    ]));
    let agent = Agent::new(
        as_chat_client(&client),
        toolbox(vec![Arc::new(PanickingTool) as Arc<dyn Tool>]),
    );

    let events = collect_run(agent.run(Message::user("prompt"), Vec::new(), run_config())).await;

    let results = first_turn_results(&events);
    assert_eq!(results.len(), 1);
    assert!(results[0].is_error);
    assert!(
        results[0].content[0]
            .as_text()
            .is_some_and(|text| text.contains("panicked"))
    );
    let (messages, status) = agent_end(&events);
    assert_eq!(status, RunStatus::Completed);
    assert_eq!(
        assistant_messages(&messages)
            .last()
            .map(|message| message.text()),
        Some("recovered".to_owned())
    );
}
