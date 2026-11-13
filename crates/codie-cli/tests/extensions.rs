//! CLI integration for extension discovery, flags, and commands.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, Mutex};

use codie_testkit::TempHome;
use codie_testkit::spec;
use tempfile::TempDir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn write_extension(dir: &Path) {
    std::fs::write(
        dir.join("codie.toml"),
        "name = \"demo\"\nversion = \"0.1.0\"\nprotocol = 1\nentrypoint = [\"python3\", \"ext.py\"]\ncapabilities = [\"commands\", \"flags\"]\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("ext.py"),
        r#"
import json, sys
for line in sys.stdin:
    request = json.loads(line)
    method = request.get("method")
    if method == "initialize":
        result = {
            "protocol": 1, "name": "demo", "version": "0.1.0",
            "capabilities": ["commands", "flags"], "hooks": [], "tools": [],
            "commands": [{"name": "demo", "description": "demo command"}],
            "flags": [{"name": "demo", "description": "demo flag", "takes_value": True}],
        }
    elif method == "command.invoke":
        result = {"display": {"kind": "text", "title": "demo", "text": "demo command ran"}}
    else:
        result = {}
    print(json.dumps({"jsonrpc": "2.0", "id": request["id"], "result": result}), flush=True)
"#,
    )
    .unwrap();
}

fn command(home: &TempHome, args: &[&str]) -> std::process::Output {
    let mut command = codie_testkit::harness::run_cli(home, args);
    command
        .env("DEEPSEEK_API_KEY", "test-key")
        .env_remove("OPENROUTER_API_KEY");
    command.output().expect("codie runs")
}

#[test]
#[spec("EXT-015")]
#[spec("CLI-008")]
fn spec_ext_015_extension_flags_are_validated() {
    let home = TempHome::new().unwrap();
    let extension = TempDir::new().unwrap();
    write_extension(extension.path());
    let path = extension.path().display().to_string();

    let output = command(
        &home,
        &[
            "--extension",
            &path,
            "--demo=value",
            "--no-session",
            "/demo",
        ],
    );
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("demo command ran"));

    let output = command(&home, &["--demo=value", "--no-session", "/demo"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("no loaded extension claims it"));

    let output = command(
        &home,
        &["--extension", &path, "--demo", "--no-session", "/demo"],
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("requires a value"));

    let output = command(&home, &["models", "--demo=value"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("unknown flag"));

    let output = command(&home, &["-Z"]);
    assert_eq!(output.status.code(), Some(1));
}

fn sse(frames: &[&str]) -> String {
    let mut body = String::new();
    for frame in frames {
        body.push_str("data: ");
        body.push_str(frame);
        body.push_str("\n\n");
    }
    body.push_str("data: [DONE]\n\n");
    body
}

fn tool_call(name: &str, arguments: &str) -> String {
    format!(
        r#"{{"id":"resp","choices":[{{"index":0,"delta":{{"tool_calls":[{{"index":0,"id":"call_ext","type":"function","function":{{"name":"{name}","arguments":{}}}}}]}}}}]}}"#,
        serde_json::to_string(arguments).expect("arguments encode")
    )
}

fn finish(reason: &str) -> String {
    format!(r#"{{"id":"resp","choices":[{{"index":0,"delta":{{}},"finish_reason":"{reason}"}}]}}"#)
}

#[tokio::test]
#[spec("EXT-010")]
async fn spec_ext_010_extension_tool_end_to_end() {
    let home = TempHome::new().unwrap();
    let work = tempfile::tempdir().unwrap();
    let todo = std::fs::canonicalize(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/extensions/todo"),
    )
    .unwrap();

    let server = MockServer::start().await;
    let scripts = Arc::new(Mutex::new(VecDeque::from([
        sse(&[
            &tool_call("todo_add", r#"{"text":"phase 6 integration"}"#),
            &finish("tool_calls"),
            r#"{"id":"resp","choices":[],"usage":{"prompt_tokens":10,"completion_tokens":2,"total_tokens":12}}"#,
        ]),
        sse(&[
            r#"{"id":"resp","choices":[{"index":0,"delta":{"role":"assistant","content":"done"}}]}"#,
            &finish("stop"),
            r#"{"id":"resp","choices":[],"usage":{"prompt_tokens":10,"completion_tokens":2,"total_tokens":12}}"#,
        ]),
    ])));
    let queue = Arc::clone(&scripts);
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(move |_request: &wiremock::Request| {
            let body = queue
                .lock()
                .unwrap()
                .pop_front()
                .expect("scripted response");
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(body)
        })
        .expect(2)
        .mount(&server)
        .await;

    let mut command = codie_testkit::harness::run_cli(
        &home,
        &[
            "-p",
            "--mode",
            "json",
            "--provider",
            "deepseek",
            "--extension",
            todo.to_str().unwrap(),
            "--allow",
            "todo_add",
            "--base-url",
            &server.uri(),
            "--cwd",
            work.path().to_str().unwrap(),
            "--max-turns",
            "5",
            "add a todo",
        ],
    );
    command
        .env("DEEPSEEK_API_KEY", "test-key")
        .env_remove("OPENROUTER_API_KEY");
    let output = command.output().expect("codie runs");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("todo_add"), "stdout: {stdout}");
    let todo_file = work.path().join(".codie/todo.json");
    assert!(todo_file.is_file(), "todo file was not written");
    let contents = std::fs::read_to_string(todo_file).unwrap();
    assert!(contents.contains("phase 6 integration"));
}
