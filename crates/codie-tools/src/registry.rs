//! Tool registration and permission policy wrappers.

use std::{collections::HashSet, fmt, sync::Arc};

use codie_core::{Tool, ToolDecl, Toolbox};

use crate::{
    bash::BashTool, edit::EditTool, find::FindTool, grep::GrepTool, ls::LsTool, read::ReadTool,
    validate::ValidatedTool, web_fetch::WebFetchTool, write::WriteTool,
};

/// A collection of tools addressed by their wire name.
#[derive(Clone, Default)]
pub struct Registry {
    tools: Vec<Arc<dyn Tool>>,
}

impl Registry {
    /// Creates a registry from an explicit list of tools.
    ///
    /// Every tool is wrapped so arguments are validated against its declared
    /// schema before execution (`TOOL-002`).
    #[must_use]
    pub fn new(tools: Vec<Arc<dyn Tool>>) -> Self {
        let tools = tools
            .into_iter()
            .map(|tool| Arc::new(ValidatedTool::new(tool)) as Arc<dyn Tool>)
            .collect();
        Self { tools }
    }

    /// Creates a registry containing every built-in tool.
    #[must_use]
    pub fn builtin() -> Self {
        Self::new(vec![
            Arc::new(ReadTool),
            Arc::new(WriteTool),
            Arc::new(EditTool),
            Arc::new(BashTool),
            Arc::new(GrepTool),
            Arc::new(FindTool),
            Arc::new(LsTool),
            Arc::new(WebFetchTool::new()),
        ])
    }

    /// Adds a tool to the registry.
    pub fn register(&mut self, tool: Arc<dyn Tool>) {
        self.tools.push(Arc::new(ValidatedTool::new(tool)));
    }
}

impl fmt::Debug for Registry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Registry")
            .field(
                "tools",
                &self
                    .tools
                    .iter()
                    .map(|tool| tool.name())
                    .collect::<Vec<_>>(),
            )
            .finish()
    }
}

impl Toolbox for Registry {
    fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools
            .iter()
            .find(|tool| tool.name() == name)
            .map(Arc::clone)
    }

    fn declarations(&self) -> Vec<ToolDecl> {
        self.tools
            .iter()
            .map(|tool| ToolDecl {
                name: tool.name().to_owned(),
                description: tool.description().to_owned(),
                parameters: tool.schema(),
            })
            .collect()
    }
}

/// Allow/deny lists for tool calls.
///
/// Read-only tools are never denied by this policy. Mutating tools are allowed
/// only when they appear in `allow` and are not in `deny`. An explicit deny
/// always wins.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Policy {
    /// Tool names that are permitted.
    pub allow: HashSet<String>,
    /// Tool names that are denied even when also present in [`Self::allow`].
    pub deny: HashSet<String>,
}

impl Policy {
    /// Creates an empty policy: read-only tools are allowed, mutating tools are not.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a headless policy that permits only the named mutating tools.
    ///
    /// `read` (and other read-only tools) is always permitted; `write`, `edit`,
    /// and `bash` require their name to appear in `allowed`.
    #[must_use]
    pub fn headless<I, S>(allowed: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            allow: allowed.into_iter().map(Into::into).collect(),
            deny: HashSet::new(),
        }
    }

    /// Adds a tool name to the allow list.
    #[must_use]
    pub fn with_allowed<I, S>(mut self, allowed: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.allow.extend(allowed.into_iter().map(Into::into));
        self
    }

    /// Adds a tool name to the deny list.
    #[must_use]
    pub fn with_denied<I, S>(mut self, denied: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.deny.extend(denied.into_iter().map(Into::into));
        self
    }

    /// Returns the denial reason for `name`, or `None` when the call is allowed.
    #[must_use]
    pub fn denial(&self, name: &str) -> Option<String> {
        if is_read_only(name) {
            return None;
        }
        if self.deny.contains(name) {
            return Some(format!("Tool '{name}' is denied by policy."));
        }
        if self.allow.contains(name) {
            return None;
        }
        Some(format!("Tool '{name}' is not allowed by policy."))
    }
}

/// Returns true for tools that do not mutate files or run commands.
#[must_use]
pub fn is_read_only(name: &str) -> bool {
    matches!(name, "read" | "grep" | "find" | "ls")
}

/// Wraps a toolbox and applies a [`Policy`] before tool execution.
pub struct PolicyToolbox {
    inner: Arc<dyn Toolbox>,
    policy: Policy,
}

impl PolicyToolbox {
    /// Creates a policy wrapper around `inner`.
    #[must_use]
    pub fn new(inner: Arc<dyn Toolbox>, policy: Policy) -> Self {
        Self { inner, policy }
    }

    /// Returns the wrapped toolbox.
    #[must_use]
    pub fn inner(&self) -> &Arc<dyn Toolbox> {
        &self.inner
    }

    /// Returns the active policy.
    #[must_use]
    pub fn policy(&self) -> &Policy {
        &self.policy
    }
}

impl fmt::Debug for PolicyToolbox {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PolicyToolbox")
            .field("policy", &self.policy)
            .finish_non_exhaustive()
    }
}

impl Toolbox for PolicyToolbox {
    fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.inner.get(name)
    }

    fn declarations(&self) -> Vec<ToolDecl> {
        self.inner.declarations()
    }

    fn denial(&self, name: &str) -> Option<String> {
        if is_read_only(name) {
            return None;
        }
        self.policy.denial(name).or_else(|| self.inner.denial(name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codie_testkit::spec;

    #[spec("TOOL-001")]
    #[test]
    fn registry_declares_builtins_in_order() {
        let registry = Registry::builtin();
        let names: Vec<String> = registry
            .declarations()
            .into_iter()
            .map(|declaration| declaration.name)
            .collect();
        assert_eq!(
            names,
            [
                "read",
                "write",
                "edit",
                "bash",
                "grep",
                "find",
                "ls",
                "web_fetch"
            ]
        );
        assert!(registry.get("read").is_some());
        assert!(registry.get("missing").is_none());
    }

    #[spec("SEC-001")]
    #[test]
    fn read_is_always_allowed() {
        let policy = Policy::default();
        assert!(policy.denial("read").is_none());
        assert!(policy.denial("grep").is_none());
    }

    #[test]
    fn headless_denies_mutations_unless_allowed() {
        let policy = Policy::headless(["edit"]);
        assert!(policy.denial("read").is_none());
        assert!(policy.denial("edit").is_none());
        assert!(policy.denial("write").is_some());
        assert!(policy.denial("bash").is_some());
    }

    #[spec("SEC-002")]
    #[test]
    fn explicit_deny_wins() {
        let policy = Policy::headless(["write"]).with_denied(["write"]);
        assert!(policy.denial("write").is_some());
    }

    #[test]
    fn policy_toolbox_delegates_lookup_and_declarations() {
        let toolbox = PolicyToolbox::new(Arc::new(Registry::builtin()), Policy::headless(["bash"]));
        assert!(toolbox.get("write").is_some());
        assert_eq!(toolbox.declarations().len(), 8);
        assert!(toolbox.denial("write").is_some());
        assert!(toolbox.denial("bash").is_none());
        assert!(toolbox.denial("read").is_none());
    }
}
