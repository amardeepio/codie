//! `codie config` subcommands.

use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use serde_json::json;
use toml::value::Table;

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
    /// Write one value to config.toml.
    Set {
        /// Key such as `provider`, `model`, `thinking`, `max-turns`,
        /// `base-url`, or `allow`.
        key: String,
        /// New value; `allow` accepts a comma-separated list.
        value: String,
    },
    /// Print the config.toml path.
    Path,
    /// Open config.toml in `$VISUAL` or `$EDITOR`.
    Edit,
}

/// Runs a configuration command.
pub(crate) fn run(args: ConfigArgs) -> Result<()> {
    let config = Config::load()?;
    let path = config.config_dir.join("config.toml");
    match args.command {
        ConfigCommand::Get { key } => get(&config, key.as_deref()),
        ConfigCommand::Set { key, value } => set(&path, &key, &value),
        ConfigCommand::Path => {
            println!("{}", path.display());
            Ok(())
        }
        ConfigCommand::Edit => edit(&path),
    }
}

fn get(config: &Config, key: Option<&str>) -> Result<()> {
    match key {
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
        Some("model") => println!("{}", config.default_model.clone().unwrap_or_default()),
        Some("thinking") => println!("{}", config.thinking.as_str()),
        Some("max-turns") => println!("{}", config.max_turns),
        Some("allow") => println!("{}", config.allow.join(",")),
        Some("base-url") => println!("{}", config.base_url.clone().unwrap_or_default()),
        Some("session-dir") => println!("{}", config.session_dir.display()),
        Some("data-dir") => println!("{}", config.data_dir.display()),
        Some(other) => bail!(
            "unknown configuration key `{other}`; expected provider, model, thinking, max-turns, allow, base-url, session-dir, or data-dir"
        ),
    }
    Ok(())
}

fn set(path: &PathBuf, key: &str, value: &str) -> Result<()> {
    let mut document = if path.exists() {
        let contents = std::fs::read_to_string(path)
            .with_context(|| format!("cannot read {}", path.display()))?;
        toml::from_str::<toml::Value>(&contents)
            .with_context(|| format!("invalid configuration in {}", path.display()))?
    } else {
        toml::Value::Table(Table::new())
    };
    apply(&mut document, key, value)?;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let rendered = toml::to_string_pretty(&document)?;
    std::fs::write(path, rendered).with_context(|| format!("cannot write {}", path.display()))?;
    println!("{key} = {value}");
    Ok(())
}

/// Applies one key/value pair to a configuration document.
fn apply(document: &mut toml::Value, key: &str, value: &str) -> Result<()> {
    let table = document
        .as_table_mut()
        .context("configuration root must be a table")?;
    match key {
        "provider" => {
            table.insert(
                "default_provider".to_owned(),
                toml::Value::String(value.to_owned()),
            );
        }
        "model" => {
            table.insert(
                "default_model".to_owned(),
                toml::Value::String(value.to_owned()),
            );
        }
        "thinking" => {
            table.insert("thinking".to_owned(), toml::Value::String(value.to_owned()));
        }
        "base-url" => {
            table.insert("base_url".to_owned(), toml::Value::String(value.to_owned()));
        }
        "max-turns" => {
            let turns = value
                .parse::<i64>()
                .with_context(|| format!("max-turns must be an integer, got `{value}`"))?;
            if turns <= 0 {
                bail!("max-turns must be greater than zero");
            }
            table.insert("max_turns".to_owned(), toml::Value::Integer(turns));
        }
        "allow" => {
            let entries = value
                .split(',')
                .map(str::trim)
                .filter(|entry| !entry.is_empty())
                .map(|entry| toml::Value::String(entry.to_owned()))
                .collect::<Vec<_>>();
            let permissions = table
                .entry("permissions".to_owned())
                .or_insert_with(|| toml::Value::Table(Table::new()));
            let permissions = permissions
                .as_table_mut()
                .context("permissions must be a table")?;
            permissions.insert("allow".to_owned(), toml::Value::Array(entries));
        }
        other => bail!(
            "unknown configuration key `{other}`; expected provider, model, thinking, max-turns, base-url, or allow"
        ),
    }
    Ok(())
}

fn edit(path: &PathBuf) -> Result<()> {
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .context("set $VISUAL or $EDITOR to edit the configuration")?;
    if !path.exists() {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, "").with_context(|| format!("cannot create {}", path.display()))?;
    }
    let status = Command::new(&editor)
        .arg(path)
        .status()
        .with_context(|| format!("cannot launch {editor}"))?;
    if !status.success() {
        bail!("{editor} exited with {status}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_supported_keys() {
        let mut document = toml::Value::Table(Table::new());
        apply(&mut document, "provider", "openrouter").unwrap();
        apply(&mut document, "max-turns", "10").unwrap();
        apply(&mut document, "allow", "write, bash").unwrap();

        assert_eq!(document["default_provider"].as_str(), Some("openrouter"));
        assert_eq!(document["max_turns"].as_integer(), Some(10));
        assert_eq!(document["permissions"]["allow"][1].as_str(), Some("bash"));
    }

    #[test]
    fn rejects_unknown_keys_and_bad_numbers() {
        let mut document = toml::Value::Table(Table::new());
        assert!(apply(&mut document, "colour", "red").is_err());
        assert!(apply(&mut document, "max-turns", "many").is_err());
        assert!(apply(&mut document, "max-turns", "0").is_err());
    }
}
