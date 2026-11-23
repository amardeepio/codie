//! RPC command-matrix conformance tests (`PROTO-003`, `PROTO-006`).

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Output, Stdio};
use std::sync::{Arc, Mutex};

use codie_testkit::TempHome;
use codie_testkit::spec;
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

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

async fn mount_completion(server: &MockServer, text: &str) {
    let body = sse(&[
        &format!(
            r#"{{"id":"resp","choices":[{{"index":0,"delta":{{"role":"assistant","content":"{text}"}}}}]}}"#
        ),
        r#"{"id":"resp","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#,
        r#"{"id":"resp","choices":[],"usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15}}"#,
    ]);
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(body),
        )
        .mount(server)
        .await;
}

struct Rpc {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Rpc {
    fn start(server: &MockServer, home: &TempHome, cwd: &Path) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_codie"));
        for (key, value) in home.envs() {
            command.env(key, value);
        }
        let mut child = command
            .env("DEEPSEEK_API_KEY", "test-key")
            .env_remove("OPENROUTER_API_KEY")
            .env_remove("GEMINI_API_KEY")
            .arg("--mode")
            .arg("rpc")
            .arg("--provider")
            .arg("deepseek")
            .arg("--base-url")
            .arg(server.uri())
            .arg("--cwd")
            .arg(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn codie rpc");

        let stdin = child.stdin.take().expect("stdin");
        let stdout = BufReader::new(child.stdout.take().expect("stdout"));
        Self {
            child,
            stdin,
            stdout,
        }
    }

    fn send(&mut self, value: Value) {
        self.send_raw(&value.to_string());
    }

    fn send_raw(&mut self, line: &str) {
        writeln!(self.stdin, "{line}").expect("write command");
        self.stdin.flush().expect("flush command");
    }

    fn next(&mut self) -> Value {
        let mut line = String::new();
        let read = self.stdout.read_line(&mut line).expect("read record");
        assert!(read > 0, "rpc server closed stdout unexpectedly");
        serde_json::from_str(&line).unwrap_or_else(|error| panic!("invalid JSON {line:?}: {error}"))
    }

    fn response(&mut self, id: &str) -> Value {
        loop {
            let record = self.next();
            if record["type"] == "response" && record["id"] == id {
                return record;
            }
        }
    }

    fn settle(&mut self) -> (Value, Vec<Value>) {
        let mut records = Vec::new();
        loop {
            let record = self.next();
            let settled = record["type"] == "agent_settled";
            records.push(record.clone());
            if settled {
                return (record, records);
            }
        }
    }

    fn finish(self) -> Output {
        drop(self.stdin);
        self.child.wait_with_output().expect("wait for codie")
    }

    fn wait(self) -> Output {
        self.child.wait_with_output().expect("wait for codie")
    }

    /// Closes stdin, then drains stdout to EOF and reaps the process.
    fn disconnect(self) -> (Vec<Value>, Output) {
        let Self {
            child,
            stdin,
            mut stdout,
        } = self;
        drop(stdin);
        let mut records = Vec::new();
        let mut line = String::new();
        loop {
            line.clear();
            let read = stdout.read_line(&mut line).expect("read record");
            if read == 0 {
                break;
            }
            if line.trim().is_empty() {
                continue;
            }
            records.push(
                serde_json::from_str(&line)
                    .unwrap_or_else(|error| panic!("invalid JSON {line:?}: {error}")),
            );
        }
        let output = child.wait_with_output().expect("wait for codie");
        (records, output)
    }
}

#[tokio::test]
#[spec("PROTO-003")]
#[spec("PROTO-006")]
async fn spec_proto_003_rpc_command_matrix() {
    let server = MockServer::start().await;
    let home = TempHome::new().expect("temp home");
    let work = tempfile::tempdir().expect("temp repo");
    let mut rpc = Rpc::start(&server, &home, work.path());

    let ready = rpc.next();
    assert_eq!(ready["type"], "rpc_ready");
    assert_eq!(ready["version"], 2);
    assert!(
        ready["features"]
            .as_array()
            .expect("features")
            .iter()
            .any(|feature| feature == "permissions")
    );
    assert_eq!(rpc.next()["type"], "session");

    rpc.send(json!({"id": "state", "command": "get_state"}));
    let response = rpc.response("state");
    assert_eq!(response["success"], true);
    assert_eq!(response["command"], "get_state");
    assert_eq!(response["version"], 2);
    assert_eq!(response["data"]["model"]["model"], "deepseek-flash");
    assert!(response["data"]["total_tokens"].is_number());
    assert!(response["data"]["cost"].is_number());

    rpc.send(json!({"id": "messages", "command": "get_messages"}));
    assert_eq!(rpc.response("messages")["data"]["messages"], json!([]));

    rpc.send(json!({"id": "thinking", "command": "set_thinking", "level": "high"}));
    assert_eq!(
        rpc.response("thinking")["data"]["thinking"],
        Value::String("high".into())
    );

    rpc.send(json!({"id": "model", "command": "set_model", "model": "deepseek-flash"}));
    assert!(rpc.response("model")["success"].as_bool().unwrap());

    rpc.send(json!({"id": "stats", "command": "get_session_stats"}));
    let stats = rpc.response("stats");
    assert!(stats["data"]["context_window"].as_u64().unwrap() > 0);

    rpc.send(json!({"id": "new", "command": "new_session"}));
    let fresh = rpc.response("new");
    assert!(fresh["success"].as_bool().unwrap());
    assert!(fresh["data"]["session"].as_str().unwrap().len() >= 8);

    rpc.send_raw("this is not json");
    let bad = rpc.next();
    assert_eq!(bad["success"], false);
    assert_eq!(bad["error"]["code"], "bad_request");

    rpc.send(json!({"id": "unknown", "command": "not_a_command"}));
    let unknown = rpc.response("unknown");
    assert_eq!(unknown["success"], false);
    assert_eq!(unknown["error"]["code"], "bad_request");

    rpc.send(json!({"id": "stop", "command": "shutdown"}));
    assert_eq!(rpc.response("stop")["success"], true);
    let output = rpc.finish();
    assert!(output.status.success(), "{output:?}");
}

#[tokio::test]
#[spec("PROTO-003")]
#[spec("PROTO-004")]
#[spec("PROTO-006")]
async fn spec_proto_003_prompt_streams_events_and_exports_html() {
    let server = MockServer::start().await;
    mount_completion(&server, "hello from rpc").await;
    let home = TempHome::new().expect("temp home");
    let work = tempfile::tempdir().expect("temp repo");
    let mut rpc = Rpc::start(&server, &home, work.path());

    assert_eq!(rpc.next()["type"], "rpc_ready");
    assert_eq!(rpc.next()["type"], "session");

    rpc.send(json!({"id": "run", "command": "prompt", "message": "say hello"}));
    let started = rpc.response("run");
    assert!(started["success"].as_bool().unwrap());
    assert_eq!(started["data"]["started"], true);

    let (settled, records) = rpc.settle();
    assert_eq!(settled["aborted"], false, "{records:?}");
    assert!(
        records
            .iter()
            .any(|record| record["type"] == "message_end"
                && record["message"]["role"] == "assistant"),
        "assistant message missing: {records:?}"
    );
    assert!(
        records.iter().any(|record| record["type"] == "agent_end"),
        "agent_end missing: {records:?}"
    );

    rpc.send(json!({"id": "usage", "command": "get_state"}));
    let usage = rpc.response("usage");
    assert!(
        usage["data"]["total_tokens"].as_u64().unwrap() >= 15,
        "{usage:?}"
    );
    assert!(usage["data"]["cost"].is_number(), "{usage:?}");

    let export = work.path().join("transcript.html");
    rpc.send(json!({
        "id": "html",
        "command": "export_html",
        "path": export.display().to_string(),
    }));
    let exported = rpc.response("html");
    assert!(exported["success"].as_bool().unwrap(), "{exported:?}");

    let html = std::fs::read_to_string(&export).expect("html export");
    assert!(html.starts_with("<!doctype html>"));
    assert!(html.contains("hello from rpc"));
    assert!(html.contains("say hello"));
    assert!(!html.contains("<script"));

    rpc.send(json!({"id": "entries", "command": "get_entries"}));
    let entries = rpc.response("entries");
    assert!(
        entries["data"]["entries"].as_array().unwrap().len() >= 3,
        "{entries:?}"
    );

    rpc.send(json!({"id": "stop", "command": "shutdown"}));
    assert!(rpc.response("stop")["success"].as_bool().unwrap());
    assert!(rpc.finish().status.success());
}

#[tokio::test]
#[spec("PROTO-006")]
async fn spec_proto_006_stdin_eof_exits_zero() {
    let server = MockServer::start().await;
    let home = TempHome::new().expect("temp home");
    let work = tempfile::tempdir().expect("temp repo");
    let rpc = Rpc::start(&server, &home, work.path());

    let output = rpc.finish();
    assert!(output.status.success(), "{output:?}");
    assert!(output.stdout.is_empty() || output.status.success());
}

#[tokio::test]
#[spec("PROTO-006")]
async fn spec_proto_006_sigterm_uses_the_signal_exit_code() {
    let server = MockServer::start().await;
    let home = TempHome::new().expect("temp home");
    let work = tempfile::tempdir().expect("temp repo");
    let mut rpc = Rpc::start(&server, &home, work.path());
    assert_eq!(rpc.next()["type"], "rpc_ready");
    assert_eq!(rpc.next()["type"], "session");

    eprintln!("checkpoint: read startup");
    let pid = rpc.child.id();
    let status = Command::new("kill")
        .arg("-TERM")
        .arg(pid.to_string())
        .status()
        .expect("send SIGTERM");
    assert!(status.success());
    eprintln!("checkpoint: sent signal to {pid}");

    let output = rpc.wait();
    eprintln!("checkpoint: waited");
    assert_eq!(output.status.code(), Some(143), "{output:?}");
}

// ---------------------------------------------------------------------------
// Protocol v2: permissions, credentials, catalog/config, and attachments.
// ---------------------------------------------------------------------------

fn tool_call_frame(id: &str, name: &str, arguments: &str) -> String {
    format!(
        r#"{{"id":"resp","choices":[{{"index":0,"delta":{{"tool_calls":[{{"index":0,"id":"{id}","type":"function","function":{{"name":"{name}","arguments":{arguments}}}}}]}}}}]}}"#,
        arguments = serde_json::to_string(arguments).expect("arguments encode")
    )
}

fn finish_frame(reason: &str) -> String {
    format!(r#"{{"id":"resp","choices":[{{"index":0,"delta":{{}},"finish_reason":"{reason}"}}]}}"#)
}

fn usage_frame() -> &'static str {
    r#"{"id":"resp","choices":[],"usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15}}"#
}

fn text_frame(text: &str) -> String {
    format!(
        r#"{{"id":"resp","choices":[{{"index":0,"delta":{{"role":"assistant","content":{}}}}}]}}"#,
        serde_json::to_string(text).expect("text encodes")
    )
}

async fn mount_sequence(server: &MockServer, bodies: Vec<String>, expected: Option<u64>) {
    let queue = Arc::new(Mutex::new(VecDeque::from(bodies)));
    let responder_queue = Arc::clone(&queue);
    let mut mock = Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(move |_request: &wiremock::Request| {
            let mut queue = responder_queue.lock().expect("script queue");
            let body = queue.pop_front().expect("a scripted response per request");
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(body)
        });
    if let Some(expected) = expected {
        mock = mock.expect(expected);
    }
    mock.mount(server).await;
}

fn write_script(path: &str, content: &str) -> Vec<String> {
    let arguments = json!({"path": path, "content": content}).to_string();
    vec![
        sse(&[
            &tool_call_frame("call_write", "write", &arguments),
            &finish_frame("tool_calls"),
            usage_frame(),
        ]),
        sse(&[&text_frame("done"), &finish_frame("stop"), usage_frame()]),
    ]
}

fn tool_results(records: &[Value]) -> Vec<&Value> {
    records
        .iter()
        .filter(|record| record["type"] == "tool_execution_end")
        .collect()
}

#[tokio::test]
#[spec("PROTO-011")]
async fn spec_proto_011_handshake_advertises_v2_features() {
    let server = MockServer::start().await;
    let home = TempHome::new().expect("temp home");
    let work = tempfile::tempdir().expect("temp repo");
    let mut rpc = Rpc::start(&server, &home, work.path());

    let ready = rpc.next();
    assert_eq!(ready["type"], "rpc_ready");
    assert_eq!(ready["version"], 2);
    let features = ready["features"].as_array().expect("features");
    for expected in ["permissions", "auth", "catalog", "config", "attachments"] {
        assert!(
            features.iter().any(|feature| feature == expected),
            "missing feature {expected}: {ready:?}"
        );
    }
    assert_eq!(rpc.next()["type"], "session");

    rpc.send(json!({"id": "stop", "command": "shutdown"}));
    assert!(rpc.response("stop")["success"].as_bool().unwrap());
    assert!(rpc.finish().status.success());
}

#[tokio::test]
#[spec("PROTO-007")]
async fn spec_proto_007_permission_request_allows_a_write() {
    let server = MockServer::start().await;
    mount_sequence(
        &server,
        write_script("created.txt", "hello from rpc\n"),
        Some(2),
    )
    .await;
    let home = TempHome::new().expect("temp home");
    let work = tempfile::tempdir().expect("temp repo");
    let mut rpc = Rpc::start(&server, &home, work.path());
    assert_eq!(rpc.next()["type"], "rpc_ready");
    assert_eq!(rpc.next()["type"], "session");

    rpc.send(json!({"id": "mode", "command": "set_permission_mode", "mode": "ask"}));
    let mode = rpc.response("mode");
    assert!(mode["success"].as_bool().unwrap());
    assert_eq!(mode["data"]["mode"], "ask");

    rpc.send(json!({"id": "run", "command": "prompt", "message": "create created.txt"}));
    assert!(rpc.response("run")["success"].as_bool().unwrap());

    let permission = loop {
        let record = rpc.next();
        if record["type"] == "permission_request" {
            break record;
        }
    };
    assert_eq!(permission["version"], 2);
    assert_eq!(permission["tool"], "write");
    assert_eq!(permission["tool_call_id"], "call_write");
    assert_eq!(permission["arguments"]["path"], "created.txt");
    let request_id = permission["id"].as_str().expect("permission id").to_owned();

    rpc.send(json!({
        "id": "answer",
        "command": "permission_response",
        "request_id": request_id,
        "decision": "allow_once",
    }));
    let (settled, records) = rpc.settle();
    assert_eq!(settled["aborted"], false, "{records:?}");
    let results = tool_results(&records);
    assert_eq!(results.len(), 1, "{records:?}");
    assert_eq!(results[0]["is_error"], false, "{records:?}");
    assert_eq!(
        std::fs::read_to_string(work.path().join("created.txt")).expect("written file"),
        "hello from rpc\n"
    );

    rpc.send(json!({"id": "stop", "command": "shutdown"}));
    assert!(rpc.response("stop")["success"].as_bool().unwrap());
    assert!(rpc.finish().status.success());
}

#[tokio::test]
#[spec("PROTO-007")]
async fn spec_proto_007_headless_mode_denies_without_a_prompt() {
    let server = MockServer::start().await;
    mount_sequence(
        &server,
        write_script("blocked.txt", "should not exist\n"),
        Some(2),
    )
    .await;
    let home = TempHome::new().expect("temp home");
    let work = tempfile::tempdir().expect("temp repo");
    let mut rpc = Rpc::start(&server, &home, work.path());
    assert_eq!(rpc.next()["type"], "rpc_ready");
    assert_eq!(rpc.next()["type"], "session");

    rpc.send(json!({"id": "run", "command": "prompt", "message": "create blocked.txt"}));
    assert!(rpc.response("run")["success"].as_bool().unwrap());
    let (settled, records) = rpc.settle();
    assert_eq!(settled["aborted"], false, "{records:?}");
    let results = tool_results(&records);
    assert_eq!(results.len(), 1, "{records:?}");
    assert_eq!(results[0]["is_error"], true, "{records:?}");
    assert!(!work.path().join("blocked.txt").exists());

    rpc.send(json!({"id": "stop", "command": "shutdown"}));
    assert!(rpc.response("stop")["success"].as_bool().unwrap());
    assert!(rpc.finish().status.success());
}

#[tokio::test]
#[spec("PROTO-007")]
async fn spec_proto_007_unknown_permission_is_rejected() {
    let server = MockServer::start().await;
    let home = TempHome::new().expect("temp home");
    let work = tempfile::tempdir().expect("temp repo");
    let mut rpc = Rpc::start(&server, &home, work.path());
    assert_eq!(rpc.next()["type"], "rpc_ready");
    assert_eq!(rpc.next()["type"], "session");

    rpc.send(json!({
        "id": "answer",
        "command": "permission_response",
        "request_id": "perm-missing",
        "decision": "allow_once",
    }));
    let response = rpc.response("answer");
    assert_eq!(response["success"], false);
    assert_eq!(response["error"]["code"], "unknown_permission");

    rpc.send(json!({"id": "stop", "command": "shutdown"}));
    assert!(rpc.response("stop")["success"].as_bool().unwrap());
    assert!(rpc.finish().status.success());
}

#[tokio::test]
#[spec("PROTO-008")]
async fn spec_proto_008_auth_commands_store_and_remove_keys() {
    let server = MockServer::start().await;
    let home = TempHome::new().expect("temp home");
    let work = tempfile::tempdir().expect("temp repo");
    let mut rpc = Rpc::start(&server, &home, work.path());
    assert_eq!(rpc.next()["type"], "rpc_ready");
    assert_eq!(rpc.next()["type"], "session");

    rpc.send(json!({"id": "status", "command": "auth_status"}));
    let status = rpc.response("status");
    assert!(status["success"].as_bool().unwrap());
    let providers = status["data"]["providers"].as_array().expect("providers");
    let deepseek = providers
        .iter()
        .find(|provider| provider["provider"] == "deepseek")
        .expect("deepseek status");
    assert_eq!(deepseek["configured"], true);
    assert_eq!(deepseek["source"], "environment");

    let secret = "sk-test-secret-value";
    rpc.send(json!({
        "id": "set",
        "command": "auth_set_key",
        "provider": "gemini",
        "key": secret,
    }));
    let stored = rpc.response("set");
    assert!(stored["success"].as_bool().unwrap(), "{stored:?}");
    assert_eq!(stored["data"]["configured"], true);
    assert!(
        !stored.to_string().contains(secret),
        "the response must not echo the key"
    );

    rpc.send(json!({"id": "status2", "command": "auth_status"}));
    let status = rpc.response("status2");
    let gemini = status["data"]["providers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|provider| provider["provider"] == "gemini")
        .expect("gemini status");
    assert_eq!(gemini["configured"], true);
    assert_eq!(gemini["source"], "stored");

    rpc.send(json!({"id": "logout", "command": "auth_logout", "provider": "gemini"}));
    let logout = rpc.response("logout");
    assert_eq!(logout["data"]["removed"], true);

    rpc.send(json!({"id": "logout2", "command": "auth_logout", "provider": "gemini"}));
    assert_eq!(rpc.response("logout2")["data"]["removed"], false);

    rpc.send(json!({
        "id": "bad",
        "command": "auth_set_key",
        "provider": "not-a-provider",
        "key": "x",
    }));
    assert_eq!(rpc.response("bad")["error"]["code"], "unknown_provider");

    rpc.send(json!({"id": "stop", "command": "shutdown"}));
    assert!(rpc.response("stop")["success"].as_bool().unwrap());
    assert!(rpc.finish().status.success());
}

#[tokio::test]
#[spec("PROTO-009")]
async fn spec_proto_009_catalog_and_config_round_trip() {
    let server = MockServer::start().await;
    let home = TempHome::new().expect("temp home");
    let work = tempfile::tempdir().expect("temp repo");
    let mut rpc = Rpc::start(&server, &home, work.path());
    assert_eq!(rpc.next()["type"], "rpc_ready");
    assert_eq!(rpc.next()["type"], "session");

    rpc.send(json!({"id": "catalog", "command": "get_catalog"}));
    let catalog = rpc.response("catalog");
    assert!(catalog["success"].as_bool().unwrap());
    assert!(!catalog["data"]["models"].as_array().unwrap().is_empty());
    assert!(
        catalog["data"]["providers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|provider| provider["id"] == "deepseek")
    );

    rpc.send(json!({"id": "config", "command": "get_config"}));
    let config = rpc.response("config");
    assert!(config["success"].as_bool().unwrap());
    assert!(
        config["data"]["config_path"]
            .as_str()
            .unwrap()
            .ends_with("config.toml")
    );
    assert!(config["data"].get("allow").is_some());
    assert!(config["data"].get("api_key").is_none());

    rpc.send(
        json!({"id": "thinking", "command": "set_config", "key": "thinking", "value": "high"}),
    );
    let applied = rpc.response("thinking");
    assert!(applied["success"].as_bool().unwrap(), "{applied:?}");
    assert_eq!(applied["data"]["restart_required"], false);

    rpc.send(json!({"id": "state", "command": "get_state"}));
    assert_eq!(rpc.response("state")["data"]["thinking"], "high");

    rpc.send(json!({"id": "turn", "command": "set_config", "key": "max-turns", "value": "7"}));
    assert_eq!(rpc.response("turn")["data"]["restart_required"], false);

    rpc.send(json!({"id": "provider", "command": "set_config", "key": "provider", "value": "openrouter"}));
    assert_eq!(rpc.response("provider")["data"]["restart_required"], true);

    rpc.send(json!({"id": "bad", "command": "set_config", "key": "thinking", "value": "extreme"}));
    assert_eq!(rpc.response("bad")["error"]["code"], "invalid_argument");

    rpc.send(json!({"id": "stop", "command": "shutdown"}));
    assert!(rpc.response("stop")["success"].as_bool().unwrap());
    assert!(rpc.finish().status.success());
}

#[tokio::test]
#[spec("PROTO-010")]
async fn spec_proto_010_prompt_attachments_are_inlined() {
    let server = MockServer::start().await;
    mount_completion(&server, "read the notes").await;
    let home = TempHome::new().expect("temp home");
    let work = tempfile::tempdir().expect("temp repo");
    std::fs::write(work.path().join("notes.txt"), "attached content").expect("write notes");
    let mut rpc = Rpc::start(&server, &home, work.path());
    assert_eq!(rpc.next()["type"], "rpc_ready");
    assert_eq!(rpc.next()["type"], "session");

    rpc.send(json!({
        "id": "run",
        "command": "prompt",
        "message": "summarize the notes",
        "attachments": [{"type": "path", "path": "notes.txt"}],
    }));
    let started = rpc.response("run");
    assert!(started["success"].as_bool().unwrap(), "{started:?}");
    assert_eq!(started["data"]["started"], true);

    let (settled, records) = rpc.settle();
    assert_eq!(settled["aborted"], false, "{records:?}");

    rpc.send(json!({"id": "messages", "command": "get_messages"}));
    let messages = rpc.response("messages");
    let user = messages["data"]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|message| message["role"] == "user")
        .expect("user message");
    let content = user["content"].as_array().unwrap();
    let text = content
        .iter()
        .filter(|block| block["type"] == "text")
        .map(|block| block["text"].as_str().unwrap_or_default())
        .collect::<String>();
    assert!(text.contains("attached content"), "{user:?}");
    assert!(text.contains("<file"), "{user:?}");

    rpc.send(json!({"id": "stop", "command": "shutdown"}));
    assert!(rpc.response("stop")["success"].as_bool().unwrap());
    assert!(rpc.finish().status.success());
}

#[tokio::test]
#[spec("PROTO-010")]
async fn spec_proto_010_invalid_inline_image_is_reported() {
    let server = MockServer::start().await;
    mount_completion(&server, "ok").await;
    let home = TempHome::new().expect("temp home");
    let work = tempfile::tempdir().expect("temp repo");
    let mut rpc = Rpc::start(&server, &home, work.path());
    assert_eq!(rpc.next()["type"], "rpc_ready");
    assert_eq!(rpc.next()["type"], "session");

    rpc.send(json!({
        "id": "run",
        "command": "prompt",
        "message": "see this",
        "attachments": [{"type": "image", "data": "not-base64", "mime_type": "image/png"}],
    }));
    let started = rpc.response("run");
    assert!(started["success"].as_bool().unwrap(), "{started:?}");
    let warnings = started["data"]["warnings"].as_array().expect("warnings");
    assert!(!warnings.is_empty(), "{started:?}");

    let _ = rpc.settle();
    rpc.send(json!({"id": "stop", "command": "shutdown"}));
    assert!(rpc.response("stop")["success"].as_bool().unwrap());
    assert!(rpc.finish().status.success());
}

#[tokio::test]
#[spec("PROTO-007")]
async fn spec_proto_007_disconnect_denies_a_pending_request() {
    let server = MockServer::start().await;
    mount_sequence(
        &server,
        write_script("disconnected.txt", "never written\n"),
        None,
    )
    .await;
    let home = TempHome::new().expect("temp home");
    let work = tempfile::tempdir().expect("temp repo");
    let mut rpc = Rpc::start(&server, &home, work.path());
    assert_eq!(rpc.next()["type"], "rpc_ready");
    assert_eq!(rpc.next()["type"], "session");

    rpc.send(json!({"id": "mode", "command": "set_permission_mode", "mode": "ask"}));
    assert!(rpc.response("mode")["success"].as_bool().unwrap());
    rpc.send(json!({"id": "run", "command": "prompt", "message": "write a file"}));
    assert!(rpc.response("run")["success"].as_bool().unwrap());

    let permission = loop {
        let record = rpc.next();
        if record["type"] == "permission_request" {
            break record;
        }
    };
    assert_eq!(permission["tool"], "write");

    let (records, output) = rpc.disconnect();
    assert!(output.status.success(), "{output:?}");

    let denied = records
        .iter()
        .find(|record| record["type"] == "tool_execution_end")
        .unwrap_or_else(|| panic!("missing tool result: {records:?}"));
    assert_eq!(denied["is_error"], true, "{records:?}");
    assert!(
        denied["result"]["content"]
            .as_array()
            .unwrap()
            .iter()
            .any(|block| block["text"]
                .as_str()
                .is_some_and(|text| text.contains("Permission denied"))),
        "{denied:?}"
    );
    assert!(
        records
            .iter()
            .any(|record| record["type"] == "agent_settled" && record["aborted"] == true)
    );
    assert!(!work.path().join("disconnected.txt").exists());
}

#[tokio::test]
#[spec("PROTO-012")]
async fn spec_proto_012_themes_are_semantic_hex_palettes() {
    let server = MockServer::start().await;
    let home = TempHome::new().expect("temp home");
    let work = tempfile::tempdir().expect("temp repo");
    let mut rpc = Rpc::start(&server, &home, work.path());
    assert_eq!(rpc.next()["type"], "rpc_ready");
    assert_eq!(rpc.next()["type"], "session");

    rpc.send(json!({"id": "themes", "command": "get_themes"}));
    let response = rpc.response("themes");
    assert!(response["success"].as_bool().unwrap(), "{response:?}");
    let themes = response["data"]["themes"].as_array().expect("themes");
    assert_eq!(themes.len(), 9, "{themes:?}");
    assert!(
        themes.iter().all(|theme| theme["name"] != "system"),
        "system is resolved by the client"
    );

    let dark = themes
        .iter()
        .find(|theme| theme["name"] == "dark")
        .expect("dark theme");
    assert_eq!(dark["colors"]["background"], "#0a0a0a");
    assert_eq!(dark["colors"]["foreground"], "#eeeeee");
    assert_eq!(dark["colors"]["diff_add"], "#4fd6be");
    for theme in themes {
        let colors = theme["colors"].as_object().expect("colors object");
        assert_eq!(colors.len(), 29, "{} semantic colors", theme["name"]);
        assert!(
            colors.values().all(|value| value
                .as_str()
                .is_some_and(|hex| hex.len() == 7 && hex.starts_with('#'))),
            "{} contains a non-hex color",
            theme["name"]
        );
    }

    rpc.send(json!({"id": "stop", "command": "shutdown"}));
    assert!(rpc.response("stop")["success"].as_bool().unwrap());
    assert!(rpc.finish().status.success());
}
