//! Configuration, paths, and precedence for the CLI.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use codie_core::ThinkingLevel;
use codie_subagents::{AgentTypeConfig, SubagentConfig, ToolSelection};
use serde::{Deserialize, Serialize};

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
    subagents: Option<FileSubagents>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct FileSubagents {
    enabled: Option<bool>,
    max_parallel: Option<usize>,
    max_depth: Option<usize>,
    max_turns: Option<usize>,
    timeout_secs: Option<u64>,
    persist_sessions: Option<bool>,
    result_max_bytes: Option<usize>,
    max_total_tokens: Option<u64>,
    max_total_cost_usd: Option<f64>,
    types: Option<BTreeMap<String, FileAgentType>>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct FileAgentType {
    model: Option<String>,
    tools: Option<FileToolSelection>,
    description: Option<String>,
    prompt: Option<String>,
    max_turns: Option<usize>,
    timeout_secs: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum FileToolSelection {
    Word(String),
    List(Vec<String>),
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
    /// True when the provider came from the environment or `config.toml`.
    pub(crate) provider_configured: bool,
    /// Default model id.
    pub(crate) default_model: Option<String>,
    /// True when the model came from the environment or `config.toml`.
    pub(crate) model_configured: bool,
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
    /// Subagent delegation configuration.
    pub(crate) subagents: SubagentConfig,
}

/// The provider and model last selected in the TUI.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub(crate) struct SelectionState {
    /// Last provider identifier.
    pub(crate) provider: Option<String>,
    /// Last model identifier.
    pub(crate) model: Option<String>,
}

impl Config {
    /// Path to the TUI selection state.
    #[must_use]
    pub(crate) fn state_path(&self) -> PathBuf {
        self.data_dir.join("state.json")
    }

    /// Loads the last TUI selection, returning defaults when it is absent or
    /// malformed. Selection state is a convenience, never a startup failure.
    #[must_use]
    pub(crate) fn load_selection(&self) -> SelectionState {
        std::fs::read_to_string(self.state_path())
            .ok()
            .and_then(|data| serde_json::from_str(&data).ok())
            .unwrap_or_default()
    }

    /// Saves the last TUI selection.
    ///
    /// # Errors
    ///
    /// Returns an error when the state file cannot be written.
    pub(crate) fn save_selection(&self, provider: &str, model: &str) -> Result<()> {
        let path = self.state_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("cannot create {}", parent.display()))?;
        }
        let state = SelectionState {
            provider: Some(provider.to_owned()),
            model: Some(model.to_owned()),
        };
        let data = serde_json::to_vec_pretty(&state)?;
        std::fs::write(&path, data).with_context(|| format!("cannot write {}", path.display()))?;
        Ok(())
    }

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
        let subagents = resolve_subagents(file.subagents)?;
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

        let provider_env = env_string("CODIE_PROVIDER");
        let provider_file = file.default_provider.clone();
        let provider_configured = provider_env.is_some() || provider_file.is_some();
        let default_provider = provider_env
            .or(provider_file)
            .unwrap_or_else(|| "deepseek".to_owned());

        let model_env = env_string("CODIE_MODEL");
        let model_file = file.default_model.clone();
        let model_configured = model_env.is_some() || model_file.is_some();
        let default_model = model_env.or(model_file);

        Ok(Self {
            data_dir,
            config_dir,
            cache_dir,
            session_dir,
            default_provider,
            provider_configured,
            default_model,
            model_configured,
            thinking,
            max_turns,
            base_url: env_string("CODIE_BASE_URL").or(file.base_url),
            allow,
            extensions_disabled,
            subagents,
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
        let key = codie_session::session_dir_key(&cwd.to_string_lossy());
        self.session_dir.join(format!("--{key}--"))
    }
}

/// Canonicalizes a working directory and removes a Windows verbatim prefix.
///
/// `Path::canonicalize` yields `\\?\C:\...` on Windows. That prefix is legal in
/// Win32 API calls but invalid in a directory name, so normalize it once before
/// the path is used for sessions, trust, prompts, or display.
pub(crate) fn normalize_cwd(cwd: &Path) -> PathBuf {
    let canonical = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
    let text = canonical.to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{rest}"))
    } else if let Some(rest) = text.strip_prefix(r"\\?\") {
        PathBuf::from(rest)
    } else {
        canonical
    }
}

fn resolve_subagents(file: Option<FileSubagents>) -> Result<SubagentConfig> {
    let defaults = SubagentConfig::default();
    let Some(file) = file else {
        return Ok(defaults);
    };
    let mut config = defaults;
    if let Some(value) = file.enabled {
        config.enabled = value;
    }
    if let Some(value) = file.max_parallel {
        config.max_parallel = value;
    }
    if let Some(value) = file.max_depth {
        config.max_depth = value;
    }
    if let Some(value) = file.max_turns {
        config.max_turns = value;
    }
    if let Some(value) = file.timeout_secs {
        config.timeout = Duration::from_secs(value);
    }
    if let Some(value) = file.persist_sessions {
        config.persist_sessions = value;
    }
    if let Some(value) = file.result_max_bytes {
        config.result_max_bytes = value;
    }
    if let Some(value) = file.max_total_tokens {
        config.max_total_tokens = Some(value);
    }
    if let Some(value) = file.max_total_cost_usd {
        config.max_total_cost_usd = Some(value);
    }
    if let Some(types) = file.types {
        for (name, declared) in types {
            let base = config.types.get(&name).cloned().unwrap_or_default();
            let tools = match declared.tools {
                Some(FileToolSelection::Word(word)) if word.eq_ignore_ascii_case("inherit") => {
                    ToolSelection::Inherit
                }
                Some(FileToolSelection::Word(word)) => {
                    bail!(
                        "subagents.types.{name}.tools must be \"inherit\" or a list of tool names, not `{word}`"
                    );
                }
                Some(FileToolSelection::List(list)) => ToolSelection::Only(list),
                None => base.tools,
            };
            config.types.insert(
                name,
                AgentTypeConfig {
                    model: declared.model.or(base.model),
                    tools,
                    description: declared.description.or(base.description),
                    prompt: declared.prompt.or(base.prompt),
                    max_turns: declared.max_turns.or(base.max_turns),
                    timeout: declared
                        .timeout_secs
                        .map(Duration::from_secs)
                        .or(base.timeout),
                },
            );
        }
    }
    config.validate().map_err(anyhow::Error::msg)?;
    Ok(config)
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
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map_or_else(|| PathBuf::from("."), PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config(session_dir: PathBuf) -> Config {
        Config {
            data_dir: PathBuf::from("/tmp/d"),
            config_dir: PathBuf::from("/tmp/cfg"),
            cache_dir: PathBuf::from("/tmp/c"),
            session_dir,
            default_provider: "deepseek".into(),
            provider_configured: false,
            default_model: None,
            model_configured: false,
            thinking: ThinkingLevel::Medium,
            max_turns: 200,
            base_url: None,
            allow: Vec::new(),
            extensions_disabled: Vec::new(),
            subagents: SubagentConfig::default(),
        }
    }

    #[test]
    #[codie_testkit::spec("CLI-012")]
    fn spec_cli_012_subagent_config_validation() {
        let file: FileSubagents = toml::from_str(
            r#"
enabled = true
max_parallel = 3
max_depth = 1
max_turns = 12
timeout_secs = 90
persist_sessions = false
result_max_bytes = 4096
max_total_tokens = 5000
max_total_cost_usd = 1.25

[types.explore]
model = "deepseek-chat"
tools = ["read", "grep"]

[types.general]
tools = "inherit"
"#,
        )
        .unwrap();
        let config = resolve_subagents(Some(file)).unwrap();
        assert!(config.enabled);
        assert_eq!(config.max_parallel, 3);
        assert_eq!(config.max_depth, 1);
        assert_eq!(config.max_turns, 12);
        assert_eq!(config.timeout, Duration::from_secs(90));
        assert!(!config.persist_sessions);
        assert_eq!(config.result_max_bytes, 4096);
        assert_eq!(config.max_total_tokens, Some(5000));
        assert_eq!(config.max_total_cost_usd, Some(1.25));
        assert_eq!(
            config.types["explore"].tools,
            ToolSelection::Only(vec!["read".to_owned(), "grep".to_owned()])
        );

        let invalid: FileSubagents = toml::from_str("max_parallel = 0").unwrap();
        let error = resolve_subagents(Some(invalid)).unwrap_err().to_string();
        assert!(error.contains("max_parallel"), "{error}");
    }

    #[test]
    fn selection_state_round_trips() {
        let directory = tempfile::tempdir().unwrap();
        let config = test_config(directory.path().join("sessions"));
        let mut config = config;
        config.data_dir = directory.path().to_path_buf();

        config.save_selection("groq", "qwen/qwen3.8-27b").unwrap();
        let state = config.load_selection();
        assert_eq!(state.provider.as_deref(), Some("groq"));
        assert_eq!(state.model.as_deref(), Some("qwen/qwen3.8-27b"));
    }

    #[test]
    fn session_directory_encodes_the_working_directory() {
        let config = test_config(PathBuf::from("/tmp/s"));
        let directory = config.session_dir_for(Path::new("/Users/me/project"));
        assert!(directory.ends_with("--Users-me-project--"));
    }

    #[test]
    fn session_directory_copes_with_windows_verbatim_paths() {
        let config = test_config(PathBuf::from("/tmp/s"));
        let directory = config.session_dir_for(Path::new(r"\\?\C:\Users\amard"));
        assert!(directory.ends_with("--C--Users-amard--"));

        let name = directory
            .file_name()
            .expect("session directory name")
            .to_string_lossy();
        for character in ['<', '>', ':', '"', '/', '\\', '|', '?', '*'] {
            assert!(
                !name.contains(character),
                "{character:?} survived in {name}"
            );
        }
    }

    #[test]
    fn normalize_cwd_strips_windows_verbatim_prefixes() {
        assert_eq!(
            normalize_cwd(Path::new(r"\\?\C:\Users\amard")),
            PathBuf::from(r"C:\Users\amard")
        );
        assert_eq!(
            normalize_cwd(Path::new(r"\\?\UNC\server\share\project")),
            PathBuf::from(r"\\server\share\project")
        );
    }

    #[cfg(windows)]
    #[test]
    fn session_directory_for_the_process_cwd_can_be_created() {
        let root = tempfile::tempdir().expect("tempdir");
        let config = test_config(root.path().to_path_buf());
        let cwd = normalize_cwd(&std::env::current_dir().expect("cwd"));
        let directory = config.session_dir_for(&cwd);
        std::fs::create_dir_all(&directory).expect("session directory must be creatable");
        assert!(directory.is_dir());
    }
}
