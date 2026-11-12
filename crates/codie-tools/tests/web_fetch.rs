//! Integration tests for `web_fetch` (`TOOL-020`..`TOOL-022`).

use std::path::Path;

use codie_core::{Tool, ToolCall, ToolContext, ToolResult, Toolbox};
use codie_testkit::spec;
use codie_tools::{Policy, Registry, WebFetchTool, is_read_only};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

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
#[spec("TOOL-020")]
async fn spec_tool_020_fetches_and_converts_html() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/docs"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            "<html><head><style>p{}</style></head><body>\
                 <h1>Title</h1><script>bad()</script><p>Hello &amp; bye</p></body></html>",
            "text/html; charset=utf-8",
        ))
        .mount(&server)
        .await;

    let directory = tempfile::tempdir().expect("dir");
    let result = run(
        &WebFetchTool::with_loopback_allowed(),
        json!({"url": format!("{}/docs", server.uri())}),
        &context(directory.path()),
    )
    .await;

    let output = text(&result);
    assert!(!result.is_error, "{output}");
    assert!(output.contains("Title"), "{output}");
    assert!(output.contains("Hello & bye"), "{output}");
    assert!(!output.contains("bad()"), "{output}");
    assert_eq!(
        result
            .details
            .as_ref()
            .and_then(|details| details["status"].as_u64()),
        Some(200)
    );
    assert_eq!(
        result
            .details
            .as_ref()
            .and_then(|details| details["content_type"].as_str()),
        Some("text/html; charset=utf-8")
    );
}

#[tokio::test]
#[spec("TOOL-020")]
async fn spec_tool_020_follows_redirects_and_returns_the_final_url() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/old"))
        .respond_with(ResponseTemplate::new(302).insert_header("Location", "/new"))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/new"))
        .respond_with(ResponseTemplate::new(200).set_body_raw("moved", "text/plain"))
        .mount(&server)
        .await;

    let directory = tempfile::tempdir().expect("dir");
    let result = run(
        &WebFetchTool::with_loopback_allowed(),
        json!({"url": format!("{}/old", server.uri())}),
        &context(directory.path()),
    )
    .await;

    assert!(!result.is_error, "{}", text(&result));
    assert!(text(&result).contains("moved"));
    let expected = format!("{}/new", server.uri());
    assert_eq!(
        result
            .details
            .as_ref()
            .and_then(|details| details["final_url"].as_str()),
        Some(expected.as_str())
    );
}

#[tokio::test]
#[spec("TOOL-021")]
async fn spec_tool_021_rejects_unsafe_targets_and_caps_output() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/large"))
        .respond_with(ResponseTemplate::new(200).set_body_string("0123456789"))
        .mount(&server)
        .await;

    let directory = tempfile::tempdir().expect("dir");
    let blocked = run(
        &WebFetchTool::new(),
        json!({"url": format!("{}/large", server.uri())}),
        &context(directory.path()),
    )
    .await;
    assert!(blocked.is_error);
    assert!(
        text(&blocked).contains("private address"),
        "{}",
        text(&blocked)
    );

    let scheme = run(
        &WebFetchTool::with_loopback_allowed(),
        json!({"url": "file:///etc/passwd"}),
        &context(directory.path()),
    )
    .await;
    assert!(scheme.is_error);
    assert!(text(&scheme).contains("Unsupported URL scheme"));

    let capped = run(
        &WebFetchTool::with_loopback_allowed(),
        json!({"url": format!("{}/large", server.uri()), "max_bytes": 4}),
        &context(directory.path()),
    )
    .await;
    assert!(!capped.is_error, "{}", text(&capped));
    assert_eq!(
        capped
            .details
            .as_ref()
            .and_then(|details| details["bytes"].as_u64()),
        Some(4)
    );
    assert_eq!(
        capped
            .details
            .as_ref()
            .and_then(|details| details["truncated"].as_bool()),
        Some(true)
    );
    assert!(text(&capped).contains("[Response truncated"));
}

#[tokio::test]
#[spec("TOOL-021")]
async fn spec_tool_021_stops_after_too_many_redirects() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/loop"))
        .respond_with(ResponseTemplate::new(302).insert_header("Location", "/loop"))
        .mount(&server)
        .await;

    let directory = tempfile::tempdir().expect("dir");
    let result = run(
        &WebFetchTool::with_loopback_allowed(),
        json!({"url": format!("{}/loop", server.uri())}),
        &context(directory.path()),
    )
    .await;

    assert!(result.is_error);
    assert!(text(&result).contains("Too many redirects"));
}

#[tokio::test]
#[spec("TOOL-022")]
async fn spec_tool_022_requires_network_permission() {
    assert!(!is_read_only("web_fetch"), "fetch is network egress");
    assert!(
        Policy::headless(Vec::<String>::new())
            .denial("web_fetch")
            .is_some()
    );
    assert!(
        Policy::headless(["web_fetch"])
            .denial("web_fetch")
            .is_none()
    );
    assert!(Registry::builtin().get("web_fetch").is_some());
}
