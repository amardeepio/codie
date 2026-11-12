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

/// Version of the RPC protocol implemented by this build.
pub const RPC_PROTOCOL_VERSION: u32 = 1;

/// One command read from stdin.
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum RpcCommand {
    /// Start a run with a user message.
    Prompt {
        /// Prompt text. `@path` attachments are expanded.
        message: String,
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
    /// Report the current model, thinking level, and session state.
    GetState,
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
            Self::GetState => "get_state",
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct RpcReady {
    /// Discriminator; always `rpc_ready`.
    #[serde(rename = "type")]
    pub record_type: ReadyType,
    /// Protocol version.
    pub version: u32,
}

/// The only value of [`RpcReady::record_type`].
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
    pub const fn new() -> Self {
        Self {
            record_type: ReadyType::Ready,
            version: RPC_PROTOCOL_VERSION,
        }
    }
}

impl Default for RpcReady {
    fn default() -> Self {
        Self::new()
    }
}

/// Session header emitted after the ready record and after `new_session`.
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum SessionType {
    /// `session`
    #[serde(rename = "session")]
    Session,
}

/// Response to one command.
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct RpcError {
    /// Stable machine-readable code.
    pub code: String,
    /// Human-readable explanation.
    pub message: String,
}

/// Completion signal for one run.
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(untagged)]
pub enum RpcServerRecord {
    /// Version handshake.
    Ready(RpcReady),
    /// Session header.
    Session(RpcSessionHeader),
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
    vec![
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
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

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
                message: "hello".to_owned()
            }
        );

        let encoded = serde_json::to_value(&request).expect("request encodes");
        assert_eq!(encoded["command"], "prompt");
        assert_eq!(encoded["message"], "hello");
    }

    #[test]
    fn every_command_has_a_stable_name() {
        let commands = [
            (RpcCommand::Abort, "abort"),
            (RpcCommand::GetState, "get_state"),
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
        assert_eq!(
            serde_json::to_value(RpcReady::new()).expect("ready")["type"],
            "rpc_ready"
        );
        assert_eq!(
            serde_json::to_value(RpcSettled::new(true)).expect("settled")["type"],
            "agent_settled"
        );
    }
}
