//! Configuration, paths, and precedence for the CLI.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use codie_core::ThinkingLevel;
use serde::Deserialize;

/// The per-user configuration file.
#[derive(Debug, Clone, Default, Deserialize)]
struct FileConfig {
    default_provider: Option<String>,
    default_model: Option<String>,
    thinking: Option<String>,
    max_turns: Option<usize>,
    base_url: Option<String>,
    permissions: Option<FilePermissions>,
    extensions: Option<FileExtensions>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct FilePermissions {
    allow: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct FileExtensions {
    disabled: Option<Vec<String>>,
}

/// Resolved CLI configuration.
#[derive(Debug, Clone)]
pub(crate) struct Config {
    /// Directory containing credentials and data.
    pub(crate) data_dir: PathBuf,
    /// Directory containing `config.toml` and `keybindings.toml`.
    pub(crate) config_dir: PathBuf,
    /// Directory for cache files such as the OpenRouter catalog.
    pub(crate) cache_dir: PathBuf,
    /// Root directory for sessions.
    pub(crate) session_dir: PathBuf,
    /// Default provider id.
    pub(crate) default_provider: String,
    /// Default model id.
    pub(crate) default_model: Option<String>,
    /// Default thinking level.
    pub(crate) thinking: ThinkingLevel,
    /// Maximum assistant requests per run.
    pub(crate) max_turns: usize,
    /// Optional provider base URL override (proxy or local test endpoint).
    pub(crate) base_url: Option<String>,
    /// Mutating tools allowed in headless mode.
    pub(crate) allow: Vec<String>,
    /// Extension names disabled by configuration.
    pub(crate) extensions_disabled: Vec<String>,
}

impl Config {
    /// Loads configuration from environment variables and `config.toml`.
    ///
    /// Precedence is CLI (applied later) > environment > file > default.
    pub(crate) fn load() -> Result<Self> {
        let config_dir = env_path("CODIE_CONFIG_DIR")
            .unwrap_or_else(|| home_dir().join(".config").join("codie"));
        let data_dir = env_path("CODIE_DATA_DIR")
            .unwrap_or_else(|| home_dir().join(".local").join("share").join("codie"));
        let session_dir =
            env_path("CODIE_SESSION_DIR").unwrap_or_else(|| data_dir.join("sessions"));
        let cache_dir = env_path("CODIE_CACHE_DIR").unwrap_or_else(|| data_dir.join("cache"));

        let file = load_file(&config_dir.join("config.toml"))?;

        let thinking_value = env_string("CODIE_THINKING").or(file.thinking);
        let thinking = match thinking_value.as_deref() {
            Some(value) => ThinkingLevel::parse(value).with_context(|| {
                format!("invalid thinking level `{value}` in configuration or CODIE_THINKING")
            })?,
            None => ThinkingLevel::default(),
        };

        let max_turns = match env_string("CODIE_MAX_TURNS") {
            Some(value) => value
                .parse::<usize>()
                .with_context(|| format!("invalid CODIE_MAX_TURNS value `{value}`"))?,
            None => file.max_turns.unwrap_or(200),
        };
        if max_turns == 0 {
            bail!("max_turns must be greater than zero");
        }

        let extensions_disabled = file
            .extensions
            .and_then(|extensions| extensions.disabled)
            .unwrap_or_default();
        let mut allow = file
            .permissions
            .and_then(|permissions| permissions.allow)
            .unwrap_or_default();
        if let Some(value) = env_string("CODIE_ALLOW") {
            allow.extend(
                value
                    .split(',')
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                    .map(str::to_owned),
            );
        }

        Ok(Self {
            data_dir,
            config_dir,
            cache_dir,
            session_dir,
            default_provider: env_string("CODIE_PROVIDER")
                .or(file.default_provider)
                .unwrap_or_else(|| "deepseek".to_owned()),
            default_model: env_string("CODIE_MODEL").or(file.default_model),
            thinking,
            max_turns,
            base_url: env_string("CODIE_BASE_URL").or(file.base_url),
            allow,
            extensions_disabled,
        })
    }

    /// Path to the credential store.
    #[must_use]
    pub(crate) fn auth_path(&self) -> PathBuf {
        self.data_dir.join("auth.json")
    }

    /// Returns the per-project session directory.
    #[must_use]
    pub(crate) fn session_dir_for(&self, cwd: &Path) -> PathBuf {
        let encoded = cwd
            .to_string_lossy()
            .trim_start_matches('/')
            .replace(['/', '\\', ':'], "-");
        self.session_dir.join(format!("--{encoded}--"))
    }
}

fn load_file(path: &Path) -> Result<FileConfig> {
    if !path.exists() {
        return Ok(FileConfig::default());
    }
    let contents =
        std::fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
    toml::from_str(&contents)
        .with_context(|| format!("invalid configuration in {}", path.display()))
}

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
}

fn env_string(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME").map_or_else(|| PathBuf::from("."), PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_directory_encodes_the_working_directory() {
        let config = Config {
            data_dir: PathBuf::from("/tmp/d"),
            config_dir: PathBuf::from("/tmp/cfg"),
            cache_dir: PathBuf::from("/tmp/c"),
            session_dir: PathBuf::from("/tmp/s"),
            default_provider: "deepseek".into(),
            default_model: None,
            thinking: ThinkingLevel::Medium,
            max_turns: 200,
            base_url: None,
            allow: Vec::new(),
            extensions_disabled: Vec::new(),
        };
        let directory = config.session_dir_for(Path::new("/Users/me/project"));
        assert!(directory.ends_with("--Users-me-project--"));
    }
}
