//! Line-coverage gate for the Phase 1 core crates.

use std::process::Command;

use anyhow::{Context, Result, bail};

/// Minimum line coverage required for each core crate.
pub(crate) const MIN_LINE_COVERAGE: f64 = 80.0;

const PACKAGES: [&str; 3] = ["codie-core", "codie-ai", "codie-tools"];

/// Runs `cargo llvm-cov` per package and enforces the line-coverage floor.
pub(crate) fn run(root: &std::path::Path) -> Result<()> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    let mut failures = Vec::new();

    for package in PACKAGES {
        println!("$ {cargo} llvm-cov -p {package} --summary-only");
        let output = Command::new(&cargo)
            .args(["llvm-cov", "-p", package, "--summary-only"])
            .current_dir(root)
            .output()
            .with_context(
                || "failed to run cargo-llvm-cov; install it with `cargo install cargo-llvm-cov`",
            )?;

        if !output.status.success() {
            bail!(
                "coverage failed for {package}: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let coverage = parse_line_coverage(&stdout)
            .with_context(|| format!("cannot find a TOTAL row in {package} coverage output"))?;

        println!("coverage: {package} lines {coverage:.2}%");
        if coverage < MIN_LINE_COVERAGE {
            failures.push(format!(
                "{package} line coverage is {coverage:.2}% (minimum {MIN_LINE_COVERAGE:.0}%)"
            ));
        }
    }

    if !failures.is_empty() {
        bail!("{}\nadd tests before shipping Phase 1", failures.join("\n"));
    }
    println!("coverage: all core crates meet the {MIN_LINE_COVERAGE:.0}% line floor");
    Ok(())
}

/// Extracts line coverage from the `TOTAL` row of `--summary-only` output.
///
/// The columns are Regions, Functions, Lines, and Branches; the third
/// percentage is line coverage.
fn parse_line_coverage(output: &str) -> Option<f64> {
    let total = output
        .lines()
        .rev()
        .find(|line| line.trim_start().starts_with("TOTAL"))?;
    let percentages = total
        .split_whitespace()
        .filter_map(|token| token.strip_suffix('%'))
        .filter_map(|token| token.parse::<f64>().ok())
        .collect::<Vec<_>>();
    percentages.get(2).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_lines_column() {
        let output = "TOTAL 1742 174 90.01% 171 29 83.04% 1211 161 86.71% 0 0 -";
        assert_eq!(parse_line_coverage(output), Some(86.71));
    }

    #[test]
    fn missing_total_is_none() {
        assert_eq!(parse_line_coverage("no rows"), None);
    }
}
