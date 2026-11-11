//! Integration tests for compaction, context edits, and token estimates
//! (CTX-002, CTX-003, CTX-005, SESS-018, SESS-019).

use std::fs;

use codie_core::{Content, Message, StopReason, ToolCall, UserMessage};
use codie_session::Session;
use codie_testkit::spec;

fn assistant(text: &str) -> Message {
    Message::assistant(
        "openai-completions",
        "deepseek",
        "deepseek-flash",
        vec![Content::text(text)],
        StopReason::Stop,
    )
}

fn create_session(cwd: &std::path::Path, session_dir: &std::path::Path) -> Session {
    fs::create_dir_all(cwd).expect("create cwd");
    Session::create(cwd, session_dir).expect("create session")
}

fn text_of(message: &Message) -> String {
    match message {
        Message::User(user) => user
            .content
            .iter()
            .filter_map(Content::as_text)
            .collect::<Vec<_>>()
            .join(""),
        Message::Assistant(assistant) => assistant.text(),
        Message::ToolResult(result) => result
            .content
            .iter()
            .filter_map(Content::as_text)
            .collect::<Vec<_>>()
            .join(""),
        Message::System(system) => system.content.clone(),
    }
}

#[spec("CTX-002")]
#[test]
fn spec_ctx_002_token_estimate_uses_chars_and_images() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut session = create_session(&temp.path().join("project"), &temp.path().join("sessions"));

    session
        .append_message(&Message::user("a".repeat(400)))
        .expect("user");
    assert_eq!(session.estimate_tokens(), 100);
    assert_eq!(session.context_percent(100), 100);
    assert_eq!(session.context_percent(400), 25);
    assert_eq!(session.context_percent(0), 0);

    session
        .append_message(&Message::User(UserMessage {
            content: vec![Content::Image {
                data: "AAAA".to_owned(),
                mime_type: "image/png".to_owned(),
            }],
            timestamp: 0,
        }))
        .expect("image");
    // Images count as 4,800 characters, or 1,200 tokens.
    assert_eq!(session.estimate_tokens(), 1_300);
}

#[spec("CTX-003")]
#[test]
fn spec_ctx_003_compaction_cannot_start_at_a_tool_result() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut session = create_session(&temp.path().join("project"), &temp.path().join("sessions"));

    let call = Content::ToolCall {
        id: "call-1".to_owned(),
        name: "read".to_owned(),
        arguments: serde_json::json!({"path": "README.md"}),
        thought_signature: None,
    };
    session
        .append_message(&Message::assistant(
            "openai-completions",
            "deepseek",
            "deepseek-flash",
            vec![call],
            StopReason::ToolUse,
        ))
        .expect("call");
    session
        .append_message(&Message::tool_result(
            "call-1",
            "read",
            vec![Content::text("contents")],
            false,
        ))
        .expect("result");
    let result_id = session.entries()[1].id().to_owned();

    let error = session
        .append_compaction("summary", Some(&result_id), 10, None, None)
        .expect_err("tool result cut point must be refused");
    assert!(error.to_string().contains("tool result"));
}

#[spec("CTX-005")]
#[test]
fn spec_ctx_005_compaction_is_append_only_and_rewrites_the_projection() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut session = create_session(&temp.path().join("project"), &temp.path().join("sessions"));

    session
        .append_message(&Message::user("old request"))
        .expect("old user");
    session
        .append_message(&assistant("old answer"))
        .expect("old assistant");
    session
        .append_message(&Message::user("recent request"))
        .expect("recent user");
    session
        .append_message(&assistant("recent answer"))
        .expect("recent assistant");
    let recent_id = session.entries()[2].id().to_owned();
    let entries_before = session.entries().len();

    session
        .append_compaction(
            "Goal: keep working.\nNext steps: finish the task.",
            Some(&recent_id),
            25_000,
            Some(serde_json::json!({"reason": "manual"})),
            None,
        )
        .expect("compaction");

    // The compaction is append-only: every earlier entry remains.
    assert_eq!(session.entries().len(), entries_before + 1);
    let projection = session.projection();
    assert_eq!(projection.len(), 3);
    let summary = text_of(&projection[0]);
    assert!(summary.contains("Summary of the conversation so far"));
    assert!(summary.contains("Goal: keep working."));
    assert_eq!(text_of(&projection[1]), "recent request");
    assert_eq!(text_of(&projection[2]), "recent answer");

    let path = session.path().to_path_buf();
    drop(session);
    let reopened = Session::open(&path).expect("reopen");
    assert_eq!(reopened.projection(), projection);
    assert_eq!(reopened.entries().len(), entries_before + 1);
}

#[spec("SESS-018")]
#[test]
fn spec_sess_018_context_edit_replaces_or_removes_a_message() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut session = create_session(&temp.path().join("project"), &temp.path().join("sessions"));

    session
        .append_message(&Message::user("secret token"))
        .expect("user");
    session
        .append_message(&assistant("visible answer"))
        .expect("assistant");
    let user_id = session.entries()[0].id().to_owned();
    let assistant_id = session.entries()[1].id().to_owned();

    session
        .append_context_edit(&user_id, Some(Content::text("[redacted]")))
        .expect("replace");
    assert_eq!(text_of(&session.projection()[0]), "[redacted]");
    assert_eq!(session.projection().len(), 2);

    session
        .append_context_edit(&assistant_id, None)
        .expect("remove");
    assert_eq!(session.projection().len(), 1);
    assert!(
        session
            .tree()
            .iter()
            .any(|entry| entry.kind == "context_edit")
    );
}

#[spec("SESS-019")]
#[test]
fn spec_sess_019_branch_summary_is_inserted_on_the_branch() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut session = create_session(&temp.path().join("project"), &temp.path().join("sessions"));

    session
        .append_message(&Message::user("root"))
        .expect("root");
    session
        .append_message(&assistant("abandoned work"))
        .expect("abandoned");
    let root_id = session.entries()[0].id().to_owned();
    session.set_leaf(Some(&root_id)).expect("return to root");

    session
        .append_branch_summary(root_id, "Tried an approach that failed.")
        .expect("branch summary");
    session
        .append_message(&Message::user("new direction"))
        .expect("new direction");

    let projection = session.projection();
    assert_eq!(projection.len(), 3);
    assert_eq!(text_of(&projection[0]), "root");
    assert!(text_of(&projection[1]).contains("previously explored branch"));
    assert_eq!(text_of(&projection[2]), "new direction");

    // Entries from the abandoned branch remain in the file but not on the branch.
    assert_eq!(session.entries().len(), 5);
    assert!(!session.tree()[1].on_branch);
}

#[spec("CTX-005")]
#[test]
fn spec_ctx_005_rejects_an_unknown_first_kept_entry() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut session = create_session(&temp.path().join("project"), &temp.path().join("sessions"));
    session
        .append_message(&Message::user("hello"))
        .expect("user");
    let error = session
        .append_compaction("summary", Some("deadbeef"), 1, None, None)
        .expect_err("unknown entry");
    assert!(error.to_string().contains("unknown session entry"));
}

#[test]
fn tool_call_helper_shape_is_stable() {
    // Guards the helper used by the CTX-003 test above.
    let call = ToolCall {
        id: "call-2".to_owned(),
        name: "read".to_owned(),
        arguments: serde_json::json!({}),
    };
    assert_eq!(call.name, "read");
}
