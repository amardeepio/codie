//! `codie auth` commands.

use anyhow::{Result, bail};
use clap::{Args, Subcommand};
use codie_ai::provider;
use codie_ai::{
    AuthStore, auth_status, canonical_provider, probe_key, provider_env_var, resolve_key,
};

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
    /// Verify and store an API key for a provider.
    Login {
        /// Provider id (`deepseek`, `openrouter`, or `gemini`).
        provider: String,
        /// API key. When omitted, codie prompts without echoing.
        #[arg(long)]
        api_key: Option<String>,
        /// Store the key without contacting the provider.
        #[arg(long)]
        no_verify: bool,
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
        /// Contact each configured provider to check its key.
        #[arg(long)]
        verify: bool,
    },
    /// Check one provider's stored or environment key.
    Verify {
        /// Provider id.
        provider: String,
    },
}

/// Runs an auth command.
///
/// Returns the process exit code. `auth status` exits `2` when no provider is
/// configured, so scripts can distinguish "unconfigured" from a runtime error
/// (`CLI-003`).
pub(crate) async fn run(args: AuthArgs, config: &Config) -> Result<u8> {
    let store = AuthStore::new(config.auth_path());
    match args.command {
        AuthCommand::Login {
            provider,
            api_key,
            no_verify,
        } => {
            let provider = validate_provider(&provider)?;
            let key = match api_key {
                Some(key) => key,
                None if !provider::requires_key(provider) => {
                    println!("{provider} does not require an API key.");
                    return Ok(0);
                }
                None => rpassword::prompt_password(format!("API key for {provider}: "))?,
            };
            let key = key.trim();
            if key.is_empty() {
                bail!("API key must not be empty");
            }

            if no_verify {
                store.set(provider, key)?;
                println!("Stored API key for {provider} (not verified).");
                return Ok(0);
            }

            let outcome = probe_key(provider, key, config.base_url.as_deref()).await;
            if !outcome.accepted() {
                bail!(
                    "did not store the API key for {provider}: {}",
                    outcome.summary()
                );
            }
            if matches!(outcome, codie_ai::probe::ProbeOutcome::Throttled { .. }) {
                eprintln!("warning: {}", outcome.summary());
            }
            store.set(provider, key)?;
            println!("Verified and stored API key for {provider}.");
        }
        AuthCommand::Logout { provider } => {
            let provider = validate_provider(&provider)?;
            if store.remove(provider)? {
                println!("Removed API key for {provider}.");
            } else {
                println!("No stored API key for {provider}.");
            }
        }
        AuthCommand::Status { json, verify } => {
            let mut statuses = Vec::new();
            for spec in provider::all() {
                let status = auth_status(Some(&store), spec.id);
                let outcome = if verify && status.configured {
                    match resolve_key(None, &store, spec.id, spec.env_var) {
                        Ok(key) => Some(probe_key(spec.id, &key, config.base_url.as_deref()).await),
                        Err(_) if !spec.requires_key => {
                            Some(probe_key(spec.id, "", config.base_url.as_deref()).await)
                        }
                        Err(_) => None,
                    }
                } else {
                    None
                };
                statuses.push((status, outcome));
            }

            let any_configured = statuses.iter().any(|(status, _)| status.configured);

            if json {
                let values = statuses
                    .iter()
                    .map(|(status, outcome)| {
                        serde_json::json!({
                            "provider": status.provider,
                            "configured": status.configured,
                            "stored": status.source == Some(codie_ai::CredentialSource::Stored),
                            "environment": status.source == Some(codie_ai::CredentialSource::Environment),
                            "env_var": status.env_var,
                            "source": status.label(),
                            "error": status.error,
                            "verified": outcome.as_ref().map(|outcome| outcome.accepted()),
                            "probe": outcome.as_ref().map(|outcome| outcome.summary()),
                        })
                    })
                    .collect::<Vec<_>>();
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({ "providers": values }))?
                );
            } else {
                for (status, outcome) in statuses {
                    let mut line = format!("{}: {}", status.provider, status.label());
                    if let Some(outcome) = outcome {
                        line.push_str(&format!(" · {}", outcome.summary()));
                    }
                    println!("{line}");
                }
            }
            return Ok(if any_configured { 0 } else { 2 });
        }
        AuthCommand::Verify { provider } => {
            let provider = validate_provider(&provider)?;
            let key = match resolve_key(None, &store, provider, provider_env_var(provider)) {
                Ok(key) => key,
                Err(_) if !provider::requires_key(provider) => String::new(),
                Err(error) => {
                    eprintln!("{error}");
                    return Ok(2);
                }
            };
            let outcome = probe_key(provider, &key, config.base_url.as_deref()).await;
            println!("{provider}: {}", outcome.summary());
            return Ok(if outcome.accepted() { 0 } else { 2 });
        }
    }
    Ok(0)
}

fn validate_provider(provider: &str) -> Result<&'static str> {
    canonical_provider(provider).ok_or_else(|| {
        anyhow::anyhow!(
            "unsupported provider `{provider}`; expected {}",
            provider::ids().join(", ")
        )
    })
}
