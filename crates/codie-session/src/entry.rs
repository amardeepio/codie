//! Session header and append-only entry types.
//!
//! A session file is JSONL: the first line is a [`SessionHeader`] and every
//! following line is one [`Entry`]. Entries form a tree through `parent_id`, so
//! appending never rewrites earlier history (SESS-001).

use codie_core::{Message, ThinkingLevel, Usage};
use serde::{Deserialize, Serialize};

/// On-disk session format version understood by this build.
///
/// Version 1 is the initial format. Readers refuse unknown newer versions and
/// migrate older ones on read (SESS-007).
pub const SESSION_FORMAT_VERSION: u32 = 1;

/// The `type` discriminator of a session file header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HeaderType {
    /// A session file header.
    Session,
}

/// The first line of every session file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionHeader {
    /// Discriminator; always `session`.
    #[serde(rename = "type")]
    pub header_type: HeaderType,
    /// On-disk format version; this build writes and accepts version 1.
    pub version: u32,
    /// Session identifier, unique within a session directory.
    pub id: String,
    /// Creation time as an RFC 3339 string.
    pub timestamp: String,
    /// Absolute working directory the session belongs to.
    pub cwd: String,
}

/// One node in the append-only session tree.
///
/// Every variant carries `id`, `parent_id`, and `timestamp`. `parent_id` is
/// `null` for the first entry and names the previous leaf for later entries.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Entry {
    /// A transcript message.
    Message {
        /// Entry identifier: eight lowercase hexadecimal characters.
        id: String,
        /// Parent entry identifier, or `null` for the first entry.
        #[serde(default)]
        parent_id: Option<String>,
        /// Entry creation time as an RFC 3339 string.
        timestamp: String,
        /// The persisted message.
        message: Message,
    },
    /// The model selected for subsequent requests.
    ModelChange {
        /// Entry identifier: eight lowercase hexadecimal characters.
        id: String,
        /// Parent entry identifier, or `null` for the first entry.
        #[serde(default)]
        parent_id: Option<String>,
        /// Entry creation time as an RFC 3339 string.
        timestamp: String,
        /// Provider identifier.
        provider: String,
        /// Model identifier within the provider.
        model_id: String,
    },
    /// The reasoning effort selected for subsequent requests.
    ThinkingLevelChange {
        /// Entry identifier: eight lowercase hexadecimal characters.
        id: String,
        /// Parent entry identifier, or `null` for the first entry.
        #[serde(default)]
        parent_id: Option<String>,
        /// Entry creation time as an RFC 3339 string.
        timestamp: String,
        /// Requested reasoning level.
        thinking_level: ThinkingLevel,
    },
    /// A human-readable session name.
    SessionInfo {
        /// Entry identifier: eight lowercase hexadecimal characters.
        id: String,
        /// Parent entry identifier, or `null` for the first entry.
        #[serde(default)]
        parent_id: Option<String>,
        /// Entry creation time as an RFC 3339 string.
        timestamp: String,
        /// Display name, or `null` when the name was cleared.
        name: Option<String>,
    },
    /// Token usage attributed outside a message, for example a summarization
    /// request.
    Usage {
        /// Entry identifier: eight lowercase hexadecimal characters.
        id: String,
        /// Parent entry identifier, or `null` for the first entry.
        #[serde(default)]
        parent_id: Option<String>,
        /// Entry creation time as an RFC 3339 string.
        timestamp: String,
        /// Usage category, such as `compaction`.
        kind: String,
        /// Provider identifier.
        provider: String,
        /// Model identifier.
        model: String,
        /// Normalized token usage.
        usage: Usage,
    },
}

impl Entry {
    /// Returns the entry identifier.
    #[must_use]
    pub fn id(&self) -> &str {
        match self {
            Self::Message { id, .. }
            | Self::ModelChange { id, .. }
            | Self::ThinkingLevelChange { id, .. }
            | Self::SessionInfo { id, .. }
            | Self::Usage { id, .. } => id,
        }
    }

    /// Returns the parent entry identifier, if any.
    #[must_use]
    pub fn parent_id(&self) -> Option<&str> {
        match self {
            Self::Message { parent_id, .. }
            | Self::ModelChange { parent_id, .. }
            | Self::ThinkingLevelChange { parent_id, .. }
            | Self::SessionInfo { parent_id, .. }
            | Self::Usage { parent_id, .. } => parent_id.as_deref(),
        }
    }

    /// Returns the entry creation time as an RFC 3339 string.
    #[must_use]
    pub fn timestamp(&self) -> &str {
        match self {
            Self::Message { timestamp, .. }
            | Self::ModelChange { timestamp, .. }
            | Self::ThinkingLevelChange { timestamp, .. }
            | Self::SessionInfo { timestamp, .. }
            | Self::Usage { timestamp, .. } => timestamp,
        }
    }

    /// Returns the message payload, when this is a message entry.
    #[must_use]
    pub const fn message(&self) -> Option<&Message> {
        match self {
            Self::Message { message, .. } => Some(message),
            _ => None,
        }
    }

    /// Returns the model selection, when this is a model-change entry.
    #[must_use]
    pub fn model_change(&self) -> Option<(&str, &str)> {
        match self {
            Self::ModelChange {
                provider, model_id, ..
            } => Some((provider, model_id)),
            _ => None,
        }
    }

    /// Returns the reasoning level, when this is a thinking-level entry.
    #[must_use]
    pub const fn thinking_level_change(&self) -> Option<ThinkingLevel> {
        match self {
            Self::ThinkingLevelChange { thinking_level, .. } => Some(*thinking_level),
            _ => None,
        }
    }

    /// Returns the session name, when this is a session-info entry.
    #[must_use]
    pub fn session_info_name(&self) -> Option<Option<&str>> {
        match self {
            Self::SessionInfo { name, .. } => Some(name.as_deref()),
            _ => None,
        }
    }
}
