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

use codie_core::{Error, Message, Result, ThinkingLevel, Usage};

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

impl Session {
    /// Creates a new, empty session rooted at `cwd`.
    ///
    /// The session directory is created eagerly. The JSONL file itself is not
    /// created until the first message is appended.
    pub fn create(cwd: impl AsRef<Path>, session_dir: impl AsRef<Path>) -> Result<Self> {
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
        if header.version < 1 {
            return Err(Error::session(format!(
                "session {} uses unsupported format version {}",
                path.display(),
                header.version
            )));
        }

        let mut entries = Vec::new();
        for line in lines {
            // Phase 1 skips lines that do not decode, including a partial final
            // line left by a crash. Phase 3 adds migration and validation.
            if let Ok(entry) = serde_json::from_str::<Entry>(line) {
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

    /// Moves the session leaf, for future tree navigation.
    ///
    /// Passing `None` clears the leaf. An unknown identifier is rejected.
    pub fn set_leaf(&mut self, entry_id: Option<&str>) -> Result<()> {
        match entry_id {
            None => {
                self.leaf = None;
                Ok(())
            }
            Some(id) => {
                if self.entries.iter().any(|entry| entry.id() == id) {
                    self.leaf = Some(id.to_string());
                    Ok(())
                } else {
                    Err(Error::session(format!("unknown session entry {id}")))
                }
            }
        }
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
    /// Phase 1 has no compaction or context-edit entries, so every message on
    /// the branch is returned.
    #[must_use]
    pub fn projection(&self) -> Vec<Message> {
        // TODO(phase-3): honor `compaction` and `context_edit` entries once
        // those variants are persisted; today the projection is the full branch.
        self.chain()
            .into_iter()
            .rev()
            .filter_map(|entry| entry.message().cloned())
            .collect()
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
