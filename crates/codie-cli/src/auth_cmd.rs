//! `codie auth` commands.

use anyhow::{Result, bail};
use clap::{Args, Subcommand};
use codie_ai::{AuthStore, PROVIDER_DEEPSEEK, PROVIDER_OPENROUTER};

use crate::config::Config;

/// Authentication subcommands.
#[derive(Debug, Args)]
pub(crate) struct AuthArgs {
    /// Authentication action.
    #[command(subcommand)]
    pub(crate) command: AuthCommand,
}

/// Supported authentication actions.
#[derive(Debug, Subcommand)]
pub(crate) enum AuthCommand {
    /// Store an API key for a provider.
    Login {
        /// Provider id (`deepseek` or `openrouter`).
        provider: String,
        /// API key. When omitted, codie prompts without echoing.
        #[arg(long)]
        api_key: Option<String>,
    },
    /// Remove a stored API key.
    Logout {
        /// Provider id.
        provider: String,
    },
    /// Report which providers are configured without printing secrets.
    Status {
        /// Emit JSON.
        #[arg(long)]
        json: bool,
    },
}

/// Runs an auth command.
pub(crate) fn run(args: AuthArgs, config: &Config) -> Result<()> {
    let store = AuthStore::new(config.auth_path());
    match args.command {
        AuthCommand::Login { provider, api_key } => {
            validate_provider(&provider)?;
            let key = match api_key {
                Some(key) => key,
                None => rpassword::prompt_password(format!("API key for {provider}: "))?,
            };
            let key = key.trim();
            if key.is_empty() {
                bail!("API key must not be empty");
            }
            store.set(&provider, key)?;
            println!("Stored API key for {provider}.");
        }
        AuthCommand::Logout { provider } => {
            validate_provider(&provider)?;
            if store.remove(&provider)? {
                println!("Removed API key for {provider}.");
            } else {
                println!("No stored API key for {provider}.");
            }
        }
        AuthCommand::Status { json } => {
            let statuses = ["deepseek", "openrouter"]
                .into_iter()
                .map(|provider| {
                    let stored = store.get(provider).ok().flatten().is_some();
                    let env_var = env_var(provider);
                    let environment = std::env::var(env_var)
                        .ok()
                        .is_some_and(|value| !value.trim().is_empty());
                    (provider, stored, environment, env_var)
                })
                .collect::<Vec<_>>();

            if json {
                let values = statuses
                    .iter()
                    .map(|(provider, stored, environment, env_var)| {
                        serde_json::json!({
                            "provider": provider,
                            "configured": *stored || *environment,
                            "stored": *stored,
                            "environment": *environment,
                            "env_var": env_var,
                        })
                    })
                    .collect::<Vec<_>>();
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({ "providers": values }))?
                );
            } else {
                for (provider, stored, environment, env_var) in statuses {
                    let source = match (stored, environment) {
                        (true, _) => "stored credential",
                        (false, true) => env_var,
                        (false, false) => "not configured",
                    };
                    println!("{provider}: {source}");
                }
            }
        }
    }
    Ok(())
}

fn validate_provider(provider: &str) -> Result<()> {
    if provider == PROVIDER_DEEPSEEK || provider == PROVIDER_OPENROUTER {
        Ok(())
    } else {
        bail!("unsupported provider `{provider}`; expected deepseek or openrouter")
    }
}

pub(crate) fn env_var(provider: &str) -> &'static str {
    match provider {
        PROVIDER_OPENROUTER => "OPENROUTER_API_KEY",
        _ => "DEEPSEEK_API_KEY",
    }
}
