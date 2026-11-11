//! Integration tests for the command-line surface.
//!
//! These exercise the compiled binary so exit codes, stdout, and stderr match
//! what a script or terminal would observe.

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

/// Returns a command handle isolated from the developer's real codie state.
fn codie() -> (Command, TempDir) {
    let home = tempfile::tempdir().expect("temp home");
    let mut command = Command::cargo_bin("codie").expect("the codie binary should be built");
    command
        .env("HOME", home.path())
        .env("CODIE_CONFIG_DIR", home.path().join("config"))
        .env("CODIE_DATA_DIR", home.path().join("data"))
        .env("CODIE_SESSION_DIR", home.path().join("sessions"))
        .env_remove("DEEPSEEK_API_KEY")
        .env_remove("OPENROUTER_API_KEY");
    (command, home)
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

#[test]
fn help_flag_documents_the_surface() {
    let (mut command, _home) = codie();
    command
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Usage:"))
        .stdout(predicate::str::contains("--mode"))
        .stdout(predicate::str::contains("auth"))
        .stdout(predicate::str::contains("models"))
        .stdout(predicate::str::contains("--verbose"))
        .stderr(predicate::str::is_empty());
}

#[test]
fn no_arguments_prints_help_to_stderr_and_fails() {
    let (mut command, _home) = codie();
    command
        .assert()
        .failure()
        .code(2)
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("Usage:"));
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

#[test]
fn auth_status_reports_configuration_without_secrets() {
    let (mut command, _home) = codie();
    let output = command
        .args(["auth", "status", "--json"])
        .output()
        .expect("auth status runs");
    assert!(output.status.success());
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
fn invalid_environment_configuration_is_rejected() {
    let (mut command, _home) = codie();
    command
        .env("CODIE_THINKING", "bogus")
        .args(["-p", "hello"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid thinking level"));
}
