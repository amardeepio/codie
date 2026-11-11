//! Session header and append-only entry types.
//!
//! A session file is JSONL: the first line is a [`SessionHeader`] and every
//! following line is one [`Entry`]. Entries form a tree through `parent_id`, so
//! appending never rewrites earlier history (SESS-001).

use codie_core::{Content, Message, ThinkingLevel, Usage};
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
    /// Identifier of the session this one was forked or cloned from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session: Option<String>,
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
    /// A compacted summary of everything before `first_kept_entry_id`.
    ///
    /// Compaction never deletes entries: the projection replaces the hidden
    /// prefix with the summary (`CTX-005`).
    Compaction {
        /// Entry identifier: eight lowercase hexadecimal characters.
        id: String,
        /// Parent entry identifier, or `null` for the first entry.
        #[serde(default)]
        parent_id: Option<String>,
        /// Entry creation time as an RFC 3339 string.
        timestamp: String,
        /// Model-written summary of the compacted prefix.
        summary: String,
        /// First message entry retained after the summary.
        #[serde(default)]
        first_kept_entry_id: Option<String>,
        /// Estimated tokens before compaction.
        tokens_before: u64,
        /// Host-only details about the compaction request.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        details: Option<serde_json::Value>,
        /// Token usage for the summarization request.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        usage: Option<Usage>,
    },
    /// A summary of an abandoned branch, retained when returning to an earlier
    /// leaf.
    BranchSummary {
        /// Entry identifier: eight lowercase hexadecimal characters.
        id: String,
        /// Parent entry identifier, or `null` for the first entry.
        #[serde(default)]
        parent_id: Option<String>,
        /// Entry creation time as an RFC 3339 string.
        timestamp: String,
        /// Entry on the summarized branch where the summary starts.
        from_id: String,
        /// Model-written summary of the branch.
        summary: String,
    },
    /// A replacement or removal for an earlier message's content.
    ContextEdit {
        /// Entry identifier: eight lowercase hexadecimal characters.
        id: String,
        /// Parent entry identifier, or `null` for the first entry.
        #[serde(default)]
        parent_id: Option<String>,
        /// Entry creation time as an RFC 3339 string.
        timestamp: String,
        /// Message entry this edit applies to.
        target_id: String,
        /// Replacement content, or `null` to remove the message.
        #[serde(default)]
        replacement: Option<Content>,
    },
    /// A user-assigned label on an entry, used by the tree view.
    Label {
        /// Entry identifier: eight lowercase hexadecimal characters.
        id: String,
        /// Parent entry identifier, or `null` for the first entry.
        #[serde(default)]
        parent_id: Option<String>,
        /// Entry creation time as an RFC 3339 string.
        timestamp: String,
        /// Entry the label applies to.
        target_id: String,
        /// Label text, or `null` to clear it.
        #[serde(default)]
        label: Option<String>,
    },
    /// Extension-owned metadata that codie stores without interpreting.
    Custom {
        /// Entry identifier: eight lowercase hexadecimal characters.
        id: String,
        /// Parent entry identifier, or `null` for the first entry.
        #[serde(default)]
        parent_id: Option<String>,
        /// Entry creation time as an RFC 3339 string.
        timestamp: String,
        /// Extension-defined type name.
        custom_type: String,
        /// Extension-defined payload.
        data: serde_json::Value,
    },
    /// A persisted branch selection.
    ///
    /// The entry's parent is the selected target, so reopening the file resumes
    /// on that branch. The marker itself is never projected.
    Leaf {
        /// Entry identifier: eight lowercase hexadecimal characters.
        id: String,
        /// Selected branch point, or `null` for an empty branch.
        #[serde(default)]
        parent_id: Option<String>,
        /// Entry creation time as an RFC 3339 string.
        timestamp: String,
        /// Selected branch point, repeated for readers that do not walk parents.
        #[serde(default)]
        target_id: Option<String>,
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
            | Self::Usage { id, .. }
            | Self::Compaction { id, .. }
            | Self::BranchSummary { id, .. }
            | Self::ContextEdit { id, .. }
            | Self::Label { id, .. }
            | Self::Custom { id, .. }
            | Self::Leaf { id, .. } => id,
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
            | Self::Usage { parent_id, .. }
            | Self::Compaction { parent_id, .. }
            | Self::BranchSummary { parent_id, .. }
            | Self::ContextEdit { parent_id, .. }
            | Self::Label { parent_id, .. }
            | Self::Custom { parent_id, .. }
            | Self::Leaf { parent_id, .. } => parent_id.as_deref(),
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
            | Self::Usage { timestamp, .. }
            | Self::Compaction { timestamp, .. }
            | Self::BranchSummary { timestamp, .. }
            | Self::ContextEdit { timestamp, .. }
            | Self::Label { timestamp, .. }
            | Self::Custom { timestamp, .. }
            | Self::Leaf { timestamp, .. } => timestamp,
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

    /// Returns the compaction summary and first retained entry, when this is a
    /// compaction entry.
    #[must_use]
    pub fn compaction(&self) -> Option<(&str, Option<&str>, u64)> {
        match self {
            Self::Compaction {
                summary,
                first_kept_entry_id,
                tokens_before,
                ..
            } => Some((summary, first_kept_entry_id.as_deref(), *tokens_before)),
            _ => None,
        }
    }

    /// Returns the branch summary, when this is a branch-summary entry.
    #[must_use]
    pub fn branch_summary(&self) -> Option<&str> {
        match self {
            Self::BranchSummary { summary, .. } => Some(summary),
            _ => None,
        }
    }

    /// Returns the context edit target and replacement, when this is a
    /// context-edit entry.
    #[must_use]
    pub fn context_edit(&self) -> Option<(&str, Option<&Content>)> {
        match self {
            Self::ContextEdit {
                target_id,
                replacement,
                ..
            } => Some((target_id, replacement.as_ref())),
            _ => None,
        }
    }

    /// Returns the label target and text, when this is a label entry.
    #[must_use]
    pub fn label(&self) -> Option<(&str, Option<&str>)> {
        match self {
            Self::Label {
                target_id, label, ..
            } => Some((target_id, label.as_deref())),
            _ => None,
        }
    }

    /// Returns the extension payload, when this is a custom entry.
    #[must_use]
    pub fn custom(&self) -> Option<(&str, &serde_json::Value)> {
        match self {
            Self::Custom {
                custom_type, data, ..
            } => Some((custom_type, data)),
            _ => None,
        }
    }
}
