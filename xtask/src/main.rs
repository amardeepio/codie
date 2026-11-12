//! Repository automation for codie.
//!
//! Commands are intentionally dependency-light so the tooling can run before
//! the product crates compile. See `cargo xtask --help` for the command list.

#![forbid(unsafe_code)]

mod ci;
mod coverage;
mod perf;
mod schema;
mod spec;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Result, bail};

const USAGE: &str = "\
usage: cargo xtask <command> [options]

commands:
  spec-check [--strict]  validate specs and requirement-to-test traceability
  spec-report            write target/spec-report.md
  ci                     run the local CI gate sequence (fail fast)
  coverage               enforce 80% line coverage in core, ai, and tools
  schemas [--check]      generate or verify published JSON schemas
  perf [--no-build]      check binary size, startup, and idle memory budgets
  dist                   build the release codie binary (Phase 0 stub)
";

fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            eprintln!("xtask: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<bool> {
    let mut args = std::env::args().skip(1);
    let Some(command) = args.next() else {
        print!("{USAGE}");
        return Ok(true);
    };
    let rest: Vec<String> = args.collect();
    let root = workspace_root();

    match command.as_str() {
        "spec-check" => spec::check(&root, parse_strict(&rest)?),
        "spec-report" => {
            reject_flags(&rest, "spec-report")?;
            spec::report(&root)
        }
        "ci" => {
            reject_flags(&rest, "ci")?;
            ci::run(&root)?;
            Ok(true)
        }
        "coverage" => {
            reject_flags(&rest, "coverage")?;
            coverage::run(&root)?;
            Ok(true)
        }
        "schemas" => schema::run(&root, &rest),
        "perf" => perf::run(&root, &rest),
        "dist" => {
            reject_flags(&rest, "dist")?;
            ci::dist(&root)?;
            Ok(true)
        }
        "help" | "--help" | "-h" => {
            print!("{USAGE}");
            Ok(true)
        }
        other => bail!("unknown command `{other}`\n\n{USAGE}"),
    }
}

fn parse_strict(args: &[String]) -> Result<bool> {
    let mut strict = false;
    for arg in args {
        match arg.as_str() {
            "--strict" => strict = true,
            other => bail!("unknown option `{other}` for spec-check (only `--strict` is accepted)"),
        }
    }
    Ok(strict)
}

fn reject_flags(args: &[String], command: &str) -> Result<()> {
    if let Some(flag) = args.first() {
        bail!("unexpected argument `{flag}` for {command}");
    }
    Ok(())
}

/// The workspace root is the parent of the xtask crate.
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}
