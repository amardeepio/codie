//! Windows integration tests for the `powershell` tool (`TOOL-030`).

#![cfg(windows)]

use std::path::Path;
use std::time::Duration;

use codie_core::{Tool, ToolCall, ToolContext, ToolResult};
use codie_testkit::spec;
use codie_tools::PowerShellTool;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

fn context(cwd: &Path) -> ToolContext {
    ToolContext {
        cwd: cwd.to_path_buf(),
        cancel: CancellationToken::new(),
        session_id: None,
    }
}

async fn run(tool: &dyn Tool, arguments: Value, context: &ToolContext) -> ToolResult {
    let call = ToolCall {
        id: "call-1".to_owned(),
        name: tool.name().to_owned(),
        arguments,
    };
    tool.execute(&call, context, None).await
}

fn text(result: &ToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|content| content.as_text())
        .collect::<Vec<_>>()
        .join("\n")
}

#[spec("TOOL-030")]
#[tokio::test]
async fn powershell_returns_output_and_exit_details() {
    let directory = tempfile::tempdir().expect("temp dir");
    let context = context(directory.path());

    let result = run(
        &PowerShellTool,
        json!({"command": "Write-Output 'hello'"}),
        &context,
    )
    .await;
    assert!(!result.is_error, "{}", text(&result));
    assert_eq!(text(&result).trim(), "hello");
    let details = result.details.expect("details");
    assert_eq!(details["exit_code"], 0);
    assert_eq!(details["truncated"], false);
    assert!(details["wall_time_seconds"].as_f64().is_some());
}

#[spec("TOOL-030")]
#[tokio::test]
async fn powershell_marks_non_zero_exit_as_error() {
    let directory = tempfile::tempdir().expect("temp dir");
    let context = context(directory.path());

    let result = run(
        &PowerShellTool,
        json!({"command": "Write-Error 'bad'; exit 3"}),
        &context,
    )
    .await;
    assert!(result.is_error);
    let output = text(&result);
    assert!(output.contains("bad"), "{output}");
    assert!(output.ends_with("Command exited with code 3"), "{output}");
    assert_eq!(result.details.expect("details")["exit_code"], 3);
}

#[spec("TOOL-030")]
#[tokio::test]
async fn powershell_times_out() {
    let directory = tempfile::tempdir().expect("temp dir");
    let context = context(directory.path());

    let result = run(
        &PowerShellTool,
        json!({"command": "Start-Sleep -Seconds 5", "timeout": 1}),
        &context,
    )
    .await;
    assert!(result.is_error);
    assert!(
        text(&result).starts_with("Command timed out after 1 seconds"),
        "{}",
        text(&result)
    );
    assert_eq!(result.details.expect("details")["exit_code"], -1);
}

#[tokio::test]
async fn powershell_supports_abort() {
    let directory = tempfile::tempdir().expect("temp dir");
    let context = context(directory.path());
    let cancel = context.cancel.clone();
    let tool_context = context.clone();

    let task = tokio::spawn(async move {
        run(
            &PowerShellTool,
            json!({"command": "Start-Sleep -Seconds 30"}),
            &tool_context,
        )
        .await
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    cancel.cancel();
    let result = task.await.expect("task");
    assert!(result.is_error);
    assert_eq!(text(&result), "Command aborted");
}
