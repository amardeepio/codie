//! Stable names for codie's configuration and data locations.
//!
//! Phase 0 defines only vocabulary: environment-variable names, file names,
//! and directory names. Path resolution, environment lookup, and file parsing
//! belong to `codie-cli` in Phase 1 — nothing in this module touches the
//! environment or the filesystem.

/// Environment variable overriding the user configuration directory.
pub const ENV_CONFIG_DIR: &str = "CODIE_CONFIG_DIR";

/// Environment variable overriding the user data directory.
pub const ENV_DATA_DIR: &str = "CODIE_DATA_DIR";

/// Environment variable overriding the session directory.
pub const ENV_SESSION_DIR: &str = "CODIE_SESSION_DIR";

/// Environment variable overriding the cache directory.
pub const ENV_CACHE_DIR: &str = "CODIE_CACHE_DIR";

/// Name of the user configuration file.
pub const CONFIG_FILE: &str = "config.toml";

/// Name of the credential file stored under the data directory.
pub const AUTH_FILE: &str = "auth.json";

/// Name of the directory holding session files under the data directory.
pub const SESSIONS_DIR: &str = "sessions";

/// Name of the project-local resource directory.
pub const PROJECT_DIR: &str = ".codie";

/// Name of the project-local configuration file inside [`PROJECT_DIR`].
pub const PROJECT_CONFIG_FILE: &str = "config.toml";

/// Returns every environment variable that can override a codie location.
#[must_use]
pub const fn env_overrides() -> [&'static str; 4] {
    [ENV_CONFIG_DIR, ENV_DATA_DIR, ENV_SESSION_DIR, ENV_CACHE_DIR]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_override_names_are_stable() {
        assert_eq!(
            env_overrides(),
            [
                "CODIE_CONFIG_DIR",
                "CODIE_DATA_DIR",
                "CODIE_SESSION_DIR",
                "CODIE_CACHE_DIR"
            ]
        );
    }

    #[test]
    fn file_names_are_stable() {
        assert_eq!(CONFIG_FILE, "config.toml");
        assert_eq!(AUTH_FILE, "auth.json");
        assert_eq!(SESSIONS_DIR, "sessions");
        assert_eq!(PROJECT_DIR, ".codie");
        assert_eq!(PROJECT_CONFIG_FILE, "config.toml");
    }
}
