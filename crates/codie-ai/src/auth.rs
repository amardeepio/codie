//! Credential storage and API-key resolution.
//!
//! Credentials live in a caller-provided `auth.json` file shaped as
//! `{"providers": {"<id>": {"api_key": "..."}}}`. Writes are atomic and the
//! file is created with mode `0600` on Unix. Keys are never logged or included
//! in error messages (`PROV-030`, `PROV-031`, `PROV-033`).

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use thiserror::Error;

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Errors produced while reading or writing credentials.
#[derive(Debug, Error)]
pub enum AuthError {
    /// The credentials file could not be read or written.
    #[error("failed to access credentials at {path}: {source}")]
    Io {
        /// Path that failed.
        path: PathBuf,
        /// Underlying filesystem error.
        #[source]
        source: std::io::Error,
    },
    /// The credentials file was not valid JSON.
    #[error("credentials file {path} is not valid JSON: {source}")]
    Json {
        /// Path that failed.
        path: PathBuf,
        /// Underlying JSON error.
        #[source]
        source: serde_json::Error,
    },
    /// No credential was found for the requested provider.
    #[error(
        "no API key configured for provider `{provider}`; set {env_var} or run `codie auth login {provider}`"
    )]
    Missing {
        /// Provider that has no credential.
        provider: String,
        /// Environment variable that would supply one.
        env_var: String,
    },
    /// A caller attempted to store an empty key.
    #[error("provider `{provider}` was given an empty API key")]
    EmptyKey {
        /// Provider whose key was empty.
        provider: String,
    },
}

/// Where a provider's credential came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialSource {
    /// An explicit key passed for this run only.
    Explicit,
    /// A key stored in `auth.json`.
    Stored,
    /// A key read from the provider's environment variable.
    Environment,
    /// The provider is a local endpoint that does not need a key.
    Keyless,
}

impl CredentialSource {
    /// Returns a short human-readable description.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Explicit => "key for this run",
            Self::Stored => "stored key",
            Self::Environment => "environment key",
            Self::Keyless => "no key required",
        }
    }
}

/// The credential state of one provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderAuthStatus {
    /// Provider identifier.
    pub provider: String,
    /// True when a non-empty key is available from any source.
    pub configured: bool,
    /// Source that supplied the key, when configured.
    pub source: Option<CredentialSource>,
    /// Environment variable checked for this provider.
    pub env_var: String,
    /// Error encountered while reading `auth.json`, if any.
    pub error: Option<String>,
}

impl ProviderAuthStatus {
    /// Builds an unconfigured status for `provider`.
    #[must_use]
    pub fn missing(provider: &str) -> Self {
        Self {
            provider: provider.to_owned(),
            configured: false,
            source: None,
            env_var: crate::model::provider_env_var(provider).to_owned(),
            error: None,
        }
    }

    /// Builds a configured status for `provider`.
    #[must_use]
    pub fn configured(provider: &str, source: CredentialSource) -> Self {
        Self {
            provider: provider.to_owned(),
            configured: true,
            source: Some(source),
            env_var: crate::model::provider_env_var(provider).to_owned(),
            error: None,
        }
    }

    /// Returns true when the provider can be used without another credential.
    ///
    /// Keyless local providers are usable but are not counted as configured
    /// credentials by `codie auth status`.
    #[must_use]
    pub fn usable(&self) -> bool {
        self.configured || self.source == Some(CredentialSource::Keyless)
    }

    /// Returns a compact description for pickers and status output.
    #[must_use]
    pub fn label(&self) -> String {
        if let Some(error) = &self.error
            && !self.configured
        {
            return format!("credentials error: {error}");
        }
        match self.source {
            Some(CredentialSource::Explicit) => CredentialSource::Explicit.label().to_owned(),
            Some(CredentialSource::Stored) => CredentialSource::Stored.label().to_owned(),
            Some(CredentialSource::Environment) => format!("env {}", self.env_var),
            Some(CredentialSource::Keyless) => CredentialSource::Keyless.label().to_owned(),
            None => "not connected".to_owned(),
        }
    }
}

/// Reads the stored and environment credential state for one provider.
///
/// This never returns the key itself and never performs a network request.
#[must_use]
pub fn auth_status(store: Option<&AuthStore>, provider: &str) -> ProviderAuthStatus {
    let env_var = crate::model::provider_env_var(provider);
    if !crate::provider::requires_key(provider) {
        let mut status = ProviderAuthStatus::missing(provider);
        status.source = Some(CredentialSource::Keyless);
        return status;
    }

    let mut status = ProviderAuthStatus::missing(provider);

    if let Some(store) = store {
        match store.get(provider) {
            Ok(Some(key)) if !key.trim().is_empty() => {
                status.configured = true;
                status.source = Some(CredentialSource::Stored);
            }
            Ok(_) => {}
            Err(error) => status.error = Some(error.to_string()),
        }
    }

    if !status.configured
        && std::env::var(env_var)
            .ok()
            .is_some_and(|value| !value.trim().is_empty())
    {
        status.configured = true;
        status.source = Some(CredentialSource::Environment);
    }

    status
}

/// A JSON-backed credential store.
#[derive(Debug, Clone)]
pub struct AuthStore {
    path: PathBuf,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct AuthFile {
    #[serde(default)]
    providers: BTreeMap<String, ProviderEntry>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ProviderEntry {
    api_key: String,
}

impl AuthStore {
    /// Creates a store backed by `path`. The file is not touched until a write.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Returns the credentials file path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns the stored key for `provider`, if any.
    ///
    /// # Errors
    ///
    /// Returns an error when the file exists but cannot be read or parsed.
    pub fn get(&self, provider: &str) -> Result<Option<String>, AuthError> {
        let file = self.load()?;
        Ok(file
            .providers
            .get(provider)
            .map(|entry| entry.api_key.clone()))
    }

    /// Stores or replaces the key for `provider`.
    ///
    /// # Errors
    ///
    /// Returns an error for an empty key or a filesystem failure.
    pub fn set(&self, provider: &str, api_key: &str) -> Result<(), AuthError> {
        if api_key.trim().is_empty() {
            return Err(AuthError::EmptyKey {
                provider: provider.to_owned(),
            });
        }
        let mut file = self.load()?;
        file.providers.insert(
            provider.to_owned(),
            ProviderEntry {
                api_key: api_key.to_owned(),
            },
        );
        self.save(&file)
    }

    /// Removes the key for `provider`, returning whether one was present.
    ///
    /// # Errors
    ///
    /// Returns an error when the file exists but cannot be read, parsed, or
    /// rewritten.
    pub fn remove(&self, provider: &str) -> Result<bool, AuthError> {
        let mut file = self.load()?;
        if file.providers.remove(provider).is_none() {
            return Ok(false);
        }
        self.save(&file)?;
        Ok(true)
    }

    /// Lists providers that have a stored credential, sorted by identifier.
    ///
    /// # Errors
    ///
    /// Returns an error when the file exists but cannot be read or parsed.
    pub fn list(&self) -> Result<Vec<String>, AuthError> {
        let file = self.load()?;
        Ok(file.providers.keys().cloned().collect())
    }

    fn load(&self) -> Result<AuthFile, AuthError> {
        let data = match fs::read(&self.path) {
            Ok(data) => data,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(AuthFile::default());
            }
            Err(source) => {
                return Err(AuthError::Io {
                    path: self.path.clone(),
                    source,
                });
            }
        };

        serde_json::from_slice(&data).map_err(|source| AuthError::Json {
            path: self.path.clone(),
            source,
        })
    }

    fn save(&self, file: &AuthFile) -> Result<(), AuthError> {
        if let Some(parent) = self.path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent).map_err(|source| AuthError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }

        let data = serde_json::to_vec_pretty(file).map_err(|source| AuthError::Json {
            path: self.path.clone(),
            source,
        })?;

        let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let file_name = self
            .path
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .unwrap_or("auth.json");
        let temp_path = self
            .path
            .with_file_name(format!(".{file_name}.{}.{counter}.tmp", std::process::id()));

        let write_result = self.write_temp(&temp_path, &data);
        if let Err(error) = write_result {
            let _ = fs::remove_file(&temp_path);
            return Err(error);
        }

        if let Err(source) = fs::rename(&temp_path, &self.path) {
            let _ = fs::remove_file(&temp_path);
            return Err(AuthError::Io {
                path: self.path.clone(),
                source,
            });
        }

        set_private_permissions(&self.path);

        if let Some(parent) = self.path.parent()
            && !parent.as_os_str().is_empty()
            && let Ok(directory) = fs::File::open(parent)
        {
            let _ = directory.sync_all();
        }

        Ok(())
    }

    fn write_temp(&self, path: &Path, data: &[u8]) -> Result<(), AuthError> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);

        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }

        let mut file = options.open(path).map_err(|source| AuthError::Io {
            path: path.to_path_buf(),
            source,
        })?;

        file.write_all(data).map_err(|source| AuthError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        file.sync_all().map_err(|source| AuthError::Io {
            path: path.to_path_buf(),
            source,
        })?;

        Ok(())
    }
}

#[cfg(unix)]
fn set_private_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn set_private_permissions(_path: &Path) {}

/// Resolves an API key using the documented precedence (`PROV-030`).
///
/// The order is explicit key, stored credential, then the provider's
/// environment variable. Resolution never falls through to a different
/// provider. Empty values are treated as absent.
///
/// # Errors
///
/// Returns [`AuthError::Missing`] when no source supplies a non-empty key.
pub fn resolve_key(
    explicit: Option<&str>,
    store: &AuthStore,
    provider: &str,
    env_var: &str,
) -> Result<String, AuthError> {
    if let Some(key) = explicit.map(str::trim).filter(|key| !key.is_empty()) {
        return Ok(key.to_owned());
    }

    if let Some(key) = store.get(provider)? {
        let key = key.trim();
        if !key.is_empty() {
            return Ok(key.to_owned());
        }
    }

    if let Ok(key) = std::env::var(env_var) {
        let key = key.trim();
        if !key.is_empty() {
            return Ok(key.to_owned());
        }
    }

    Err(AuthError::Missing {
        provider: provider.to_owned(),
        env_var: env_var.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_is_an_empty_store() {
        let directory = tempfile::tempdir().unwrap();
        let store = AuthStore::new(directory.path().join("auth.json"));
        assert_eq!(store.get("deepseek").unwrap(), None);
        assert!(store.list().unwrap().is_empty());
    }

    #[test]
    fn status_reports_the_source_without_the_key() {
        let directory = tempfile::tempdir().unwrap();
        let store = AuthStore::new(directory.path().join("auth.json"));
        let missing = auth_status(Some(&store), "gemini");
        assert!(!missing.configured);
        assert_eq!(missing.source, None);

        store.set("gemini", "sk-secret").unwrap();
        let stored = auth_status(Some(&store), "gemini");
        assert!(stored.configured);
        assert_eq!(stored.source, Some(CredentialSource::Stored));
        assert!(!format!("{stored:?}").contains("sk-secret"));
        assert!(!stored.label().contains("sk-secret"));
    }

    #[test]
    fn explicit_key_beats_store_and_environment() {
        let directory = tempfile::tempdir().unwrap();
        let store = AuthStore::new(directory.path().join("auth.json"));
        store.set("deepseek", "stored").unwrap();
        assert_eq!(
            resolve_key(Some("explicit"), &store, "deepseek", "PATH").unwrap(),
            "explicit"
        );
    }
}
