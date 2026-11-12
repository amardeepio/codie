//! Performance budgets: binary size, cold start, and idle memory.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

const BINARY_BUDGET: u64 = 20 * 1024 * 1024;
const STARTUP_P95_MS: u128 = 50;
const IDLE_RSS_BUDGET_KB: u64 = 80 * 1024;

/// Builds the release binary (unless `--no-build`) and checks the budgets.
pub(crate) fn run(root: &Path, args: &[String]) -> Result<bool> {
    let no_build = match args {
        [] => false,
        [flag] if flag == "--no-build" => true,
        _ => bail!("usage: cargo xtask perf [--no-build]"),
    };

    let binary = root.join("target/release/codie");
    if !no_build || !binary.exists() {
        let status = Command::new("cargo")
            .args(["build", "--release", "-p", "codie-cli"])
            .current_dir(root)
            .status()
            .context("cannot run cargo build")?;
        if !status.success() {
            bail!("release build failed");
        }
    }

    let size = std::fs::metadata(&binary)
        .with_context(|| format!("cannot stat {}", binary.display()))?
        .len();
    let size_ok = size <= BINARY_BUDGET;

    // Warm the page cache, then sample cold process starts.
    let mut samples = Vec::new();
    for iteration in 0..20 {
        let start = Instant::now();
        let status = Command::new(&binary)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .context("cannot run codie --version")?;
        if !status.success() {
            bail!("codie --version failed");
        }
        if iteration > 0 {
            samples.push(start.elapsed());
        }
    }
    samples.sort_unstable();
    let p95 = samples[samples.len() * 95 / 100];
    let startup_ok = p95.as_millis() <= STARTUP_P95_MS;

    let cwd = root.join("target/perf-cwd");
    std::fs::create_dir_all(&cwd).context("cannot create the perf working directory")?;
    let mut child = Command::new(&binary)
        .args(["--mode", "rpc", "--provider", "deepseek", "--cwd"])
        .arg(&cwd)
        .env("DEEPSEEK_API_KEY", "perf-check")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("cannot start codie --mode rpc")?;

    std::thread::sleep(Duration::from_millis(750));
    let pid = child.id().to_string();
    let rss = Command::new("ps")
        .args(["-o", "rss=", "-p", &pid])
        .output()
        .context("cannot read process memory")?;
    let rss_kb = String::from_utf8_lossy(&rss.stdout)
        .trim()
        .parse::<u64>()
        .unwrap_or_default();
    let _ = child.kill();
    let _ = child.wait();

    let idle_ok = rss_kb > 0 && rss_kb <= IDLE_RSS_BUDGET_KB;

    println!(
        "binary size: {:.1} MB / {:.1} MB  {}",
        size as f64 / (1024.0 * 1024.0),
        BINARY_BUDGET as f64 / (1024.0 * 1024.0),
        verdict(size_ok)
    );
    println!(
        "startup p95: {} ms / {} ms  {}",
        p95.as_millis(),
        STARTUP_P95_MS,
        verdict(startup_ok)
    );
    println!(
        "idle RSS: {:.1} MB / {:.1} MB  {}",
        rss_kb as f64 / 1024.0,
        IDLE_RSS_BUDGET_KB as f64 / 1024.0,
        verdict(idle_ok)
    );

    Ok(size_ok && startup_ok && idle_ok)
}

const fn verdict(ok: bool) -> &'static str {
    if ok { "ok" } else { "FAIL" }
}
