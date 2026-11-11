//! Command-line entry point for codie.

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

mod auth_cmd;
mod config;
mod config_cmd;
mod models_cmd;
mod run;

/// A fast, extensible coding agent for the terminal.
#[derive(Debug, Parser)]
#[command(
    name = "codie",
    version,
    about = "A fast, extensible coding agent for the terminal.",
    long_about = None,
    propagate_version = true,
    args_conflicts_with_subcommands = true
)]
struct Cli {
    /// Management subcommands.
    #[command(subcommand)]
    command: Option<Commands>,

    /// One-shot run arguments.
    #[command(flatten)]
    run: run::RunArgs,

    /// Increase log verbosity (-v, -vv, -vvv).
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,

    /// Load environment variables from a file.
    ///
    /// Without this flag, codie loads `./.env` when it exists. Existing process
    /// variables always win over values from the file.
    #[arg(long, value_name = "FILE", global = true)]
    env_file: Option<PathBuf>,
}

/// Management subcommands.
#[derive(Debug, Subcommand)]
enum Commands {
    /// Manage API keys.
    Auth(auth_cmd::AuthArgs),
    /// List the bundled model catalog.
    Models(models_cmd::ModelsArgs),
    /// Inspect effective configuration.
    Config(config_cmd::ConfigArgs),
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            let code = match error.kind() {
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion => 0,
                _ => 1,
            };
            let _ = error.print();
            return ExitCode::from(code);
        }
    };
    if let Err(error) = load_env(cli.env_file.as_deref()) {
        eprintln!("error: {error:#}");
        return ExitCode::from(1);
    }
    init_tracing(cli.verbose);

    let result = match cli.command {
        Some(Commands::Auth(args)) => {
            config::Config::load().and_then(|config| auth_cmd::run(args, &config))
        }
        Some(Commands::Models(args)) => models_cmd::run(args).map(|()| 0),
        Some(Commands::Config(args)) => config_cmd::run(args).map(|()| 0),
        None => {
            return match run::run(cli.run).await {
                Ok(code) => ExitCode::from(code),
                Err(error) => {
                    eprintln!("error: {error:#}");
                    ExitCode::from(1)
                }
            };
        }
    };

    match result {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::from(1)
        }
    }
}

/// Initializes structured logging on stderr.
fn init_tracing(verbosity: u8) {
    let default_level = match verbosity {
        0 => "warn",
        1 => "info",
        2 => "debug",
        _ => "trace",
    };

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(format!("codie={default_level},xtask={default_level}")));

    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_target(false)
        .try_init();
}

/// Loads environment variables from `.env` or an explicit file.
///
/// Existing process variables are never overwritten. The implicit `.env` is
/// read only from the current directory, never from parents, so running codie
/// in an unrelated directory cannot pick up a surprising file.
fn load_env(explicit: Option<&Path>) -> Result<()> {
    let explicit = explicit
        .map(Path::to_path_buf)
        .or_else(|| std::env::var_os("CODIE_ENV_FILE").map(PathBuf::from));

    if let Some(path) = explicit {
        if !path.exists() {
            bail!("environment file not found: {}", path.display());
        }
        dotenvy::from_path(&path)
            .with_context(|| format!("cannot load environment file {}", path.display()))?;
        return Ok(());
    }

    let default = Path::new(".env");
    if default.exists() {
        dotenvy::from_path(default)
            .with_context(|| format!("cannot load environment file {}", default.display()))?;
    }
    Ok(())
}
