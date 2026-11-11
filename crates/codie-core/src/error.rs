//! Stable, machine-readable errors for codie.
//!
//! Errors crossing a process or protocol boundary carry a stable [`ErrorKind`]
//! code. Human-readable messages may change; codes may not.

use std::path::PathBuf;

use serde::Serialize;

/// A stable category for an error.
///
/// The [`ErrorKind::code`] string is part of codie's public contract and is
/// used by `--mode json`, RPC responses, and extension protocols.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    /// Invalid or missing configuration.
    Config,
    /// Filesystem or operating-system I/O failure.
    Io,
    /// Provider request, stream, or protocol failure.
    Provider,
    /// Tool validation or execution failure.
    Tool,
    /// Session storage or projection failure.
    Session,
    /// A bug or invariant violation inside codie.
    Internal,
}

impl ErrorKind {
    /// Every error kind in declaration order.
    pub const ALL: [Self; 6] = [
        Self::Config,
        Self::Io,
        Self::Provider,
        Self::Tool,
        Self::Session,
        Self::Internal,
    ];

    /// Returns the stable wire code for this category.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Config => "config_error",
            Self::Io => "io_error",
            Self::Provider => "provider_error",
            Self::Tool => "tool_error",
            Self::Session => "session_error",
            Self::Internal => "internal_error",
        }
    }
}

/// The error type shared by codie libraries.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A configuration value is missing or invalid.
    #[error("configuration error: {message}")]
    Config {
        /// Description of the problem.
        message: String,
    },

    /// A filesystem operation failed.
    #[error("i/o error at {path}: {source}")]
    Io {
        /// Path involved in the failed operation.
        path: PathBuf,
        /// Underlying operating-system error.
        #[source]
        source: std::io::Error,
    },

    /// A provider request or response failed.
    #[error("provider error: {message}")]
    Provider {
        /// Description of the failure.
        message: String,
    },

    /// A tool failed to validate or execute.
    #[error("tool error: {message}")]
    Tool {
        /// Description of the failure.
        message: String,
    },

    /// The session store rejected a read or write.
    #[error("session error: {message}")]
    Session {
        /// Description of the failure.
        message: String,
    },

    /// An internal invariant was violated.
    #[error("internal error: {message}")]
    Internal {
        /// Description of the failure.
        message: String,
    },
}

impl Error {
    /// Creates a configuration error.
    #[must_use]
    pub fn config(message: impl Into<String>) -> Self {
        Self::Config {
            message: message.into(),
        }
    }

    /// Creates an I/O error for the path that caused it.
    #[must_use]
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }

    /// Creates a provider error.
    #[must_use]
    pub fn provider(message: impl Into<String>) -> Self {
        Self::Provider {
            message: message.into(),
        }
    }

    /// Creates a tool error.
    #[must_use]
    pub fn tool(message: impl Into<String>) -> Self {
        Self::Tool {
            message: message.into(),
        }
    }

    /// Creates a session error.
    #[must_use]
    pub fn session(message: impl Into<String>) -> Self {
        Self::Session {
            message: message.into(),
        }
    }

    /// Creates an internal error.
    #[must_use]
    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal {
            message: message.into(),
        }
    }

    /// Returns the stable category for this error.
    #[must_use]
    pub const fn kind(&self) -> ErrorKind {
        match self {
            Self::Config { .. } => ErrorKind::Config,
            Self::Io { .. } => ErrorKind::Io,
            Self::Provider { .. } => ErrorKind::Provider,
            Self::Tool { .. } => ErrorKind::Tool,
            Self::Session { .. } => ErrorKind::Session,
            Self::Internal { .. } => ErrorKind::Internal,
        }
    }

    /// Returns the stable wire code for this error.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        self.kind().code()
    }
}

/// Convenience result type used throughout codie.
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_stable() {
        assert_eq!(Error::config("x").code(), "config_error");
        assert_eq!(
            Error::io("x", std::io::Error::other("x")).code(),
            "io_error"
        );
        assert_eq!(Error::provider("x").code(), "provider_error");
        assert_eq!(Error::tool("x").code(), "tool_error");
        assert_eq!(Error::session("x").code(), "session_error");
        assert_eq!(Error::internal("x").code(), "internal_error");
    }

    #[test]
    fn every_kind_has_a_unique_code() {
        let mut codes: Vec<&str> = ErrorKind::ALL.iter().map(|kind| kind.code()).collect();
        codes.sort_unstable();
        let count = codes.len();
        codes.dedup();
        assert_eq!(codes.len(), count);
        assert_eq!(count, 6);
    }

    #[test]
    fn kind_matches_the_constructor() {
        let io = std::io::Error::other("boom");

        assert_eq!(Error::config("x").kind(), ErrorKind::Config);
        assert_eq!(Error::io("/tmp/x", io).kind(), ErrorKind::Io);
        assert_eq!(Error::provider("x").kind(), ErrorKind::Provider);
        assert_eq!(Error::tool("x").kind(), ErrorKind::Tool);
        assert_eq!(Error::session("x").kind(), ErrorKind::Session);
        assert_eq!(Error::internal("x").kind(), ErrorKind::Internal);
    }

    #[test]
    fn display_prepends_the_category() {
        assert_eq!(
            Error::config("missing model").to_string(),
            "configuration error: missing model"
        );
        assert_eq!(
            Error::provider("status 500").to_string(),
            "provider error: status 500"
        );
        assert_eq!(
            Error::tool("unknown tool").to_string(),
            "tool error: unknown tool"
        );
        assert_eq!(
            Error::session("bad header").to_string(),
            "session error: bad header"
        );
        assert_eq!(
            Error::internal("invariant").to_string(),
            "internal error: invariant"
        );
    }

    #[test]
    fn io_display_includes_path_and_source() {
        let error = Error::io("/tmp/codie", std::io::Error::other("boom"));
        assert_eq!(error.to_string(), "i/o error at /tmp/codie: boom");
    }
}
