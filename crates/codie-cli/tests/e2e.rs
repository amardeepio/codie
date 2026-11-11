//! Golden-path end-to-end test with a fake provider (`P1-T17`).
//!
//! The compiled binary runs against a local wiremock server that replays
//! recorded-shape SSE responses. No network and no credentials are used. The
//! scenario is the Phase 1 acceptance path: read a file, edit it, run the test,
//! and report the result.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use assert_cmd::Command;
use codie_testkit::TempHome;
use codie_testkit::spec;
use serde_json::Value;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TOOL_CALL: &str = "tool_calls";

/// Wraps JSON payloads in SSE `data:` frames and terminates with `[DONE]`.
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

fn usage() -> &'static str {
    r#"{"id":"resp","choices":[],"usage":{"prompt_tokens":100,"completion_tokens":10,"total_tokens":110}}"#
}

fn tool_call(id: &str, name: &str, arguments: &str) -> String {
    format!(
        r#"{{"id":"resp","choices":[{{"index":0,"delta":{{"tool_calls":[{{"index":0,"id":"{id}","type":"function","function":{{"name":"{name}","arguments":{arguments}}}}}]}}}}]}}"#,
        arguments = serde_json::to_string(arguments).expect("arguments encode")
    )
}

fn finish(reason: &str) -> String {
    format!(r#"{{"id":"resp","choices":[{{"index":0,"delta":{{}},"finish_reason":"{reason}"}}]}}"#)
}

fn scripts() -> Vec<String> {
    vec![
        sse(&[
            &tool_call("call_read", "read", r#"{"path":"math.py"}"#),
            &finish(TOOL_CALL),
            usage(),
        ]),
        sse(&[
            &tool_call(
                "call_edit",
                "edit",
                r#"{"path":"math.py","edits":[{"oldText":"return a - b","newText":"return a + b"}]}"#,
            ),
            &finish(TOOL_CALL),
            usage(),
        ]),
        sse(&[
            &tool_call("call_bash", "bash", r#"{"command":"python3 test_math.py"}"#),
            &finish(TOOL_CALL),
            usage(),
        ]),
        sse(&[
            r#"{"id":"resp","choices":[{"index":0,"delta":{"role":"assistant","content":"Fixed the bug and the test passes."}}]}"#,
            &finish("stop"),
            usage(),
        ]),
    ]
}

async fn mount_scripts(server: &MockServer) -> Arc<Mutex<VecDeque<String>>> {
    let queue = Arc::new(Mutex::new(VecDeque::from(scripts())));
    let responder_queue = Arc::clone(&queue);
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(move |_request: &wiremock::Request| {
            let mut queue = responder_queue.lock().expect("script queue");
            let body = queue.pop_front().expect("a scripted response per request");
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(body)
        })
        .expect(4)
        .mount(server)
        .await;
    queue
}

fn freequeue_is_empty(queue: &Arc<Mutex<VecDeque<String>>>) {
    assert!(
        queue.lock().expect("script queue").is_empty(),
        "the agent did not consume every scripted turn"
    );
}

/// Returns every session file under `dir`, recursively.
fn session_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return files;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            files.extend(session_files(&path));
        } else if path
            .extension()
            .is_some_and(|extension| extension == "jsonl")
        {
            files.push(path);
        }
    }
    files
}

fn text_of(value: &Value) -> String {
    value["content"]
        .as_array()
        .map(|blocks| {
            blocks
                .iter()
                .filter_map(|block| block["text"].as_str())
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default()
}

/// Reduces the JSONL event stream to a stable, human-reviewable transcript.
fn normalize(events: &[Value]) -> String {
    let mut transcript = String::new();
    for event in events {
        match event["type"].as_str() {
            Some("message_end") => {
                let message = &event["message"];
                match message["role"].as_str() {
                    Some("user") => {
                        transcript.push_str(&format!("user: {}\n", text_of(message)));
                    }
                    Some("assistant") => {
                        for block in message["content"].as_array().into_iter().flatten() {
                            match block["type"].as_str() {
                                Some("text") => {
                                    transcript.push_str(&format!(
                                        "assistant: {}\n",
                                        block["text"].as_str().unwrap_or_default()
                                    ));
                                }
                                Some("tool_call") => {
                                    transcript.push_str(&format!(
                                        "assistant -> {}({})\n",
                                        block["name"].as_str().unwrap_or_default(),
                                        block["arguments"]
                                    ));
                                }
                                _ => {}
                            }
                        }
                    }
                    Some("tool_result") => {
                        transcript.push_str(&format!(
                            "{}: {}\n",
                            message["tool_name"].as_str().unwrap_or_default(),
                            text_of(message).replace('\n', "\\n")
                        ));
                    }
                    _ => {}
                }
            }
            Some("agent_end") => {
                transcript.push_str(&format!(
                    "end: {}\n",
                    event["status"].as_str().unwrap_or_default()
                ));
            }
            _ => {}
        }
    }
    transcript
}

#[spec("CLI-004")]
#[spec("PROTO-001")]
#[spec("PROTO-002")]
#[spec("PROTO-004")]
#[spec("PROTO-005")]
#[tokio::test]
async fn golden_path_end_to_end_with_a_fake_provider() {
    let server = MockServer::start().await;
    let queue = mount_scripts(&server).await;

    let home = TempHome::new().expect("temp home");
    let work = tempfile::tempdir().expect("temp repo");
    std::fs::write(
        work.path().join("math.py"),
        "def add(a, b):\n    return a - b\n",
    )
    .expect("write math.py");
    std::fs::write(
        work.path().join("test_math.py"),
        "from math import add\nassert add(2, 3) == 5\n",
    )
    .expect("write test_math.py");

    let mut command = Command::cargo_bin("codie").expect("codie binary");
    for (key, value) in home.envs() {
        command.env(key, value);
    }
    let output = command
        .env("DEEPSEEK_API_KEY", "test-key")
        .env_remove("OPENROUTER_API_KEY")
        .arg("-p")
        .arg("--mode")
        .arg("json")
        .arg("--provider")
        .arg("deepseek")
        .arg("--allow")
        .arg("edit,bash")
        .arg("--base-url")
        .arg(server.uri())
        .arg("--cwd")
        .arg(work.path())
        .arg("--max-turns")
        .arg("10")
        .arg("Make the test pass. Read the files, fix the bug, and run the test.")
        .output()
        .expect("run codie");

    assert!(
        output.status.success(),
        "codie failed: {}\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stderr.is_empty(),
        "JSON mode must reserve stdout for events and leave stderr clean: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    freequeue_is_empty(&queue);

    let fixed = std::fs::read_to_string(work.path().join("math.py")).expect("read math.py");
    assert!(
        fixed.contains("return a + b"),
        "math.py was not fixed: {fixed}"
    );

    let stdout = String::from_utf8(output.stdout).expect("utf-8 stdout");
    let events = stdout
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str::<Value>(line).expect("JSON event line"))
        .collect::<Vec<_>>();

    let schema: Value =
        serde_json::from_str(include_str!("../../../schemas/json-events.schema.json"))
            .expect("event schema parses");
    let validator = jsonschema::validator_for(&schema).expect("event schema compiles");
    for event in &events {
        if let Err(error) = validator.validate(event) {
            panic!("event does not match the published schema: {error}\n{event}");
        }
    }

    assert_eq!(
        events.first().map(|event| &event["type"]),
        Some(&Value::from("session"))
    );
    assert_eq!(
        events.last().map(|event| &event["type"]),
        Some(&Value::from("agent_settled"))
    );

    insta::assert_snapshot!("golden_path_transcript", normalize(&events));

    let files = session_files(&home.session_dir());
    assert_eq!(files.len(), 1, "exactly one session should be written");
    let session = codie_session::Session::open(&files[0]).expect("open session");
    assert!(
        !session.projection().is_empty(),
        "the session must persist the run"
    );
}

/// An ephemeral key is used for one run and never written to disk.
#[tokio::test]
#[spec("PROV-034")]
#[spec("CLI-006")]
async fn spec_prov_034_api_key_is_ephemeral() {
    let home = TempHome::new().expect("temp home");
    let work = tempfile::tempdir().expect("temp repo");
    let key = "sk-ephemeral-do-not-store";

    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_codie"));
    for (name, value) in home.envs() {
        command.env(name, value);
    }
    let output = command
        .env_remove("DEEPSEEK_API_KEY")
        .env_remove("OPENROUTER_API_KEY")
        .arg("-p")
        .arg("--no-session")
        .arg("--api-key")
        .arg(key)
        .arg("--model")
        .arg("deepseek-flash")
        .arg("--base-url")
        .arg("http://127.0.0.1:9")
        .arg("--cwd")
        .arg(work.path())
        .arg("hello")
        .output()
        .expect("run codie");

    assert!(
        !output.status.success(),
        "the unreachable endpoint must fail"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains(key), "the key leaked into stderr");
    assert!(
        !home.data_dir().join("auth.json").exists(),
        "--api-key must never persist a credential"
    );
}

/// A signal received during a run must map to the documented exit code.
#[spec("CLI-003")]
#[cfg(unix)]
#[tokio::test]
async fn signals_map_to_documented_exit_codes() {
    for (signal, expected) in [("INT", 130), ("TERM", 143), ("HUP", 129)] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(|_request: &wiremock::Request| {
                std::thread::sleep(Duration::from_secs(2));
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string("")
            })
            .mount(&server)
            .await;

        let home = TempHome::new().expect("temp home");
        let work = tempfile::tempdir().expect("temp repo");
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_codie"));
        for (key, value) in home.envs() {
            command.env(key, value);
        }
        command
            .env("DEEPSEEK_API_KEY", "test-key")
            .env_remove("OPENROUTER_API_KEY")
            .arg("-p")
            .arg("--no-session")
            .arg("--base-url")
            .arg(server.uri())
            .arg("--cwd")
            .arg(work.path())
            .arg("hello")
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        let mut child = command.spawn().expect("spawn codie");
        tokio::time::sleep(Duration::from_millis(500)).await;
        let signal_status = std::process::Command::new("kill")
            .arg(format!("-{signal}"))
            .arg(child.id().to_string())
            .status()
            .expect("send signal");
        assert!(signal_status.success(), "kill -{signal} failed");

        let exit = child.wait().expect("wait for codie");
        assert_eq!(
            exit.code(),
            Some(expected),
            "SIG{signal} should exit {expected}"
        );
    }
}

#[spec("CTX-006")]
#[tokio::test]
async fn context_overflow_compacts_and_retries_once() {
    let server = MockServer::start().await;
    let queue: Arc<Mutex<VecDeque<String>>> = Arc::new(Mutex::new(VecDeque::from(vec![
        r#"{"error":{"message":"This model's maximum context length is 128000 tokens"}}"#
            .to_owned(),
        sse(&[
            r#"{"id":"resp","choices":[{"index":0,"delta":{"role":"assistant","content":"Goal: continue"}}]}"#,
            &finish("stop"),
            usage(),
        ]),
        sse(&[
            r#"{"id":"resp","choices":[{"index":0,"delta":{"role":"assistant","content":"recovered answer"}}]}"#,
            &finish("stop"),
            usage(),
        ]),
    ])));
    let responder = Arc::clone(&queue);
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(move |_request: &wiremock::Request| {
            let mut queue = responder.lock().expect("script queue");
            let body = queue.pop_front().expect("a scripted response per request");
            if body.starts_with('{') {
                ResponseTemplate::new(400)
                    .insert_header("content-type", "application/json")
                    .set_body_string(body)
            } else {
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(body)
            }
        })
        .expect(3)
        .mount(&server)
        .await;

    let home = TempHome::new().expect("temp home");
    let work = tempfile::tempdir().expect("temp repo");
    let mut command = Command::cargo_bin("codie").expect("codie binary");
    for (key, value) in home.envs() {
        command.env(key, value);
    }
    let output = command
        .env("DEEPSEEK_API_KEY", "test-key")
        .env_remove("OPENROUTER_API_KEY")
        .arg("-p")
        .arg("--cwd")
        .arg(work.path())
        .arg("--base-url")
        .arg(server.uri())
        .arg("hello")
        .output()
        .expect("run codie");

    assert!(
        output.status.success(),
        "recovery failed: {}\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("utf-8 stdout");
    assert!(stdout.contains("recovered answer"));
    let stderr = String::from_utf8(output.stderr).expect("utf-8 stderr");
    assert!(stderr.contains("Context overflow recovered"));
    assert!(queue.lock().expect("script queue").is_empty());
}
