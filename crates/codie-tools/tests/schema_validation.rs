//! Tool argument validation (`TOOL-002`).

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use codie_core::{ExecutionMode, Tool, ToolCall, ToolContext, ToolProgress, ToolResult, Toolbox};
use codie_testkit::spec;
use codie_tools::Registry;
use serde_json::json;
use tokio::sync::mpsc;

struct CountingTool {
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl Tool for CountingTool {
    fn name(&self) -> &str {
        "count"
    }

    fn description(&self) -> &str {
        "counts executions"
    }

    fn schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {"value": {"type": "integer", "minimum": 1}},
            "required": ["value"],
            "additionalProperties": false
        })
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
        ToolResult::text("executed")
    }
}

fn context(dir: &std::path::Path) -> ToolContext {
    ToolContext {
        cwd: dir.to_path_buf(),
        cancel: tokio_util::sync::CancellationToken::new(),
        session_id: None,
    }
}

fn call(name: &str, arguments: serde_json::Value) -> ToolCall {
    ToolCall {
        id: "call_1".to_owned(),
        name: name.to_owned(),
        arguments,
    }
}

#[tokio::test]
#[spec("TOOL-001")]
async fn spec_tool_001_registry_declares_schemas() {
    let registry = Registry::builtin();
    let declarations = registry.declarations();
    assert_eq!(declarations.len(), 8);
    for declaration in declarations {
        assert_eq!(declaration.parameters["type"], "object");
        assert!(
            declaration.parameters["properties"].is_object(),
            "{} must declare properties",
            declaration.name
        );
    }
}

#[tokio::test]
#[spec("TOOL-002")]
#[spec("SEC-004")]
async fn spec_tool_002_invalid_arguments_never_execute() {
    let calls = Arc::new(AtomicUsize::new(0));
    let registry = Registry::new(vec![Arc::new(CountingTool {
        calls: Arc::clone(&calls),
    })]);
    let tool = registry.get("count").expect("tool is registered");
    let dir = tempfile::tempdir().expect("temp dir");

    let missing = tool
        .execute(&call("count", json!({})), &context(dir.path()), None)
        .await;
    assert!(missing.is_error);
    assert!(
        missing.content[0]
            .as_text()
            .is_some_and(|text| text.contains("Invalid arguments") && text.contains("value"))
    );

    let wrong_type = tool
        .execute(
            &call("count", json!({"value": "not-a-number"})),
            &context(dir.path()),
            None,
        )
        .await;
    assert!(wrong_type.is_error);

    let below_minimum = tool
        .execute(
            &call("count", json!({"value": 0})),
            &context(dir.path()),
            None,
        )
        .await;
    assert!(below_minimum.is_error);

    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "invalid arguments must not reach the tool"
    );

    let valid = tool
        .execute(
            &call("count", json!({"value": 2})),
            &context(dir.path()),
            None,
        )
        .await;
    assert!(!valid.is_error);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
#[spec("TOOL-002")]
async fn spec_tool_002_builtin_read_validates_and_runs() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("hello.txt");
    std::fs::write(&path, "hello\n").expect("write file");

    let registry = Registry::builtin();
    let read = registry.get("read").expect("read tool");

    let invalid = read
        .execute(
            &call("read", json!({"path": "hello.txt", "offset": 0})),
            &context(dir.path()),
            None,
        )
        .await;
    assert!(invalid.is_error, "offset 0 violates minimum 1");

    let valid = read
        .execute(
            &call("read", json!({"path": "hello.txt"})),
            &context(dir.path()),
            None,
        )
        .await;
    assert!(!valid.is_error);
    assert!(
        valid.content[0]
            .as_text()
            .is_some_and(|text| text.contains("hello"))
    );
}
