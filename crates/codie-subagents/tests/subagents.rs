//! Conformance tests for `SPEC-AGENT-003` (AGENT-014..AGENT-027).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use codie_core::{
    Agent, AgentConfig, AgentEvent, AgentEventStream, AssistantDelta, AssistantEventStream,
    AssistantMessage, AssistantStreamSender, ChatClient, Content, ExecutionMode, HookContext,
    Hooks, Message, ModelRef, RunStatus, StopReason, StreamOptions, Tool, ToolCall, ToolContext,
    ToolDecl, ToolProgress, ToolResult, Toolbox, Usage,
};
use codie_prompt::{AgentDefinition, PromptInput};
use codie_session::Session;
use codie_subagents::{
    AgentTypeConfig, DelegationMode, SubagentConfig, SubagentRuntime, SubagentToolbox, TaskTool,
    ToolSelection, describe_types, schema,
};
use codie_testkit::spec;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

const API: &str = "test-api";
const PROVIDER: &str = "test-provider";
const MODEL: &str = "test-model";

// ---------------------------------------------------------------------------
// Scripted client
// ---------------------------------------------------------------------------

enum Response {
    Text(String),
    Call(ToolCall),
    Error(String),
    Abort,
}

type Responder = Arc<dyn Fn(&[Message]) -> Response + Send + Sync>;

struct TestClient {
    responder: Responder,
    requests: Arc<Mutex<Vec<Vec<Message>>>>,
    models: Arc<Mutex<Vec<ModelRef>>>,
    active: Arc<AtomicUsize>,
    max_active: Arc<AtomicUsize>,
    delay: Duration,
}

impl TestClient {
    fn new(responder: impl Fn(&[Message]) -> Response + Send + Sync + 'static) -> Self {
        Self {
            responder: Arc::new(responder),
            requests: Arc::new(Mutex::new(Vec::new())),
            models: Arc::new(Mutex::new(Vec::new())),
            active: Arc::new(AtomicUsize::new(0)),
            max_active: Arc::new(AtomicUsize::new(0)),
            delay: Duration::ZERO,
        }
    }

    fn with_delay(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }

    fn requests(&self) -> Vec<Vec<Message>> {
        self.requests.lock().expect("request log").clone()
    }

    fn models(&self) -> Vec<ModelRef> {
        self.models.lock().expect("model log").clone()
    }

    fn max_active(&self) -> usize {
        self.max_active.load(Ordering::SeqCst)
    }
}

impl ChatClient for TestClient {
    fn stream(&self, context: &[Message], options: &StreamOptions) -> AssistantEventStream {
        self.requests
            .lock()
            .expect("request log")
            .push(context.to_vec());
        self.models
            .lock()
            .expect("model log")
            .push(options.model.clone());
        let response = (self.responder)(context);
        let (stream, sender) = AssistantEventStream::channel(32);
        let active = Arc::clone(&self.active);
        let max_active = Arc::clone(&self.max_active);
        let delay = self.delay;
        let cancel = options.cancel.clone();
        tokio::spawn(async move {
            let current = active.fetch_add(1, Ordering::SeqCst) + 1;
            max_active.fetch_max(current, Ordering::SeqCst);
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
            match response {
                Response::Text(text) => emit_text(&sender, &text).await,
                Response::Call(call) => emit_calls(&sender, vec![call]).await,
                Response::Error(message) => {
                    let message =
                        AssistantMessage::error(API, PROVIDER, MODEL, StopReason::Error, message);
                    sender
                        .send(AssistantDelta::Error {
                            reason: StopReason::Error,
                            message,
                        })
                        .await;
                }
                Response::Abort => {
                    sender
                        .send(AssistantDelta::Start {
                            partial: AssistantMessage::partial(API, PROVIDER, MODEL),
                        })
                        .await;
                    cancel.cancelled().await;
                    let message = AssistantMessage::error(
                        API,
                        PROVIDER,
                        MODEL,
                        StopReason::Aborted,
                        "aborted",
                    );
                    sender
                        .send(AssistantDelta::Error {
                            reason: StopReason::Aborted,
                            message,
                        })
                        .await;
                }
            }
            active.fetch_sub(1, Ordering::SeqCst);
        });
        stream
    }
}

async fn emit_text(sender: &AssistantStreamSender, text: &str) {
    sender
        .send(AssistantDelta::Start {
            partial: AssistantMessage::partial(API, PROVIDER, MODEL),
        })
        .await;
    sender.send(AssistantDelta::TextStart { index: 0 }).await;
    sender
        .send(AssistantDelta::TextEnd {
            index: 0,
            content: text.to_owned(),
        })
        .await;
    let mut message = AssistantMessage::partial(API, PROVIDER, MODEL);
    message.content = vec![Content::text(text)];
    message.usage = Usage::new(1, 2, 0, 0);
    message.stop_reason = StopReason::Stop;
    sender
        .send(AssistantDelta::Done {
            reason: StopReason::Stop,
            message,
        })
        .await;
}

async fn emit_calls(sender: &AssistantStreamSender, calls: Vec<ToolCall>) {
    sender
        .send(AssistantDelta::Start {
            partial: AssistantMessage::partial(API, PROVIDER, MODEL),
        })
        .await;
    let mut content = Vec::new();
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
    message.usage = Usage::new(1, 2, 0, 0);
    message.stop_reason = StopReason::ToolUse;
    sender
        .send(AssistantDelta::Done {
            reason: StopReason::ToolUse,
            message,
        })
        .await;
}

// ---------------------------------------------------------------------------
// Toolboxes and tools
// ---------------------------------------------------------------------------

struct EchoTool {
    name: String,
    calls: AtomicUsize,
}

impl EchoTool {
    fn new(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            calls: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl Tool for EchoTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        "echo"
    }

    fn schema(&self) -> Value {
        json!({"type": "object", "additionalProperties": true})
    }

    fn execution_mode(&self) -> ExecutionMode {
        ExecutionMode::Parallel
    }

    async fn execute(
        &self,
        _call: &ToolCall,
        _context: &ToolContext,
        _updates: Option<mpsc::UnboundedSender<ToolProgress>>,
    ) -> ToolResult {
        self.calls.fetch_add(1, Ordering::SeqCst);
        ToolResult::text("echo ok")
    }
}

struct TestToolbox {
    tools: HashMap<String, Arc<dyn Tool>>,
    denials: HashMap<String, String>,
}

impl TestToolbox {
    fn new(tools: Vec<Arc<dyn Tool>>) -> Self {
        Self {
            tools: tools
                .into_iter()
                .map(|tool| (tool.name().to_owned(), tool))
                .collect(),
            denials: HashMap::new(),
        }
    }
}

impl Toolbox for TestToolbox {
    fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.get(name).cloned()
    }

    fn declarations(&self) -> Vec<ToolDecl> {
        let mut declarations = self
            .tools
            .values()
            .map(|tool| ToolDecl {
                name: tool.name().to_owned(),
                description: tool.description().to_owned(),
                parameters: tool.schema(),
            })
            .collect::<Vec<_>>();
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

fn base_config() -> SubagentConfig {
    SubagentConfig {
        enabled: true,
        ..SubagentConfig::default()
    }
}

fn prompt_input(dir: &Path) -> PromptInput {
    PromptInput {
        cwd: dir.to_path_buf(),
        tools: Vec::new(),
        preamble: None,
        append: None,
        skills: Vec::new(),
        project_context: Vec::new(),
        project_memory: None,
    }
}

fn make_runtime(
    config: SubagentConfig,
    client: Arc<TestClient>,
    parent: Arc<dyn Toolbox>,
    dir: &Path,
) -> Arc<SubagentRuntime> {
    let runtime = SubagentRuntime::new(
        config,
        client,
        parent,
        prompt_input(dir),
        dir.join("sessions"),
        Some("ses_parent".to_owned()),
        model_ref(),
        vec![MODEL.to_owned()],
    )
    .expect("valid runtime");
    runtime.set_mode(DelegationMode::Auto);
    Arc::new(runtime)
}

fn base_toolbox() -> Arc<TestToolbox> {
    Arc::new(TestToolbox::new(vec![
        Arc::new(EchoTool::new("read")),
        Arc::new(EchoTool::new("bash")),
    ]))
}

fn tool_context(dir: &Path, depth: usize) -> ToolContext {
    ToolContext {
        cwd: dir.to_path_buf(),
        cancel: CancellationToken::new(),
        session_id: Some("ses_parent".to_owned()),
        delegation_depth: depth,
        agent: None,
    }
}

fn task_call(prompt: &str, agent: &str) -> ToolCall {
    ToolCall {
        id: "call_task".to_owned(),
        name: "task".to_owned(),
        arguments: json!({
            "description": "test task",
            "prompt": prompt,
            "agent": agent,
        }),
    }
}

async fn run_task(
    runtime: &Arc<SubagentRuntime>,
    context: &ToolContext,
    prompt: &str,
    agent: &str,
) -> ToolResult {
    TaskTool::new(Arc::clone(runtime))
        .execute(&task_call(prompt, agent), context, None)
        .await
}

fn user_text(message: &Message) -> Option<String> {
    match message {
        Message::User(user) => Some(
            user.content
                .iter()
                .filter_map(Content::as_text)
                .collect::<Vec<_>>()
                .join(""),
        ),
        _ => None,
    }
}

fn system_text(message: &Message) -> Option<String> {
    match message {
        Message::System(system) => Some(system.content.clone()),
        _ => None,
    }
}

fn persona(name: &str, description: &str, prompt: &str) -> AgentDefinition {
    AgentDefinition {
        name: name.to_owned(),
        description: description.to_owned(),
        prompt: prompt.to_owned(),
        model: None,
        tools: Some(vec!["read".to_owned()]),
        max_turns: None,
        timeout_secs: None,
        path: PathBuf::new(),
    }
}

fn tool_result_text(message: &Message) -> Option<String> {
    match message {
        Message::ToolResult(result) => Some(
            result
                .content
                .iter()
                .filter_map(Content::as_text)
                .collect::<Vec<_>>()
                .join(""),
        ),
        _ => None,
    }
}

async fn collect_run(mut stream: AgentEventStream) -> Vec<AgentEvent> {
    let mut events = Vec::new();
    while let Some(event) = stream.recv().await {
        let end = matches!(event, AgentEvent::AgentEnd { .. });
        events.push(event);
        if end {
            break;
        }
    }
    events
}

fn run_messages(events: &[AgentEvent]) -> Vec<Message> {
    events
        .iter()
        .find_map(|event| match event {
            AgentEvent::AgentEnd { messages, .. } => Some(messages.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

fn run_status(events: &[AgentEvent]) -> RunStatus {
    events
        .iter()
        .find_map(|event| match event {
            AgentEvent::AgentEnd { status, .. } => Some(*status),
            _ => None,
        })
        .unwrap_or(RunStatus::Error)
}

fn session_files(dir: &Path) -> Vec<PathBuf> {
    let sessions = dir.join("sessions");
    let Ok(entries) = std::fs::read_dir(sessions) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "jsonl"))
        .collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
#[spec("PRMPT-013")]
fn spec_prmpt_013_task_description_guidance() {
    let description = codie_subagents::TASK_DESCRIPTION;
    assert!(description.contains("no conversation history"));
    assert!(description.contains("general"));
    assert!(description.contains("explore"));
    assert!(description.contains("parallel"));
}

#[test]
#[spec("PRMPT-014")]
fn spec_prmpt_014_forced_directive() {
    let directive = codie_subagents::FORCED_DIRECTIVE;
    assert!(directive.contains("independent subtasks"));
    assert!(directive.contains("same files"));
}

#[tokio::test]
#[spec("AGENT-014")]
async fn spec_agent_014_task_tool_contract() {
    let dir = tempfile::tempdir().unwrap();
    let client = Arc::new(TestClient::new(|_| Response::Text("child".to_owned())));
    let runtime = make_runtime(base_config(), client, base_toolbox(), dir.path());

    let tool = TaskTool::new(runtime);
    let schema = tool.schema();
    assert_eq!(schema["required"], json!(["description", "prompt"]));
    assert_eq!(
        schema["properties"]["agent"]["enum"],
        json!(["general", "explore"])
    );
    assert_eq!(tool.name(), "task");

    let missing = ToolCall {
        id: "c1".to_owned(),
        name: "task".to_owned(),
        arguments: json!({"description": "x"}),
    };
    let result = tool
        .execute(&missing, &tool_context(dir.path(), 0), None)
        .await;
    assert!(result.is_error);
    assert!(result.content[0].as_text().unwrap().contains("`prompt`"));

    let unknown = task_call("do it", "nonexistent");
    let result = tool
        .execute(&unknown, &tool_context(dir.path(), 0), None)
        .await;
    assert!(result.is_error);
    assert!(
        result.content[0]
            .as_text()
            .unwrap()
            .contains("Unknown subagent type")
    );
}

#[tokio::test]
#[spec("AGENT-015")]
#[spec("AGENT-024")]
async fn spec_agent_015_child_context_isolated() {
    let dir = tempfile::tempdir().unwrap();
    let client = Arc::new(TestClient::new(|context| {
        if context
            .iter()
            .filter_map(user_text)
            .any(|text| text.contains("PARENT"))
        {
            if context
                .iter()
                .any(|message| matches!(message, Message::ToolResult(_)))
            {
                return Response::Text("parent done".to_owned());
            }
            Response::Call(task_call("CHILD TASK", "general"))
        } else {
            Response::Text("child done".to_owned())
        }
    }));
    let runtime = make_runtime(
        base_config(),
        Arc::clone(&client),
        base_toolbox(),
        dir.path(),
    );
    let toolbox: Arc<dyn Toolbox> = Arc::new(SubagentToolbox::new(base_toolbox(), runtime));

    let agent = Agent::new(client.clone(), toolbox);
    let mut parent_config = AgentConfig::new(model_ref(), dir.path().to_path_buf()).unwrap();
    parent_config.max_turns = 5;
    let events = collect_run(agent.run(Message::user("PARENT"), Vec::new(), parent_config)).await;
    assert_eq!(run_status(&events), RunStatus::Completed);

    let messages = run_messages(&events);
    assert!(
        !messages.iter().any(|message| {
            message
                .as_assistant()
                .is_some_and(|assistant| assistant.text().contains("child done"))
        }),
        "child output leaked into the parent transcript: {messages:?}"
    );
    let requests = client.requests();
    assert_eq!(requests.len(), 3, "{requests:?}");
    assert_eq!(requests[1].len(), 2, "child sees system + prompt only");
    assert!(matches!(&requests[1][0], Message::System(_)));
    assert_eq!(user_text(&requests[1][1]).as_deref(), Some("CHILD TASK"));
}

#[tokio::test]
#[spec("AGENT-016")]
async fn spec_agent_016_child_turn_budget_independent() {
    let dir = tempfile::tempdir().unwrap();
    let client = Arc::new(TestClient::new(|context| {
        if context
            .iter()
            .filter_map(user_text)
            .any(|text| text.contains("PARENT"))
        {
            if context.iter().any(|message| {
                message.as_assistant().is_some()
                    && !message.as_assistant().unwrap().tool_calls().is_empty()
            }) && context
                .iter()
                .any(|message| matches!(message, Message::ToolResult(_)))
            {
                Response::Text("parent done".to_owned())
            } else {
                Response::Call(task_call("CHILD TASK", "general"))
            }
        } else {
            Response::Call(ToolCall {
                id: "loop".to_owned(),
                name: "read".to_owned(),
                arguments: json!({}),
            })
        }
    }));
    let mut config = base_config();
    config.max_turns = 1;
    let runtime = make_runtime(config, Arc::clone(&client), base_toolbox(), dir.path());
    let toolbox: Arc<dyn Toolbox> = Arc::new(SubagentToolbox::new(base_toolbox(), runtime));

    let agent = Agent::new(client, toolbox);
    let mut parent_config = AgentConfig::new(model_ref(), dir.path().to_path_buf()).unwrap();
    parent_config.max_turns = 5;
    let events = collect_run(agent.run(Message::user("PARENT"), Vec::new(), parent_config)).await;
    assert_eq!(run_status(&events), RunStatus::Completed);
    let result = run_messages(&events)
        .iter()
        .find_map(|message| match message {
            Message::ToolResult(result) => Some(result.clone()),
            _ => None,
        })
        .expect("task result");
    assert!(result.is_error);
    assert_eq!(result.details.unwrap()["status"], "turn_limit");
}

#[tokio::test]
#[spec("AGENT-017")]
async fn spec_agent_017_parallel_children_bounded() {
    let dir = tempfile::tempdir().unwrap();
    let client = Arc::new(
        TestClient::new(|_| Response::Text("child".to_owned()))
            .with_delay(Duration::from_millis(30)),
    );
    let mut config = base_config();
    config.max_parallel = 2;
    let runtime = make_runtime(config, Arc::clone(&client), base_toolbox(), dir.path());

    let mut handles = Vec::new();
    for index in 0..6 {
        let runtime = Arc::clone(&runtime);
        let context = tool_context(dir.path(), 0);
        handles.push(tokio::spawn(async move {
            let tool = TaskTool::new(runtime);
            tool.execute(
                &task_call(&format!("task {index}"), "general"),
                &context,
                None,
            )
            .await
        }));
    }
    for handle in handles {
        let result = handle.await.unwrap();
        assert!(!result.is_error, "{result:?}");
    }
    assert!(
        client.max_active() <= 2,
        "observed {} concurrent children",
        client.max_active()
    );
}

#[tokio::test]
#[spec("AGENT-018")]
async fn spec_agent_018_cancellation_propagates() {
    let dir = tempfile::tempdir().unwrap();
    let client = Arc::new(TestClient::new(|_| Response::Abort));
    let runtime = make_runtime(base_config(), client, base_toolbox(), dir.path());
    let context = tool_context(dir.path(), 0);
    let cancel = context.cancel.clone();

    let handle = {
        let runtime = Arc::clone(&runtime);
        let context = context.clone();
        tokio::spawn(async move { run_task(&runtime, &context, "work", "general").await })
    };
    tokio::time::sleep(Duration::from_millis(20)).await;
    cancel.cancel();
    let result = handle.await.unwrap();
    assert!(result.is_error);
    assert_eq!(result.details.unwrap()["status"], "aborted");

    // A child that outlives the wall-clock limit is aborted.
    let slow_client = Arc::new(
        TestClient::new(|_| Response::Text("slow".to_owned()))
            .with_delay(Duration::from_millis(200)),
    );
    let mut config = base_config();
    config.timeout = Duration::from_millis(30);
    let slow = make_runtime(config, slow_client, base_toolbox(), dir.path());
    let result = run_task(&slow, &tool_context(dir.path(), 0), "work", "general").await;
    assert!(result.is_error);
    assert!(
        result.content[0].as_text().unwrap().contains("timed out"),
        "{result:?}"
    );
}

#[tokio::test]
#[spec("AGENT-019")]
async fn spec_agent_019_depth_limit_blocks_recursion() {
    let dir = tempfile::tempdir().unwrap();
    let client = Arc::new(TestClient::new(|_| Response::Text("child".to_owned())));
    let runtime = make_runtime(base_config(), client, base_toolbox(), dir.path());

    let result = run_task(&runtime, &tool_context(dir.path(), 1), "work", "general").await;
    assert!(result.is_error);
    assert!(result.content[0].as_text().unwrap().contains("depth"));

    // A child at the limit cannot see `task`, even if the model hallucinates it.
    let client = Arc::new(TestClient::new(|context| {
        if context
            .iter()
            .filter_map(user_text)
            .any(|text| text.contains("PARENT"))
        {
            if context
                .iter()
                .any(|message| matches!(message, Message::ToolResult(_)))
            {
                return Response::Text("parent done".to_owned());
            }
            return Response::Call(task_call("CHILD", "general"));
        }
        if context
            .iter()
            .any(|message| tool_result_text(message).is_some())
        {
            return Response::Text("child done".to_owned());
        }
        Response::Call(ToolCall {
            id: "nested".to_owned(),
            name: "task".to_owned(),
            arguments: json!({"description": "nested", "prompt": "nested"}),
        })
    }));
    let runtime = make_runtime(
        base_config(),
        Arc::clone(&client),
        base_toolbox(),
        dir.path(),
    );
    let toolbox: Arc<dyn Toolbox> = Arc::new(SubagentToolbox::new(base_toolbox(), runtime));
    let agent = Agent::new(client.clone(), toolbox);
    let mut parent_config = AgentConfig::new(model_ref(), dir.path().to_path_buf()).unwrap();
    parent_config.max_turns = 5;
    let events = collect_run(agent.run(Message::user("PARENT"), Vec::new(), parent_config)).await;
    assert_eq!(run_status(&events), RunStatus::Completed);
    let requests = client.requests();
    let nested = requests
        .iter()
        .find(|request| {
            request.iter().any(|message| {
                tool_result_text(message).is_some_and(|text| text.contains("not found"))
            })
        })
        .expect("nested task must resolve to an unknown tool");
    assert!(nested.iter().any(|message| {
        tool_result_text(message).is_some_and(|text| text.contains("`task` not found"))
    }));
}

#[tokio::test]
#[spec("AGENT-020")]
async fn spec_agent_020_result_contract_and_truncation() {
    let dir = tempfile::tempdir().unwrap();
    let long = format!("{}TAIL-MARKER", "a".repeat(5_000));
    let client = Arc::new(TestClient::new(move |_| Response::Text(long.clone())));
    let mut config = base_config();
    config.result_max_bytes = 1_024;
    let runtime = make_runtime(config, client, base_toolbox(), dir.path());
    let result = run_task(&runtime, &tool_context(dir.path(), 0), "work", "general").await;
    assert!(!result.is_error);
    let text = result.content[0].as_text().unwrap();
    assert!(text.contains("output truncated"), "{text}");
    assert!(
        text.contains("TAIL-MARKER"),
        "the conclusion survives: {text}"
    );
    assert!(text.contains("untrusted data"), "{text}");
    assert!(text.len() < 1_300, "{}", text.len());

    let failing = Arc::new(TestClient::new(|_| Response::Error("boom".to_owned())));
    let runtime = make_runtime(base_config(), failing, base_toolbox(), dir.path());
    let result = run_task(&runtime, &tool_context(dir.path(), 0), "work", "general").await;
    assert!(result.is_error);
    assert_eq!(result.details.unwrap()["status"], "error");
}

#[tokio::test]
#[spec("AGENT-021")]
async fn spec_agent_021_child_session_linked() {
    let dir = tempfile::tempdir().unwrap();
    let client = Arc::new(TestClient::new(|_| Response::Text("child".to_owned())));
    let runtime = make_runtime(base_config(), client, base_toolbox(), dir.path());
    let result = run_task(&runtime, &tool_context(dir.path(), 0), "work", "explore").await;
    assert!(!result.is_error);
    let details = result.details.unwrap();
    let session_id = details["session_id"].as_str().expect("session id");
    assert!(details["persisted"].as_bool().unwrap());

    let files = session_files(dir.path());
    assert_eq!(files.len(), 1, "{files:?}");
    let child = Session::open(&files[0]).unwrap();
    assert_eq!(child.id(), session_id);
    assert_eq!(child.parent_session(), Some("ses_parent"));
    let custom = child.latest_custom("subagent").expect("subagent entry");
    assert_eq!(custom["agent"], "explore");
    assert_eq!(custom["depth"], 1);
    assert!(custom["prompt_sha256"].as_str().unwrap().len() == 64);
}

#[tokio::test]
#[spec("AGENT-022")]
async fn spec_agent_022_tool_subset_no_escalation() {
    let dir = tempfile::tempdir().unwrap();
    let bash = Arc::new(EchoTool::new("bash"));
    let bash_calls = Arc::clone(&bash);
    let client = Arc::new(TestClient::new(|context| {
        if context
            .iter()
            .filter_map(user_text)
            .any(|text| text.contains("PARENT"))
        {
            if context
                .iter()
                .any(|message| matches!(message, Message::ToolResult(_)))
            {
                return Response::Text("parent done".to_owned());
            }
            return Response::Call(task_call("CHILD", "explore"));
        }
        if context
            .iter()
            .any(|message| tool_result_text(message).is_some())
        {
            return Response::Text("child done".to_owned());
        }
        Response::Call(ToolCall {
            id: "bash".to_owned(),
            name: "bash".to_owned(),
            arguments: json!({"command": "rm -rf /"}),
        })
    }));
    let mut config = base_config();
    config.types.insert(
        "explore".to_owned(),
        AgentTypeConfig {
            model: None,
            tools: ToolSelection::Only(vec!["read".to_owned()]),
            ..AgentTypeConfig::default()
        },
    );
    let parent = Arc::new(TestToolbox::new(vec![
        Arc::new(EchoTool::new("read")) as Arc<dyn Tool>,
        bash as Arc<dyn Tool>,
    ]));
    let runtime = make_runtime(config, Arc::clone(&client), parent, dir.path());
    let toolbox: Arc<dyn Toolbox> = Arc::new(SubagentToolbox::new(base_toolbox(), runtime));
    let agent = Agent::new(client.clone(), toolbox);
    let mut parent_config = AgentConfig::new(model_ref(), dir.path().to_path_buf()).unwrap();
    parent_config.max_turns = 5;
    let events = collect_run(agent.run(Message::user("PARENT"), Vec::new(), parent_config)).await;
    assert_eq!(run_status(&events), RunStatus::Completed);
    assert_eq!(bash_calls.calls.load(Ordering::SeqCst), 0);
    assert!(client.requests().iter().any(|request| {
        request.iter().any(|message| {
            tool_result_text(message).is_some_and(|text| text.contains("`bash` not found"))
        })
    }));
}

#[tokio::test]
#[spec("AGENT-023")]
async fn spec_agent_023_aggregate_budget_enforced() {
    let dir = tempfile::tempdir().unwrap();
    let client = Arc::new(TestClient::new(|_| Response::Text("child".to_owned())));
    let mut config = base_config();
    config.max_total_tokens = Some(1);
    let runtime = make_runtime(config, client, base_toolbox(), dir.path());
    let first = run_task(&runtime, &tool_context(dir.path(), 0), "one", "general").await;
    assert!(!first.is_error, "{first:?}");
    let second = run_task(&runtime, &tool_context(dir.path(), 0), "two", "general").await;
    assert!(second.is_error);
    assert!(
        second.content[0]
            .as_text()
            .unwrap()
            .contains("token budget exhausted"),
        "{second:?}"
    );
}

#[tokio::test]
#[spec("AGENT-024")]
async fn spec_agent_024_progress_without_parent_pollution() {
    let dir = tempfile::tempdir().unwrap();
    let client = Arc::new(TestClient::new(|context| {
        if context
            .iter()
            .any(|message| tool_result_text(message).is_some())
        {
            Response::Text("child done".to_owned())
        } else {
            Response::Call(ToolCall {
                id: "read".to_owned(),
                name: "read".to_owned(),
                arguments: json!({}),
            })
        }
    }));
    let runtime = make_runtime(base_config(), client, base_toolbox(), dir.path());
    let (tx, mut rx) = mpsc::unbounded_channel();
    let result = TaskTool::new(runtime)
        .execute(
            &task_call("work", "general"),
            &tool_context(dir.path(), 0),
            Some(tx),
        )
        .await;
    assert!(!result.is_error);
    let mut updates = Vec::new();
    while let Ok(update) = rx.try_recv() {
        updates.push(update);
    }
    assert!(!updates.is_empty(), "expected at least one progress update");
    assert!(
        updates.iter().any(|update| update
            .content
            .iter()
            .filter_map(Content::as_text)
            .any(|text| text.contains("read"))),
        "the current tool should be visible in the activity label: {updates:?}"
    );
    let details = updates[0].details.clone().unwrap();
    assert!(details["subagent"]["session_id"].is_string());
    assert!(details["subagent"]["turns"].as_u64().unwrap() >= 1);
}

#[tokio::test]
#[spec("AGENT-025")]
async fn spec_agent_025_model_selection_validated() {
    let dir = tempfile::tempdir().unwrap();
    let client = Arc::new(TestClient::new(|_| Response::Text("child".to_owned())));
    let runtime = make_runtime(base_config(), client, base_toolbox(), dir.path());

    let mut call = task_call("work", "general");
    call.arguments["model"] = json!("unknown-model");
    let result = TaskTool::new(Arc::clone(&runtime))
        .execute(&call, &tool_context(dir.path(), 0), None)
        .await;
    assert!(result.is_error);
    assert!(
        result.content[0]
            .as_text()
            .unwrap()
            .contains("Unknown subagent model")
    );

    call.arguments["model"] = json!(MODEL);
    let result = TaskTool::new(runtime)
        .execute(&call, &tool_context(dir.path(), 0), None)
        .await;
    assert!(!result.is_error, "{result:?}");
}

#[tokio::test]
#[spec("AGENT-027")]
async fn spec_agent_027_child_failure_is_parent_result() {
    let dir = tempfile::tempdir().unwrap();
    let client = Arc::new(TestClient::new(|context| {
        if context
            .iter()
            .filter_map(user_text)
            .any(|text| text.contains("PARENT"))
        {
            if context
                .iter()
                .any(|message| matches!(message, Message::ToolResult(_)))
            {
                return Response::Text("parent done".to_owned());
            }
            Response::Call(task_call("CHILD", "general"))
        } else {
            Response::Error("child exploded".to_owned())
        }
    }));
    let runtime = make_runtime(
        base_config(),
        Arc::clone(&client),
        base_toolbox(),
        dir.path(),
    );
    let toolbox: Arc<dyn Toolbox> = Arc::new(SubagentToolbox::new(base_toolbox(), runtime));
    let agent = Agent::new(client, toolbox);
    let mut parent_config = AgentConfig::new(model_ref(), dir.path().to_path_buf()).unwrap();
    parent_config.max_turns = 3;
    let events = collect_run(agent.run(Message::user("PARENT"), Vec::new(), parent_config)).await;
    assert_eq!(run_status(&events), RunStatus::Completed);
    let result = run_messages(&events)
        .iter()
        .find_map(|message| match message {
            Message::ToolResult(result) => Some(result.clone()),
            _ => None,
        })
        .expect("task result");
    assert!(result.is_error);
    assert_eq!(result.details.unwrap()["status"], "error");
}

#[test]
#[spec("AGENT-031")]
fn agent_definitions_extend_the_task_surface() {
    let mut config = base_config();
    let mut reviewer = persona(
        "ui-reviewer",
        "Reviews rendered interfaces and accessibility.",
        "PERSONA-MARKER: check contrast and focus order.",
    );
    reviewer.max_turns = Some(4);
    reviewer.timeout_secs = Some(45);
    config.apply_agent_definitions(&[reviewer]);
    config.validate().expect("valid configuration");

    assert_eq!(
        schema(&config)["properties"]["agent"]["enum"],
        json!(["general", "explore", "ui-reviewer"]),
        "general stays first and the rest are sorted"
    );
    let description = describe_types(&config);
    assert!(description.contains("ui-reviewer: Reviews rendered interfaces"));
    let reviewer = config.agent_type("ui-reviewer").expect("persona");
    assert_eq!(reviewer.max_turns, Some(4));
    assert_eq!(reviewer.timeout, Some(Duration::from_secs(45)));
    assert_eq!(reviewer.tools, ToolSelection::Only(vec!["read".to_owned()]));
}

#[tokio::test]
#[spec("AGENT-032")]
async fn persona_prompt_reaches_the_child_system_message() {
    let dir = tempfile::tempdir().unwrap();
    let client = Arc::new(TestClient::new(|_| Response::Text("reviewed".to_owned())));
    let mut config = base_config();
    config.apply_agent_definitions(&[persona(
        "ui-reviewer",
        "Reviews interfaces.",
        "PERSONA-MARKER: check contrast and focus order.",
    )]);
    let runtime = make_runtime(config, Arc::clone(&client), base_toolbox(), dir.path());
    let result = run_task(
        &runtime,
        &tool_context(dir.path(), 0),
        "review the header",
        "ui-reviewer",
    )
    .await;
    assert!(!result.is_error, "{result:?}");
    let system = client
        .requests()
        .first()
        .and_then(|request| request.iter().find_map(system_text))
        .expect("system prompt");
    assert!(system.contains("PERSONA-MARKER"), "{system}");
    assert!(
        system.contains("You are a subagent spawned by another session"),
        "{system}"
    );
}

#[tokio::test]
#[spec("AGENT-032")]
async fn per_type_turn_budget_applies() {
    let dir = tempfile::tempdir().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&calls);
    let client = Arc::new(TestClient::new(move |_| {
        let index = counter.fetch_add(1, Ordering::SeqCst);
        Response::Call(ToolCall {
            id: format!("call_{index}"),
            name: "read".to_owned(),
            arguments: json!({"path": "src/lib.rs"}),
        })
    }));
    let mut config = base_config();
    let mut reviewer = persona("ui-reviewer", "Reviews interfaces.", "Review.");
    reviewer.max_turns = Some(1);
    config.apply_agent_definitions(&[reviewer]);
    let runtime = make_runtime(config, client, base_toolbox(), dir.path());
    let result = run_task(
        &runtime,
        &tool_context(dir.path(), 0),
        "review the header",
        "ui-reviewer",
    )
    .await;
    assert!(result.is_error);
    assert!(
        result.content[0].as_text().unwrap().contains("turn_limit"),
        "{result:?}"
    );
    assert!(calls.load(Ordering::SeqCst) <= 2);
}

#[tokio::test]
#[spec("AGENT-031")]
async fn unknown_types_list_the_configured_names() {
    let dir = tempfile::tempdir().unwrap();
    let client = Arc::new(TestClient::new(|_| Response::Text("unused".to_owned())));
    let mut config = base_config();
    config.apply_agent_definitions(&[persona("ui-reviewer", "Reviews interfaces.", "Review.")]);
    let runtime = make_runtime(config, client, base_toolbox(), dir.path());
    let result = run_task(&runtime, &tool_context(dir.path(), 0), "x", "nope").await;
    assert!(result.is_error);
    let text = result.content[0].as_text().unwrap();
    assert!(text.contains("ui-reviewer"), "{text}");
    assert!(text.contains("explore"), "{text}");
    assert!(text.contains("general"), "{text}");
}

#[tokio::test]
#[spec("AGENT-036")]
async fn changing_the_parent_model_routes_children_to_it() {
    let dir = tempfile::tempdir().unwrap();
    let client = Arc::new(TestClient::new(|_| Response::Text("child".to_owned())));
    let runtime = make_runtime(
        base_config(),
        Arc::clone(&client),
        base_toolbox(),
        dir.path(),
    );
    let next = ModelRef {
        api: API.to_owned(),
        provider: "openrouter".to_owned(),
        model: "tencent/hy4-flash".to_owned(),
    };
    runtime.set_parent_model(next.clone(), vec![next.model.clone()]);
    assert_eq!(runtime.parent_model(), next);

    let result = run_task(&runtime, &tool_context(dir.path(), 0), "review", "general").await;
    assert!(!result.is_error, "{result:?}");
    let used = client.models().pop().expect("child request");
    assert_eq!(used.provider, "openrouter");
    assert_eq!(used.model, "tencent/hy4-flash");
}

#[tokio::test]
#[spec("AGENT-037")]
async fn queued_children_are_visible_and_cancellable() {
    let dir = tempfile::tempdir().unwrap();
    let client = Arc::new(
        TestClient::new(|_| Response::Text("done".to_owned()))
            .with_delay(Duration::from_millis(300)),
    );
    let mut config = base_config();
    config.max_parallel = 1;
    let runtime = make_runtime(config, client, base_toolbox(), dir.path());

    let first = tokio::spawn({
        let runtime = Arc::clone(&runtime);
        let dir = dir.path().to_path_buf();
        async move {
            let mut call = task_call("one", "general");
            call.id = "call_one".to_owned();
            TaskTool::new(runtime)
                .execute(&call, &tool_context(&dir, 0), None)
                .await
        }
    });
    for _ in 0..100 {
        if runtime
            .runs()
            .iter()
            .any(|run| run.state == codie_subagents::RunState::Running)
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }

    let second = tokio::spawn({
        let runtime = Arc::clone(&runtime);
        let dir = dir.path().to_path_buf();
        async move {
            let mut call = task_call("two", "general");
            call.id = "call_two".to_owned();
            TaskTool::new(runtime)
                .execute(&call, &tool_context(&dir, 0), None)
                .await
        }
    });
    let queued = loop {
        if let Some(run) = runtime
            .runs()
            .into_iter()
            .find(|run| run.state == codie_subagents::RunState::Queued)
        {
            break run;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    };
    assert_eq!(queued.activity, "waiting for a slot");
    assert!(runtime.cancel_run(&queued.call_id));

    let cancelled = second.await.unwrap();
    assert!(cancelled.is_error);
    assert!(
        cancelled.content[0]
            .as_text()
            .unwrap()
            .contains("cancelled before it started"),
        "{cancelled:?}"
    );
    assert!(!first.await.unwrap().is_error);
}

#[tokio::test]
#[spec("AGENT-038")]
async fn nested_task_uses_the_installed_gate() {
    let dir = tempfile::tempdir().unwrap();
    let client = Arc::new(TestClient::new(|context| {
        if context
            .iter()
            .filter_map(user_text)
            .any(|text| text.contains("PARENT"))
        {
            if context
                .iter()
                .any(|message| matches!(message, Message::ToolResult(_)))
            {
                return Response::Text("parent done".to_owned());
            }
            return Response::Call(task_call("CHILD", "general"));
        }
        if context
            .iter()
            .any(|message| tool_result_text(message).is_some())
        {
            return Response::Text("child done".to_owned());
        }
        Response::Call(task_call("GRANDCHILD", "general"))
    }));
    let mut config = base_config();
    config.max_depth = 1;
    let runtime = make_runtime(config, Arc::clone(&client), base_toolbox(), dir.path());
    let gate = Arc::new(EchoTool::new("task"));
    runtime.set_nested_task(gate.clone());

    let toolbox: Arc<dyn Toolbox> = Arc::new(SubagentToolbox::new(base_toolbox(), runtime));
    let chat: Arc<dyn ChatClient> = client.clone();
    let agent = Agent::new(chat, toolbox);
    let mut parent_config = AgentConfig::new(model_ref(), dir.path().to_path_buf()).unwrap();
    parent_config.max_turns = 5;
    let events = collect_run(agent.run(Message::user("PARENT"), Vec::new(), parent_config)).await;
    assert_eq!(run_status(&events), RunStatus::Completed);
    assert_eq!(
        gate.calls.load(Ordering::SeqCst),
        1,
        "the nested task must go through the installed gate"
    );
    assert!(
        !client
            .requests()
            .iter()
            .flatten()
            .any(|message| { user_text(message).is_some_and(|text| text.contains("GRANDCHILD")) }),
        "no grandchild may start behind the gate"
    );
}

#[test]
#[spec("TUI-048")]
fn parallelism_changes_only_while_idle() {
    let dir = tempfile::tempdir().unwrap();
    let client = Arc::new(TestClient::new(|_| Response::Text("done".to_owned())));
    let runtime = make_runtime(base_config(), client, base_toolbox(), dir.path());
    assert!(runtime.set_max_parallel(2));
    assert_eq!(runtime.max_parallel(), 2);
    assert!(!runtime.set_max_parallel(0));
    assert!(!runtime.set_max_parallel(99));
}

#[tokio::test]
#[spec("AGENT-039")]
async fn child_provider_errors_are_reported() {
    let dir = tempfile::tempdir().unwrap();
    let client = Arc::new(TestClient::new(|_| {
        Response::Error("model not found on this provider".to_owned())
    }));
    let runtime = make_runtime(base_config(), client, base_toolbox(), dir.path());
    let result = run_task(&runtime, &tool_context(dir.path(), 0), "review", "general").await;
    assert!(result.is_error);
    let text = result.content[0].as_text().unwrap();
    assert!(
        text.contains("model not found on this provider"),
        "the provider error must reach the parent: {text}"
    );
    assert_eq!(
        result.details.as_ref().unwrap()["error"],
        "model not found on this provider"
    );
}

struct CountingHooks {
    tools: Mutex<Vec<String>>,
}

#[async_trait]
impl Hooks for CountingHooks {
    async fn before_tool(&self, call: &ToolCall) -> Result<(), String> {
        self.tools.lock().expect("hooks").push(call.name.clone());
        Ok(())
    }

    async fn transform_context(&self, context: HookContext) -> Result<HookContext, String> {
        Ok(context)
    }
}

#[tokio::test]
#[spec("AGENT-026")]
async fn extension_hooks_do_not_run_inside_children() {
    let dir = tempfile::tempdir().unwrap();
    let client = Arc::new(TestClient::new(|context| {
        let child = context
            .iter()
            .filter_map(user_text)
            .any(|text| text.contains("CHILD"));
        let results = context
            .iter()
            .filter(|message| matches!(message, Message::ToolResult(_)))
            .count();
        if child {
            if results > 0 {
                return Response::Text("child done".to_owned());
            }
            return Response::Call(ToolCall {
                id: "child_read".to_owned(),
                name: "read".to_owned(),
                arguments: json!({"path": "src/lib.rs"}),
            });
        }
        match results {
            0 => Response::Call(ToolCall {
                id: "parent_read".to_owned(),
                name: "read".to_owned(),
                arguments: json!({"path": "Cargo.toml"}),
            }),
            1 => Response::Call(task_call("CHILD", "general")),
            _ => Response::Text("parent done".to_owned()),
        }
    }));
    let runtime = make_runtime(
        base_config(),
        Arc::clone(&client),
        base_toolbox(),
        dir.path(),
    );
    let toolbox: Arc<dyn Toolbox> = Arc::new(SubagentToolbox::new(base_toolbox(), runtime));
    let hooks = Arc::new(CountingHooks {
        tools: Mutex::new(Vec::new()),
    });
    let agent = Agent::with_hooks(client, toolbox, hooks.clone());
    let mut parent_config = AgentConfig::new(model_ref(), dir.path().to_path_buf()).unwrap();
    parent_config.max_turns = 5;
    let events = collect_run(agent.run(Message::user("PARENT"), Vec::new(), parent_config)).await;
    assert_eq!(run_status(&events), RunStatus::Completed);
    let tools = hooks.tools.lock().expect("hooks").clone();
    assert_eq!(
        tools,
        vec!["read".to_owned(), "task".to_owned()],
        "the child's read must not reach the parent's hook"
    );
}

#[tokio::test]
#[spec("AGENT-048")]
async fn timeout_reports_a_timed_out_result_without_waiting() {
    let dir = tempfile::tempdir().unwrap();
    let client = Arc::new(
        TestClient::new(|_| Response::Text("late answer".to_owned()))
            .with_delay(Duration::from_secs(10)),
    );
    let mut config = base_config();
    config.timeout = Duration::from_millis(50);
    let runtime = make_runtime(config, client, base_toolbox(), dir.path());

    let result = tokio::time::timeout(
        Duration::from_secs(2),
        run_task(&runtime, &tool_context(dir.path(), 0), "work", "general"),
    )
    .await
    .expect("the timeout must not wait for the provider");
    assert!(result.is_error);
    let text = result.content[0].as_text().unwrap();
    assert!(text.contains("timed out"), "{text}");
    let details = result.details.unwrap();
    assert_eq!(details["status"], "aborted");
    assert!(details["error"].as_str().unwrap().contains("timed out"));
}
