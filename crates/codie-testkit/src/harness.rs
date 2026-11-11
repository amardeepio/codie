//! Process-level helpers for CLI integration tests.
//!
//! The helpers return [`std::process::Command`] instead of wrapping
//! `assert_cmd`, because `assert_cmd` is a dev-dependency of the CLI crate and
//! not of `codie-testkit`. Callers choose how to capture output (`.output()`,
//! `assert_cmd::Command::from_std`, or a PTY harness).

use std::env;
use std::path::PathBuf;
use std::process::Command;

use crate::TempHome;

/// Returns the path to the `codie` binary used by [`run_cli`].
///
/// Resolution order:
///
/// 1. `CODIE_BIN` — explicit override, useful for release-binary tests.
/// 2. `CARGO_BIN_EXE_codie` — set by Cargo for integration tests when present.
/// 3. `target/debug/codie` under `CARGO_TARGET_DIR` or the workspace root.
/// 4. `codie` on `PATH`.
#[must_use]
pub fn cli_binary() -> PathBuf {
    if let Some(path) = env::var_os("CODIE_BIN").filter(|value| !value.is_empty()) {
        return PathBuf::from(path);
    }
    if let Some(path) = env::var_os("CARGO_BIN_EXE_codie").filter(|value| !value.is_empty()) {
        return PathBuf::from(path);
    }

    if let Some(root) = workspace_root() {
        let target = env::var_os("CARGO_TARGET_DIR")
            .map(PathBuf::from)
            .map_or_else(|| root.join("target"), |dir| root.join(dir));
        let candidate = target
            .join("debug")
            .join(format!("codie{}", env::consts::EXE_SUFFIX));
        if candidate.is_file() {
            return candidate;
        }
    }

    PathBuf::from(format!("codie{}", env::consts::EXE_SUFFIX))
}

/// Creates a [`Command`] that runs `codie` with `args` inside `home`.
///
/// The command receives the environment returned by [`TempHome::envs`], so the
/// process under test never touches the developer's real config, data, cache,
/// or session directories. The process environment of the test itself is left
/// untouched, which keeps tests safe under parallel execution.
#[must_use]
pub fn run_cli(home: &TempHome, args: &[&str]) -> Command {
    let mut command = Command::new(cli_binary());
    command.args(args);
    home.apply(&mut command);
    command
}

fn workspace_root() -> Option<PathBuf> {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .map(std::path::Path::to_path_buf)
}
