//! `codie update --check`: report the latest published release without
//! installing anything.
//!
//! Self-update execution is intentionally out of scope for v1. The installer
//! script remains the upgrade path.

use anyhow::Result;
use clap::Args;

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

/// Reports whether a newer release is available.
pub(crate) async fn run(args: UpdateArgs) -> Result<u8> {
    let endpoint = args
        .endpoint
        .clone()
        .or_else(|| std::env::var("CODIE_UPDATE_URL").ok());

    let status = codie_ai::update::check(endpoint.as_deref())
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;

    if !args.check {
        println!("Self-update is not implemented; checking for updates instead.");
    }

    match status.ordering {
        std::cmp::Ordering::Less => {
            println!(
                "codie {} — update available: {}",
                status.current, status.latest
            );
            println!(
                "  curl -fsSL https://raw.githubusercontent.com/amardeepio/codie/main/install.sh | sh"
            );
        }
        std::cmp::Ordering::Equal => {
            println!(
                "codie {} — up to date (latest {})",
                status.current, status.latest
            );
        }
        std::cmp::Ordering::Greater => {
            println!(
                "codie {} — newer than the latest published release ({})",
                status.current, status.latest
            );
        }
    }

    Ok(0)
}
