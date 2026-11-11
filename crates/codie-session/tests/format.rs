//! Format-freeze, migration, and resume-performance tests (SESS-020..SESS-022).

use std::fs;
use std::io::Write;
use std::time::{Duration, Instant};

use codie_core::{Content, Message, StopReason};
use codie_session::{Entry, SESSION_FORMAT_VERSION, Session, format_version};
use codie_testkit::spec;

fn assistant(text: impl Into<String>) -> Message {
    Message::assistant(
        "openai-completions",
        "deepseek",
        "deepseek-flash",
        vec![Content::text(text)],
        StopReason::Stop,
    )
}

#[spec("SESS-020")]
#[test]
fn spec_sess_020_ten_thousand_entries_resume_within_budget() {
    let temp = tempfile::tempdir().expect("tempdir");
    let path = temp.path().join("large.jsonl");
    let header = serde_json::json!({
        "type": "session",
        "version": SESSION_FORMAT_VERSION,
        "id": "large",
        "timestamp": "2026-11-11T00:00:00.000Z",
        "cwd": temp.path().display().to_string(),
    });
    let mut payload = format!("{header}\n");
    for index in 0..10_000 {
        let message = if index % 2 == 0 {
            Message::user(format!("user message {index} with some words"))
        } else {
            assistant(format!("assistant message {index} with some words"))
        };
        let entry = Entry::Message {
            id: format!("{index:08x}"),
            parent_id: (index > 0).then(|| format!("{:08x}", index - 1)),
            timestamp: "2026-11-11T00:00:01.000Z".to_owned(),
            message,
        };
        payload.push_str(&serde_json::to_string(&entry).expect("entry json"));
        payload.push('\n');
    }
    let mut file = fs::File::create(&path).expect("create");
    file.write_all(payload.as_bytes()).expect("write");
    drop(file);

    let start = Instant::now();
    let session = Session::open(&path).expect("open");
    let projection = session.projection();
    let elapsed = start.elapsed();

    assert_eq!(projection.len(), 10_000);
    assert!(
        elapsed < Duration::from_millis(150),
        "resume took {elapsed:?}, budget is 150 ms"
    );
    assert!(
        session.estimate_tokens() > 0,
        "token estimation must be available after resume"
    );
}

#[spec("SESS-021")]
#[test]
fn spec_sess_021_every_entry_type_roundtrips_with_a_stable_tag() {
    let temp = tempfile::tempdir().expect("tempdir");
    let cwd = temp.path().join("project");
    fs::create_dir_all(&cwd).expect("cwd");
    let mut session = Session::create(&cwd, temp.path().join("sessions")).expect("create");

    session
        .append_message(&Message::user("hello"))
        .expect("user");
    let message_id = session.entries()[0].id().to_owned();
    session
        .append_model_change("deepseek", "deepseek-flash")
        .expect("model");
    session
        .append_thinking_level_change(codie_core::ThinkingLevel::High)
        .expect("thinking");
    session.set_name("freeze").expect("name");
    session
        .append_usage(
            "compaction",
            "deepseek",
            "deepseek-flash",
            codie_core::Usage::new(1, 2, 0, 0),
        )
        .expect("usage");
    session
        .append_compaction("summary", Some(&message_id), 5, None, None)
        .expect("compaction");
    session
        .append_branch_summary(&message_id, "branch summary")
        .expect("branch");
    session
        .append_context_edit(&message_id, Some(Content::text("edited")))
        .expect("edit");
    session
        .set_label(&message_id, Some("important".to_owned()))
        .expect("label");
    session
        .append_custom("extension", serde_json::json!({"ok": true}))
        .expect("custom");
    session.set_leaf(Some(&message_id)).expect("leaf");
    session
        .append_message(&assistant("done"))
        .expect("assistant");

    let path = session.path().to_path_buf();
    let before = session.entries().to_vec();
    drop(session);

    let contents = fs::read_to_string(&path).expect("read");
    let tags: Vec<String> = contents
        .lines()
        .skip(1)
        .map(|line| {
            let value: serde_json::Value = serde_json::from_str(line).expect("json");
            value["type"].as_str().expect("type tag").to_owned()
        })
        .collect();
    for tag in [
        "message",
        "model_change",
        "thinking_level_change",
        "session_info",
        "usage",
        "compaction",
        "branch_summary",
        "context_edit",
        "label",
        "custom",
        "leaf",
    ] {
        assert!(tags.iter().any(|value| value == tag), "missing tag {tag}");
    }

    let reopened = Session::open(&path).expect("reopen");
    assert_eq!(reopened.entries(), before.as_slice());
    assert_eq!(reopened.header().version, SESSION_FORMAT_VERSION);
}

#[spec("SESS-022")]
#[test]
fn spec_sess_022_unsupported_versions_are_refused() {
    assert_eq!(format_version(), 1);
    assert_eq!(SESSION_FORMAT_VERSION, 1);

    let temp = tempfile::tempdir().expect("tempdir");
    for version in [0_u32, 2_u32] {
        let path = temp.path().join(format!("v{version}.jsonl"));
        let header = serde_json::json!({
            "type": "session",
            "version": version,
            "id": "id",
            "timestamp": "2026-11-11T00:00:00.000Z",
            "cwd": temp.path().display().to_string(),
        });
        fs::write(&path, format!("{header}\n")).expect("write header");
        let error = Session::open(&path).expect_err("version must be refused");
        assert!(
            error.to_string().contains(&format!("version {version}")),
            "unexpected error: {error}"
        );
    }
}

#[test]
fn format_version_is_frozen_for_1x() {
    // The freeze is a compatibility promise: bumping this constant requires an
    // RFC and a migration table (SESS-007).
    assert_eq!(SESSION_FORMAT_VERSION, 1);
}

#[test]
#[spec("SESS-025")]
fn spec_sess_025_migration_table_covers_the_reader_range() {
    codie_session::validate_table().expect("migration table");
    assert!(codie_session::is_supported(1));
    assert!(!codie_session::is_supported(0));
    assert!(!codie_session::is_supported(2));

    let value = serde_json::json!({"type":"message","id":"00000001"});
    assert_eq!(
        codie_session::migrate_value(value.clone(), 1).expect("current version"),
        value
    );
    assert!(codie_session::migrate_value(value, 0).is_err());
}
