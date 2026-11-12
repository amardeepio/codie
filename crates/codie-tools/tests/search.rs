//! Integration tests for the read-only search tools (`TOOL-010`..`TOOL-014`).

use std::path::Path;
use std::sync::Arc;

use codie_core::{Tool, ToolCall, ToolContext, ToolResult, Toolbox};
use codie_testkit::spec;
use codie_tools::{FindTool, GrepTool, LsTool, Policy, PolicyToolbox, Registry};
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

fn write(root: &Path, path: &str, content: &str) {
    let path = root.join(path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("parent directories");
    }
    std::fs::write(path, content).expect("write file");
}

#[tokio::test]
#[spec("TOOL-010")]
async fn spec_tool_010_grep_reports_path_line_and_text() {
    let directory = tempfile::tempdir().expect("temp dir");
    write(
        directory.path(),
        "src/main.rs",
        "fn main() {\n    println!(\"hi\");\n}\n",
    );
    write(directory.path(), "README.md", "hello world\n");
    let context = context(directory.path());

    let result = run(&GrepTool, json!({"pattern": "fn main"}), &context).await;
    assert!(!result.is_error, "{}", text(&result));
    assert_eq!(text(&result), "src/main.rs:1:fn main() {");

    let nested = run(&GrepTool, json!({"pattern": "println"}), &context).await;
    assert_eq!(text(&nested), "src/main.rs:2:    println!(\"hi\");");
}

#[tokio::test]
#[spec("TOOL-010")]
async fn spec_tool_010_grep_honors_case_and_glob_filters() {
    let directory = tempfile::tempdir().expect("temp dir");
    write(directory.path(), "src/lib.rs", "pub fn Helper() {}\n");
    write(directory.path(), "README.md", "helper notes\n");
    let context = context(directory.path());

    let sensitive = run(&GrepTool, json!({"pattern": "HELPER"}), &context).await;
    assert_eq!(text(&sensitive), "No matches found.");

    let insensitive = run(
        &GrepTool,
        json!({"pattern": "HELPER", "case_sensitive": false}),
        &context,
    )
    .await;
    assert!(text(&insensitive).contains("src/lib.rs:1:pub fn Helper() {}"));

    let filtered = run(
        &GrepTool,
        json!({"pattern": "helper", "case_sensitive": false, "glob": "*.md"}),
        &context,
    )
    .await;
    assert_eq!(text(&filtered), "README.md:1:helper notes");

    let rooted = run(
        &GrepTool,
        json!({"pattern": "Helper", "path": "src", "glob": "lib.rs"}),
        &context,
    )
    .await;
    assert_eq!(text(&rooted), "src/lib.rs:1:pub fn Helper() {}");
}

#[tokio::test]
#[spec("TOOL-010")]
async fn spec_tool_010_grep_rejects_invalid_patterns_and_missing_paths() {
    let directory = tempfile::tempdir().expect("temp dir");
    let context = context(directory.path());

    let invalid = run(&GrepTool, json!({"pattern": "("}), &context).await;
    assert!(invalid.is_error);
    assert!(text(&invalid).contains("Invalid regular expression"));

    let missing = run(
        &GrepTool,
        json!({"pattern": "x", "path": "does-not-exist"}),
        &context,
    )
    .await;
    assert!(missing.is_error);
    assert_eq!(text(&missing), "Path does not exist: does-not-exist");

    let bad_glob = run(&GrepTool, json!({"pattern": "x", "glob": "["}), &context).await;
    assert!(bad_glob.is_error);
    assert!(text(&bad_glob).contains("Invalid glob pattern"));
}

#[tokio::test]
#[spec("TOOL-011")]
async fn spec_tool_011_grep_respects_ignore_rules_and_skips_binaries() {
    let directory = tempfile::tempdir().expect("temp dir");
    std::fs::create_dir(directory.path().join(".git")).expect("git dir");
    write(directory.path(), ".gitignore", "ignored.txt\n");
    write(directory.path(), "ignored.txt", "needle\n");
    write(directory.path(), ".hidden.txt", "needle\n");
    write(directory.path(), "kept.txt", "needle\n");
    std::fs::write(directory.path().join("binary.bin"), [0xff, 0xfe, 0x00]).expect("binary");
    let context = context(directory.path());

    let result = run(&GrepTool, json!({"pattern": "needle"}), &context).await;
    assert!(!result.is_error, "{}", text(&result));
    assert_eq!(text(&result), "kept.txt:1:needle");
}

#[tokio::test]
#[spec("TOOL-011")]
async fn spec_tool_011_grep_caps_results_and_says_so() {
    let directory = tempfile::tempdir().expect("temp dir");
    write(directory.path(), "a.txt", "hit\nhit\nhit\nhit\nhit\n");
    let context = context(directory.path());

    let result = run(
        &GrepTool,
        json!({"pattern": "hit", "max_results": 2}),
        &context,
    )
    .await;
    let output = text(&result);
    assert_eq!(
        output.lines().filter(|line| line.contains(":hit")).count(),
        2,
        "{output}"
    );
    assert!(output.contains("2-match limit"), "{output}");
    assert_eq!(
        result
            .details
            .as_ref()
            .and_then(|details| details["capped"].as_bool()),
        Some(true)
    );
    assert_eq!(
        result
            .details
            .as_ref()
            .and_then(|details| details["matches"].as_u64()),
        Some(2)
    );
}

#[tokio::test]
#[spec("TOOL-010")]
async fn spec_tool_010_grep_searches_a_single_file() {
    let directory = tempfile::tempdir().expect("temp dir");
    write(directory.path(), "src/main.rs", "alpha\nbeta\n");
    write(directory.path(), "other.txt", "alpha\n");
    let context = context(directory.path());

    let result = run(
        &GrepTool,
        json!({"pattern": "alpha", "path": "src/main.rs"}),
        &context,
    )
    .await;
    assert_eq!(text(&result), "src/main.rs:1:alpha");
}

#[tokio::test]
#[spec("TOOL-012")]
async fn spec_tool_012_find_matches_globs_at_any_depth() {
    let directory = tempfile::tempdir().expect("temp dir");
    write(directory.path(), "a.rs", "");
    write(directory.path(), "src/b.rs", "");
    write(directory.path(), "src/nested/c.rs", "");
    write(directory.path(), "README.md", "");
    let context = context(directory.path());

    let all_rust = run(&FindTool, json!({"pattern": "*.rs"}), &context).await;
    assert_eq!(
        text(&all_rust),
        "a.rs\nsrc/b.rs\nsrc/nested/c.rs",
        "{}",
        text(&all_rust)
    );

    let nested = run(&FindTool, json!({"pattern": "src/**/*.rs"}), &context).await;
    assert_eq!(text(&nested), "src/b.rs\nsrc/nested/c.rs");

    let single_level = run(&FindTool, json!({"pattern": "src/*.rs"}), &context).await;
    assert_eq!(text(&single_level), "src/b.rs");

    let rooted = run(
        &FindTool,
        json!({"pattern": "nested/*.rs", "path": "src"}),
        &context,
    )
    .await;
    assert_eq!(text(&rooted), "src/nested/c.rs");
}

#[tokio::test]
#[spec("TOOL-012")]
async fn spec_tool_012_find_caps_results_and_handles_missing_paths() {
    let directory = tempfile::tempdir().expect("temp dir");
    for index in 0..5 {
        write(directory.path(), &format!("file{index}.rs"), "");
    }
    let context = context(directory.path());

    let capped = run(
        &FindTool,
        json!({"pattern": "*.rs", "max_results": 2}),
        &context,
    )
    .await;
    let output = text(&capped);
    assert_eq!(
        output.lines().filter(|line| line.ends_with(".rs")).count(),
        2,
        "{output}"
    );
    assert!(output.contains("2-file limit"), "{output}");
    assert_eq!(
        capped
            .details
            .as_ref()
            .and_then(|details| details["count"].as_u64()),
        Some(2)
    );

    let missing = run(
        &FindTool,
        json!({"pattern": "*.rs", "path": "shadow"}),
        &context,
    )
    .await;
    assert!(missing.is_error);
    assert_eq!(text(&missing), "Path does not exist: shadow");

    let invalid = run(&FindTool, json!({"pattern": "["}), &context).await;
    assert!(invalid.is_error);
    assert!(text(&invalid).contains("Invalid glob pattern"));
}

#[tokio::test]
#[spec("TOOL-012")]
async fn spec_tool_012_find_skips_hidden_and_ignored_files() {
    let directory = tempfile::tempdir().expect("temp dir");
    std::fs::create_dir(directory.path().join(".git")).expect("git dir");
    write(directory.path(), ".gitignore", "ignored.rs\n");
    write(directory.path(), "ignored.rs", "");
    write(directory.path(), ".hidden.rs", "");
    write(directory.path(), "kept.rs", "");
    let context = context(directory.path());

    let result = run(&FindTool, json!({"pattern": "*.rs"}), &context).await;
    assert_eq!(text(&result), "kept.rs");
}

#[tokio::test]
#[spec("TOOL-013")]
async fn spec_tool_013_ls_lists_directories_files_and_symlinks() {
    let directory = tempfile::tempdir().expect("temp dir");
    write(directory.path(), "src/main.rs", "fn main() {}\n");
    write(directory.path(), "a.txt", "hello\n");
    write(directory.path(), ".hidden", "abc\n");
    #[cfg(unix)]
    std::os::unix::fs::symlink("a.txt", directory.path().join("link")).expect("symlink");
    let context = context(directory.path());

    let result = run(&LsTool, json!({}), &context).await;
    assert!(!result.is_error, "{}", text(&result));
    let output = text(&result);
    let lines: Vec<&str> = output.lines().collect();
    assert_eq!(lines[0], "src/");
    assert_eq!(lines[1], "a.txt (6B)");
    #[cfg(unix)]
    assert_eq!(lines[2], "link -> a.txt");
    assert!(!output.contains(".hidden"));

    let all = run(&LsTool, json!({"all": true}), &context).await;
    assert!(text(&all).contains(".hidden (4B)"));

    let hidden_only = run(&LsTool, json!({"path": ".", "all": true}), &context).await;
    assert_eq!(
        hidden_only
            .details
            .as_ref()
            .and_then(|details| details["entries"].as_u64()),
        Some(if cfg!(unix) { 4 } else { 3 })
    );
}

#[tokio::test]
#[spec("TOOL-013")]
async fn spec_tool_013_ls_rejects_files_and_missing_paths() {
    let directory = tempfile::tempdir().expect("temp dir");
    write(directory.path(), "a.txt", "x");
    let ctx = context(directory.path());

    let file = run(&LsTool, json!({"path": "a.txt"}), &ctx).await;
    assert!(file.is_error);
    assert_eq!(text(&file), "Not a directory: a.txt");

    let missing = run(&LsTool, json!({"path": "nope"}), &ctx).await;
    assert!(missing.is_error);
    assert_eq!(text(&missing), "Path does not exist: nope");

    let empty = tempfile::tempdir().expect("empty dir");
    let empty_context = context(empty.path());
    let result = run(&LsTool, json!({}), &empty_context).await;
    assert_eq!(text(&result), "(empty directory)");
}

#[tokio::test]
#[spec("TOOL-014")]
async fn spec_tool_014_search_tools_are_read_only() {
    let policy = Policy::headless(Vec::<String>::new());
    for name in ["read", "grep", "find", "ls"] {
        assert!(policy.denial(name).is_none(), "{name} must be allowed");
    }
    for name in ["write", "edit", "bash"] {
        assert!(policy.denial(name).is_some(), "{name} must be denied");
    }

    let toolbox = PolicyToolbox::new(Arc::new(Registry::builtin()), policy);
    assert!(toolbox.get("grep").is_some());
    assert!(toolbox.get("find").is_some());
    assert!(toolbox.get("ls").is_some());
}
