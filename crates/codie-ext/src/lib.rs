//! Out-of-process extensions for codie.
//!
//! Extensions are ordinary processes that speak line-delimited JSON-RPC 2.0
//! over stdio. This crate owns discovery, manifests, supervision, the hook
//! bridge, extension tools, extension commands, and structured render hints.
//! It never loads foreign code into the host process.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod command;
mod convert;
mod hooks;
mod manifest;
mod process;
mod tool;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use codie_core::{Hooks, Tool};
use codie_protocol::ext::{
    EXT_PROTOCOL_VERSION, ExtCapability, ExtCommandDecl, ExtFlagDecl, ExtHook, ExtHookParams,
    ExtHookResult, ExtInitializeParams, ExtInitializeResult, ExtSessionInfo, ExtToolDecl,
    MAX_DECLARATIONS, MAX_HOOK_TIMEOUT_MS, MAX_TOOL_TIMEOUT_MS,
};

pub use command::{CommandInvocation, ExtensionCommand};
pub use hooks::ExtensionHooks;
pub use manifest::{LoadedManifest, discover, read_manifest};
pub use process::ExtensionClient;
pub use tool::ExtTool;

/// Errors produced while loading or talking to an extension.
#[derive(Debug, thiserror::Error)]
pub enum ExtError {
    /// Filesystem or process error.
    #[error("extension I/O error: {0}")]
    Io(String),
    /// Manifest or protocol validation failed.
    #[error("extension protocol error: {0}")]
    Protocol(String),
    /// A request timed out and the extension was killed.
    #[error("extension timed out: {0}")]
    Timeout(String),
    /// The extension process is no longer running.
    #[error("extension is not running: {0}")]
    Dead(String),
    /// The extension returned a JSON-RPC error.
    #[error("extension error: {0}")]
    Rpc(String),
    /// Command or tool input was invalid.
    #[error("extension input error: {0}")]
    Invalid(String),
}

/// Options for discovering and starting extensions.
#[derive(Debug, Clone, Copy)]
pub struct LoadOptions<'a> {
    /// Directory containing `config.toml`, `extensions/`, and `trust.toml`.
    pub config_dir: &'a Path,
    /// Trusted project root, when the project has been approved.
    pub project_root: Option<&'a Path>,
    /// Explicit extension manifests or directories from `--extension`.
    pub explicit: &'a [PathBuf],
    /// Extension names disabled by configuration.
    pub disabled: &'a [String],
    /// Working directory passed to extensions.
    pub cwd: &'a Path,
    /// Session identifier, when persisted.
    pub session_id: Option<&'a str>,
    /// Deferred command-line flags passed at initialize.
    pub deferred_flags: &'a BTreeMap<String, String>,
}

/// One started extension.
pub struct Extension {
    /// Manifest name.
    pub name: String,
    /// Manifest version.
    pub version: String,
    /// Parsed manifest.
    pub manifest: codie_protocol::ext::ExtManifest,
    /// Running process.
    pub client: Arc<ExtensionClient>,
    /// Hook points this extension handles.
    pub hooks: BTreeSet<ExtHook>,
    /// Tools this extension declares.
    pub tools: Vec<ExtToolDecl>,
    /// Commands this extension declares.
    pub commands: Vec<ExtCommandDecl>,
    /// Flags this extension declares.
    pub flags: Vec<ExtFlagDecl>,
    /// Renderer kinds this extension declares.
    pub renderers: Vec<String>,
    /// Hook timeout.
    pub hook_timeout: Duration,
    /// Tool timeout.
    pub tool_timeout: Duration,
}

impl std::fmt::Debug for Extension {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Extension")
            .field("name", &self.name)
            .field("version", &self.version)
            .field("hooks", &self.hooks)
            .field("tools", &self.tools.len())
            .field("commands", &self.commands.len())
            .finish()
    }
}

impl Extension {
    async fn start(
        loaded: &LoadedManifest,
        cwd: &Path,
        session_id: Option<&str>,
        deferred_flags: &BTreeMap<String, String>,
    ) -> Result<Self, ExtError> {
        let client = Arc::new(
            ExtensionClient::spawn(&loaded.manifest, &loaded.dir)
                .map_err(|error| ExtError::Io(error.to_string()))?,
        );
        let params = ExtInitializeParams {
            protocol: EXT_PROTOCOL_VERSION,
            client_version: codie_core::VERSION.to_owned(),
            cwd: cwd.display().to_string(),
            session_id: session_id.map(str::to_owned),
            flags: deferred_flags.clone(),
            host_capabilities: vec![
                ExtCapability::Hooks,
                ExtCapability::Tools,
                ExtCapability::Commands,
                ExtCapability::Renderers,
                ExtCapability::Flags,
            ],
        };
        let value = match client
            .request(
                "initialize",
                serde_json::to_value(params)
                    .map_err(|error| ExtError::Protocol(error.to_string()))?,
                Duration::from_secs(5),
            )
            .await
        {
            Ok(value) => value,
            Err(error) => {
                client.kill();
                return Err(error);
            }
        };
        let result: ExtInitializeResult = serde_json::from_value(value)
            .map_err(|error| ExtError::Protocol(format!("invalid initialize result: {error}")))?;
        if result.protocol != EXT_PROTOCOL_VERSION {
            client.kill();
            return Err(ExtError::Protocol(format!(
                "protocol {} is not supported (host speaks {EXT_PROTOCOL_VERSION})",
                result.protocol
            )));
        }
        if result.name.trim().is_empty() {
            client.kill();
            return Err(ExtError::Protocol(
                "initialize result has an empty name".to_owned(),
            ));
        }
        let hook_timeout = Duration::from_millis(
            loaded
                .manifest
                .limits
                .hook_timeout_ms
                .clamp(1, MAX_HOOK_TIMEOUT_MS),
        );
        let tool_timeout = Duration::from_millis(
            loaded
                .manifest
                .limits
                .tool_timeout_ms
                .clamp(1, MAX_TOOL_TIMEOUT_MS),
        );
        Ok(Self {
            name: loaded.manifest.name.clone(),
            version: loaded.manifest.version.clone(),
            manifest: loaded.manifest.clone(),
            client,
            hooks: result.hooks.into_iter().collect(),
            tools: result.tools,
            commands: result.commands,
            flags: result.flags,
            renderers: result.renderers,
            hook_timeout,
            tool_timeout,
        })
    }

    pub(crate) async fn call_hook(
        &self,
        hook: ExtHook,
        params: ExtHookParams,
    ) -> Result<ExtHookResult, ExtError> {
        let value = self
            .client
            .request(
                hook.as_str(),
                serde_json::to_value(params)
                    .map_err(|error| ExtError::Protocol(error.to_string()))?,
                self.hook_timeout,
            )
            .await?;
        if value.is_null() {
            return Ok(ExtHookResult::default());
        }
        serde_json::from_value(value).map_err(|error| {
            ExtError::Protocol(format!("invalid {} result: {error}", hook.as_str()))
        })
    }
}

/// All extensions loaded for one session.
#[derive(Default)]
pub struct ExtensionSet {
    extensions: Vec<Arc<Extension>>,
    warnings: Vec<String>,
    commands: Vec<ExtensionCommand>,
    hooks: Option<Arc<dyn Hooks>>,
    cwd: PathBuf,
    session_id: Arc<Mutex<Option<String>>>,
}

impl std::fmt::Debug for ExtensionSet {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ExtensionSet")
            .field("extensions", &self.extensions.len())
            .field("commands", &self.commands.len())
            .field("warnings", &self.warnings)
            .finish()
    }
}

impl ExtensionSet {
    /// Discovers, starts, and validates every enabled extension.
    ///
    /// A broken extension is reported as a warning and skipped; it never
    /// prevents the agent from starting (`EXT-004`).
    pub async fn load(options: LoadOptions<'_>) -> Self {
        let (loaded, mut warnings) = discover(
            options.config_dir,
            options.project_root,
            options.explicit,
            options.disabled,
            options.cwd,
        );
        let mut extensions = Vec::new();
        for item in loaded {
            match Extension::start(
                &item,
                options.cwd,
                options.session_id,
                options.deferred_flags,
            )
            .await
            {
                Ok(extension) => extensions.push(extension),
                Err(error) => warnings.push(format!("extension `{}`: {error}", item.manifest.name)),
            }
        }

        let mut seen_tools = BTreeSet::new();
        let mut seen_commands = BTreeSet::new();
        let mut seen_flags = BTreeSet::new();
        let mut commands = Vec::new();
        for extension in &mut extensions {
            let name = extension.name.clone();
            for (label, count) in [
                ("tools", extension.tools.len()),
                ("commands", extension.commands.len()),
                ("flags", extension.flags.len()),
            ] {
                if count > MAX_DECLARATIONS {
                    warnings.push(format!(
                        "extension `{name}` declared {count} {label}; only the first {MAX_DECLARATIONS} are kept"
                    ));
                }
            }
            extension.tools.truncate(MAX_DECLARATIONS);
            extension.commands.truncate(MAX_DECLARATIONS);
            extension.flags.truncate(MAX_DECLARATIONS);
            extension.tools.retain(|declaration| {
                if !valid_tool_name(&declaration.name) {
                    warnings.push(format!(
                        "extension `{name}` skipped invalid tool name `{}`",
                        declaration.name
                    ));
                    return false;
                }
                if !declaration.parameters.is_object() {
                    warnings.push(format!(
                        "extension `{name}` skipped tool `{}` with a non-object schema",
                        declaration.name
                    ));
                    return false;
                }
                if !seen_tools.insert(declaration.name.clone()) {
                    warnings.push(format!(
                        "extension `{name}` skipped duplicate tool `{}`",
                        declaration.name
                    ));
                    return false;
                }
                true
            });
            extension.commands.retain(|declaration| {
                if !valid_command_name(&declaration.name) {
                    warnings.push(format!(
                        "extension `{name}` skipped invalid command name `{}`",
                        declaration.name
                    ));
                    return false;
                }
                let key = format!("{name}:{}", declaration.name);
                if !seen_commands.insert(key) {
                    warnings.push(format!(
                        "extension `{name}` skipped duplicate command `{}`",
                        declaration.name
                    ));
                    return false;
                }
                commands.push(ExtensionCommand {
                    extension: name.clone(),
                    declaration: declaration.clone(),
                });
                true
            });
            extension.flags.retain(|declaration| {
                if !valid_flag_name(&declaration.name) {
                    warnings.push(format!(
                        "extension `{name}` skipped invalid flag name `{}`",
                        declaration.name
                    ));
                    return false;
                }
                if !seen_flags.insert(declaration.name.clone()) {
                    warnings.push(format!(
                        "extension `{name}` skipped duplicate flag `--{}`",
                        declaration.name
                    ));
                    return false;
                }
                true
            });
        }
        extensions.retain(|extension| {
            !extension.hooks.is_empty()
                || !extension.tools.is_empty()
                || !extension.commands.is_empty()
                || !extension.flags.is_empty()
        });
        let extensions = extensions.into_iter().map(Arc::new).collect::<Vec<_>>();
        let cwd = options.cwd.to_path_buf();
        let session_id = Arc::new(Mutex::new(options.session_id.map(str::to_owned)));
        let hooks = hooks::build(&extensions, cwd.clone(), Arc::clone(&session_id));
        Self {
            extensions,
            warnings,
            commands,
            hooks,
            cwd,
            session_id,
        }
    }

    /// Returns a warning for each skipped or failed resource.
    #[must_use]
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// Returns true when no extension loaded successfully.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.extensions.is_empty()
    }

    /// Returns the started extensions.
    #[must_use]
    pub fn extensions(&self) -> &[Arc<Extension>] {
        &self.extensions
    }

    /// Returns the hook bridge, when at least one extension handles hooks.
    #[must_use]
    pub fn hooks(&self) -> Option<Arc<dyn Hooks>> {
        self.hooks.clone()
    }

    /// Builds one tool for every valid extension declaration.
    #[must_use]
    pub fn tools(&self) -> Vec<Arc<dyn Tool>> {
        self.extensions
            .iter()
            .flat_map(|extension| {
                extension.tools.iter().cloned().map(move |declaration| {
                    Arc::new(ExtTool::new(Arc::clone(extension), declaration)) as Arc<dyn Tool>
                })
            })
            .collect()
    }

    /// Returns every declared slash command.
    #[must_use]
    pub fn commands(&self) -> &[ExtensionCommand] {
        &self.commands
    }

    /// Returns every declared flag.
    #[must_use]
    pub fn flags(&self) -> Vec<(String, String, bool)> {
        self.extensions
            .iter()
            .flat_map(|extension| {
                extension.flags.iter().map(|flag| {
                    (
                        flag.name.clone(),
                        if flag.description.is_empty() {
                            format!("from extension `{}`", extension.name)
                        } else {
                            flag.description.clone()
                        },
                        flag.takes_value,
                    )
                })
            })
            .collect()
    }

    /// Validates deferred flags against the extensions that claimed them.
    ///
    /// # Errors
    ///
    /// Returns a usage message when a flag is unclaimed or its value shape is
    /// wrong.
    pub fn validate_flags(&self, flags: &BTreeMap<String, String>) -> Result<(), String> {
        if flags.is_empty() {
            return Ok(());
        }
        let declarations = self.flags();
        for (name, value) in flags {
            let Some((_, _, takes_value)) = declarations.iter().find(|(flag, _, _)| flag == name)
            else {
                return Err(format!(
                    "unexpected argument `--{name}`: no loaded extension claims it"
                ));
            };
            if *takes_value && value.is_empty() {
                return Err(format!(
                    "flag `--{name}` requires a value: `--{name}=VALUE`"
                ));
            }
            if !*takes_value && !value.is_empty() {
                return Err(format!("flag `--{name}` does not take a value"));
            }
        }
        Ok(())
    }

    /// Updates the session identifier used by later hooks.
    pub fn set_session_id(&self, id: Option<String>) {
        if let Ok(mut current) = self.session_id.lock() {
            *current = id;
        }
    }

    /// Returns the current session identifier.
    #[must_use]
    pub fn session_id(&self) -> Option<String> {
        self.session_id.lock().ok().and_then(|id| id.clone())
    }

    /// Returns true when a hook is handled by at least one extension.
    #[must_use]
    pub fn handles(&self, hook: ExtHook) -> bool {
        self.extensions
            .iter()
            .any(|extension| extension.hooks.contains(&hook))
    }

    /// Runs `session_start` for every interested extension.
    pub async fn session_start(&self, session: &ExtSessionInfo) {
        let params = ExtHookParams {
            session: session.clone(),
            ..ExtHookParams::default()
        };
        for extension in &self.extensions {
            if extension.hooks.contains(&ExtHook::SessionStart)
                && let Err(error) = extension
                    .call_hook(ExtHook::SessionStart, params.clone())
                    .await
            {
                tracing::warn!(extension = %extension.name, %error, "session_start hook failed");
            }
        }
    }

    /// Runs `user_submit` and returns the first decisive result.
    pub async fn user_submit(&self, session: &ExtSessionInfo, text: &str) -> ExtHookResult {
        let mut outcome = ExtHookResult::default();
        for extension in &self.extensions {
            if !extension.hooks.contains(&ExtHook::UserSubmit) {
                continue;
            }
            let params = ExtHookParams {
                session: session.clone(),
                text: Some(text.to_owned()),
                ..ExtHookParams::default()
            };
            match extension.call_hook(ExtHook::UserSubmit, params).await {
                Ok(result) => {
                    if result.block {
                        return result;
                    }
                    if let Some(prompt) = result.prompt {
                        outcome.prompt = Some(prompt);
                    }
                }
                Err(error) => {
                    tracing::warn!(extension = %extension.name, %error, "user_submit hook failed");
                }
            }
        }
        outcome
    }

    /// Resolves a namespaced or unambiguous extension command.
    ///
    /// Returns `Ok(None)` when the text is not an extension command, and an
    /// error when a namespaced command is unknown or a bare name is ambiguous.
    ///
    /// # Errors
    ///
    /// Returns a human-readable message for unknown or ambiguous commands.
    pub fn resolve_command(&self, text: &str) -> Result<Option<CommandInvocation>, String> {
        let text = text.trim();
        let Some(rest) = text.strip_prefix('/') else {
            return Ok(None);
        };
        let (head, arguments) = rest
            .split_once(char::is_whitespace)
            .map_or((rest, ""), |(head, arguments)| (head, arguments.trim()));
        if let Some(namespaced) = head.strip_prefix("ext:") {
            let Some((extension, command)) = namespaced.split_once(':') else {
                return Err(format!(
                    "unknown extension command `/{head}`; use `/ext:<extension>:<command>`"
                ));
            };
            let found = self.commands.iter().any(|candidate| {
                candidate.extension == extension && candidate.declaration.name == command
            });
            if !found {
                return Err(format!("unknown extension command `/{head}`"));
            }
            return Ok(Some(CommandInvocation {
                extension: extension.to_owned(),
                command: command.to_owned(),
                arguments: arguments.to_owned(),
            }));
        }
        let matches = self
            .commands
            .iter()
            .filter(|candidate| candidate.declaration.name == head)
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [] => Ok(None),
            [candidate] => Ok(Some(CommandInvocation {
                extension: candidate.extension.clone(),
                command: candidate.declaration.name.clone(),
                arguments: arguments.to_owned(),
            })),
            _ => Err(format!(
                "command `/{head}` is ambiguous; use /ext:<extension>:{head}"
            )),
        }
    }

    /// Invokes one extension command.
    ///
    /// # Errors
    ///
    /// Returns an error when the extension is unknown, dead, or times out.
    pub async fn invoke_command(
        &self,
        invocation: &CommandInvocation,
        session: &ExtSessionInfo,
    ) -> Result<codie_protocol::ext::ExtCommandResult, ExtError> {
        let extension = self
            .extensions
            .iter()
            .find(|extension| extension.name == invocation.extension)
            .ok_or_else(|| {
                ExtError::Invalid(format!("unknown extension `{}`", invocation.extension))
            })?;
        let params = codie_protocol::ext::ExtCommandParams {
            session: session.clone(),
            name: invocation.command.clone(),
            arguments: invocation.arguments.clone(),
        };
        let value = extension
            .client
            .request(
                "command.invoke",
                serde_json::to_value(params)
                    .map_err(|error| ExtError::Protocol(error.to_string()))?,
                extension.tool_timeout,
            )
            .await?;
        serde_json::from_value(value)
            .map_err(|error| ExtError::Protocol(format!("invalid command result: {error}")))
    }

    /// Returns the slash-command names used for completion.
    #[must_use]
    pub fn command_names(&self) -> Vec<String> {
        let mut names = Vec::new();
        for command in &self.commands {
            names.push(format!(
                "/ext:{}:{}",
                command.extension, command.declaration.name
            ));
            if self
                .commands
                .iter()
                .filter(|candidate| candidate.declaration.name == command.declaration.name)
                .count()
                == 1
            {
                names.push(format!("/{}", command.declaration.name));
            }
        }
        names.sort();
        names.dedup();
        names
    }

    /// Runs `session_shutdown` without killing the extension processes.
    pub async fn session_shutdown(&self, reason: &str) {
        for extension in &self.extensions {
            if extension.hooks.contains(&ExtHook::SessionShutdown) {
                let params = ExtHookParams {
                    session: ExtSessionInfo {
                        id: self.session_id(),
                        cwd: self.cwd.display().to_string(),
                        model: None,
                    },
                    reason: Some(reason.to_owned()),
                    ..ExtHookParams::default()
                };
                let _ = extension.call_hook(ExtHook::SessionShutdown, params).await;
            }
        }
    }

    /// Sends `session_shutdown` where supported and kills every process.
    pub async fn shutdown(&self, reason: &str) {
        self.session_shutdown(reason).await;
        for extension in &self.extensions {
            extension.client.kill();
        }
    }
}

fn valid_tool_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_lowercase())
        && name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_')
}

fn valid_command_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
}

fn valid_flag_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_validated() {
        assert!(valid_tool_name("todo_add"));
        assert!(!valid_tool_name("Todo"));
        assert!(!valid_tool_name("todo-add"));
        assert!(valid_command_name("todo-list"));
        assert!(valid_flag_name("todo-file"));
    }

    #[test]
    fn empty_set_is_empty() {
        let set = ExtensionSet::default();
        assert!(set.is_empty());
        assert!(set.hooks().is_none());
        assert!(set.tools().is_empty());
    }
}
