//! Integration tests for the Phase 1 built-in tools.

use std::{path::Path, time::Duration};

use codie_core::{Tool, ToolCall, ToolContext, ToolResult};
use codie_tools::{BashTool, EditTool, ReadTool, WriteTool};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

fn context(cwd: &Path) -> ToolContext {
    ToolContext {
        cwd: cwd.to_path_buf(),
        cancel: CancellationToken::new(),
        session_id: None,
    }
}

async fn run(tool: &dyn Tool, arguments: Value, context: &ToolContext) -> ToolResult {
    let call = ToolCall {
        id: "call-1".to_owned(),
        name: tool.name().to_owned(),
        arguments,
    };
    tool.execute(&call, context, None).await
}

fn text(result: &ToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|content| content.as_text())
        .collect::<Vec<_>>()
        .join("\n")
}

#[tokio::test]
async fn read_returns_file_text_without_line_numbers() {
    let directory = tempfile::tempdir().expect("temp dir");
    std::fs::write(directory.path().join("file.txt"), "one\ntwo\nthree").expect("write");
    let context = context(directory.path());

    let result = run(&ReadTool, json!({"path": "file.txt"}), &context).await;
    assert!(!result.is_error, "{}", text(&result));
    assert_eq!(text(&result), "one\ntwo\nthree");
}

#[tokio::test]
async fn read_applies_offset_and_limit() {
    let directory = tempfile::tempdir().expect("temp dir");
    std::fs::write(directory.path().join("file.txt"), "one\ntwo\nthree\nfour").expect("write");
    let context = context(directory.path());

    let result = run(
        &ReadTool,
        json!({"path": "file.txt", "offset": 2, "limit": 1}),
        &context,
    )
    .await;
    assert!(!result.is_error, "{}", text(&result));
    assert_eq!(
        text(&result),
        "two\n\n[2 more lines in file. Use offset=3 to continue.]"
    );
}

#[tokio::test]
async fn read_reports_an_offset_beyond_eof() {
    let directory = tempfile::tempdir().expect("temp dir");
    std::fs::write(directory.path().join("file.txt"), "one\ntwo\nthree\nfour").expect("write");
    let context = context(directory.path());

    let result = run(
        &ReadTool,
        json!({"path": "file.txt", "offset": 5}),
        &context,
    )
    .await;
    assert!(result.is_error);
    assert_eq!(
        text(&result),
        "Offset 5 is beyond end of file (4 lines total)"
    );
}

#[tokio::test]
async fn read_rejects_invalid_offset_and_limit() {
    let directory = tempfile::tempdir().expect("temp dir");
    std::fs::write(directory.path().join("file.txt"), "one").expect("write");
    let context = context(directory.path());

    let offset = run(
        &ReadTool,
        json!({"path": "file.txt", "offset": 0}),
        &context,
    )
    .await;
    assert!(offset.is_error);
    assert_eq!(text(&offset), "offset must be a positive integer");

    let limit = run(&ReadTool, json!({"path": "file.txt", "limit": 0}), &context).await;
    assert!(limit.is_error);
    assert_eq!(text(&limit), "limit must be a positive integer");
}

#[tokio::test]
async fn read_truncates_at_two_thousand_lines_with_offset_notice() {
    let directory = tempfile::tempdir().expect("temp dir");
    let content = (1..=2001)
        .map(|line| format!("line {line}"))
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(directory.path().join("file.txt"), content).expect("write");
    let context = context(directory.path());

    let result = run(&ReadTool, json!({"path": "file.txt"}), &context).await;
    assert!(!result.is_error, "{}", text(&result));
    let output = text(&result);
    assert!(output.starts_with("line 1\nline 2\n"));
    assert!(output.contains("line 2000"));
    assert!(!output.contains("line 2001"));
    assert!(output.ends_with("[Showing lines 1-2000 of 2001. Use offset=2001 to continue.]"));
}

#[tokio::test]
async fn read_reports_the_byte_limit() {
    let directory = tempfile::tempdir().expect("temp dir");
    let line = "x".repeat(100);
    let content = (1..=600)
        .map(|_| line.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(directory.path().join("file.txt"), content).expect("write");
    let context = context(directory.path());

    let result = run(&ReadTool, json!({"path": "file.txt"}), &context).await;
    let output = text(&result);
    assert!(!result.is_error, "{output}");
    assert!(output.contains("(50.0KB limit)"));
    assert!(output.contains("Use offset="));
    let details = result.details.as_ref().expect("details");
    let output_lines = details["truncation"]["output_lines"]
        .as_u64()
        .expect("output lines");
    assert!(output.ends_with(&format!(
        "[Showing lines 1-{output_lines} of 600 (50.0KB limit). Use offset={} to continue.]",
        output_lines + 1
    )));
}

#[tokio::test]
async fn read_reports_binary_files_as_errors() {
    let directory = tempfile::tempdir().expect("temp dir");
    std::fs::write(directory.path().join("binary.bin"), [0xff, 0xfe]).expect("write");
    let context = context(directory.path());

    let result = run(&ReadTool, json!({"path": "binary.bin"}), &context).await;
    assert!(result.is_error);
    assert!(text(&result).contains("not valid UTF-8"));
}

#[tokio::test]
async fn write_creates_parent_directories() {
    let directory = tempfile::tempdir().expect("temp dir");
    let context = context(directory.path());

    let result = run(
        &WriteTool,
        json!({"path": "nested/deeper/file.txt", "content": "hello"}),
        &context,
    )
    .await;
    assert!(!result.is_error, "{}", text(&result));
    assert_eq!(
        text(&result),
        "Successfully wrote to nested/deeper/file.txt"
    );
    assert_eq!(
        std::fs::read_to_string(directory.path().join("nested/deeper/file.txt")).expect("read"),
        "hello"
    );
}

#[tokio::test]
async fn write_serializes_concurrent_updates_to_one_file() {
    let directory = tempfile::tempdir().expect("temp dir");
    let context = context(directory.path());

    let first = run(
        &WriteTool,
        json!({"path": "file.txt", "content": "first"}),
        &context,
    );
    let second = run(
        &WriteTool,
        json!({"path": "file.txt", "content": "second"}),
        &context,
    );
    let (first, second) = tokio::join!(first, second);
    assert!(!first.is_error);
    assert!(!second.is_error);

    let content = std::fs::read_to_string(directory.path().join("file.txt")).expect("read");
    assert!(content == "first" || content == "second");
}

#[tokio::test]
async fn edit_replaces_unique_text_and_reports_first_changed_line() {
    let directory = tempfile::tempdir().expect("temp dir");
    std::fs::write(directory.path().join("file.txt"), "alpha\nbeta\ngamma").expect("write");
    let context = context(directory.path());

    let result = run(
        &EditTool,
        json!({
            "path": "file.txt",
            "edits": [{"oldText": "beta", "newText": "BETA"}]
        }),
        &context,
    )
    .await;
    assert!(!result.is_error, "{}", text(&result));
    assert_eq!(
        text(&result),
        "Successfully replaced 1 block(s) in file.txt."
    );
    assert_eq!(result.details.expect("details")["first_changed_line"], 2);
    assert_eq!(
        std::fs::read_to_string(directory.path().join("file.txt")).expect("read"),
        "alpha\nBETA\ngamma"
    );
}

#[tokio::test]
async fn edit_applies_multiple_edits_against_the_original() {
    let directory = tempfile::tempdir().expect("temp dir");
    std::fs::write(directory.path().join("file.txt"), "one two three").expect("write");
    let context = context(directory.path());

    let result = run(
        &EditTool,
        json!({
            "path": "file.txt",
            "edits": [
                {"oldText": "one", "newText": "1"},
                {"oldText": "three", "newText": "3"}
            ]
        }),
        &context,
    )
    .await;
    assert!(!result.is_error, "{}", text(&result));
    assert_eq!(
        text(&result),
        "Successfully replaced 2 block(s) in file.txt."
    );
    assert_eq!(
        std::fs::read_to_string(directory.path().join("file.txt")).expect("read"),
        "1 two 3"
    );
}

#[tokio::test]
async fn edit_rejects_duplicate_matches() {
    let directory = tempfile::tempdir().expect("temp dir");
    std::fs::write(directory.path().join("file.txt"), "foo\nfoo").expect("write");
    let context = context(directory.path());

    let result = run(
        &EditTool,
        json!({
            "path": "file.txt",
            "edits": [{"oldText": "foo", "newText": "bar"}]
        }),
        &context,
    )
    .await;
    assert!(result.is_error);
    assert_eq!(
        text(&result),
        "Found 2 occurrences of the text in file.txt. The text must be unique. Please provide more context to make it unique."
    );
}

#[tokio::test]
async fn edit_rejects_overlapping_matches() {
    let directory = tempfile::tempdir().expect("temp dir");
    std::fs::write(directory.path().join("file.txt"), "abcdef").expect("write");
    let context = context(directory.path());

    let result = run(
        &EditTool,
        json!({
            "path": "file.txt",
            "edits": [
                {"oldText": "abc", "newText": "x"},
                {"oldText": "bcd", "newText": "y"}
            ]
        }),
        &context,
    )
    .await;
    assert!(result.is_error);
    assert_eq!(
        text(&result),
        "edits[0] and edits[1] overlap in file.txt. Merge them into one edit or target disjoint regions."
    );
}

#[tokio::test]
async fn edit_reports_missing_exact_text() {
    let directory = tempfile::tempdir().expect("temp dir");
    std::fs::write(directory.path().join("file.txt"), "alpha beta").expect("write");
    let context = context(directory.path());

    let result = run(
        &EditTool,
        json!({
            "path": "file.txt",
            "edits": [{"oldText": "gamma", "newText": "delta"}]
        }),
        &context,
    )
    .await;
    assert!(result.is_error);
    assert_eq!(
        text(&result),
        "Could not find the exact text in file.txt. The old text must match exactly including all whitespace and newlines."
    );
}

#[tokio::test]
async fn edit_reports_missing_text_for_a_later_edit() {
    let directory = tempfile::tempdir().expect("temp dir");
    std::fs::write(directory.path().join("file.txt"), "alpha beta").expect("write");
    let context = context(directory.path());

    let result = run(
        &EditTool,
        json!({
            "path": "file.txt",
            "edits": [
                {"oldText": "alpha", "newText": "ALPHA"},
                {"oldText": "gamma", "newText": "GAMMA"}
            ]
        }),
        &context,
    )
    .await;
    assert!(result.is_error);
    assert_eq!(
        text(&result),
        "Could not find edits[1] in file.txt. The old text must match exactly including all whitespace and newlines."
    );
    assert_eq!(
        std::fs::read_to_string(directory.path().join("file.txt")).expect("read"),
        "alpha beta"
    );
}

#[tokio::test]
async fn edit_rejects_no_op_replacements() {
    let directory = tempfile::tempdir().expect("temp dir");
    std::fs::write(directory.path().join("file.txt"), "alpha").expect("write");
    let context = context(directory.path());

    let result = run(
        &EditTool,
        json!({
            "path": "file.txt",
            "edits": [{"oldText": "alpha", "newText": "alpha"}]
        }),
        &context,
    )
    .await;
    assert!(result.is_error);
    assert_eq!(
        text(&result),
        "No changes made to file.txt. The replacement produced identical content."
    );
}

#[tokio::test]
async fn edit_rejects_empty_old_text() {
    let directory = tempfile::tempdir().expect("temp dir");
    std::fs::write(directory.path().join("file.txt"), "alpha").expect("write");
    let context = context(directory.path());

    let result = run(
        &EditTool,
        json!({
            "path": "file.txt",
            "edits": [{"oldText": "", "newText": "beta"}]
        }),
        &context,
    )
    .await;
    assert!(result.is_error);
    assert_eq!(text(&result), "oldText must not be empty in file.txt.");
}

#[tokio::test]
async fn edit_preserves_a_utf8_bom() {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = directory.path().join("file.txt");
    std::fs::write(&path, "\u{feff}hello").expect("write");
    let context = context(directory.path());

    let result = run(
        &EditTool,
        json!({
            "path": "file.txt",
            "edits": [{"oldText": "hello", "newText": "world"}]
        }),
        &context,
    )
    .await;
    assert!(!result.is_error, "{}", text(&result));
    let bytes = std::fs::read(&path).expect("read");
    assert_eq!(String::from_utf8(bytes).expect("utf-8"), "\u{feff}world");
}

#[tokio::test]
async fn edit_preserves_crlf_outside_changed_ranges() {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = directory.path().join("file.txt");
    std::fs::write(&path, "a\r\nb\r\nc\r\n").expect("write");
    let context = context(directory.path());

    let result = run(
        &EditTool,
        json!({
            "path": "file.txt",
            "edits": [{"oldText": "b", "newText": "B"}]
        }),
        &context,
    )
    .await;
    assert!(!result.is_error, "{}", text(&result));
    assert_eq!(
        std::fs::read_to_string(&path).expect("read"),
        "a\r\nB\r\nc\r\n"
    );
}

#[tokio::test]
async fn edit_uses_crlf_for_new_lines_in_a_crlf_file() {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = directory.path().join("file.txt");
    std::fs::write(&path, "a\r\nb\r\nc\r\n").expect("write");
    let context = context(directory.path());

    let result = run(
        &EditTool,
        json!({
            "path": "file.txt",
            "edits": [{"oldText": "b\nc", "newText": "B\nC"}]
        }),
        &context,
    )
    .await;
    assert!(!result.is_error, "{}", text(&result));
    assert_eq!(
        std::fs::read_to_string(&path).expect("read"),
        "a\r\nB\r\nC\r\n"
    );
}

#[tokio::test]
async fn bash_returns_output_and_exit_details() {
    let directory = tempfile::tempdir().expect("temp dir");
    let context = context(directory.path());

    let result = run(&BashTool, json!({"command": "printf 'hello'"}), &context).await;
    assert!(!result.is_error, "{}", text(&result));
    assert_eq!(text(&result), "hello");
    let details = result.details.expect("details");
    assert_eq!(details["exit_code"], 0);
    assert_eq!(details["truncated"], false);
    assert!(details["wall_time_seconds"].as_f64().is_some());
}

#[tokio::test]
async fn bash_marks_non_zero_exit_as_error() {
    let directory = tempfile::tempdir().expect("temp dir");
    let context = context(directory.path());

    let result = run(
        &BashTool,
        json!({"command": "printf 'bad' >&2; exit 3"}),
        &context,
    )
    .await;
    assert!(result.is_error);
    let output = text(&result);
    assert!(output.contains("bad"), "{output}");
    assert!(output.ends_with("Command exited with code 3"), "{output}");
    assert_eq!(result.details.expect("details")["exit_code"], 3);
}

#[tokio::test]
async fn bash_reports_no_output() {
    let directory = tempfile::tempdir().expect("temp dir");
    let context = context(directory.path());

    let result = run(&BashTool, json!({"command": "true"}), &context).await;
    assert!(!result.is_error);
    assert_eq!(text(&result), "(no output)");
}

#[tokio::test]
async fn bash_times_out_and_kills_the_child() {
    let directory = tempfile::tempdir().expect("temp dir");
    let context = context(directory.path());

    let result = run(
        &BashTool,
        json!({"command": "sleep 2", "timeout": 1}),
        &context,
    )
    .await;
    assert!(result.is_error);
    assert_eq!(text(&result), "Command timed out after 1 seconds");
    assert_eq!(result.details.expect("details")["exit_code"], -1);
}

#[tokio::test]
async fn bash_rejects_invalid_timeouts() {
    let directory = tempfile::tempdir().expect("temp dir");
    let context = context(directory.path());

    for timeout in [json!(0), json!(-1), json!(MAX_TIMEOUT_PLUS_ONE)] {
        let result = run(
            &BashTool,
            json!({"command": "true", "timeout": timeout}),
            &context,
        )
        .await;
        assert!(result.is_error);
        assert!(text(&result).starts_with("Invalid timeout:"));
    }
}

const MAX_TIMEOUT_PLUS_ONE: f64 = 2_147_483.648;

#[tokio::test]
async fn bash_supports_abort() {
    let directory = tempfile::tempdir().expect("temp dir");
    let context = context(directory.path());
    let cancel = context.cancel.clone();
    let tool_context = context.clone();

    let task =
        tokio::spawn(
            async move { run(&BashTool, json!({"command": "sleep 5"}), &tool_context).await },
        );
    tokio::time::sleep(Duration::from_millis(100)).await;
    cancel.cancel();
    let result = task.await.expect("task");
    assert!(result.is_error);
    assert_eq!(text(&result), "Command aborted");
}

#[tokio::test]
async fn bash_truncates_the_tail_and_keeps_the_end_of_output() {
    let directory = tempfile::tempdir().expect("temp dir");
    let context = context(directory.path());

    let result = run(
        &BashTool,
        json!({"command": "for i in $(seq 1 2100); do echo \"line $i\"; done"}),
        &context,
    )
    .await;
    assert!(!result.is_error, "{}", text(&result));
    let output = text(&result);
    assert!(output.contains("line 2100"));
    assert!(!output.contains("\nline 1\n"));
    assert!(output.contains("[Showing last "));
    assert_eq!(result.details.expect("details")["truncated"], true);
}
