//! SIGKILL recovery test (`SESS-024`), Unix only.
//!
//! A real child process is killed with `SIGKILL` while the provider is streaming.
//! The session on disk must contain every finalized message, no in-flight partial
//! assistant message, and must be resumable by a later invocation.

#![cfg(unix)]

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use codie_core::Message;
use codie_session::Session;
use codie_testkit::spec;

const PROMPT: &str = "hello from kill test";

fn session_dir(home: &Path, project: &Path) -> PathBuf {
    let encoded = project
        .to_string_lossy()
        .trim_start_matches('/')
        .replace(['/', '\\', ':'], "-");
    home.join("sessions").join(format!("--{encoded}--"))
}

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

/// Spawns a server that sends one partial SSE frame, signals, then hangs.
fn start_stalled_server() -> (u16, Arc<AtomicBool>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let partial = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&partial);
    std::thread::spawn(move || {
        let Ok((mut stream, _)) = listener.accept() else {
            return;
        };
        let mut request = [0_u8; 4096];
        let _ = stream.read(&mut request);
        let body = concat!(
            "HTTP/1.1 200 OK\r\n",
            "Content-Type: text/event-stream\r\n",
            "Connection: close\r\n\r\n",
            "data: {\"id\":\"resp\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"partial answer that is still streaming\"}}]}\n\n"
        );
        let _ = stream.write_all(body.as_bytes());
        let _ = stream.flush();
        flag.store(true, Ordering::SeqCst);
        // Keep the stream open long enough for the child to receive the delta.
        std::thread::sleep(Duration::from_secs(10));
    });
    (port, partial)
}

/// Spawns a server that completes one response.
fn start_complete_server() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    std::thread::spawn(move || {
        let Ok((mut stream, _)) = listener.accept() else {
            return;
        };
        let mut request = [0_u8; 4096];
        let _ = stream.read(&mut request);
        let body = concat!(
            "HTTP/1.1 200 OK\r\n",
            "Content-Type: text/event-stream\r\n",
            "Connection: close\r\n\r\n",
            "data: {\"id\":\"resp\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"resumed response\"}}]}\n\n",
            "data: {\"id\":\"resp\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
            "data: {\"id\":\"resp\",\"choices\":[],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":2,\"total_tokens\":12}}\n\n",
            "data: [DONE]\n\n"
        );
        let _ = stream.write_all(body.as_bytes());
        let _ = stream.flush();
    });
    port
}

fn codie(home: &Path, project: &Path) -> Command {
    let mut command = Command::new(assert_cmd::cargo::cargo_bin("codie"));
    command
        .current_dir(project)
        .env("HOME", home)
        .env("CODIE_CONFIG_DIR", home.join("config"))
        .env("CODIE_DATA_DIR", home.join("data"))
        .env("CODIE_SESSION_DIR", home.join("sessions"))
        .env("DEEPSEEK_API_KEY", "test-key")
        .env_remove("OPENROUTER_API_KEY")
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    command
}

fn wait_for(predicate: impl Fn() -> bool, timeout: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if predicate() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    false
}

#[test]
#[spec("SESS-024")]
fn spec_sess_024_resume_after_sigkill_keeps_completed_messages() {
    let home = tempfile::tempdir().expect("home");
    let project = home.path().join("project");
    std::fs::create_dir_all(&project).expect("project");
    let project = project.canonicalize().expect("canonical project");
    let sessions = session_dir(home.path(), &project);

    let (port, partial) = start_stalled_server();
    let mut child = codie(home.path(), &project)
        .env("CODIE_BASE_URL", format!("http://127.0.0.1:{port}"))
        .args(["-p", PROMPT])
        .spawn()
        .expect("spawn codie");

    let saw_partial = wait_for(|| partial.load(Ordering::SeqCst), Duration::from_secs(10));
    let wrote_user = wait_for(
        || {
            session_files(&sessions).iter().any(|path| {
                std::fs::read_to_string(path).is_ok_and(|contents| contents.contains(PROMPT))
            })
        },
        Duration::from_secs(10),
    );
    assert!(saw_partial, "the provider never streamed a partial answer");
    assert!(
        wrote_user,
        "the user message was not persisted before the kill"
    );

    child.kill().expect("SIGKILL the child");
    let _ = child.wait();

    let path = session_files(&sessions)
        .into_iter()
        .next()
        .expect("session file");
    let session = Session::open(&path).expect("reopen killed session");
    let projection = session.projection();
    assert!(
        projection
            .iter()
            .any(|message| matches!(message, Message::User(user) if user.content.iter().filter_map(codie_core::Content::as_text).collect::<String>().contains(PROMPT))),
        "the finalized user message must survive SIGKILL"
    );
    assert!(
        !projection.iter().any(|message| matches!(message, Message::Assistant(assistant) if assistant.text().contains("partial answer"))),
        "the in-flight partial assistant message must not be persisted"
    );

    let resume_port = start_complete_server();
    let output = codie(home.path(), &project)
        .env("CODIE_BASE_URL", format!("http://127.0.0.1:{resume_port}"))
        .args(["--continue", "-p", "resume now"])
        .output()
        .expect("resume run");
    assert!(
        output.status.success(),
        "resume failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let resumed = Session::open(&path).expect("reopen resumed session");
    let text = resumed
        .projection()
        .iter()
        .filter_map(|message| match message {
            Message::User(user) => Some(
                user.content
                    .iter()
                    .filter_map(codie_core::Content::as_text)
                    .collect::<String>(),
            ),
            Message::Assistant(assistant) => Some(assistant.text()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains(PROMPT));
    assert!(text.contains("resume now"));
    assert!(text.contains("resumed response"));
}
