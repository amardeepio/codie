//! Built-in tools for codie.
//!
//! The built-in surface is `read`, `write`, `edit`, a platform shell,
//! `grep`, `find`, `ls`, and `web_fetch`, plus path resolution, output
//! truncation, a tool registry, and a policy wrapper. The shell is `bash` on
//! macOS and Linux and `powershell` on Windows. `read`, `grep`, `find`, and
//! `ls` are read-only and never denied by the default permission policy.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

use std::{fmt, str::FromStr};

pub mod bash;
pub mod edit;
mod file_mutation;
pub mod find;
pub mod grep;
pub mod ls;
pub mod path;
pub mod powershell;
pub mod read;
pub mod registry;
mod search;
mod shell;
pub mod truncate;
pub mod validate;
pub mod web_fetch;
pub mod write;

pub use bash::BashTool;
pub use edit::EditTool;
pub use find::FindTool;
pub use grep::GrepTool;
pub use ls::LsTool;
pub use path::resolve_path;
pub use powershell::PowerShellTool;
pub use read::ReadTool;
pub use registry::{Policy, PolicyToolbox, Registry, is_read_only};
pub use shell::MAX_TIMEOUT_SECONDS;
pub use truncate::{
    DEFAULT_MAX_BYTES, DEFAULT_MAX_LINE_CHARS, DEFAULT_MAX_LINES, TruncatedBy, TruncationResult,
    format_size, truncate_head, truncate_head_with_limits, truncate_line, truncate_line_with_limit,
    truncate_tail, truncate_tail_with_limits,
};
pub use validate::ValidatedTool;
pub use web_fetch::WebFetchTool;
pub use write::WriteTool;

/// Names of the built-in tools.
///
/// The [`ToolName::as_str`] spelling is part of codie's public contract: it is
/// used on the wire, in prompts, and in permission policies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToolName {
    /// Read a file.
    Read,
    /// Create or overwrite a file.
    Write,
    /// Apply exact-text replacements to a file.
    Edit,
    /// Execute a POSIX shell command.
    Bash,
    /// Execute a Windows PowerShell command.
    PowerShell,
    /// Search file contents.
    Grep,
    /// Find files by glob pattern.
    Find,
    /// List a directory.
    Ls,
    /// Fetch an http(s) URL.
    WebFetch,
}

/// The shell tool registered by default on this platform.
const PLATFORM_SHELL: ToolName = if cfg!(windows) {
    ToolName::PowerShell
} else {
    ToolName::Bash
};

impl ToolName {
    /// Returns the name used on the wire and in prompts.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::Edit => "edit",
            Self::Bash => "bash",
            Self::PowerShell => "powershell",
            Self::Grep => "grep",
            Self::Find => "find",
            Self::Ls => "ls",
            Self::WebFetch => "web_fetch",
        }
    }

    /// Returns every built-in tool name in declaration order.
    ///
    /// The shell slot is `bash` on macOS and Linux and `powershell` on
    /// Windows, so [`ToolName::Bash`] and [`ToolName::PowerShell`] always
    /// parse but only the platform's shell appears in the registry.
    #[must_use]
    pub const fn all() -> [Self; 8] {
        [
            Self::Read,
            Self::Write,
            Self::Edit,
            PLATFORM_SHELL,
            Self::Grep,
            Self::Find,
            Self::Ls,
            Self::WebFetch,
        ]
    }
}

impl fmt::Display for ToolName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for ToolName {
    type Err = UnknownToolName;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::all()
            .into_iter()
            .find(|tool| tool.as_str() == value)
            .ok_or_else(|| UnknownToolName {
                name: value.to_owned(),
            })
    }
}

/// Error returned when text does not name a built-in tool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownToolName {
    name: String,
}

impl UnknownToolName {
    /// Returns the text that failed to parse.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
}

impl fmt::Display for UnknownToolName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "unknown tool name: {}", self.name)
    }
}

impl std::error::Error for UnknownToolName {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_names_are_wire_stable() {
        assert_eq!(ToolName::Read.as_str(), "read");
        assert_eq!(ToolName::Write.as_str(), "write");
        assert_eq!(ToolName::Edit.as_str(), "edit");
        assert_eq!(ToolName::Bash.as_str(), "bash");
        assert_eq!(ToolName::PowerShell.as_str(), "powershell");
        assert_eq!(ToolName::Grep.as_str(), "grep");
        assert_eq!(ToolName::Find.as_str(), "find");
        assert_eq!(ToolName::Ls.as_str(), "ls");
        assert_eq!(ToolName::WebFetch.as_str(), "web_fetch");
        assert_eq!(ToolName::all().len(), 8);
    }

    #[test]
    fn the_platform_shell_matches_the_operating_system() {
        let shell = ToolName::all()[3];
        if cfg!(windows) {
            assert_eq!(shell, ToolName::PowerShell);
        } else {
            assert_eq!(shell, ToolName::Bash);
        }
    }

    #[test]
    fn display_matches_the_wire_name() {
        assert_eq!(ToolName::Bash.to_string(), "bash");
        assert_eq!(ToolName::PowerShell.to_string(), "powershell");
    }

    #[test]
    fn every_name_round_trips() {
        for tool in ToolName::all() {
            assert_eq!(tool.as_str().parse::<ToolName>(), Ok(tool));
        }
    }

    #[test]
    fn unknown_names_report_the_original_text() {
        let error = "cmd".parse::<ToolName>().unwrap_err();
        assert_eq!(error.name(), "cmd");
        assert_eq!(error.to_string(), "unknown tool name: cmd");
    }

    #[test]
    fn the_non_platform_shell_is_not_registered() {
        let other = if cfg!(windows) {
            ToolName::Bash
        } else {
            ToolName::PowerShell
        };
        assert!(other.as_str().parse::<ToolName>().is_err());
    }

    #[test]
    fn names_are_case_sensitive() {
        assert!("Bash".parse::<ToolName>().is_err());
    }
}
