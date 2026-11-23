//! RPC protocol types for editor integrations.
//!
//! The protocol is line-delimited JSON over stdin/stdout, versioned by
//! [`RPC_PROTOCOL_VERSION`]. Commands are parsed with [`RpcRequest`] and the
//! server emits [`RpcReady`], [`RpcSessionHeader`], [`RpcResponse`], agent
//! events, and a final [`RpcSettled`] record.
//!
//! JSON schemas for commands and server records are generated from these types
//! by `cargo xtask schemas` when the `schema` feature is enabled.

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub mod ext;

/// Version of the RPC protocol implemented by this build.
///
/// Version 2 adds interactive permissions, credentials, catalog/config reads,
/// and prompt attachments. A client that never sends the new commands sees the
/// version 1 behavior, including the headless permission policy.
pub const RPC_PROTOCOL_VERSION: u32 = 2;

fn is_false(value: &bool) -> bool {
    !*value
}

/// Optional server capabilities advertised in [`RpcReady`].
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum RpcFeature {
    /// `permission_request` records and `permission_response` commands.
    Permissions,
    /// `auth_status`, `auth_set_key`, and `auth_logout` commands.
    Auth,
    /// `get_catalog` command.
    Catalog,
    /// `get_config` and `set_config` commands.
    Config,
    /// `prompt` attachments.
    Attachments,
}

impl RpcFeature {
    /// Every feature this build supports, in documentation order.
    #[must_use]
    pub const fn all() -> [Self; 5] {
        [
            Self::Permissions,
            Self::Auth,
            Self::Catalog,
            Self::Config,
            Self::Attachments,
        ]
    }
}

/// How the server treats mutating tool calls.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum PermissionMode {
    /// Version 1 behavior: mutating tools are denied unless listed in `--allow`.
    #[default]
    Headless,
    /// Ask the client before each mutating call that is not pre-approved.
    Ask,
    /// Allow every mutating call without asking.
    Allow,
}

impl PermissionMode {
    /// Returns the wire spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Headless => "headless",
            Self::Ask => "ask",
            Self::Allow => "allow",
        }
    }
}

/// A client answer to a [`RpcPermissionRequest`].
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum PermissionDecision {
    /// Allow this call only.
    AllowOnce,
    /// Allow this tool for the rest of the session.
    AllowAlways,
    /// Deny the call.
    Deny,
}

impl PermissionDecision {
    /// Returns the wire spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AllowOnce => "allow_once",
            Self::AllowAlways => "allow_always",
            Self::Deny => "deny",
        }
    }
}

/// One explicit attachment in a `prompt` command.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RpcAttachment {
    /// A path relative to the session working directory.
    Path {
        /// File path to read and attach.
        path: String,
    },
    /// A base64-encoded image supplied by the client.
    Image {
        /// Base64-encoded image bytes.
        data: String,
        /// MIME type such as `image/png`.
        mime_type: String,
        /// Optional display name for prompts and logs.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
}

/// One command read from stdin.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct RpcRequest {
    /// Optional client identifier echoed in the response.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// The command to execute.
    #[serde(flatten)]
    pub command: RpcCommand,
}

impl RpcRequest {
    /// Returns the command name used in responses.
    #[must_use]
    pub fn command_name(&self) -> &'static str {
        self.command.name()
    }
}

/// Commands accepted by `codie --mode rpc`.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum RpcCommand {
    /// Start a run with a user message.
    Prompt {
        /// Prompt text. `@path` attachments are expanded.
        message: String,
        /// Explicit attachments appended to the message.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        attachments: Vec<RpcAttachment>,
    },
    /// Steer the active run with an extra message.
    Steer {
        /// Steering text.
        message: String,
    },
    /// Queue a message for after the active run settles.
    FollowUp {
        /// Follow-up text.
        message: String,
    },
    /// Cancel the active run.
    Abort,
    /// Change how mutating tools are authorized.
    SetPermissionMode {
        /// One of `headless`, `ask`, or `allow`.
        mode: PermissionMode,
    },
    /// Answer a pending `permission_request`.
    PermissionResponse {
        /// Identifier from the `permission_request` record.
        request_id: String,
        /// Client decision.
        decision: PermissionDecision,
    },
    /// Report the current model, thinking level, and session state.
    GetState,
    /// List selectable models and provider details.
    GetCatalog,
    /// List the built-in theme palettes as semantic hex colors.
    GetThemes,
    /// Report effective configuration without secrets.
    GetConfig,
    /// Write one value to `config.toml`.
    SetConfig {
        /// Key such as `provider`, `model`, `thinking`, `max-turns`, `allow`,
        /// or `base-url`.
        key: String,
        /// New value; `allow` accepts a comma-separated list.
        value: String,
    },
    /// Report credential state for every provider.
    AuthStatus,
    /// Store an API key for one provider.
    AuthSetKey {
        /// Provider identifier such as `deepseek`, `openrouter`, or `gemini`.
        provider: String,
        /// API key. Never echoed in responses or logs.
        key: String,
        /// Contact the provider to verify the key before storing it.
        #[serde(default, skip_serializing_if = "is_false")]
        verify: bool,
    },
    /// Remove a stored API key.
    AuthLogout {
        /// Provider identifier.
        provider: String,
    },
    /// Return the projected transcript messages.
    GetMessages,
    /// Return the session entry tree.
    GetEntries,
    /// Return token, cost, and context statistics.
    GetSessionStats,
    /// Start a new session for the current directory.
    NewSession,
    /// Change the model for subsequent runs.
    SetModel {
        /// Model identifier, used verbatim.
        model: String,
        /// Optional provider override; defaults to the current provider.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider: Option<String>,
    },
    /// Change the reasoning effort for subsequent runs.
    SetThinking {
        /// Thinking level: off, minimal, low, medium, high, xhigh, or max.
        level: String,
    },
    /// Compact the context once.
    Compact {
        /// Optional custom summarization instructions.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        instructions: Option<String>,
    },
    /// Export the current session as a self-contained HTML file.
    ExportHtml {
        /// Destination path. Defaults to `codie-session-<id>.html` in the cwd.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
    },
    /// Respond, then stop reading commands.
    Shutdown,
}

impl RpcCommand {
    /// Returns the wire name of this command.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Prompt { .. } => "prompt",
            Self::Steer { .. } => "steer",
            Self::FollowUp { .. } => "follow_up",
            Self::Abort => "abort",
            Self::SetPermissionMode { .. } => "set_permission_mode",
            Self::PermissionResponse { .. } => "permission_response",
            Self::GetState => "get_state",
            Self::GetCatalog => "get_catalog",
            Self::GetThemes => "get_themes",
            Self::GetConfig => "get_config",
            Self::SetConfig { .. } => "set_config",
            Self::AuthStatus => "auth_status",
            Self::AuthSetKey { .. } => "auth_set_key",
            Self::AuthLogout { .. } => "auth_logout",
            Self::GetMessages => "get_messages",
            Self::GetEntries => "get_entries",
            Self::GetSessionStats => "get_session_stats",
            Self::NewSession => "new_session",
            Self::SetModel { .. } => "set_model",
            Self::SetThinking { .. } => "set_thinking",
            Self::Compact { .. } => "compact",
            Self::ExportHtml { .. } => "export_html",
            Self::Shutdown => "shutdown",
        }
    }
}

/// First record emitted by the server, before any session header.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct RpcReady {
    /// Discriminator; always `rpc_ready`.
    #[serde(rename = "type")]
    pub record_type: ReadyType,
    /// Protocol version.
    pub version: u32,
    /// Optional capabilities this server implements. Absent in version 1.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub features: Vec<RpcFeature>,
}

/// The only value of [`RpcReady::record_type`].
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum ReadyType {
    /// `rpc_ready`
    #[serde(rename = "rpc_ready")]
    Ready,
}

impl RpcReady {
    /// Creates the ready record for this build.
    #[must_use]
    pub fn new() -> Self {
        Self {
            record_type: ReadyType::Ready,
            version: RPC_PROTOCOL_VERSION,
            features: RpcFeature::all().to_vec(),
        }
    }
}

impl Default for RpcReady {
    fn default() -> Self {
        Self::new()
    }
}

/// Session header emitted after the ready record and after `new_session`.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct RpcSessionHeader {
    /// Discriminator; always `session`.
    #[serde(rename = "type")]
    pub record_type: SessionType,
    /// Session format version.
    pub version: u32,
    /// Session identifier.
    pub id: String,
    /// Creation time as an RFC 3339 string.
    pub timestamp: String,
    /// Absolute working directory.
    pub cwd: String,
    /// Parent session identifier when forked or cloned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session: Option<String>,
}

/// The only value of [`RpcSessionHeader::record_type`].
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum SessionType {
    /// `session`
    #[serde(rename = "session")]
    Session,
}

/// A mutating tool call waiting for a client decision.
///
/// Emitted only after the client sends [`RpcCommand::SetPermissionMode`] with
/// `ask` or `allow`; version 1 clients never receive it.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct RpcPermissionRequest {
    /// Discriminator; always `permission_request`.
    #[serde(rename = "type")]
    pub record_type: PermissionRequestType,
    /// Protocol version.
    pub version: u32,
    /// Identifier to send back in [`RpcCommand::PermissionResponse`].
    pub id: String,
    /// Identifier of the tool call being authorized.
    pub tool_call_id: String,
    /// Tool name, for example `edit` or `bash`.
    pub tool: String,
    /// Parsed tool arguments.
    pub arguments: Value,
}

/// The only value of [`RpcPermissionRequest::record_type`].
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum PermissionRequestType {
    /// `permission_request`
    #[serde(rename = "permission_request")]
    PermissionRequest,
}

impl RpcPermissionRequest {
    /// Creates a permission request for the current protocol version.
    #[must_use]
    pub fn new(
        id: impl Into<String>,
        tool_call_id: impl Into<String>,
        tool: impl Into<String>,
        arguments: Value,
    ) -> Self {
        Self {
            record_type: PermissionRequestType::PermissionRequest,
            version: RPC_PROTOCOL_VERSION,
            id: id.into(),
            tool_call_id: tool_call_id.into(),
            tool: tool.into(),
            arguments,
        }
    }
}

/// Response to one command.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct RpcResponse {
    /// Discriminator; always `response`.
    #[serde(rename = "type")]
    pub record_type: ResponseType,
    /// Protocol version.
    pub version: u32,
    /// Client identifier echoed from the request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Command name.
    pub command: String,
    /// True when the command succeeded.
    pub success: bool,
    /// Command payload on success.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    /// Failure details.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

/// The only value of [`RpcResponse::record_type`].
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum ResponseType {
    /// `response`
    #[serde(rename = "response")]
    Response,
}

impl RpcResponse {
    /// Creates a successful response.
    #[must_use]
    pub fn success(id: Option<String>, command: impl Into<String>, data: Value) -> Self {
        Self {
            record_type: ResponseType::Response,
            version: RPC_PROTOCOL_VERSION,
            id,
            command: command.into(),
            success: true,
            data: Some(data),
            error: None,
        }
    }

    /// Creates a failed response.
    #[must_use]
    pub fn failure(
        id: Option<String>,
        command: impl Into<String>,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            record_type: ResponseType::Response,
            version: RPC_PROTOCOL_VERSION,
            id,
            command: command.into(),
            success: false,
            data: None,
            error: Some(RpcError {
                code: code.into(),
                message: message.into(),
            }),
        }
    }
}

/// Error payload carried by a failed [`RpcResponse`].
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct RpcError {
    /// Stable machine-readable code.
    pub code: String,
    /// Human-readable explanation.
    pub message: String,
}

/// Completion signal for one run.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct RpcSettled {
    /// Discriminator; always `agent_settled`.
    #[serde(rename = "type")]
    pub record_type: SettledType,
    /// True when the run was aborted.
    pub aborted: bool,
}

/// The only value of [`RpcSettled::record_type`].
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum SettledType {
    /// `agent_settled`
    #[serde(rename = "agent_settled")]
    Settled,
}

impl RpcSettled {
    /// Creates the settled record.
    #[must_use]
    pub const fn new(aborted: bool) -> Self {
        Self {
            record_type: SettledType::Settled,
            aborted,
        }
    }
}

/// Any record the server may write to stdout.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(untagged)]
pub enum RpcServerRecord {
    /// Version handshake.
    Ready(RpcReady),
    /// Session header.
    Session(RpcSessionHeader),
    /// Permission request.
    Permission(RpcPermissionRequest),
    /// Command response.
    Response(RpcResponse),
    /// Completion signal.
    Settled(RpcSettled),
}

/// Returns the published schema documents as `(file name, JSON value)` pairs.
///
/// Schemas are generated from the Rust types above so the wire format and the
/// schema cannot drift (`PROTO-005`). `cargo xtask schemas` writes them to
/// `schemas/` and CI verifies the checked-in copies are current.
#[cfg(feature = "schema")]
#[must_use]
pub fn schema_documents() -> Vec<(&'static str, serde_json::Value)> {
    let mut documents = vec![
        (
            "rpc-commands.schema.json",
            serde_json::to_value(schemars::schema_for!(RpcRequest))
                .expect("RPC command schema serializes"),
        ),
        (
            "rpc-server.schema.json",
            serde_json::to_value(schemars::schema_for!(RpcServerRecord))
                .expect("RPC server schema serializes"),
        ),
    ];
    documents.extend(ext::schema_documents());
    documents
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn commands_round_trip_with_ids() {
        let request: RpcRequest =
            serde_json::from_str(r#"{"id":"1","command":"prompt","message":"hello"}"#)
                .expect("prompt parses");
        assert_eq!(request.id.as_deref(), Some("1"));
        assert_eq!(request.command_name(), "prompt");
        assert_eq!(
            request.command,
            RpcCommand::Prompt {
                message: "hello".to_owned(),
                attachments: Vec::new(),
            }
        );

        let encoded = serde_json::to_value(&request).expect("request encodes");
        assert_eq!(encoded["command"], "prompt");
        assert_eq!(encoded["message"], "hello");
        assert!(encoded.get("attachments").is_none());
    }

    #[test]
    fn v2_commands_round_trip() {
        let requests = [
            (
                json!({"command": "set_permission_mode", "mode": "ask"}),
                RpcCommand::SetPermissionMode {
                    mode: PermissionMode::Ask,
                },
            ),
            (
                json!({
                    "command": "permission_response",
                    "request_id": "p1",
                    "decision": "allow_always",
                }),
                RpcCommand::PermissionResponse {
                    request_id: "p1".to_owned(),
                    decision: PermissionDecision::AllowAlways,
                },
            ),
            (json!({"command": "get_catalog"}), RpcCommand::GetCatalog),
            (json!({"command": "get_themes"}), RpcCommand::GetThemes),
            (json!({"command": "get_config"}), RpcCommand::GetConfig),
            (
                json!({"command": "set_config", "key": "thinking", "value": "high"}),
                RpcCommand::SetConfig {
                    key: "thinking".to_owned(),
                    value: "high".to_owned(),
                },
            ),
            (json!({"command": "auth_status"}), RpcCommand::AuthStatus),
            (
                json!({"command": "auth_set_key", "provider": "deepseek", "key": "sk-test"}),
                RpcCommand::AuthSetKey {
                    provider: "deepseek".to_owned(),
                    key: "sk-test".to_owned(),
                    verify: false,
                },
            ),
            (
                json!({"command": "auth_logout", "provider": "deepseek"}),
                RpcCommand::AuthLogout {
                    provider: "deepseek".to_owned(),
                },
            ),
        ];

        for (value, expected) in requests {
            assert_eq!(expected.name(), value["command"].as_str().expect("name"));
            let parsed: RpcCommand = serde_json::from_value(value.clone()).expect("command parses");
            assert_eq!(parsed, expected);
            let encoded = serde_json::to_value(&parsed).expect("command encodes");
            assert_eq!(encoded, value);
        }
    }

    #[test]
    fn prompt_attachments_round_trip() {
        let request: RpcRequest = serde_json::from_value(json!({
            "command": "prompt",
            "message": "look",
            "attachments": [
                {"type": "path", "path": "src/main.rs"},
                {"type": "image", "data": "AAAA", "mime_type": "image/png", "name": "clip.png"}
            ]
        }))
        .expect("prompt parses");
        let RpcCommand::Prompt {
            message,
            attachments,
        } = request.command
        else {
            panic!("expected a prompt");
        };
        assert_eq!(message, "look");
        assert_eq!(attachments.len(), 2);
        assert!(matches!(
            &attachments[0],
            RpcAttachment::Path { path } if path == "src/main.rs"
        ));
        assert!(matches!(
            &attachments[1],
            RpcAttachment::Image { mime_type, name: Some(name), .. }
                if mime_type == "image/png" && name == "clip.png"
        ));
    }

    #[test]
    fn every_command_has_a_stable_name() {
        let commands = [
            (RpcCommand::Abort, "abort"),
            (
                RpcCommand::SetPermissionMode {
                    mode: PermissionMode::Headless,
                },
                "set_permission_mode",
            ),
            (
                RpcCommand::PermissionResponse {
                    request_id: "p".into(),
                    decision: PermissionDecision::Deny,
                },
                "permission_response",
            ),
            (RpcCommand::GetState, "get_state"),
            (RpcCommand::GetCatalog, "get_catalog"),
            (RpcCommand::GetThemes, "get_themes"),
            (RpcCommand::GetConfig, "get_config"),
            (
                RpcCommand::SetConfig {
                    key: "thinking".into(),
                    value: "low".into(),
                },
                "set_config",
            ),
            (RpcCommand::AuthStatus, "auth_status"),
            (
                RpcCommand::AuthSetKey {
                    provider: "deepseek".into(),
                    key: "sk".into(),
                    verify: false,
                },
                "auth_set_key",
            ),
            (
                RpcCommand::AuthLogout {
                    provider: "deepseek".into(),
                },
                "auth_logout",
            ),
            (RpcCommand::GetMessages, "get_messages"),
            (RpcCommand::GetEntries, "get_entries"),
            (RpcCommand::GetSessionStats, "get_session_stats"),
            (RpcCommand::NewSession, "new_session"),
            (RpcCommand::Shutdown, "shutdown"),
        ];
        for (command, name) in commands {
            assert_eq!(command.name(), name);
        }
    }

    #[test]
    fn responses_carry_version_and_errors() {
        let response = RpcResponse::failure(Some("7".into()), "abort", "no_active_run", "no run");
        let value = serde_json::to_value(&response).expect("response encodes");
        assert_eq!(value["type"], "response");
        assert_eq!(value["version"], RPC_PROTOCOL_VERSION);
        assert_eq!(value["id"], "7");
        assert_eq!(value["success"], false);
        assert_eq!(value["error"]["code"], "no_active_run");
    }

    #[test]
    fn settled_and_ready_records_use_the_documented_discriminators() {
        let ready = serde_json::to_value(RpcReady::new()).expect("ready");
        assert_eq!(ready["type"], "rpc_ready");
        assert_eq!(ready["version"], 2);
        assert_eq!(
            ready["features"],
            json!(["permissions", "auth", "catalog", "config", "attachments"])
        );
        assert_eq!(
            serde_json::to_value(RpcSettled::new(true)).expect("settled")["type"],
            "agent_settled"
        );
    }

    #[test]
    fn permission_requests_carry_correlation_and_arguments() {
        let request =
            RpcPermissionRequest::new("perm-1", "call-7", "edit", json!({"path": "src/main.rs"}));
        let value = serde_json::to_value(&request).expect("permission encodes");
        assert_eq!(value["type"], "permission_request");
        assert_eq!(value["version"], RPC_PROTOCOL_VERSION);
        assert_eq!(value["id"], "perm-1");
        assert_eq!(value["tool_call_id"], "call-7");
        assert_eq!(value["tool"], "edit");
        assert_eq!(value["arguments"]["path"], "src/main.rs");

        let decoded: RpcPermissionRequest =
            serde_json::from_value(value).expect("permission decodes");
        assert_eq!(decoded, request);
    }

    #[test]
    fn v1_ready_records_still_parse_without_features() {
        let ready: RpcReady =
            serde_json::from_str(r#"{"type":"rpc_ready","version":1}"#).expect("v1 ready parses");
        assert_eq!(ready.version, 1);
        assert!(ready.features.is_empty());
    }
}
