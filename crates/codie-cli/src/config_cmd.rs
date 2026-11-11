//! `codie config` command.

use anyhow::{Result, bail};
use clap::{Args, Subcommand};
use serde_json::json;

use crate::config::Config;

/// Configuration subcommands.
#[derive(Debug, Args)]
pub(crate) struct ConfigArgs {
    /// Configuration action.
    #[command(subcommand)]
    pub(crate) command: ConfigCommand,
}

/// Supported configuration actions.
#[derive(Debug, Subcommand)]
pub(crate) enum ConfigCommand {
    /// Print one effective configuration value, or all values as JSON.
    Get {
        /// Key such as `provider`, `model`, `thinking`, `max-turns`, `allow`,
        /// or `base-url`.
        key: Option<String>,
    },
}

/// Runs a configuration command.
pub(crate) fn run(args: ConfigArgs) -> Result<()> {
    let config = Config::load()?;
    match args.command {
        ConfigCommand::Get { key } => match key.as_deref() {
            None => {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({
                        "provider": config.default_provider,
                        "model": config.default_model,
                        "thinking": config.thinking.as_str(),
                        "max-turns": config.max_turns,
                        "allow": config.allow,
                        "base-url": config.base_url,
                        "session-dir": config.session_dir.display().to_string(),
                        "data-dir": config.data_dir.display().to_string(),
                    }))?
                );
            }
            Some("provider") => println!("{}", config.default_provider),
            Some("model") => println!("{}", config.default_model.unwrap_or_default()),
            Some("thinking") => println!("{}", config.thinking.as_str()),
            Some("max-turns") => println!("{}", config.max_turns),
            Some("allow") => println!("{}", config.allow.join(",")),
            Some("base-url") => println!("{}", config.base_url.unwrap_or_default()),
            Some("session-dir") => println!("{}", config.session_dir.display()),
            Some("data-dir") => println!("{}", config.data_dir.display()),
            Some(other) => bail!(
                "unknown configuration key `{other}`; expected provider, model, thinking, max-turns, allow, base-url, session-dir, or data-dir"
            ),
        },
    }
    Ok(())
}
