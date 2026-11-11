//! Integration tests for the session store (SESS-001..005, SESS-007).

use std::fs;
use std::io::Write;
use std::path::Path;
use std::time::Duration;

use codie_core::{Content, Message, StopReason, ThinkingLevel};
use codie_session::{Entry, SESSION_FORMAT_VERSION, Session};

fn assistant(text: &str) -> Message {
    Message::assistant(
        "openai-completions",
        "deepseek",
        "deepseek-flash",
        vec![Content::text(text)],
        StopReason::Stop,
    )
}

fn create_session(root: &Path, cwd_name: &str) -> Session {
    let cwd = root.join(cwd_name);
    fs::create_dir_all(&cwd).expect("create cwd");
    Session::create(&cwd, root.join("sessions")).expect("create session")
}

fn message_text(message: &Message) -> Option<String> {
    match message {
        Message::User(user) => Some(
            user.content
                .iter()
                .filter_map(Content::as_text)
                .collect::<Vec<_>>()
                .join(""),
        ),
        Message::Assistant(assistant) => Some(assistant.text()),
        _ => None,
    }
}

#[test]
fn create_append_reopen_and_project() {
    let temp = tempfile::tempdir().expect("tempdir");
    let cwd = temp.path().join("project");
    fs::create_dir_all(&cwd).expect("create cwd");
    let session_dir = temp.path().join("sessions");

    let mut session = Session::create(&cwd, &session_dir).expect("create session");
    assert_eq!(session.cwd(), cwd.to_string_lossy());
    assert_eq!(session.entries().len(), 0);
    assert_eq!(session.projection(), Vec::new());

    session
        .append_message(&Message::user("hello"))
        .expect("user");
    session.append_message(&assistant("hi")).expect("assistant");

    let projection = session.projection();
    assert_eq!(projection.len(), 2);
    assert_eq!(projection[0].role(), "user");
    assert_eq!(projection[1].role(), "assistant");

    let entries = session.entries().to_vec();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].parent_id(), None);
    assert_eq!(entries[1].parent_id(), Some(entries[0].id()));
    assert_eq!(session.leaf(), Some(entries[1].id()));
    assert!(matches!(entries[0], Entry::Message { .. }));

    let path = session.path().to_path_buf();
    let id = session.id().to_string();
    drop(session);

    let reopened = Session::open(&path).expect("reopen");
    assert_eq!(reopened.id(), id);
    assert_eq!(reopened.cwd(), cwd.to_string_lossy());
    assert_eq!(reopened.projection(), projection);
    assert_eq!(reopened.entries(), entries.as_slice());
}

#[test]
fn file_is_created_lazily_on_the_first_message() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut session = create_session(temp.path(), "project");

    assert!(!session.path().exists());
    session
        .append_model_change("deepseek", "deepseek-flash")
        .expect("model change");
    session
        .append_thinking_level_change(ThinkingLevel::High)
        .expect("thinking change");
    session.set_name("refactor").expect("name");
    // Metadata alone must not create the file (SESS-002).
    assert!(!session.path().exists());
    assert_eq!(session.latest_model(), Some(("deepseek", "deepseek-flash")));
    assert_eq!(session.latest_thinking_level(), Some(ThinkingLevel::High));
    assert_eq!(session.name(), Some("refactor"));

    session
        .append_message(&Message::user("hello"))
        .expect("user");
    assert!(session.path().exists());

    let path = session.path().to_path_buf();
    drop(session);

    let reopened = Session::open(&path).expect("reopen");
    // Model, thinking, name, and message entries were flushed in order.
    assert_eq!(reopened.entries().len(), 4);
    assert_eq!(
        reopened.latest_model(),
        Some(("deepseek", "deepseek-flash"))
    );
    assert_eq!(reopened.latest_thinking_level(), Some(ThinkingLevel::High));
    assert_eq!(reopened.name(), Some("refactor"));
    assert_eq!(reopened.projection().len(), 1);
}

#[test]
fn file_is_one_json_object_per_line_with_a_header_first() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut session = create_session(temp.path(), "project");
    session.append_message(&Message::user("one")).expect("user");
    session
        .append_message(&assistant("two"))
        .expect("assistant");

    let contents = fs::read_to_string(session.path()).expect("read session");
    let lines: Vec<&str> = contents.lines().collect();
    assert_eq!(lines.len(), 3, "header plus two entries");

    let header: serde_json::Value = serde_json::from_str(lines[0]).expect("header json");
    assert_eq!(header["type"], "session");
    assert_eq!(header["version"], SESSION_FORMAT_VERSION);
    assert_eq!(header["id"], session.id());
    assert!(header["timestamp"].as_str().is_some());
    assert_eq!(header["cwd"], session.cwd());

    let message: serde_json::Value = serde_json::from_str(lines[1]).expect("entry json");
    assert_eq!(message["type"], "message");
    assert_eq!(message["message"]["role"], "user");
    assert!(message["parent_id"].is_null());
    assert!(message["timestamp"].as_str().is_some());
}

#[test]
fn malformed_entry_line_is_skipped() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut session = create_session(temp.path(), "project");
    session
        .append_message(&Message::user("kept"))
        .expect("user");
    session
        .append_message(&assistant("also kept"))
        .expect("assistant");
    let path = session.path().to_path_buf();
    drop(session);

    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .expect("open for append");
    writeln!(file, "this is not a session entry").expect("append garbage");
    drop(file);

    let reopened = Session::open(&path).expect("reopen skips bad line");
    assert_eq!(reopened.entries().len(), 2);
    assert_eq!(reopened.projection().len(), 2);
    assert_eq!(reopened.leaf(), Some(reopened.entries()[1].id()));
}

#[test]
fn newer_format_version_is_rejected() {
    let temp = tempfile::tempdir().expect("tempdir");
    let path = temp.path().join("future.jsonl");
    let header = serde_json::json!({
        "type": "session",
        "version": 2,
        "id": "future",
        "timestamp": "2026-11-11T00:00:00.000Z",
        "cwd": "/tmp/project",
    });
    fs::write(&path, format!("{header}\n")).expect("write future session");

    let error = Session::open(&path).expect_err("future version must fail");
    let message = error.to_string();
    assert!(message.contains("version 2"), "unexpected error: {message}");
    assert!(
        message.contains("at most version 1"),
        "unexpected error: {message}"
    );
}

#[test]
fn model_and_thinking_changes_persist_across_reopen() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut session = create_session(temp.path(), "project");
    session
        .append_model_change("openrouter", "deepseek/deepseek-flash")
        .expect("first model");
    session
        .append_thinking_level_change(ThinkingLevel::Low)
        .expect("first thinking");
    session
        .append_message(&Message::user("hello"))
        .expect("user");
    session
        .append_model_change("deepseek", "deepseek-flash")
        .expect("second model");
    session
        .append_thinking_level_change(ThinkingLevel::Max)
        .expect("second thinking");

    let path = session.path().to_path_buf();
    drop(session);

    let reopened = Session::open(&path).expect("reopen");
    assert_eq!(
        reopened.latest_model(),
        Some(("deepseek", "deepseek-flash"))
    );
    assert_eq!(reopened.latest_thinking_level(), Some(ThinkingLevel::Max));
    // Model and thinking entries are on the branch but are not messages.
    assert_eq!(reopened.projection().len(), 1);
}

#[test]
fn sequential_appends_preserve_order_and_parent_links() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut session = create_session(temp.path(), "project");
    session
        .append_message(&Message::user("first"))
        .expect("first");
    session
        .append_message(&assistant("second"))
        .expect("second");

    let entries = session.entries().to_vec();
    assert_eq!(entries.len(), 2);
    assert_ne!(entries[0].id(), entries[1].id());
    assert_eq!(entries[0].parent_id(), None);
    assert_eq!(entries[1].parent_id(), Some(entries[0].id()));
    assert!(entries[0].timestamp() <= entries[1].timestamp());

    // Reparse the file to prove disk order matches memory order.
    let path = session.path().to_path_buf();
    drop(session);
    let reopened = Session::open(&path).expect("reopen");
    assert_eq!(reopened.entries()[0].id(), entries[0].id());
    assert_eq!(reopened.entries()[1].id(), entries[1].id());
    assert_eq!(
        message_text(&reopened.projection()[0]).as_deref(),
        Some("first")
    );
}

#[test]
fn entry_identifiers_are_eight_lowercase_hex_characters() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut session = create_session(temp.path(), "project");
    for index in 0..16 {
        session
            .append_message(&Message::user(format!("message {index}")))
            .expect("message");
    }

    let mut seen = std::collections::HashSet::new();
    for entry in session.entries() {
        assert_eq!(entry.id().len(), 8, "id was {}", entry.id());
        assert!(
            entry
                .id()
                .chars()
                .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)),
            "id was {}",
            entry.id()
        );
        assert!(seen.insert(entry.id().to_string()), "duplicate id");
    }
}

#[test]
fn set_leaf_moves_the_projection_and_rejects_unknown_ids() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut session = create_session(temp.path(), "project");
    session.append_message(&Message::user("one")).expect("user");
    session
        .append_message(&assistant("two"))
        .expect("assistant");
    let first = session.entries()[0].id().to_string();

    session.set_leaf(Some(&first)).expect("set leaf");
    assert_eq!(session.projection().len(), 1);

    session.set_leaf(None).expect("clear leaf");
    assert_eq!(session.projection(), Vec::new());
    assert!(session.set_leaf(Some("deadbeef")).is_err());
}

#[test]
fn continue_recent_returns_the_matching_working_directory() {
    let temp = tempfile::tempdir().expect("tempdir");
    let session_dir = temp.path().join("sessions");
    let first_cwd = temp.path().join("first");
    let second_cwd = temp.path().join("second");
    fs::create_dir_all(&first_cwd).expect("first cwd");
    fs::create_dir_all(&second_cwd).expect("second cwd");

    let mut older = Session::create(&first_cwd, &session_dir).expect("first session");
    older
        .append_message(&Message::user("first"))
        .expect("append");
    let older_path = older.path().to_path_buf();
    drop(older);

    std::thread::sleep(Duration::from_millis(5));
    let mut newer = Session::create(&first_cwd, &session_dir).expect("second session");
    newer
        .append_message(&Message::user("second"))
        .expect("append");
    drop(newer);

    let resumed = Session::continue_recent(&first_cwd, &session_dir)
        .expect("continue")
        .expect("a session");
    assert_eq!(resumed.entries().len(), 1);
    assert_eq!(
        message_text(&resumed.projection()[0]).as_deref(),
        Some("second")
    );
    assert_ne!(resumed.path(), older_path);

    let other_cwd = Session::create(&second_cwd, &session_dir).expect("other session");
    drop(other_cwd);
    assert!(
        Session::continue_recent(temp.path().join("missing"), &session_dir)
            .expect("continue")
            .is_none()
    );
}
