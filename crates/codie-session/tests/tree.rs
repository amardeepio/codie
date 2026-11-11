//! Integration tests for session tree operations (SESS-010..SESS-017).

use std::fs;
use std::io::Write;
use std::path::Path;
use std::time::Duration;

use codie_core::{Content, Message, StopReason};
use codie_session::Session;
use codie_testkit::spec;
use proptest::prelude::*;

fn assistant(text: impl Into<String>) -> Message {
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

#[spec("SESS-010")]
#[test]
fn spec_sess_010_fork_copies_branch_without_touching_source() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut source = create_session(temp.path(), "project");
    source.append_message(&Message::user("one")).expect("user");
    source.append_message(&assistant("two")).expect("assistant");
    let first = source.entries()[0].id().to_owned();
    let source_entries = source.entries().to_vec();
    let source_path = source.path().to_path_buf();

    let fork_cwd = temp.path().join("forked");
    fs::create_dir_all(&fork_cwd).expect("fork cwd");
    let forked = source
        .fork_at(Some(&first), &fork_cwd, temp.path().join("sessions"))
        .expect("fork");

    assert_eq!(forked.parent_session(), Some(source.id()));
    assert_eq!(forked.cwd(), fork_cwd.to_string_lossy());
    assert_ne!(forked.path(), source_path);
    assert_eq!(forked.entries().len(), 1);
    assert_eq!(forked.entries()[0].id(), first);
    assert_eq!(forked.leaf(), Some(first.as_str()));
    assert_eq!(
        message_text(&forked.projection()[0]).as_deref(),
        Some("one")
    );

    // The source is never mutated (SESS-006).
    assert_eq!(source.entries(), source_entries.as_slice());
    assert_eq!(source.leaf(), source_entries.last().map(|entry| entry.id()));

    let reopened = Session::open(forked.path()).expect("reopen fork");
    assert_eq!(reopened.entries(), forked.entries());
    assert_eq!(reopened.parent_session(), Some(source.id()));
}

#[spec("SESS-011")]
#[test]
fn spec_sess_011_clone_preserves_entire_tree() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut source = create_session(temp.path(), "project");
    source.append_message(&Message::user("root")).expect("root");
    source.append_message(&assistant("main")).expect("main");
    let first = source.entries()[0].id().to_owned();
    source.set_leaf(Some(&first)).expect("branch");
    source.append_message(&assistant("side")).expect("side");
    source.set_name("main line").expect("name");

    let clone = source
        .clone_session(temp.path().join("project"), temp.path().join("sessions"))
        .expect("clone");

    assert_eq!(clone.parent_session(), Some(source.id()));
    assert_eq!(clone.leaf(), source.leaf());
    assert_eq!(clone.entries(), source.entries());
    assert_eq!(clone.projection(), source.projection());
    assert_eq!(clone.name(), Some("main line"));
    assert_ne!(clone.path(), source.path());
    assert_ne!(clone.id(), source.id());

    let reopened = Session::open(clone.path()).expect("reopen clone");
    assert_eq!(reopened.entries(), source.entries());
    assert_eq!(reopened.leaf(), source.leaf());
}

#[spec("SESS-012")]
#[test]
fn spec_sess_012_tree_marks_branch_and_depth() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut session = create_session(temp.path(), "project");
    for index in 0..3 {
        session
            .append_message(&Message::user(format!("message {index}")))
            .expect("message");
    }
    let second = session.entries()[1].id().to_owned();
    session.set_leaf(Some(&second)).expect("set leaf");

    let tree = session.tree();
    assert_eq!(tree.len(), 4, "three messages plus a leaf marker");
    assert_eq!(tree[0].depth, 0);
    assert_eq!(tree[1].depth, 1);
    assert_eq!(tree[2].depth, 2);
    assert!(tree[0].on_branch);
    assert!(tree[1].on_branch);
    assert!(!tree[2].on_branch);
    assert_eq!(tree[0].kind, "message");
    assert!(tree[0].description.contains("message 0"));
    assert_eq!(tree[2].parent_id.as_deref(), Some(tree[1].id.as_str()));
    assert_eq!(tree[3].kind, "leaf");
    assert!(tree[3].on_branch);
    assert_eq!(tree[3].parent_id.as_deref(), Some(tree[1].id.as_str()));
}

#[spec("SESS-013")]
#[test]
fn spec_sess_013_resolve_by_id_and_prefix() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut first = create_session(temp.path(), "project");
    first
        .append_message(&Message::user("first"))
        .expect("message");
    std::thread::sleep(Duration::from_millis(5));
    let mut second = create_session(temp.path(), "project");
    second
        .append_message(&Message::user("second"))
        .expect("message");
    let session_dir = temp.path().join("sessions");

    let by_id = Session::resolve(&session_dir, first.id()).expect("exact id");
    assert_eq!(by_id.id(), first.id());
    let prefix = &first.id()[..12];
    let by_prefix = Session::resolve(&session_dir, prefix).expect("prefix");
    assert_eq!(by_prefix.id(), first.id());
    let by_path =
        Session::resolve(&session_dir, first.path().to_string_lossy().as_ref()).expect("path");
    assert_eq!(by_path.id(), first.id());

    // An empty prefix matches both sessions and must be refused.
    let error = Session::resolve(&session_dir, "").expect_err("ambiguous");
    assert!(error.to_string().contains("matches 2 sessions"));
    let error = Session::resolve(&session_dir, "ffffffff").expect_err("missing");
    assert!(error.to_string().contains("no session matches"));
}

#[spec("SESS-014")]
#[test]
fn spec_sess_014_export_matches_backing_file() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut session = create_session(temp.path(), "project");
    session.set_name("export me").expect("name");
    session
        .append_message(&Message::user("hello"))
        .expect("user");
    session
        .append_message(&assistant("world"))
        .expect("assistant");

    let original = fs::read_to_string(session.path()).expect("read session");
    let mut exported = Vec::new();
    session.export(&mut exported).expect("export");
    assert_eq!(String::from_utf8(exported).expect("utf8"), original);

    // Export does not modify the file.
    let after = fs::read_to_string(session.path()).expect("read again");
    assert_eq!(after, original);

    let empty = create_session(temp.path(), "other");
    let mut header_only = Vec::new();
    empty.export(&mut header_only).expect("export header");
    let text = String::from_utf8(header_only).expect("utf8");
    assert_eq!(text.lines().count(), 1);
    assert!(text.contains("\"type\":\"session\""));
}

#[spec("SESS-015")]
#[test]
fn spec_sess_015_list_reports_newest_first() {
    let temp = tempfile::tempdir().expect("tempdir");
    let session_dir = temp.path().join("sessions");
    let cwd = temp.path().join("project");
    fs::create_dir_all(&cwd).expect("cwd");

    let mut oldest = Session::create(&cwd, &session_dir).expect("oldest");
    oldest.set_name("oldest").expect("name");
    oldest
        .append_message(&Message::user("old"))
        .expect("message");
    let oldest_id = oldest.id().to_owned();
    drop(oldest);
    std::thread::sleep(Duration::from_millis(10));

    let mut newest = Session::create(&cwd, &session_dir).expect("newest");
    newest.set_name("newest").expect("name");
    newest
        .append_message(&Message::user("new"))
        .expect("message");
    let newest_id = newest.id().to_owned();
    let forked = newest.fork_at(None, &cwd, &session_dir).expect("fork");
    let forked_id = forked.id().to_owned();
    drop(newest);
    drop(forked);

    let summaries = Session::list(&session_dir).expect("list");
    assert_eq!(summaries.len(), 3);
    assert_eq!(summaries[0].id, forked_id);
    assert_eq!(
        summaries[0].parent_session.as_deref(),
        Some(newest_id.as_str())
    );
    assert_eq!(summaries[0].message_count, 1);
    assert_eq!(summaries[2].id, oldest_id);
    assert_eq!(summaries[2].name.as_deref(), Some("oldest"));

    let for_cwd = Session::list_for_cwd(&cwd, &session_dir).expect("list cwd");
    assert_eq!(for_cwd.len(), 3);
    let elsewhere =
        Session::list_for_cwd(temp.path().join("elsewhere"), &session_dir).expect("list elsewhere");
    assert!(elsewhere.is_empty());
}

#[spec("SESS-016")]
#[test]
fn spec_sess_016_truncated_final_line_loses_only_partial_message() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut session = create_session(temp.path(), "project");
    session
        .append_message(&Message::user("complete"))
        .expect("first");
    session
        .append_message(&assistant("complete too"))
        .expect("second");
    let path = session.path().to_path_buf();
    let complete_entries = session.entries().len();
    drop(session);

    // Simulate a crash in the middle of the third append: the line has no LF
    // and is not valid JSON.
    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .expect("open for append");
    file.write_all(b"{\"type\":\"message\",\"id\":\"deadbeef\",\"message\":")
        .expect("partial write");
    drop(file);

    let reopened = Session::open(&path).expect("reopen");
    assert_eq!(reopened.entries().len(), complete_entries);
    assert_eq!(reopened.projection().len(), 2);
    assert_eq!(
        message_text(&reopened.projection()[1]).as_deref(),
        Some("complete too")
    );
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[spec("SESS-017")]
    #[test]
    fn spec_sess_017_random_tree_operations_reopen_identically(
        operations in prop::collection::vec(0_u8..5, 0..40)
    ) {
        let temp = tempfile::tempdir().expect("tempdir");
        let cwd = temp.path().join("project");
        fs::create_dir_all(&cwd).expect("cwd");
        let session_dir = temp.path().join("sessions");
        let mut session = Session::create(&cwd, &session_dir).expect("create");

        for (index, operation) in operations.iter().enumerate() {
            match operation % 5 {
                0 => {
                    session
                        .append_message(&Message::user(format!("user {index}")))
                        .expect("user");
                }
                1 => {
                    session
                        .append_message(&assistant(format!("assistant {index}")))
                        .expect("assistant");
                }
                2 => {
                    if !session.entries().is_empty() {
                        let pick = usize::from(*operation)
                            % session.entries().len();
                        let id = session.entries()[pick].id().to_owned();
                        session.set_leaf(Some(&id)).expect("branch");
                    }
                }
                3 => {
                    let messages: Vec<String> = session
                        .entries()
                        .iter()
                        .filter(|entry| entry.message().is_some())
                        .map(|entry| entry.id().to_owned())
                        .collect();
                    if let Some(first_kept) = messages.first() {
                        session
                            .append_compaction(
                                format!("summary {index}"),
                                Some(first_kept),
                                42,
                                None,
                                None,
                            )
                            .expect("compaction");
                    }
                }
                _ => {
                    session
                        .append_model_change("deepseek", "deepseek-flash")
                        .expect("model");
                }
            }
        }

        let projection = session.projection();
        let entries = session.entries().to_vec();
        let leaf = session.leaf().map(str::to_owned);
        let path = session.path().to_path_buf();
        drop(session);

        if path.exists() {
            let reopened = Session::open(&path).expect("reopen");
            prop_assert_eq!(reopened.entries(), entries.as_slice());
            prop_assert_eq!(reopened.projection(), projection);
            prop_assert_eq!(reopened.leaf(), leaf.as_deref());
        } else {
            // A session with only buffered metadata is deliberately not
            // persisted yet (SESS-002).
            prop_assert!(projection.is_empty());
            prop_assert!(entries.iter().all(|entry| entry.message().is_none()));
        }
    }
}
