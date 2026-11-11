//! Command-line entry point for codie.
//!
//! Phase 0 exposes the version banner, help, and log verbosity only. Prompt
//! arguments, modes, and subcommands arrive in Phase 1.
//!
//! stdout is reserved for program output and protocol traffic; every log line
//! goes to stderr.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

use std::{
    io::{self, IsTerminal},
    process::ExitCode,
};

use clap::Parser;
use tracing_subscriber::EnvFilter;

/// A fast, extensible coding agent for the terminal.
#[derive(Debug, Parser)]
#[command(
    name = "codie",
    version = codie_core::VERSION,
    about = "A fast, extensible coding agent for the terminal.",
    long_about = None,
    propagate_version = true,
    arg_required_else_help = true
)]
struct Cli {
    /// Increase log verbosity: -v (info), -vv (debug), -vvv (trace).
    ///
    /// Logs are written to stderr; stdout carries program output only.
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    init_tracing(cli.verbose);

    tracing::info!(version = codie_core::VERSION, "codie starting");
    println!("codie {}", codie_core::VERSION);

    ExitCode::SUCCESS
}

/// Maps a verbosity count to the default log level.
const fn level_for(verbosity: u8) -> &'static str {
    match verbosity {
        0 => "warn",
        1 => "info",
        2 => "debug",
        _ => "trace",
    }
}

/// Initializes structured logging on stderr.
///
/// `RUST_LOG` overrides the level derived from `-v`. ANSI colors are enabled
/// only when stderr is a terminal so redirected logs stay machine-readable.
fn init_tracing(verbosity: u8) {
    let default_level = level_for(verbosity);
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(format!("codie={default_level},xtask={default_level}")));

    let stderr = io::stderr();
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(io::stderr)
        .with_ansi(stderr.is_terminal())
        .with_target(false)
        .try_init();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verbosity_maps_to_log_levels() {
        assert_eq!(level_for(0), "warn");
        assert_eq!(level_for(1), "info");
        assert_eq!(level_for(2), "debug");
        assert_eq!(level_for(3), "trace");
        assert_eq!(level_for(255), "trace");
    }
}
