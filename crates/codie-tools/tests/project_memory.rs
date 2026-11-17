//! Integration tests for the `project_memory` tool (`MEM-012`..`MEM-014`).

use std::path::Path;

use codie_core::{Tool, ToolCall, ToolContext, ToolResult, Toolbox};
use codie_testkit::spec;
use codie_tools::{Policy, ProjectMemoryTool, Registry, is_read_only};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

fn context(cwd: &Path) -> ToolContext {
    ToolContext {
        cwd: cwd.to_path_buf(),
        cancel: CancellationToken::new(),
        session_id: None,
    }
}

async fn run(arguments: Value, context: &ToolContext) -> ToolResult {
    let tool = ProjectMemoryTool::new();
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

fn project() -> tempfile::TempDir {
    let directory = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(directory.path().join(".git")).expect("git marker");
    directory
}

#[tokio::test]
#[spec("MEM-012")]
async fn spec_mem_012_tool_appends_creates_sections_and_dedupes() {
    let directory = project();
    let cwd = context(directory.path());

    let first = run(
        json!({"section": "Pitfalls", "text": "Do not edit generated files."}),
        &cwd,
    )
    .await;
    assert!(!first.is_error, "{}", text(&first));

    let duplicate = run(
        json!({"section": "Pitfalls", "text": "do not  EDIT generated files"}),
        &cwd,
    )
    .await;
    assert!(!duplicate.is_error);

    let section = run(
        json!({"section": "Commands", "text": "`cargo test` runs the suite."}),
        &cwd,
    )
    .await;
    assert!(!section.is_error);

    let memory = std::fs::read_to_string(directory.path().join("CODIE.MD")).expect("memory file");
    assert_eq!(memory.matches("generated files").count(), 1, "{memory}");
    assert!(memory.contains("## Commands"));
    assert!(memory.contains("`cargo test` runs the suite."));

    let replaced = run(
        json!({"mode": "replace", "section": "Commands", "text": "- `cargo xtask ci`"}),
        &cwd,
    )
    .await;
    assert!(!replaced.is_error);
    let memory = std::fs::read_to_string(directory.path().join("CODIE.MD")).expect("memory file");
    assert!(memory.contains("- `cargo xtask ci`"));
    assert!(!memory.contains("runs the suite"));
    assert!(memory.contains("generated files"), "other sections survive");
}

#[tokio::test]
#[spec("MEM-013")]
async fn spec_mem_013_tool_redacts_credentials_and_enforces_caps() {
    let directory = project();
    let cwd = context(directory.path());

    let result = run(
        json!({"section": "Commands", "text": "deploy with DEPLOY_TOKEN=abcdef1234567890"}),
        &cwd,
    )
    .await;
    assert!(!result.is_error, "{}", text(&result));
    let memory = std::fs::read_to_string(directory.path().join("CODIE.MD")).expect("memory file");
    assert!(memory.contains("DEPLOY_TOKEN=[redacted]"));
    assert!(!memory.contains("abcdef1234567890"));

    let long = "x".repeat(codie_tools::project_memory::MAX_ENTRY_BYTES + 1);
    let too_long = run(json!({"section": "Pitfalls", "text": long}), &cwd).await;
    assert!(too_long.is_error);

    let unknown = run(json!({"file": "notes", "section": "A", "text": "b"}), &cwd).await;
    assert!(unknown.is_error);
    assert!(text(&unknown).contains("codie` or `plan"));

    let plan = run(
        json!({"file": "plan", "section": "Goal", "text": "Ship the thing."}),
        &cwd,
    )
    .await;
    assert!(!plan.is_error);
    let plan = std::fs::read_to_string(directory.path().join("PLAN.MD")).expect("plan file");
    assert!(plan.contains("- Ship the thing."));
}

#[tokio::test]
#[spec("MEM-014")]
async fn spec_mem_014_tool_is_mutating_for_permissions() {
    assert!(!is_read_only("project_memory"));
    assert!(
        Policy::headless(Vec::<String>::new())
            .denial("project_memory")
            .is_some()
    );
    assert!(
        Policy::headless(["project_memory"])
            .denial("project_memory")
            .is_none()
    );
    assert!(Registry::builtin().get("project_memory").is_some());
    assert!(is_read_only("read"));
}
