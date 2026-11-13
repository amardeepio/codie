//! The append-only session store.
//!
//! A [`Session`] owns the in-memory tree and the JSONL file backing it. The file
//! is created lazily when the first message is appended (SESS-002); metadata
//! entries written before that point are buffered in memory and flushed together
//! with the header. Later appends use `O_APPEND` and flush after each line
//! (SESS-003).

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use codie_core::{Content, Error, Message, Result, ThinkingLevel, Usage};

use crate::entry::{Entry, HeaderType, SESSION_FORMAT_VERSION, SessionHeader};
use crate::time::{rfc3339_now, unix_nanos};

/// An open session backed by an append-only JSONL file.
#[derive(Debug)]
pub struct Session {
    path: PathBuf,
    header: SessionHeader,
    entries: Vec<Entry>,
    leaf: Option<String>,
    counter: u64,
    persisted: bool,
}

/// A session file discovered in a session directory.
///
/// Used by the session picker and the `codie sessions` commands. Discovery is
/// read-only (`SESS-008`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSummary {
    /// Path of the JSONL file.
    pub path: PathBuf,
    /// Session identifier from the header.
    pub id: String,
    /// Latest display name, when set.
    pub name: Option<String>,
    /// Working directory recorded in the header.
    pub cwd: String,
    /// Creation timestamp from the header.
    pub created_at: String,
    /// Last modification time of the file.
    pub updated_at: SystemTime,
    /// Number of message entries in the whole file.
    pub message_count: usize,
    /// Session this one was cloned or forked from.
    pub parent_session: Option<String>,
    /// Most recent model used on the branch.
    pub model: Option<String>,
    /// Estimated tokens on the branch.
    pub tokens: u64,
    /// Latest recorded cost in nano-USD.
    pub cost: u64,
    /// Most recent user prompt.
    pub last_user: Option<String>,
    /// Most recent assistant reply.
    pub last_assistant: Option<String>,
}

/// One row in a session tree listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeEntry {
    /// Entry identifier.
    pub id: String,
    /// Parent entry identifier, if any.
    pub parent_id: Option<String>,
    /// Number of parents between this entry and the root.
    pub depth: usize,
    /// True when the entry is on the branch from the root to the current leaf.
    pub on_branch: bool,
    /// Entry kind, such as `message`, `compaction`, or `model_change`.
    pub kind: &'static str,
    /// Short human-readable description.
    pub description: String,
}

impl Session {
    /// Creates a new, empty session rooted at `cwd`.
    ///
    /// The session directory is created eagerly. The JSONL file itself is not
    /// created until the first message is appended.
    pub fn create(cwd: impl AsRef<Path>, session_dir: impl AsRef<Path>) -> Result<Self> {
        Self::create_with_parent(cwd, session_dir, None)
    }

    /// Creates a new session that records the session it was derived from.
    fn create_with_parent(
        cwd: impl AsRef<Path>,
        session_dir: impl AsRef<Path>,
        parent_session: Option<String>,
    ) -> Result<Self> {
        let session_dir = session_dir.as_ref();
        fs::create_dir_all(session_dir).map_err(|error| Error::io(session_dir, error))?;

        let id = new_session_id();
        let path = session_dir.join(format!("{id}.jsonl"));
        let header = SessionHeader {
            header_type: HeaderType::Session,
            version: SESSION_FORMAT_VERSION,
            id,
            timestamp: rfc3339_now(),
            cwd: cwd.as_ref().display().to_string(),
            parent_session,
        };

        Ok(Self {
            path,
            header,
            entries: Vec::new(),
            leaf: None,
            counter: 0,
            persisted: false,
        })
    }

    /// Opens an existing session file.
    ///
    /// Malformed entry lines are skipped so that a truncated write at the end of
    /// a file does not make the whole session unreadable. Malformed headers and
    /// unsupported format versions are errors.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let contents = fs::read_to_string(path).map_err(|error| Error::io(path, error))?;
        let mut lines = contents.lines().filter(|line| !line.trim().is_empty());

        let Some(header_line) = lines.next() else {
            return Err(Error::session(format!(
                "session file {} is empty",
                path.display()
            )));
        };
        let header: SessionHeader = serde_json::from_str(header_line).map_err(|error| {
            Error::session(format!(
                "invalid session header in {}: {error}",
                path.display()
            ))
        })?;

        if header.version > SESSION_FORMAT_VERSION {
            return Err(Error::session(format!(
                "session {} uses format version {} but this build supports at most version {}",
                path.display(),
                header.version,
                SESSION_FORMAT_VERSION
            )));
        }
        if !crate::migration::is_supported(header.version) {
            return Err(Error::session(format!(
                "session {} uses unsupported format version {}",
                path.display(),
                header.version
            )));
        }

        let mut entries = Vec::new();
        for line in lines {
            // A partial final line left by a crash is skipped. Older format
            // versions run through the migration table before decoding
            // (SESS-025).
            if header.version == SESSION_FORMAT_VERSION {
                if let Ok(entry) = serde_json::from_str::<Entry>(line) {
                    entries.push(entry);
                }
                continue;
            }
            let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            let Ok(value) = crate::migration::migrate_value(value, header.version) else {
                continue;
            };
            if let Ok(entry) = serde_json::from_value::<Entry>(value) {
                entries.push(entry);
            }
        }

        let leaf = entries.last().map(|entry| entry.id().to_string());
        let counter = u64::try_from(entries.len()).unwrap_or(u64::MAX);

        Ok(Self {
            path: path.to_path_buf(),
            header,
            entries,
            leaf,
            counter,
            persisted: true,
        })
    }

    /// Returns the most recent session for `cwd`, if one exists.
    ///
    /// Sessions whose header names a different working directory are ignored.
    /// Ordering is by header timestamp, then by file modification time.
    pub fn continue_recent(
        cwd: impl AsRef<Path>,
        session_dir: impl AsRef<Path>,
    ) -> Result<Option<Self>> {
        let cwd = cwd.as_ref();
        let session_dir = session_dir.as_ref();

        let read_dir = match fs::read_dir(session_dir) {
            Ok(read_dir) => read_dir,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(Error::io(session_dir, error)),
        };

        let mut best: Option<(String, SystemTime, PathBuf)> = None;
        for dir_entry in read_dir {
            let Ok(dir_entry) = dir_entry else {
                continue;
            };
            let path = dir_entry.path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("jsonl") {
                continue;
            }
            let Some(header) = read_header(&path) else {
                continue;
            };
            if !cwd_matches(&header.cwd, cwd) {
                continue;
            }
            let modified = dir_entry
                .metadata()
                .and_then(|metadata| metadata.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            let candidate = (header.timestamp, modified, path);
            let newer = best.as_ref().is_none_or(|current| {
                (candidate.0.as_str(), candidate.1) > (current.0.as_str(), current.1)
            });
            if newer {
                best = Some(candidate);
            }
        }

        match best {
            None => Ok(None),
            Some((_, _, path)) => Self::open(path).map(Some),
        }
    }

    /// Returns the path of the backing JSONL file.
    ///
    /// The file may not exist yet when no message has been appended.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Lists every readable session in `session_dir`, newest first.
    ///
    /// Unreadable or malformed files are skipped so that one damaged session
    /// cannot break the picker (`SESS-008`).
    pub fn list(session_dir: impl AsRef<Path>) -> Result<Vec<SessionSummary>> {
        let session_dir = session_dir.as_ref();
        let read_dir = match fs::read_dir(session_dir) {
            Ok(read_dir) => read_dir,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(Error::io(session_dir, error)),
        };

        let mut summaries = Vec::new();
        for dir_entry in read_dir {
            let Ok(dir_entry) = dir_entry else {
                continue;
            };
            let path = dir_entry.path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("jsonl") {
                continue;
            }
            let Ok(session) = Self::open(&path) else {
                continue;
            };
            let updated_at = dir_entry
                .metadata()
                .and_then(|metadata| metadata.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            let projection = session.projection();
            let last_user = projection.iter().rev().find_map(|message| {
                let Message::User(user) = message else {
                    return None;
                };
                let text = user
                    .content
                    .iter()
                    .filter_map(Content::as_text)
                    .collect::<Vec<_>>()
                    .join(" ");
                (!text.trim().is_empty()).then_some(text)
            });
            let last_assistant = projection.iter().rev().find_map(|message| {
                let Message::Assistant(assistant) = message else {
                    return None;
                };
                let text = assistant.text();
                (!text.trim().is_empty()).then_some(text)
            });
            let model = projection.iter().rev().find_map(|message| {
                let Message::Assistant(assistant) = message else {
                    return None;
                };
                Some(assistant.model.clone())
            });
            let tokens = projection.iter().map(estimate_message_tokens).sum::<u64>();
            let mut cost = 0_u64;
            for entry in session.branch() {
                match entry {
                    Entry::Message {
                        message: Message::Assistant(assistant),
                        ..
                    } => {
                        cost = cost.saturating_add(assistant.usage.cost.total);
                    }
                    Entry::Usage { usage, .. } => {
                        cost = cost.saturating_add(usage.cost.total);
                    }
                    _ => {}
                }
            }
            summaries.push(SessionSummary {
                path,
                id: session.id().to_owned(),
                name: session.name().map(str::to_owned),
                cwd: session.cwd().to_owned(),
                created_at: session.header.timestamp.clone(),
                updated_at,
                message_count: session
                    .entries
                    .iter()
                    .filter(|entry| entry.message().is_some())
                    .count(),
                parent_session: session.header.parent_session.clone(),
                model,
                tokens,
                cost,
                last_user,
                last_assistant,
            });
        }
        summaries.sort_by(|left, right| {
            right
                .updated_at
                .cmp(&left.updated_at)
                .then_with(|| right.created_at.cmp(&left.created_at))
        });
        Ok(summaries)
    }

    /// Lists sessions whose recorded working directory matches `cwd`.
    ///
    /// Paths are compared after canonicalization when possible, matching
    /// `continue_recent` (`SESS-004`).
    pub fn list_for_cwd(
        cwd: impl AsRef<Path>,
        session_dir: impl AsRef<Path>,
    ) -> Result<Vec<SessionSummary>> {
        let cwd = cwd.as_ref();
        Ok(Self::list(session_dir)?
            .into_iter()
            .filter(|summary| cwd_matches(&summary.cwd, cwd))
            .collect())
    }

    /// Opens a session by file path or by identifier.
    ///
    /// An exact identifier wins; otherwise a unique prefix is accepted. An
    /// ambiguous prefix is an error rather than a guess (`SESS-004`).
    pub fn resolve(session_dir: impl AsRef<Path>, needle: &str) -> Result<Self> {
        let path = Path::new(needle);
        if path.is_file() {
            return Self::open(path);
        }

        let session_dir = session_dir.as_ref();
        let summaries = Self::list(session_dir)?;
        let exact: Vec<&SessionSummary> = summaries
            .iter()
            .filter(|summary| summary.id == needle)
            .collect();
        let matches = if exact.is_empty() {
            summaries
                .iter()
                .filter(|summary| summary.id.starts_with(needle))
                .collect::<Vec<_>>()
        } else {
            exact
        };

        match matches.as_slice() {
            [] => Err(Error::session(format!(
                "no session matches `{needle}` in {}",
                session_dir.display()
            ))),
            [summary] => Self::open(&summary.path),
            _ => Err(Error::session(format!(
                "`{needle}` matches {} sessions; use a longer identifier",
                matches.len()
            ))),
        }
    }

    /// Returns the branch from the root to the current leaf.
    #[must_use]
    pub fn branch(&self) -> Vec<&Entry> {
        let mut branch = self.chain();
        branch.reverse();
        branch
    }

    /// Returns the branch from the root to `entry_id`.
    fn branch_at(&self, entry_id: &str) -> Result<Vec<&Entry>> {
        let by_id: HashMap<&str, &Entry> = self
            .entries
            .iter()
            .map(|entry| (entry.id(), entry))
            .collect();
        let mut branch = Vec::new();
        let mut cursor = Some(entry_id);
        while let Some(id) = cursor {
            let Some(entry) = by_id.get(id).copied() else {
                return Err(Error::session(format!("unknown session entry {id}")));
            };
            branch.push(entry);
            if branch.len() > self.entries.len() {
                return Err(Error::session("session parent chain is cyclic"));
            }
            cursor = entry.parent_id();
        }
        branch.reverse();
        Ok(branch)
    }

    /// Lists every entry as a tree row, marking the current branch.
    ///
    /// Entries appear in file order so the result is stable across reads.
    #[must_use]
    pub fn tree(&self) -> Vec<TreeEntry> {
        let on_branch: std::collections::HashSet<&str> =
            self.branch().into_iter().map(Entry::id).collect();
        let by_id: HashMap<&str, &Entry> = self
            .entries
            .iter()
            .map(|entry| (entry.id(), entry))
            .collect();

        self.entries
            .iter()
            .map(|entry| {
                let mut depth = 0;
                let mut cursor = entry.parent_id();
                while let Some(parent) = cursor {
                    if depth > self.entries.len() {
                        break;
                    }
                    depth += 1;
                    cursor = by_id.get(parent).and_then(|entry| entry.parent_id());
                }
                TreeEntry {
                    id: entry.id().to_owned(),
                    parent_id: entry.parent_id().map(str::to_owned),
                    depth,
                    on_branch: on_branch.contains(entry.id()),
                    kind: entry_kind(entry),
                    description: describe_entry(entry),
                }
            })
            .collect()
    }

    /// Creates a new session file containing the branch that ends at
    /// `entry_id`, or at the current leaf when `None`.
    ///
    /// The source session is never modified (`SESS-006`). The new header records
    /// the source identifier in `parent_session`.
    pub fn fork_at(
        &self,
        entry_id: Option<&str>,
        cwd: impl AsRef<Path>,
        session_dir: impl AsRef<Path>,
    ) -> Result<Self> {
        let leaf = entry_id.or(self.leaf.as_deref());
        let entries: Vec<Entry> = match leaf {
            Some(leaf) => self
                .branch_at(leaf)?
                .into_iter()
                .cloned()
                .collect::<Vec<_>>(),
            None => Vec::new(),
        };
        let mut forked = Self::create_with_parent(cwd, session_dir, Some(self.header.id.clone()))?;
        if !entries.is_empty() {
            forked.persist_entries(&entries)?;
            forked.entries = entries;
            forked.leaf = leaf.map(str::to_owned);
            forked.counter = u64::try_from(forked.entries.len()).unwrap_or(u64::MAX);
            forked.persisted = true;
        }
        Ok(forked)
    }

    /// Copies every entry into a new session file, preserving the whole tree and
    /// the current leaf.
    ///
    /// Unlike [`Session::fork_at`], later branches are retained as well.
    pub fn clone_session(
        &self,
        cwd: impl AsRef<Path>,
        session_dir: impl AsRef<Path>,
    ) -> Result<Self> {
        let mut cloned = Self::create_with_parent(cwd, session_dir, Some(self.header.id.clone()))?;
        if !self.entries.is_empty() {
            cloned.persist_entries(&self.entries)?;
            cloned.entries = self.entries.clone();
            cloned.leaf = self.leaf.clone();
            cloned.counter = u64::try_from(self.entries.len()).unwrap_or(u64::MAX);
            cloned.persisted = true;
        }
        Ok(cloned)
    }

    /// Returns the identifier of the session this one was derived from.
    #[must_use]
    pub fn parent_session(&self) -> Option<&str> {
        self.header.parent_session.as_deref()
    }

    /// Returns the session identifier from the header.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.header.id
    }

    /// Returns the working directory recorded in the header.
    #[must_use]
    pub fn cwd(&self) -> &str {
        &self.header.cwd
    }

    /// Returns the parsed session header.
    #[must_use]
    pub const fn header(&self) -> &SessionHeader {
        &self.header
    }

    /// Returns every entry in file order.
    #[must_use]
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Returns the identifier of the current leaf entry.
    #[must_use]
    pub fn leaf(&self) -> Option<&str> {
        self.leaf.as_deref()
    }

    /// Moves the session leaf, for tree navigation.
    ///
    /// The selection is persisted as a leaf marker whose parent is the target,
    /// so reopening the file resumes on the same branch. Passing `None` clears
    /// the branch. An unknown identifier is rejected.
    pub fn set_leaf(&mut self, entry_id: Option<&str>) -> Result<()> {
        if let Some(id) = entry_id
            && !self.entries.iter().any(|entry| entry.id() == id)
        {
            return Err(Error::session(format!("unknown session entry {id}")));
        }
        let id = self.next_id();
        let entry = Entry::Leaf {
            id,
            parent_id: entry_id.map(str::to_owned),
            timestamp: rfc3339_now(),
            target_id: entry_id.map(str::to_owned),
        };
        self.commit(entry)
    }

    /// Returns the branch point selected by the current leaf, if any.
    ///
    /// For an ordinary append the leaf is the last entry itself; for an
    /// explicit branch selection it is the marker's target.
    #[must_use]
    pub fn leaf_target(&self) -> Option<&str> {
        let leaf = self.leaf.as_deref()?;
        self.entries
            .iter()
            .find(|entry| entry.id() == leaf)
            .and_then(|entry| match entry {
                Entry::Leaf { target_id, .. } => target_id.as_deref(),
                _ => Some(leaf),
            })
    }

    /// Appends a transcript message as the new leaf.
    pub fn append_message(&mut self, message: &Message) -> Result<()> {
        let id = self.next_id();
        let entry = Entry::Message {
            id,
            parent_id: self.leaf.clone(),
            timestamp: rfc3339_now(),
            message: message.clone(),
        };
        self.commit(entry)
    }

    /// Records the model selected for subsequent requests.
    pub fn append_model_change(
        &mut self,
        provider: impl Into<String>,
        model_id: impl Into<String>,
    ) -> Result<()> {
        let id = self.next_id();
        let entry = Entry::ModelChange {
            id,
            parent_id: self.leaf.clone(),
            timestamp: rfc3339_now(),
            provider: provider.into(),
            model_id: model_id.into(),
        };
        self.commit(entry)
    }

    /// Records the reasoning level selected for subsequent requests.
    pub fn append_thinking_level_change(&mut self, thinking_level: ThinkingLevel) -> Result<()> {
        let id = self.next_id();
        let entry = Entry::ThinkingLevelChange {
            id,
            parent_id: self.leaf.clone(),
            timestamp: rfc3339_now(),
            thinking_level,
        };
        self.commit(entry)
    }

    /// Records token usage attributed outside a message, such as a compaction
    /// or title-generation request.
    pub fn append_usage(
        &mut self,
        kind: impl Into<String>,
        provider: impl Into<String>,
        model: impl Into<String>,
        usage: Usage,
    ) -> Result<()> {
        let id = self.next_id();
        let entry = Entry::Usage {
            id,
            parent_id: self.leaf.clone(),
            timestamp: rfc3339_now(),
            kind: kind.into(),
            provider: provider.into(),
            model: model.into(),
            usage,
        };
        self.commit(entry)
    }

    /// Records a compaction summary over the prefix that ends before
    /// `first_kept_entry_id`.
    ///
    /// The summary is an ordinary entry; earlier entries stay in the file
    /// (`CTX-005`). `first_kept_entry_id` must name an existing entry.
    pub fn append_compaction(
        &mut self,
        summary: impl Into<String>,
        first_kept_entry_id: Option<&str>,
        tokens_before: u64,
        details: Option<serde_json::Value>,
        usage: Option<Usage>,
    ) -> Result<()> {
        if let Some(id) = first_kept_entry_id
            && !self.entries.iter().any(|entry| entry.id() == id)
        {
            return Err(Error::session(format!("unknown session entry {id}")));
        }
        if let Some(id) = first_kept_entry_id
            && let Some(entry) = self.entries.iter().find(|entry| entry.id() == id)
            && entry.message().is_none()
        {
            return Err(Error::session("compaction must start at a message entry"));
        }
        if let Some(id) = first_kept_entry_id
            && let Some(entry) = self.entries.iter().find(|entry| entry.id() == id)
            && matches!(entry.message(), Some(Message::ToolResult(_)))
        {
            // CTX-003: never start the retained tail at a tool result, because
            // that would separate it from the assistant tool call.
            return Err(Error::session("compaction cannot start at a tool result"));
        }
        let id = self.next_id();
        let entry = Entry::Compaction {
            id,
            parent_id: self.leaf.clone(),
            timestamp: rfc3339_now(),
            summary: summary.into(),
            first_kept_entry_id: first_kept_entry_id.map(str::to_owned),
            tokens_before,
            details,
            usage,
        };
        self.commit(entry)
    }

    /// Records a summary of an abandoned branch.
    pub fn append_branch_summary(
        &mut self,
        from_id: impl Into<String>,
        summary: impl Into<String>,
    ) -> Result<()> {
        let id = self.next_id();
        let entry = Entry::BranchSummary {
            id,
            parent_id: self.leaf.clone(),
            timestamp: rfc3339_now(),
            from_id: from_id.into(),
            summary: summary.into(),
        };
        self.commit(entry)
    }

    /// Replaces or removes the content of an earlier message.
    pub fn append_context_edit(
        &mut self,
        target_id: impl Into<String>,
        replacement: Option<Content>,
    ) -> Result<()> {
        let target_id = target_id.into();
        if !self.entries.iter().any(|entry| entry.id() == target_id) {
            return Err(Error::session(format!("unknown session entry {target_id}")));
        }
        let id = self.next_id();
        let entry = Entry::ContextEdit {
            id,
            parent_id: self.leaf.clone(),
            timestamp: rfc3339_now(),
            target_id,
            replacement,
        };
        self.commit(entry)
    }

    /// Sets or clears a label on an earlier entry.
    pub fn set_label(&mut self, target_id: impl Into<String>, label: Option<String>) -> Result<()> {
        let target_id = target_id.into();
        if !self.entries.iter().any(|entry| entry.id() == target_id) {
            return Err(Error::session(format!("unknown session entry {target_id}")));
        }
        let id = self.next_id();
        let entry = Entry::Label {
            id,
            parent_id: self.leaf.clone(),
            timestamp: rfc3339_now(),
            target_id,
            label,
        };
        self.commit(entry)
    }

    /// Stores extension-owned metadata without interpreting it.
    pub fn append_custom(
        &mut self,
        custom_type: impl Into<String>,
        data: serde_json::Value,
    ) -> Result<()> {
        let id = self.next_id();
        let entry = Entry::Custom {
            id,
            parent_id: self.leaf.clone(),
            timestamp: rfc3339_now(),
            custom_type: custom_type.into(),
            data,
        };
        self.commit(entry)
    }

    /// Sets the human-readable session name.
    pub fn set_name(&mut self, name: impl Into<String>) -> Result<()> {
        self.append_session_info(Some(name.into()))
    }

    /// Clears the human-readable session name.
    pub fn clear_name(&mut self) -> Result<()> {
        self.append_session_info(None)
    }

    /// Returns the latest session name, or `None` when unnamed or cleared.
    #[must_use]
    pub fn name(&self) -> Option<&str> {
        self.chain()
            .into_iter()
            .find_map(Entry::session_info_name)
            .flatten()
    }

    /// Returns the most recent model selection as `(provider, model_id)`.
    #[must_use]
    pub fn latest_model(&self) -> Option<(&str, &str)> {
        self.chain().into_iter().find_map(Entry::model_change)
    }

    /// Returns the most recent reasoning level.
    #[must_use]
    pub fn latest_thinking_level(&self) -> Option<ThinkingLevel> {
        self.chain()
            .into_iter()
            .find_map(Entry::thinking_level_change)
    }

    /// Builds the model-visible transcript by walking from the leaf to the root
    /// and collecting messages in chronological order.
    ///
    /// Compaction entries replace the hidden prefix with a summary message;
    /// branch summaries are inserted in place, and context edits are applied to
    /// the messages they target (`SESS-005`, `CTX-005`).
    #[must_use]
    pub fn projection(&self) -> Vec<Message> {
        let mut messages: Vec<(String, Message)> = Vec::new();

        for entry in self.branch() {
            if let Some(message) = entry.message() {
                messages.push((entry.id().to_owned(), message.clone()));
                continue;
            }
            if let Some((summary, first_kept, _)) = entry.compaction() {
                let system = matches!(
                    messages.first().map(|(_, message)| message),
                    Some(Message::System(_))
                )
                .then(|| messages[0].clone());
                let keep_index = first_kept
                    .and_then(|id| messages.iter().position(|(message_id, _)| message_id == id))
                    .unwrap_or(messages.len());
                messages.drain(..keep_index);
                if let Some(system) = system
                    && !matches!(
                        messages.first().map(|(_, message)| message),
                        Some(Message::System(_))
                    )
                {
                    messages.insert(0, system);
                }
                let insert_at = usize::from(matches!(
                    messages.first().map(|(_, message)| message),
                    Some(Message::System(_))
                ));
                messages.insert(
                    insert_at,
                    (
                        format!("compaction:{}", entry.id()),
                        compaction_message(summary),
                    ),
                );
                continue;
            }
            if let Some(summary) = entry.branch_summary() {
                messages.push((entry.id().to_owned(), branch_summary_message(summary)));
                continue;
            }
            if let Some((target_id, replacement)) = entry.context_edit() {
                let Some(index) = messages
                    .iter()
                    .position(|(message_id, _)| message_id == target_id)
                else {
                    continue;
                };
                match replacement {
                    Some(content) => replace_message_content(&mut messages[index].1, content),
                    None => {
                        messages.remove(index);
                    }
                }
            }
        }

        messages.into_iter().map(|(_, message)| message).collect()
    }

    /// Estimates the token count of the current projection.
    ///
    /// This is the documented `chars / 4` estimate with images counted as 4,800
    /// characters (`CTX-002`). It is never used for billing.
    #[must_use]
    pub fn estimate_tokens(&self) -> u64 {
        self.projection().iter().map(estimate_message_tokens).sum()
    }

    /// Returns the estimated context usage as a percentage of
    /// `context_window`, capped at 100.
    #[must_use]
    pub fn context_percent(&self, context_window: u64) -> u8 {
        if context_window == 0 {
            return 0;
        }
        let percent = self
            .estimate_tokens()
            .saturating_mul(100)
            .checked_div(context_window)
            .unwrap_or(0);
        u8::try_from(percent.min(100)).unwrap_or(100)
    }

    /// Returns the most recent provider usage on the branch, if any.
    ///
    /// Message usage wins over standalone usage entries because it is recorded
    /// later for each assistant turn.
    #[must_use]
    pub fn last_usage(&self) -> Option<Usage> {
        let mut usage = None;
        for entry in self.branch() {
            if let Some(message) = entry.message()
                && let Message::Assistant(assistant) = message
                && !assistant.usage.is_zero()
            {
                usage = Some(assistant.usage.clone());
            }
            if let Entry::Usage {
                usage: entry_usage, ..
            } = entry
            {
                usage = Some(entry_usage.clone());
            }
        }
        usage
    }

    /// Writes the canonical header and every entry as JSONL.
    ///
    /// Export is read-only and never changes the backing file.
    pub fn export(&self, mut writer: impl Write) -> Result<()> {
        let header = encode_line(&self.header, &self.path)?;
        writer
            .write_all(header.as_bytes())
            .map_err(|error| Error::session(format!("cannot export session: {error}")))?;
        for entry in &self.entries {
            let line = encode_line(entry, &self.path)?;
            writer
                .write_all(line.as_bytes())
                .map_err(|error| Error::session(format!("cannot export session: {error}")))?;
        }
        writer
            .flush()
            .map_err(|error| Error::session(format!("cannot export session: {error}")))
    }

    fn append_session_info(&mut self, name: Option<String>) -> Result<()> {
        let id = self.next_id();
        let entry = Entry::SessionInfo {
            id,
            parent_id: self.leaf.clone(),
            timestamp: rfc3339_now(),
            name,
        };
        self.commit(entry)
    }

    /// Returns the branch from the current leaf back to its root.
    fn chain(&self) -> Vec<&Entry> {
        let by_id: HashMap<&str, &Entry> = self
            .entries
            .iter()
            .map(|entry| (entry.id(), entry))
            .collect();
        let mut chain = Vec::new();
        let mut cursor = self.leaf.as_deref();
        while let Some(id) = cursor {
            let Some(entry) = by_id.get(id).copied() else {
                break;
            };
            chain.push(entry);
            if chain.len() > self.entries.len() {
                // A cyclic parent chain from a corrupt file must not loop.
                break;
            }
            cursor = entry.parent_id();
        }
        chain
    }

    /// Commits an entry, writing it to disk first and then updating memory.
    fn commit(&mut self, entry: Entry) -> Result<()> {
        if !self.persisted {
            if !matches!(entry, Entry::Message { .. }) {
                // SESS-002: metadata entries are buffered until the first
                // message, then flushed with the header.
                self.leaf = Some(entry.id().to_string());
                self.entries.push(entry);
                return Ok(());
            }
            self.persist_from_scratch(&entry)?;
            self.leaf = Some(entry.id().to_string());
            self.entries.push(entry);
            self.persisted = true;
            return Ok(());
        }

        let line = encode_line(&entry, &self.path)?;
        let mut file = OpenOptions::new()
            .append(true)
            .open(&self.path)
            .map_err(|error| Error::io(&self.path, error))?;
        file.write_all(line.as_bytes())
            .and_then(|()| file.flush())
            .map_err(|error| Error::io(&self.path, error))?;

        self.leaf = Some(entry.id().to_string());
        self.entries.push(entry);
        Ok(())
    }

    /// Creates the file with the header followed by every buffered entry and the
    /// message that triggers persistence.
    fn persist_from_scratch(&self, entry: &Entry) -> Result<()> {
        let mut payload = encode_line(&self.header, &self.path)?;
        for buffered in &self.entries {
            payload.push_str(&encode_line(buffered, &self.path)?);
        }
        payload.push_str(&encode_line(entry, &self.path)?);

        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&self.path)
            .map_err(|error| Error::io(&self.path, error))?;
        if let Err(error) = file
            .write_all(payload.as_bytes())
            .and_then(|()| file.flush())
        {
            drop(file);
            // The file did not exist before this call, so removing a partial
            // write is safe and leaves no half-created session behind.
            let _ = fs::remove_file(&self.path);
            return Err(Error::io(&self.path, error));
        }
        Ok(())
    }

    /// Writes a new file containing the header and `entries`.
    ///
    /// Used by fork and clone, which never touch the source file.
    fn persist_entries(&self, entries: &[Entry]) -> Result<()> {
        let mut payload = encode_line(&self.header, &self.path)?;
        for entry in entries {
            payload.push_str(&encode_line(entry, &self.path)?);
        }

        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&self.path)
            .map_err(|error| Error::io(&self.path, error))?;
        if let Err(error) = file
            .write_all(payload.as_bytes())
            .and_then(|()| file.flush())
        {
            drop(file);
            let _ = fs::remove_file(&self.path);
            return Err(Error::io(&self.path, error));
        }
        Ok(())
    }

    /// Generates an eight-character lowercase hexadecimal entry identifier.
    fn next_id(&mut self) -> String {
        loop {
            self.counter = self.counter.wrapping_add(1);
            let seed = unix_nanos() ^ self.counter.wrapping_mul(0x9E37_79B9_7F4A_7C15);
            let mixed = mix(seed);
            let id = format!("{:08x}", (mixed >> 32) as u32);
            if self.entries.iter().all(|entry| entry.id() != id) {
                return id;
            }
        }
    }
}

/// Builds the user message that represents a compaction summary.
fn compaction_message(summary: &str) -> Message {
    synthetic_user_message(format!(
        "Summary of the conversation so far:\n\n{}",
        summary.trim()
    ))
}

/// Builds the user message that represents an abandoned branch.
fn branch_summary_message(summary: &str) -> Message {
    synthetic_user_message(format!(
        "Summary of a previously explored branch:\n\n{}",
        summary.trim()
    ))
}

/// Builds a deterministic user message for a synthesized entry.
fn synthetic_user_message(text: String) -> Message {
    Message::User(codie_core::UserMessage {
        content: vec![Content::text(text)],
        timestamp: 0,
    })
}

/// Replaces the content of a message in place, preserving its role.
fn replace_message_content(message: &mut Message, replacement: &Content) {
    match message {
        Message::System(system) => {
            system.content = replacement
                .as_text()
                .map_or_else(|| replacement_text(replacement), str::to_owned);
        }
        Message::User(user) => user.content = vec![replacement.clone()],
        Message::Assistant(assistant) => assistant.content = vec![replacement.clone()],
        Message::ToolResult(result) => result.content = vec![replacement.clone()],
    }
}

/// Renders non-text replacement content for system messages.
fn replacement_text(content: &Content) -> String {
    match content {
        Content::Text { text, .. } => text.clone(),
        Content::Thinking { thinking, .. } => thinking.clone(),
        Content::ToolCall {
            name, arguments, ..
        } => format!("{name} {arguments}"),
        Content::Image { mime_type, .. } => format!("[image {mime_type}]"),
    }
}

/// Returns the short kind name of an entry.
fn entry_kind(entry: &Entry) -> &'static str {
    match entry {
        Entry::Message { .. } => "message",
        Entry::ModelChange { .. } => "model_change",
        Entry::ThinkingLevelChange { .. } => "thinking_level_change",
        Entry::SessionInfo { .. } => "session_info",
        Entry::Usage { .. } => "usage",
        Entry::Compaction { .. } => "compaction",
        Entry::BranchSummary { .. } => "branch_summary",
        Entry::ContextEdit { .. } => "context_edit",
        Entry::Label { .. } => "label",
        Entry::Custom { .. } => "custom",
        Entry::Leaf { .. } => "leaf",
    }
}

/// Returns a one-line description for the tree view.
fn describe_entry(entry: &Entry) -> String {
    match entry {
        Entry::Message { message, .. } => match message {
            Message::System(_) => "system prompt".to_owned(),
            Message::User(user) => truncate_preview(&plain_text(&user.content, "user")),
            Message::Assistant(assistant) => {
                let text = assistant.text();
                if text.is_empty() {
                    let calls = assistant.tool_calls();
                    if calls.is_empty() {
                        "assistant".to_owned()
                    } else {
                        format!(
                            "assistant → {}",
                            calls
                                .iter()
                                .map(|call| call.name.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        )
                    }
                } else {
                    truncate_preview(&text)
                }
            }
            Message::ToolResult(result) => {
                format!("tool result · {}", result.tool_name)
            }
        },
        Entry::ModelChange {
            provider, model_id, ..
        } => format!("model → {provider}/{model_id}"),
        Entry::ThinkingLevelChange { thinking_level, .. } => {
            format!("thinking → {}", thinking_level.as_str())
        }
        Entry::SessionInfo { name, .. } => name.as_ref().map_or_else(
            || "name cleared".to_owned(),
            |name| format!("name → {name}"),
        ),
        Entry::Usage { kind, .. } => format!("usage · {kind}"),
        Entry::Compaction { summary, .. } => {
            format!("compaction · {}", truncate_preview(summary))
        }
        Entry::BranchSummary { summary, .. } => {
            format!("branch summary · {}", truncate_preview(summary))
        }
        Entry::ContextEdit { .. } => "context edit".to_owned(),
        Entry::Label { label, .. } => label.as_ref().map_or_else(
            || "label cleared".to_owned(),
            |label| format!("label → {label}"),
        ),
        Entry::Custom { custom_type, .. } => format!("custom · {custom_type}"),
        Entry::Leaf { target_id, .. } => target_id
            .as_ref()
            .map_or_else(|| "empty branch".to_owned(), |id| format!("leaf → {id}")),
    }
}

/// Joins text content for a preview.
fn plain_text(content: &[Content], fallback: &str) -> String {
    let text = content
        .iter()
        .filter_map(Content::as_text)
        .collect::<Vec<_>>()
        .join("");
    if text.is_empty() {
        fallback.to_owned()
    } else {
        text
    }
}

/// Truncates a preview to 72 characters on a character boundary.
fn truncate_preview(text: &str) -> String {
    let flattened = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flattened.chars().count() <= 72 {
        return flattened;
    }
    let mut preview: String = flattened.chars().take(69).collect();
    preview.push_str("...");
    preview
}

/// Estimates tokens for one message with the documented `chars / 4` rule.
fn estimate_message_tokens(message: &Message) -> u64 {
    let chars = match message {
        Message::System(system) => system.content.chars().count() as u64,
        Message::User(user) => message_chars(&user.content),
        Message::Assistant(assistant) => message_chars(&assistant.content),
        Message::ToolResult(result) => message_chars(&result.content),
    };
    chars.div_ceil(4)
}

/// Counts characters across content blocks, counting images as 4,800.
fn message_chars(content: &[Content]) -> u64 {
    content
        .iter()
        .map(|content| match content {
            Content::Text { text, .. } => text.chars().count() as u64,
            Content::Thinking { thinking, .. } => thinking.chars().count() as u64,
            Content::ToolCall { arguments, .. } => arguments.to_string().chars().count() as u64,
            Content::Image { .. } => 4_800,
        })
        .sum()
}

/// Truncates a 64-bit mixed value to an identifier-sized value.
const fn mix(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9E37_79B9_7F4A_7C15);
    value = (value ^ (value >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    value ^ (value >> 31)
}

/// Generates a timestamp-based session identifier.
fn new_session_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQUENCE: AtomicU64 = AtomicU64::new(0);

    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let nanos = unix_nanos();
    format!("{nanos:016x}-{sequence:04x}")
}

/// Serializes a value as one newline-terminated JSON line.
fn encode_line<T: serde::Serialize>(value: &T, path: &Path) -> Result<String> {
    let mut line = serde_json::to_string(value).map_err(|error| {
        Error::session(format!(
            "failed to encode session entry for {}: {error}",
            path.display()
        ))
    })?;
    line.push('\n');
    Ok(line)
}

/// Reads only the header line of a session file, ignoring failures.
fn read_header(path: &Path) -> Option<SessionHeader> {
    let file = File::open(path).ok()?;
    let mut line = String::new();
    BufReader::new(file).read_line(&mut line).ok()?;
    if line.trim().is_empty() {
        return None;
    }
    serde_json::from_str(line.trim_end()).ok()
}

/// Returns true when a stored cwd string names the same directory as `cwd`.
fn cwd_matches(stored: &str, cwd: &Path) -> bool {
    let stored = Path::new(stored);
    if stored == cwd {
        return true;
    }
    match (stored.canonicalize(), cwd.canonicalize()) {
        (Ok(stored), Ok(cwd)) => stored == cwd,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixing_produces_distinct_values() {
        assert_ne!(mix(1), mix(2));
        assert_ne!(mix(0), 0);
    }

    #[test]
    fn session_ids_are_unique() {
        let first = new_session_id();
        let second = new_session_id();
        assert_ne!(first, second);
        assert!(first.chars().all(|c| c.is_ascii_hexdigit() || c == '-'));
    }

    #[test]
    fn identifiers_are_eight_lowercase_hex_characters() {
        let mut session = Session::create(".", ".").expect("create");
        // Avoid leaving a session file behind; identifiers can be generated
        // without persistence.
        let id = session.next_id();
        assert_eq!(id.len(), 8);
        assert!(
            id.chars()
                .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
        );
    }
}
