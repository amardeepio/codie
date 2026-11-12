//! Local CI gate and release-build helpers.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

/// Runs the local CI sequence, printing each command and stopping at the first
/// failure. Network-only tools (`cargo deny`, `cargo audit`, MSRV checks) are
/// intentionally out of scope here; CI owns those.
pub(crate) fn run(root: &Path) -> Result<()> {
    let commands: &[&[&str]] = &[
        &["fmt", "--all", "--", "--check"],
        &[
            "clippy",
            "--workspace",
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ],
        &["test", "--workspace"],
        &["xtask", "spec-check"],
        &["xtask", "schemas", "--check"],
        &["xtask", "coverage"],
        &["doc", "--workspace", "--no-deps"],
    ];

    for args in commands {
        run_cargo(root, args)?;
    }
    println!("ci: all checks passed");
    Ok(())
}

/// Builds the release CLI binary and reports its path and size.
pub(crate) fn dist(root: &Path) -> Result<()> {
    run_cargo(root, &["build", "--release", "-p", "codie-cli"])?;

    let target = target_dir(root);
    let binary = target
        .join("release")
        .join(format!("codie{}", env::consts::EXE_SUFFIX));
    let metadata = fs::metadata(&binary)
        .with_context(|| format!("release binary not found at {}", binary.display()))?;
    let mebibytes = metadata.len() as f64 / (1024.0 * 1024.0);
    println!("dist: binary {}", binary.display());
    println!("dist: size {:.2} MiB ({} bytes)", mebibytes, metadata.len());
    Ok(())
}

fn run_cargo(root: &Path, args: &[&str]) -> Result<()> {
    let cargo = env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    println!("$ {cargo} {}", args.join(" "));
    let status = Command::new(&cargo)
        .args(args)
        .current_dir(root)
        .status()
        .with_context(|| format!("failed to spawn `{cargo}`"))?;
    if !status.success() {
        bail!("`{cargo} {}` failed ({status})", args.join(" "));
    }
    Ok(())
}

fn target_dir(root: &Path) -> PathBuf {
    match env::var_os("CARGO_TARGET_DIR").map(PathBuf::from) {
        Some(dir) if dir.is_absolute() => dir,
        Some(dir) => root.join(dir),
        None => root.join("target"),
    }
}
