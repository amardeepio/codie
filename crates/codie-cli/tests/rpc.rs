//! RPC command-matrix conformance tests (`PROTO-003`, `PROTO-006`).

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Output, Stdio};

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
    assert_eq!(ready["version"], 1);
    assert_eq!(rpc.next()["type"], "session");

    rpc.send(json!({"id": "state", "command": "get_state"}));
    let response = rpc.response("state");
    assert_eq!(response["success"], true);
    assert_eq!(response["command"], "get_state");
    assert_eq!(response["version"], 1);
    assert_eq!(response["data"]["model"]["model"], "deepseek-flash");

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
