//! Conformance and supervision tests for out-of-process extensions.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use codie_core::{ToolCall, ToolContext};
use codie_ext::{ExtensionSet, LoadOptions};
use codie_protocol::ext::ExtHook;
use codie_testkit::spec;
use serde_json::json;

fn examples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("examples")
        .join("extensions")
}

async fn load_example(name: &str, cwd: &Path, config: &Path) -> ExtensionSet {
    let explicit = vec![examples().join(name)];
    let flags = BTreeMap::new();
    ExtensionSet::load(LoadOptions {
        config_dir: config,
        project_root: None,
        explicit: &explicit,
        disabled: &[],
        cwd,
        session_id: Some("test-session"),
        deferred_flags: &flags,
    })
    .await
}

fn node_available() -> bool {
    std::process::Command::new("node")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

fn context(cwd: &Path) -> ToolContext {
    ToolContext {
        cwd: cwd.to_path_buf(),
        cancel: tokio_util::sync::CancellationToken::new(),
        session_id: Some("test-session".to_owned()),
    }
}

#[tokio::test]
#[spec("EXT-001")]
async fn spec_ext_001_process_boundary() {
    let home = tempfile::tempdir().expect("home");
    let project = tempfile::tempdir().expect("project");
    let set = load_example("todo", project.path(), home.path()).await;
    assert!(!set.is_empty(), "warnings: {:?}", set.warnings());
    assert_eq!(set.extensions()[0].name, "todo");
    set.shutdown("test").await;
}

#[tokio::test]
#[spec("EXT-002")]
async fn spec_ext_002_hook_surface() {
    if !node_available() {
        eprintln!("skipping: node is not available");
        return;
    }
    let home = tempfile::tempdir().expect("home");
    let project = tempfile::tempdir().expect("project");
    let set = load_example("plan-mode", project.path(), home.path()).await;
    assert!(set.handles(ExtHook::Context));
    assert!(set.handles(ExtHook::BeforeToolCall));
    set.shutdown("test").await;
}

#[tokio::test]
#[spec("EXT-003")]
async fn spec_ext_003_before_tool_can_block() {
    if !node_available() {
        eprintln!("skipping: node is not available");
        return;
    }
    let home = tempfile::tempdir().expect("home");
    let project = tempfile::tempdir().expect("project");
    let set = load_example("plan-mode", project.path(), home.path()).await;
    let invocation = set
        .resolve_command("/plan on")
        .expect("resolve")
        .expect("command");
    let session = codie_protocol::ext::ExtSessionInfo {
        id: Some("test-session".to_owned()),
        cwd: project.path().display().to_string(),
        model: None,
    };
    set.invoke_command(&invocation, &session)
        .await
        .expect("command works");

    let hooks = set.hooks().expect("hooks");
    let call = ToolCall {
        id: "call_1".to_owned(),
        name: "write".to_owned(),
        arguments: json!({"path": "a.txt", "content": "x"}),
    };
    let error = hooks.before_tool(&call).await.expect_err("blocked");
    assert!(error.contains("Plan mode"));
    set.shutdown("test").await;
}

#[tokio::test]
#[spec("EXT-006")]
async fn spec_ext_006_manifest_round_trip() {
    let manifest =
        codie_ext::read_manifest(&examples().join("todo").join("codie.toml")).expect("manifest");
    assert_eq!(manifest.manifest.name, "todo");
    assert_eq!(manifest.manifest.protocol, 1);
    assert!(!manifest.manifest.entrypoint.is_empty());
}

#[tokio::test]
#[spec("EXT-010")]
async fn spec_ext_010_extension_tools() {
    let home = tempfile::tempdir().expect("home");
    let project = tempfile::tempdir().expect("project");
    let set = load_example("todo", project.path(), home.path()).await;
    let tools = set.tools();
    let add = tools
        .iter()
        .find(|tool| tool.name() == "todo_add")
        .expect("todo_add");
    let result = add
        .execute(
            &ToolCall {
                id: "call_1".to_owned(),
                name: "todo_add".to_owned(),
                arguments: json!({"text": "write the phase 6 tests"}),
            },
            &context(project.path()),
            None,
        )
        .await;
    assert!(!result.is_error, "result: {result:?}");
    assert!(result.content[0].as_text().unwrap().contains("Added"));
    set.shutdown("test").await;
}

#[tokio::test]
#[spec("EXT-011")]
async fn spec_ext_011_extension_commands() {
    let home = tempfile::tempdir().expect("home");
    let project = tempfile::tempdir().expect("project");
    let set = load_example("todo", project.path(), home.path()).await;
    let invocation = set
        .resolve_command("/todo")
        .expect("resolve")
        .expect("command");
    let session = codie_protocol::ext::ExtSessionInfo {
        id: Some("test-session".to_owned()),
        cwd: project.path().display().to_string(),
        model: None,
    };
    let result = set
        .invoke_command(&invocation, &session)
        .await
        .expect("command");
    assert!(result.display.is_some());
    set.shutdown("test").await;
}

#[tokio::test]
#[spec("EXT-013")]
async fn spec_ext_013_hung_extension_is_killed() {
    let home = tempfile::tempdir().expect("home");
    let project = tempfile::tempdir().expect("project");
    let extension = project.path().join("hung");
    std::fs::create_dir_all(&extension).unwrap();
    std::fs::write(
        extension.join("codie.toml"),
        r#"
name = "hung"
version = "0.1.0"
protocol = 1
entrypoint = ["python3", "hung.py"]
capabilities = ["tools"]

[limits]
hook_timeout_ms = 100
tool_timeout_ms = 100
"#,
    )
    .unwrap();
    std::fs::write(
        extension.join("hung.py"),
        r#"
import json
import sys
import time
for line in sys.stdin:
    request = json.loads(line)
    if request.get("method") == "initialize":
        print(json.dumps({"jsonrpc": "2.0", "id": request["id"], "result": {
            "protocol": 1, "name": "hung", "version": "0.1.0",
            "capabilities": ["tools"], "tools": [{
                "name": "hung_tool", "description": "hangs",
                "parameters": {"type": "object"}
            }]
        }}), flush=True)
    elif "id" in request:
        time.sleep(10)
"#,
    )
    .unwrap();
    let flags = BTreeMap::new();
    let set = ExtensionSet::load(LoadOptions {
        config_dir: home.path(),
        project_root: None,
        explicit: &[extension],
        disabled: &[],
        cwd: project.path(),
        session_id: None,
        deferred_flags: &flags,
    })
    .await;
    let tool = set.tools().into_iter().next().expect("tool");
    let started = std::time::Instant::now();
    let result = tool
        .execute(
            &ToolCall {
                id: "call_1".to_owned(),
                name: "hung_tool".to_owned(),
                arguments: json!({}),
            },
            &context(project.path()),
            None,
        )
        .await;
    assert!(result.is_error);
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "hung extension blocked for {:?}",
        started.elapsed()
    );
    set.shutdown("test").await;
}

#[tokio::test]
#[spec("EXT-015")]
async fn spec_ext_015_extension_flags_are_validated() {
    let set = ExtensionSet::default();
    let mut flags = BTreeMap::new();
    flags.insert("missing".to_owned(), String::new());
    assert!(set.validate_flags(&flags).is_err());
    assert!(set.validate_flags(&BTreeMap::new()).is_ok());
}

#[tokio::test]
#[spec("EXT-004")]
async fn spec_ext_004_failure_is_isolated() {
    let home = tempfile::tempdir().expect("home");
    let project = tempfile::tempdir().expect("project");
    let missing = project.path().join("missing");
    let explicit = vec![missing, examples().join("todo")];
    let flags = BTreeMap::new();
    let set = ExtensionSet::load(LoadOptions {
        config_dir: home.path(),
        project_root: None,
        explicit: &explicit,
        disabled: &[],
        cwd: project.path(),
        session_id: None,
        deferred_flags: &flags,
    })
    .await;
    assert!(!set.is_empty());
    assert!(!set.warnings().is_empty());
    set.shutdown("test").await;
}

#[tokio::test]
#[spec("EXT-005")]
async fn spec_ext_005_declarative_resources_are_optional() {
    let home = tempfile::tempdir().expect("home");
    let project = tempfile::tempdir().expect("project");
    let flags = BTreeMap::new();
    let set = ExtensionSet::load(LoadOptions {
        config_dir: home.path(),
        project_root: Some(project.path()),
        explicit: &[],
        disabled: &[],
        cwd: project.path(),
        session_id: None,
        deferred_flags: &flags,
    })
    .await;
    assert!(set.is_empty());
    assert!(set.warnings().is_empty());
    assert!(set.tools().is_empty());
    assert!(set.hooks().is_none());
}

#[tokio::test]
#[spec("EXT-008")]
async fn spec_ext_008_handshake_and_versioning() {
    let home = tempfile::tempdir().expect("home");
    let project = tempfile::tempdir().expect("project");
    let extension = project.path().join("mismatch");
    std::fs::create_dir_all(&extension).unwrap();
    std::fs::write(
        extension.join("codie.toml"),
        "name = \"mismatch\"\nversion = \"0.1.0\"\nprotocol = 1\nentrypoint = [\"python3\", \"ext.py\"]\ncapabilities = []\n",
    )
    .unwrap();
    std::fs::write(
        extension.join("ext.py"),
        r#"
import json, sys
for line in sys.stdin:
    request = json.loads(line)
    if request.get("method") == "initialize":
        print(json.dumps({"jsonrpc": "2.0", "id": request["id"], "result": {
            "protocol": 2, "name": "mismatch", "version": "0.1.0", "capabilities": []
        }}), flush=True)
"#,
    )
    .unwrap();
    let flags = BTreeMap::new();
    let set = ExtensionSet::load(LoadOptions {
        config_dir: home.path(),
        project_root: None,
        explicit: &[extension],
        disabled: &[],
        cwd: project.path(),
        session_id: None,
        deferred_flags: &flags,
    })
    .await;
    assert!(set.is_empty());
    assert!(
        set.warnings()
            .iter()
            .any(|warning| warning.contains("protocol"))
    );
}

#[tokio::test]
#[spec("EXT-012")]
async fn spec_ext_012_structured_renderers() {
    let home = tempfile::tempdir().expect("home");
    let project = tempfile::tempdir().expect("project");
    let set = load_example("todo", project.path(), home.path()).await;
    let tool = set
        .tools()
        .into_iter()
        .find(|tool| tool.name() == "todo_add")
        .expect("todo_add");
    let result = tool
        .execute(
            &ToolCall {
                id: "call_1".to_owned(),
                name: "todo_add".to_owned(),
                arguments: json!({"text": "render this"}),
            },
            &context(project.path()),
            None,
        )
        .await;
    let render = result
        .details
        .as_ref()
        .and_then(|details| details.get("codie_render"))
        .expect("render hint");
    assert_eq!(render["kind"], "markdown");
    assert!(render["text"].as_str().unwrap().contains("render this"));
    set.shutdown("test").await;
}

#[tokio::test]
#[spec("EXT-016")]
async fn spec_ext_016_duplicate_tool_names_are_skipped() {
    let home = tempfile::tempdir().expect("home");
    let project = tempfile::tempdir().expect("project");
    let first = project.path().join("first");
    let second = project.path().join("second");
    for (dir, name) in [(&first, "first"), (&second, "second")] {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(
            dir.join("codie.toml"),
            format!(
                "name = \"{name}\"\nversion = \"0.1.0\"\nprotocol = 1\nentrypoint = [\"python3\", \"ext.py\"]\ncapabilities = [\"tools\"]\n"
            ),
        )
        .unwrap();
        std::fs::write(
            dir.join("ext.py"),
            format!(
                r#"
import json, sys
for line in sys.stdin:
    request = json.loads(line)
    if request.get("method") == "initialize":
        print(json.dumps({{"jsonrpc": "2.0", "id": request["id"], "result": {{
            "protocol": 1, "name": "{name}", "version": "0.1.0",
            "capabilities": ["tools"], "tools": [{{"name": "same", "description": "d", "parameters": {{"type": "object"}}}}]
        }}}}), flush=True)
    elif "id" in request:
        print(json.dumps({{"jsonrpc": "2.0", "id": request["id"], "result": {{}}}}), flush=True)
"#
            ),
        )
        .unwrap();
    }
    let flags = BTreeMap::new();
    let set = ExtensionSet::load(LoadOptions {
        config_dir: home.path(),
        project_root: None,
        explicit: &[first, second],
        disabled: &[],
        cwd: project.path(),
        session_id: None,
        deferred_flags: &flags,
    })
    .await;
    assert_eq!(set.tools().len(), 1);
    assert!(
        set.warnings()
            .iter()
            .any(|warning| warning.contains("duplicate tool"))
    );
    set.shutdown("test").await;
}
