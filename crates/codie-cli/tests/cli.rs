//! Integration tests for the command-line surface.
//!
//! These exercise the compiled binary so exit codes, stdout, and stderr match
//! what a script or terminal would observe.

use assert_cmd::Command;
use codie_core::Message;
use codie_session::Session;
use codie_testkit::spec;
use predicates::prelude::*;
use tempfile::TempDir;

/// Returns a command handle isolated from the developer's real codie state.
fn codie() -> (Command, TempDir) {
    let home = tempfile::tempdir().expect("temp home");
    let command = codie_with_home(&home);
    (command, home)
}

/// Returns a command handle using an existing isolated home.
fn codie_with_home(home: &TempDir) -> Command {
    let mut command = Command::cargo_bin("codie").expect("the codie binary should be built");
    command
        .env("HOME", home.path())
        .env("CODIE_CONFIG_DIR", home.path().join("config"))
        .env("CODIE_DATA_DIR", home.path().join("data"))
        .env("CODIE_SESSION_DIR", home.path().join("sessions"))
        .env_remove("DEEPSEEK_API_KEY")
        .env_remove("OPENROUTER_API_KEY");
    command
}

#[test]
fn version_flag_prints_the_version_to_stdout() {
    let (mut command, _home) = codie();
    command
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("codie"))
        .stdout(predicate::str::contains(codie_core::VERSION))
        .stderr(predicate::str::is_empty());
}

#[spec("CLI-001")]
#[test]
fn help_flag_documents_the_surface() {
    let (mut command, _home) = codie();
    command
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Usage:"))
        .stdout(predicate::str::contains("--mode"))
        .stdout(predicate::str::contains("--tui-mode"))
        .stdout(predicate::str::contains("--theme"))
        .stdout(predicate::str::contains("--resume"))
        .stdout(predicate::str::contains("--fork"))
        .stdout(predicate::str::contains("--clone"))
        .stdout(predicate::str::contains("sessions"))
        .stdout(predicate::str::contains("auth"))
        .stdout(predicate::str::contains("models"))
        .stdout(predicate::str::contains("--verbose"))
        .stderr(predicate::str::is_empty());
}

#[test]
fn bare_invocation_without_a_prompt_fails_cleanly() {
    let (mut command, _home) = codie();
    command
        .env("DEEPSEEK_API_KEY", "dummy")
        .assert()
        .failure()
        .code(1)
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("prompt is empty"));
}

#[test]
fn models_lists_the_bundled_catalog() {
    let (mut command, _home) = codie();
    command
        .arg("models")
        .assert()
        .success()
        .stdout(predicate::str::contains("deepseek-flash"))
        .stdout(predicate::str::contains(
            "deepseek/deepseek-v4.1-flash-1017",
        ))
        .stderr(predicate::str::is_empty());
}

#[spec("PROV-032")]
#[test]
fn auth_status_reports_configuration_without_secrets() {
    let (mut command, _home) = codie();
    let output = command
        .args(["auth", "status", "--json"])
        .output()
        .expect("auth status runs");
    assert_eq!(
        output.status.code(),
        Some(2),
        "an unconfigured auth check must exit 2"
    );
    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("auth status is valid JSON");
    let providers = value["providers"].as_array().expect("providers array");
    assert_eq!(providers.len(), 2);
    assert!(
        providers
            .iter()
            .all(|provider| provider["configured"] == false)
    );
}

#[test]
fn a_run_without_a_key_fails_with_an_actionable_error() {
    let (mut command, _home) = codie();
    command
        .args(["-p", "hello"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("no API key"));
}

#[test]
fn an_unknown_provider_is_rejected() {
    let (mut command, _home) = codie();
    command
        .args(["-p", "--provider", "nope", "hello"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unsupported provider"));
}

#[test]
fn a_local_dot_env_is_loaded_without_overriding_the_process() {
    let (mut command, home) = codie();
    std::fs::write(
        home.path().join(".env"),
        "OPENROUTER_API_KEY=from-dot-env\n",
    )
    .expect("write .env");
    command
        .current_dir(home.path())
        .args(["auth", "status", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"environment\": true"));
}

#[test]
fn an_explicit_env_file_must_exist() {
    let (mut command, _home) = codie();
    command
        .args(["--env-file", "/definitely/missing", "models"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("environment file not found"));
}

#[test]
#[spec("CLI-005")]
fn spec_cli_005_unknown_flags_fail_fast() {
    let (mut command, _home) = codie();
    command
        .args(["--definitely-not-a-flag"])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("unexpected argument"));
}

#[test]
#[spec("CLI-007")]
fn spec_cli_007_startup_makes_no_network_call() {
    let (mut command, _home) = codie();
    command
        .env("CODIE_BASE_URL", "http://127.0.0.1:9")
        .arg("models")
        .assert()
        .success()
        .stdout(predicate::str::contains("deepseek-flash"));

    let (mut command, _home) = codie();
    command
        .env("CODIE_BASE_URL", "http://127.0.0.1:9")
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("codie"));
}

#[test]
fn api_key_requires_an_explicit_model() {
    let (mut command, _home) = codie();
    command
        .args(["-p", "--api-key", "secret", "hello"])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("--api-key requires"))
        .stderr(predicate::str::contains("--model"));
}

#[spec("CLI-002")]
#[test]
fn config_get_reports_effective_values() {
    let (mut command, _home) = codie();
    command
        .args(["config", "get", "provider"])
        .assert()
        .success()
        .stdout("deepseek\n")
        .stderr(predicate::str::is_empty());

    let (mut command, _home) = codie();
    command
        .args(["config", "get"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"thinking\""))
        .stdout(predicate::str::contains("\"max-turns\": 200"));
}

#[test]
fn config_get_rejects_unknown_keys() {
    let (mut command, _home) = codie();
    command
        .args(["config", "get", "nope"])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("unknown configuration key"));
}

#[test]
fn invalid_environment_configuration_is_rejected() {
    let (mut command, _home) = codie();
    command
        .env("CODIE_THINKING", "bogus")
        .args(["-p", "hello"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid thinking level"));
}

#[test]
#[spec("SESS-023")]
fn sessions_list_show_and_export_are_read_only() {
    let (mut command, home) = codie();
    let project = home.path().join("project");
    std::fs::create_dir_all(&project).expect("project");
    let project = project.canonicalize().expect("canonical project");
    let session_dir = home.path().join("sessions").join(format!(
        "--{}--",
        codie_session::session_dir_key(&project.to_string_lossy())
    ));

    let mut session = Session::create(&project, &session_dir).expect("session");
    session.set_name("demo session").expect("name");
    session
        .append_message(&Message::user("hello"))
        .expect("user");
    let id = session.id().to_owned();
    let path = session.path().to_path_buf();
    drop(session);
    let before = std::fs::read(&path).expect("read session");

    command
        .current_dir(&project)
        .args(["sessions", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("demo session"))
        .stdout(predicate::str::contains(&id))
        .stderr(predicate::str::is_empty());

    codie_with_home(&home)
        .current_dir(&project)
        .args(["sessions", "list", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"id\":\""))
        .stdout(predicate::str::contains("\"messages\":1"));

    codie_with_home(&home)
        .current_dir(&project)
        .args(["sessions", "show", &id, "--tree"])
        .assert()
        .success()
        .stdout(predicate::str::contains("demo session"))
        .stdout(predicate::str::contains("●"));

    let exported = home.path().join("exported.jsonl");
    codie_with_home(&home)
        .current_dir(&project)
        .args([
            "sessions",
            "export",
            &id,
            "--output",
            exported.to_str().expect("path"),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Exported"));

    let after = std::fs::read(&path).expect("read session again");
    assert_eq!(after, before, "session inspection must never write");
    assert_eq!(
        std::fs::read(&exported).expect("export"),
        before,
        "export must reproduce the file"
    );
}

#[test]
fn session_flags_conflict_cleanly() {
    let (mut command, _home) = codie();
    command
        .args(["--no-session", "--continue", "-p", "hello"])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("cannot be used with"));

    let (mut command, _home) = codie();
    command
        .args(["--fork", "abc", "--clone", "def", "-p", "hello"])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("cannot be used with"));
}

#[tokio::test]
async fn update_check_reports_a_newer_release() {
    use serde_json::json;
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"tag_name": "v9.9.9"})))
        .mount(&server)
        .await;

    let (mut command, _home) = codie();
    command
        .env("CODIE_UPDATE_URL", server.uri())
        .arg("update")
        .arg("--check")
        .assert()
        .success()
        .stdout(predicate::str::contains("update available: 9.9.9"));
}

#[tokio::test]
async fn update_check_reports_up_to_date() {
    use serde_json::json;
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"tag_name": format!("v{}", env!("CARGO_PKG_VERSION"))})),
        )
        .mount(&server)
        .await;

    let (mut command, _home) = codie();
    command
        .env("CODIE_UPDATE_URL", server.uri())
        .arg("update")
        .arg("--check")
        .assert()
        .success()
        .stdout(predicate::str::contains("up to date"));
}

#[test]
fn sessions_export_html_is_self_contained() {
    use codie_core::{Content, StopReason};
    use codie_session::Session;

    let (mut command, home) = codie();
    let cwd = std::env::current_dir()
        .expect("cwd")
        .canonicalize()
        .expect("canonical cwd");
    let sessions = home.path().join("sessions").join(format!(
        "--{}--",
        codie_session::session_dir_key(&cwd.to_string_lossy())
    ));
    std::fs::create_dir_all(&sessions).expect("sessions dir");

    let mut session = Session::create(&cwd, &sessions).expect("session");
    session
        .append_message(&Message::user("hello <script>alert(1)</script>"))
        .expect("user");
    session
        .append_message(&Message::assistant(
            "openai-completions",
            "deepseek",
            "deepseek-flash",
            vec![Content::text("hi there")],
            StopReason::Stop,
        ))
        .expect("assistant");
    let id = session.id().to_owned();
    drop(session);

    let output = command
        .arg("sessions")
        .arg("export")
        .arg(&id)
        .arg("--format")
        .arg("html")
        .output()
        .expect("export html");

    assert!(output.status.success(), "{output:?}");
    let html = String::from_utf8(output.stdout).expect("utf8");
    assert!(html.starts_with("<!doctype html>"));
    assert!(html.contains("hi there"));
    assert!(html.contains("&lt;script&gt;"));
    assert!(!html.contains("<script>"));
    assert!(!html.contains("<script "));
}

#[spec("CLI-010")]
#[test]
fn spec_cli_010_windows_installer_is_one_command() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let script = std::fs::read_to_string(root.join("install.ps1")).expect("install.ps1");
    assert!(
        script.contains("irm https://raw.githubusercontent.com/amardeepio/codie/main/install.ps1")
    );
    assert!(script.contains("x86_64-pc-windows-msvc"));
    assert!(script.contains("Get-FileHash -Algorithm SHA256"));
    assert!(script.contains("Expand-Archive"));
    assert!(script.contains("LOCALAPPDATA"));
    assert!(script.contains("CODIE_VERSION"));
    assert!(script.contains("CODIE_NO_PATH"));
    assert!(script.contains("SetEnvironmentVariable('Path'"));
}
