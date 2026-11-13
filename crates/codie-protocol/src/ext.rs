//! Wire types for out-of-process extensions (`codie-ext`).
//!
//! Extensions are separate processes that speak line-delimited JSON-RPC 2.0
//! over stdio. The types here define the manifest, the initialize handshake,
//! every hook payload, and the tool/command/render results an extension may
//! return. The core never loads foreign code into its address space.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Version of the extension protocol implemented by this build.
pub const EXT_PROTOCOL_VERSION: u32 = 1;

/// Filename of an extension manifest.
pub const EXT_MANIFEST_FILE: &str = "codie.toml";

/// Directories searched for user and project extensions.
pub const EXTENSIONS_DIR: &str = "extensions";

/// Maximum size of one manifest file.
pub const MAX_MANIFEST_BYTES: u64 = 64 * 1024;

/// Maximum size of one JSON-RPC line.
pub const MAX_RPC_LINE_BYTES: usize = 1024 * 1024;

/// Maximum number of extensions loaded in one session.
pub const MAX_EXTENSIONS: usize = 32;

/// Maximum number of tools or commands declared by one extension.
pub const MAX_DECLARATIONS: usize = 64;

/// Maximum model-visible text returned by one extension tool.
pub const MAX_TOOL_RESULT_BYTES: usize = 1024 * 1024;

/// Default hook timeout in milliseconds.
pub const DEFAULT_HOOK_TIMEOUT_MS: u64 = 2_000;

/// Default tool timeout in milliseconds.
pub const DEFAULT_TOOL_TIMEOUT_MS: u64 = 120_000;

/// Absolute maximum hook timeout an extension may request.
pub const MAX_HOOK_TIMEOUT_MS: u64 = 30_000;

/// Absolute maximum tool timeout an extension may request.
pub const MAX_TOOL_TIMEOUT_MS: u64 = 600_000;

/// A capability an extension may request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum ExtCapability {
    /// The extension wants hook calls.
    Hooks,
    /// The extension declares model-visible tools.
    Tools,
    /// The extension declares slash commands.
    Commands,
    /// The extension returns structured render hints.
    Renderers,
    /// The extension declares command-line flags.
    Flags,
}

impl ExtCapability {
    /// Returns the wire name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Hooks => "hooks",
            Self::Tools => "tools",
            Self::Commands => "commands",
            Self::Renderers => "renderers",
            Self::Flags => "flags",
        }
    }
}

/// One hook point supported by the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum ExtHook {
    /// Runs once when a session is opened.
    SessionStart,
    /// Runs once when a session is closed.
    SessionShutdown,
    /// Runs before an agent run starts.
    BeforeAgentStart,
    /// Runs before every provider request.
    Context,
    /// Runs before a tool call executes.
    BeforeToolCall,
    /// Runs after a tool call settles.
    AfterToolCall,
    /// Runs after a turn ends.
    TurnEnd,
    /// Runs when the user submits input.
    UserSubmit,
}

impl ExtHook {
    /// Returns the wire name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SessionStart => "session_start",
            Self::SessionShutdown => "session_shutdown",
            Self::BeforeAgentStart => "before_agent_start",
            Self::Context => "context",
            Self::BeforeToolCall => "before_tool_call",
            Self::AfterToolCall => "after_tool_call",
            Self::TurnEnd => "turn_end",
            Self::UserSubmit => "user_submit",
        }
    }
}

/// How an extension tool may be scheduled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum ExtExecutionMode {
    /// May run concurrently with other parallel tools.
    #[default]
    Parallel,
    /// Forces the whole batch to run sequentially.
    Sequential,
}

/// Extension-declared timeout limits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ExtLimits {
    /// Hook timeout in milliseconds.
    #[serde(default = "default_hook_timeout")]
    pub hook_timeout_ms: u64,
    /// Tool timeout in milliseconds.
    #[serde(default = "default_tool_timeout")]
    pub tool_timeout_ms: u64,
}

impl Default for ExtLimits {
    fn default() -> Self {
        Self {
            hook_timeout_ms: DEFAULT_HOOK_TIMEOUT_MS,
            tool_timeout_ms: DEFAULT_TOOL_TIMEOUT_MS,
        }
    }
}

const fn default_hook_timeout() -> u64 {
    DEFAULT_HOOK_TIMEOUT_MS
}

const fn default_tool_timeout() -> u64 {
    DEFAULT_TOOL_TIMEOUT_MS
}

/// An extension manifest (`codie.toml`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ExtManifest {
    /// Stable extension name.
    pub name: String,
    /// Extension version.
    pub version: String,
    /// Protocol version the extension implements.
    pub protocol: u32,
    /// One-line description.
    #[serde(default)]
    pub description: String,
    /// Executable and arguments, relative to the manifest directory.
    pub entrypoint: Vec<String>,
    /// Capabilities the extension intends to use.
    #[serde(default)]
    pub capabilities: Vec<ExtCapability>,
    /// Timeout overrides.
    #[serde(default)]
    pub limits: ExtLimits,
}

/// One tool declaration from an extension.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ExtToolDecl {
    /// Wire name used by the model.
    pub name: String,
    /// One-paragraph description.
    pub description: String,
    /// JSON Schema for the arguments object.
    pub parameters: Value,
    /// Scheduling mode.
    #[serde(default)]
    pub execution: ExtExecutionMode,
}

/// One slash-command declaration from an extension.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ExtCommandDecl {
    /// Command name without a leading slash.
    pub name: String,
    /// One-line description.
    #[serde(default)]
    pub description: String,
    /// Usage hint such as `<path>`.
    #[serde(default)]
    pub usage: String,
}

/// One command-line flag declaration from an extension.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ExtFlagDecl {
    /// Flag name without leading dashes.
    pub name: String,
    /// Help text.
    #[serde(default)]
    pub description: String,
    /// True when the flag requires `=VALUE`.
    #[serde(default)]
    pub takes_value: bool,
}

/// Initialize request parameters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ExtInitializeParams {
    /// Protocol version implemented by the host.
    pub protocol: u32,
    /// Host version.
    pub client_version: String,
    /// Absolute working directory.
    pub cwd: String,
    /// Session identifier, when the session is persisted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Deferred flags claimed by this extension, keyed by name.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub flags: BTreeMap<String, String>,
    /// Capabilities the host supports.
    pub host_capabilities: Vec<ExtCapability>,
}

/// Initialize response payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ExtInitializeResult {
    /// Protocol version implemented by the extension.
    pub protocol: u32,
    /// Extension name.
    pub name: String,
    /// Extension version.
    pub version: String,
    /// Capabilities the extension will use.
    #[serde(default)]
    pub capabilities: Vec<ExtCapability>,
    /// Hook points the extension handles.
    #[serde(default)]
    pub hooks: Vec<ExtHook>,
    /// Model-visible tools.
    #[serde(default)]
    pub tools: Vec<ExtToolDecl>,
    /// Slash commands.
    #[serde(default)]
    pub commands: Vec<ExtCommandDecl>,
    /// Command-line flags.
    #[serde(default)]
    pub flags: Vec<ExtFlagDecl>,
    /// Structured renderer kinds, such as `text`, `markdown`, `code`, or `diff`.
    #[serde(default)]
    pub renderers: Vec<String>,
}

/// Session identity passed to lifecycle hooks.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ExtSessionInfo {
    /// Session identifier, when persisted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Absolute working directory.
    pub cwd: String,
    /// Active model identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

/// A compact transcript message passed to context hooks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ExtMessage {
    /// One of `system`, `user`, `assistant`, or `tool_result`.
    pub role: String,
    /// Text content, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Tool name for tool-result messages.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    /// Tool-call identifier for tool-result messages.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

/// A tool call passed to tool hooks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ExtToolCall {
    /// Provider-assigned identifier.
    pub id: String,
    /// Tool name.
    pub name: String,
    /// Parsed arguments.
    pub arguments: Value,
}

/// Parameters for lifecycle and turn hooks.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ExtHookParams {
    /// Session identity.
    pub session: ExtSessionInfo,
    /// Optional user or assistant text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Compact transcript for `context`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub messages: Vec<ExtMessage>,
    /// Tool call for `before_tool_call` and `after_tool_call`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call: Option<ExtToolCall>,
    /// Tool result for `after_tool_call` and `turn_end`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<ExtToolResult>,
    /// Final assistant text for `turn_end`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assistant_text: Option<String>,
    /// Run status for `turn_end`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// Shutdown reason.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Result returned by a hook request.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ExtHookResult {
    /// True when a hook wants to block an action.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub block: bool,
    /// Human-readable block reason.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Extra context injected for the current request only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    /// Replacement user prompt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    /// Replacement tool result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<ExtToolResult>,
}

/// One content block returned by an extension tool.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ExtContent {
    /// Model-visible text.
    Text {
        /// Text payload.
        text: String,
    },
    /// A base64-encoded image.
    Image {
        /// Base64 data.
        data: String,
        /// MIME type.
        mime_type: String,
    },
}

impl ExtContent {
    /// Creates a text block.
    #[must_use]
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text { text: text.into() }
    }
}

/// A structured render hint. ANSI escape sequences are not permitted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum ExtRenderKind {
    /// Plain text.
    Text,
    /// Markdown.
    Markdown,
    /// Fenced code.
    Code,
    /// Unified diff.
    Diff,
}

/// Structured output for a host renderer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ExtRenderHint {
    /// Renderer kind.
    pub kind: ExtRenderKind,
    /// Optional title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Renderer body.
    pub text: String,
}

/// Result of one extension tool call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ExtToolResult {
    /// Model-visible content.
    #[serde(default)]
    pub content: Vec<ExtContent>,
    /// Host-only structured details.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
    /// True when the tool failed.
    #[serde(default)]
    pub is_error: bool,
    /// Requests early termination of the batch.
    #[serde(default)]
    pub terminate: bool,
    /// Optional structured render hint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub render: Option<ExtRenderHint>,
}

impl ExtToolResult {
    /// Creates a successful text result.
    #[must_use]
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            content: vec![ExtContent::text(text)],
            details: None,
            is_error: false,
            terminate: false,
            render: None,
        }
    }

    /// Creates an error text result.
    #[must_use]
    pub fn error(text: impl Into<String>) -> Self {
        Self {
            content: vec![ExtContent::text(text)],
            details: None,
            is_error: true,
            terminate: false,
            render: None,
        }
    }
}

/// Parameters for `tool.call`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ExtToolCallParams {
    /// Session identity.
    pub session: ExtSessionInfo,
    /// Call identifier.
    pub call_id: String,
    /// Tool name.
    pub name: String,
    /// Parsed arguments.
    pub arguments: Value,
}

/// Parameters for `command.invoke`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ExtCommandParams {
    /// Session identity.
    pub session: ExtSessionInfo,
    /// Command name.
    pub name: String,
    /// Raw argument string.
    #[serde(default)]
    pub arguments: String,
}

/// Result of one extension command.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ExtCommandResult {
    /// Prompt text to submit as the next user message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    /// Structured output to show without submitting a prompt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<ExtRenderHint>,
}

/// Parameters for an extension log notification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ExtLogParams {
    /// One of `debug`, `info`, `warn`, or `error`.
    pub level: String,
    /// Human-readable message.
    pub message: String,
}

/// A JSON-RPC request sent by the host.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ExtRequest {
    /// Always `2.0`.
    pub jsonrpc: String,
    /// Request identifier.
    pub id: u64,
    /// Method name.
    pub method: String,
    /// Method parameters.
    #[serde(default)]
    pub params: Value,
}

impl ExtRequest {
    /// Creates a request with the JSON-RPC version set.
    #[must_use]
    pub fn new(id: u64, method: impl Into<String>, params: Value) -> Self {
        Self {
            jsonrpc: "2.0".to_owned(),
            id,
            method: method.into(),
            params,
        }
    }
}

/// A JSON-RPC notification sent by the host.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ExtNotification {
    /// Always `2.0`.
    pub jsonrpc: String,
    /// Method name.
    pub method: String,
    /// Method parameters.
    #[serde(default)]
    pub params: Value,
}

impl ExtNotification {
    /// Creates a notification with the JSON-RPC version set.
    #[must_use]
    pub fn new(method: impl Into<String>, params: Value) -> Self {
        Self {
            jsonrpc: "2.0".to_owned(),
            method: method.into(),
            params,
        }
    }
}

/// A JSON-RPC error object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ExtRpcError {
    /// Numeric JSON-RPC error code.
    pub code: i64,
    /// Human-readable message.
    pub message: String,
}

/// A JSON-RPC response sent by an extension.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ExtResponse {
    /// Always `2.0`.
    pub jsonrpc: String,
    /// Request identifier being answered.
    pub id: u64,
    /// Success payload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    /// Failure payload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ExtRpcError>,
}

/// Any JSON-RPC value an extension may write to stdout.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(untagged)]
pub enum ExtServerRecord {
    /// A response to a host request.
    Response(ExtResponse),
    /// An unsolicited notification.
    Notification(ExtNotification),
}

/// Returns the published extension protocol schemas.
#[cfg(feature = "schema")]
#[must_use]
pub fn schema_documents() -> Vec<(&'static str, Value)> {
    vec![
        (
            "ext-manifest.schema.json",
            serde_json::to_value(schemars::schema_for!(ExtManifest))
                .expect("extension manifest schema serializes"),
        ),
        (
            "ext-protocol.schema.json",
            serde_json::to_value(schemars::schema_for!(ExtServerRecord))
                .expect("extension protocol schema serializes"),
        ),
        (
            "ext-hooks.schema.json",
            serde_json::to_value(schemars::schema_for!(ExtHookResult))
                .expect("extension hook schema serializes"),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use codie_testkit::spec;

    #[test]
    fn manifests_round_trip() {
        let manifest = ExtManifest {
            name: "todo".to_owned(),
            version: "0.1.0".to_owned(),
            protocol: EXT_PROTOCOL_VERSION,
            description: "todos".to_owned(),
            entrypoint: vec!["python3".to_owned(), "todo.py".to_owned()],
            capabilities: vec![ExtCapability::Tools, ExtCapability::Commands],
            limits: ExtLimits::default(),
        };
        let text = toml::to_string(&manifest).expect("manifest encodes");
        let decoded: ExtManifest = toml::from_str(&text).expect("manifest decodes");
        assert_eq!(decoded, manifest);
    }

    #[test]
    #[spec("EXT-009")]
    fn spec_ext_009_jsonrpc_framing() {
        let request = ExtRequest::new(7, "initialize", serde_json::json!({}));
        let value = serde_json::to_value(&request).expect("request");
        assert_eq!(value["jsonrpc"], "2.0");
        assert_eq!(value["id"], 7);
        assert_eq!(value["method"], "initialize");
    }

    #[test]
    #[spec("EXT-002")]
    fn hook_names_are_stable() {
        let names = [
            ExtHook::SessionStart,
            ExtHook::SessionShutdown,
            ExtHook::BeforeAgentStart,
            ExtHook::Context,
            ExtHook::BeforeToolCall,
            ExtHook::AfterToolCall,
            ExtHook::TurnEnd,
            ExtHook::UserSubmit,
        ]
        .map(ExtHook::as_str);
        assert_eq!(
            names,
            [
                "session_start",
                "session_shutdown",
                "before_agent_start",
                "context",
                "before_tool_call",
                "after_tool_call",
                "turn_end",
                "user_submit"
            ]
        );
    }

    #[test]
    fn tool_results_can_carry_a_render_hint() {
        let mut result = ExtToolResult::text("ok");
        result.render = Some(ExtRenderHint {
            kind: ExtRenderKind::Markdown,
            title: Some("Todo".to_owned()),
            text: "- one".to_owned(),
        });
        let value = serde_json::to_value(&result).expect("result");
        assert_eq!(value["render"]["kind"], "markdown");
        assert_eq!(value["content"][0]["type"], "text");
    }
}
