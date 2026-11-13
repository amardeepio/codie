//! `codie update`: report, install, or verify the latest published release.
//!
//! Installation is checksum-verified and only uses prebuilt archives. When a
//! release has no artifact for this platform, the command prints the installer
//! command instead of building from source.

use std::cmp::Ordering;
use std::io::IsTerminal;

use anyhow::Result;
use clap::Args;

use codie_ai::update::INSTALLER_COMMAND;

/// Arguments for `codie update`.
#[derive(Debug, Args)]
pub(crate) struct UpdateArgs {
    /// Only check for a newer release; do not install it.
    #[arg(long)]
    pub(crate) check: bool,

    /// Override the release metadata endpoint. Intended for tests and mirrors.
    #[arg(long, hide = true, value_name = "URL")]
    pub(crate) endpoint: Option<String>,
}

/// Reports and, unless `--check` is passed, installs a newer release.
pub(crate) async fn run(args: UpdateArgs) -> Result<u8> {
    let endpoint = args
        .endpoint
        .clone()
        .or_else(|| std::env::var("CODIE_UPDATE_URL").ok());

    let status = codie_ai::update::check(endpoint.as_deref())
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;

    match status.ordering {
        Ordering::Less => {
            println!(
                "codie {} — update available: {}",
                status.current, status.latest
            );

            if args.check {
                if status.installable == Some(false) {
                    println!(
                        "  no prebuilt binary is published for codie {} on this platform.",
                        status.latest
                    );
                    println!("  reinstall with: {INSTALLER_COMMAND}");
                } else {
                    println!("  run `codie update` without --check to install it.");
                }
                return Ok(0);
            }

            if status.installable == Some(false) {
                println!(
                    "  no prebuilt binary is published for codie {} on this platform.",
                    status.latest
                );
                println!("  reinstall with: {INSTALLER_COMMAND}");
                return Ok(1);
            }

            println!("  installing…");
            let interactive = std::io::stdout().is_terminal();
            let mut last_percent = None;
            codie_ai::update::install_with_progress(&status.latest, |progress| {
                let Some(percent) = progress.percent() else {
                    return;
                };
                if !interactive || last_percent == Some(percent) {
                    return;
                }
                last_percent = Some(percent);
                print!("\r  downloading… {percent}%   ");
                let _ = std::io::Write::flush(&mut std::io::stdout());
            })
            .await
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
            if last_percent.is_some() {
                println!();
            }
            println!(
                "Installed codie {}. Restart codie to use the new version.",
                status.latest
            );
        }
        Ordering::Equal => {
            println!(
                "codie {} — up to date (latest {})",
                status.current, status.latest
            );
        }
        Ordering::Greater => {
            println!(
                "codie {} — newer than the latest published release ({})",
                status.current, status.latest
            );
        }
    }

    Ok(0)
}
