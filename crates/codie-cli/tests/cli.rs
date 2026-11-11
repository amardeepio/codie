//! Integration tests for the Phase 0 command-line surface.
//!
//! These exercise the compiled binary so that exit codes, stdout, and stderr
//! match what a script or a terminal would observe.

use assert_cmd::Command;
use predicates::prelude::*;

/// Returns a fresh command handle for the `codie` binary.
fn codie() -> Command {
    Command::cargo_bin("codie").expect("the codie binary should be built for tests")
}

#[test]
fn version_flag_prints_the_version_to_stdout() {
    codie()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("codie"))
        .stdout(predicate::str::contains(codie_core::VERSION))
        .stderr(predicate::str::is_empty());
}

#[test]
fn help_flag_documents_the_phase_0_surface() {
    codie()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Usage:"))
        .stdout(predicate::str::contains("--verbose"))
        .stdout(predicate::str::contains("--version"))
        .stderr(predicate::str::is_empty());
}

#[test]
fn no_arguments_prints_help_to_stderr_and_fails() {
    codie()
        .assert()
        .failure()
        .code(2)
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("Usage:"));
}

#[test]
fn verbose_flag_sends_logs_to_stderr_only() {
    codie()
        .arg("-v")
        .assert()
        .success()
        .stdout(predicate::str::contains("codie"))
        .stdout(predicate::str::contains("INFO").not())
        .stderr(predicate::str::contains("codie starting"));
}
