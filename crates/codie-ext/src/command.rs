//! Extension slash-command declarations and invocation.

use codie_protocol::ext::ExtCommandDecl;

/// One command declared by an extension.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionCommand {
    /// Extension name.
    pub extension: String,
    /// Command declaration.
    pub declaration: ExtCommandDecl,
}

/// A resolved invocation of an extension command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandInvocation {
    /// Extension name.
    pub extension: String,
    /// Command name.
    pub command: String,
    /// Raw argument string.
    pub arguments: String,
}
