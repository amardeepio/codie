//! Test harnesses, fixtures, and the specification attribute for codie.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod harness;

use std::io::Error;
use std::path::{Component, Path, PathBuf};

use tempfile::TempDir;

pub use codie_spec_macro::spec;
pub use insta::{assert_json_snapshot, assert_snapshot};

/// An isolated home directory for tests.
///
/// The harness never mutates the process environment, which keeps tests safe
/// under parallel execution. Pass [`TempHome::envs`] to child processes
/// instead, usually via [`TempHome::apply`] or [`harness::run_cli`].
#[derive(Debug)]
pub struct TempHome {
    root: TempDir,
}

impl TempHome {
    /// Creates an isolated home with config, data, cache, and session folders.
    pub fn new() -> std::io::Result<Self> {
        let root = tempfile::tempdir()?;
        for dir in ["config", "data", "cache", "sessions"] {
            std::fs::create_dir_all(root.path().join(dir))?;
        }
        Ok(Self { root })
    }

    /// Returns the root of the temporary home.
    #[must_use]
    pub fn path(&self) -> &Path {
        self.root.path()
    }

    /// Returns the config directory.
    #[must_use]
    pub fn config_dir(&self) -> PathBuf {
        self.root.path().join("config")
    }

    /// Returns the data directory.
    #[must_use]
    pub fn data_dir(&self) -> PathBuf {
        self.root.path().join("data")
    }

    /// Returns the cache directory.
    #[must_use]
    pub fn cache_dir(&self) -> PathBuf {
        self.root.path().join("cache")
    }

    /// Returns the session directory.
    #[must_use]
    pub fn session_dir(&self) -> PathBuf {
        self.root.path().join("sessions")
    }

    /// Returns environment variables that point codie at this home.
    #[must_use]
    pub fn envs(&self) -> Vec<(String, PathBuf)> {
        vec![
            ("HOME".to_owned(), self.root.path().to_path_buf()),
            ("CODIE_CONFIG_DIR".to_owned(), self.config_dir()),
            ("CODIE_DATA_DIR".to_owned(), self.data_dir()),
            ("CODIE_CACHE_DIR".to_owned(), self.cache_dir()),
            ("CODIE_SESSION_DIR".to_owned(), self.session_dir()),
        ]
    }

    /// Applies this home's environment to a child process.
    pub fn apply(&self, command: &mut std::process::Command) {
        for (key, value) in self.envs() {
            command.env(key, value);
        }
    }

    /// Writes `contents` to a path relative to the home root.
    ///
    /// Parent directories are created. Absolute paths and paths containing
    /// `..` are rejected so a test cannot escape its temporary home.
    ///
    /// # Errors
    ///
    /// Returns an error for unsafe paths or filesystem failures.
    pub fn write_file(
        &self,
        relative: impl AsRef<Path>,
        contents: impl AsRef<[u8]>,
    ) -> std::io::Result<PathBuf> {
        let path = self.resolve(relative.as_ref())?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, contents)?;
        Ok(path)
    }

    /// Writes `contents` under the home's config directory.
    ///
    /// This is a convenience wrapper around [`TempHome::write_file`] for the
    /// common `config/config.toml` case.
    ///
    /// # Errors
    ///
    /// Returns an error for unsafe paths or filesystem failures.
    pub fn write_config(
        &self,
        relative: impl AsRef<Path>,
        contents: impl AsRef<[u8]>,
    ) -> std::io::Result<PathBuf> {
        self.write_file(Path::new("config").join(relative), contents)
    }

    fn resolve(&self, relative: &Path) -> std::io::Result<PathBuf> {
        let unsafe_path = relative.is_absolute()
            || relative
                .components()
                .any(|component| matches!(component, Component::ParentDir));
        if unsafe_path {
            return Err(Error::new(
                std::io::ErrorKind::InvalidInput,
                "path must be relative and stay inside the temp home",
            ));
        }
        Ok(self.root.path().join(relative))
    }
}

/// A deterministic scripted Server-Sent Events response.
#[derive(Debug, Default, Clone)]
pub struct SseScript {
    frames: Vec<String>,
}

impl SseScript {
    /// Creates an empty script.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends one `data:` frame.
    #[must_use]
    pub fn data(mut self, payload: impl Into<String>) -> Self {
        self.frames.push(format!("data: {}\n\n", payload.into()));
        self
    }

    /// Appends one raw frame without a trailing blank line.
    ///
    /// Useful for malformed-input and keepalive tests.
    #[must_use]
    pub fn raw(mut self, frame: impl Into<String>) -> Self {
        self.frames.push(frame.into());
        self
    }

    /// Appends the terminal `[DONE]` frame.
    #[must_use]
    pub fn done(mut self) -> Self {
        self.frames.push("data: [DONE]\n\n".to_owned());
        self
    }

    /// Returns the complete HTTP body.
    #[must_use]
    pub fn body(&self) -> String {
        self.frames.concat()
    }

    /// Splits the body into chunks using the provided sizes, cycling when the
    /// script is longer than the size list.
    ///
    /// Chunks are byte-oriented so tests can split in the middle of UTF-8
    /// sequences and verify reassembly.
    #[must_use]
    pub fn chunk_bytes(&self, sizes: &[usize]) -> Vec<Vec<u8>> {
        let body = self.body().into_bytes();
        if sizes.is_empty() || body.is_empty() {
            return vec![body];
        }

        let mut chunks = Vec::new();
        let mut offset = 0;
        let mut index = 0;
        while offset < body.len() {
            let size = sizes[index % sizes.len()].max(1);
            let end = (offset + size).min(body.len());
            chunks.push(body[offset..end].to_vec());
            offset = end;
            index += 1;
        }
        chunks
    }
}

/// A deterministic, transport-free stand-in for a real provider.
///
/// The provider owns a model identifier and an [`SseScript`]. [`FakeProvider::body`]
/// and [`FakeProvider::chunks`] return exactly what an HTTP layer would deliver,
/// so SSE reassembly can be tested without networking. Provider trait
/// integration with `codie-ai` arrives in a later phase.
#[derive(Debug, Clone)]
pub struct FakeProvider {
    model: String,
    script: SseScript,
}

impl FakeProvider {
    /// Creates a provider for `model` that replays `script`.
    #[must_use]
    pub fn new(model: impl Into<String>, script: SseScript) -> Self {
        Self {
            model: model.into(),
            script,
        }
    }

    /// Returns the model identifier reported by this provider.
    #[must_use]
    pub fn model(&self) -> &str {
        &self.model
    }

    /// Returns the scripted response.
    #[must_use]
    pub fn script(&self) -> &SseScript {
        &self.script
    }

    /// Returns the complete response body.
    #[must_use]
    pub fn body(&self) -> String {
        self.script.body()
    }

    /// Splits the response body into chunks using the provided sizes.
    #[must_use]
    pub fn chunks(&self, sizes: &[usize]) -> Vec<Vec<u8>> {
        self.script.chunk_bytes(sizes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_script_assembles_frames() {
        let script = SseScript::new().data(r#"{"a":1}"#).done();
        assert!(script.body().contains("data: {\"a\":1}"));
        assert!(script.body().ends_with("data: [DONE]\n\n"));
    }

    #[test]
    fn raw_frames_are_appended_verbatim() {
        let script = SseScript::new().raw(": keepalive\n\n").data("{}").done();
        assert!(script.body().starts_with(": keepalive\n\n"));
    }

    #[test]
    fn chunking_round_trips() {
        let script = SseScript::new().data("hello").done();
        let body = script.body();
        let chunks = script.chunk_bytes(&[1, 2, 3]);
        let joined: Vec<u8> = chunks.into_iter().flatten().collect();
        assert_eq!(String::from_utf8(joined).unwrap(), body);
    }

    #[test]
    fn chunking_cycles_requested_sizes() {
        let script = SseScript::new().data("abcdef").done();
        let chunks = script.chunk_bytes(&[2, 3]);
        assert!(chunks.len() > 2);
        for (index, chunk) in chunks.iter().enumerate() {
            let requested = if index % 2 == 0 { 2 } else { 3 };
            assert!(chunk.len() <= requested, "chunk {index} exceeds its size");
            if index + 1 < chunks.len() {
                assert_eq!(chunk.len(), requested);
            }
        }
    }

    #[test]
    fn empty_sizes_return_one_chunk() {
        let script = SseScript::new().data("hello").done();
        let chunks = script.chunk_bytes(&[]);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], script.body().into_bytes());
    }

    #[test]
    fn utf8_body_can_be_split_at_every_byte() {
        let script = SseScript::new().data("héllo—世界 🦀").done();
        let body = script.body().into_bytes();

        for split in 1..body.len() {
            let chunks = script.chunk_bytes(&[split]);
            let joined: Vec<u8> = chunks.iter().flatten().copied().collect();
            assert_eq!(
                joined, body,
                "reassembly failed when splitting with size {split}"
            );
            assert_eq!(
                String::from_utf8(joined).unwrap(),
                script.body(),
                "utf8 reassembly failed when splitting with size {split}"
            );
            assert!(
                chunks.len() >= 2,
                "expected multiple chunks for size {split}"
            );
        }
    }

    #[test]
    fn fake_provider_exposes_script_and_model() {
        let provider = FakeProvider::new(
            "deepseek/deepseek-flash",
            SseScript::new().data(r#"{"choices":[]}"#).done(),
        );
        assert_eq!(provider.model(), "deepseek/deepseek-flash");
        assert!(provider.body().contains("[DONE]"));
        let joined: Vec<u8> = provider.chunks(&[5]).into_iter().flatten().collect();
        assert_eq!(joined, provider.body().into_bytes());
        assert_eq!(provider.script().body(), provider.body());
    }

    #[test]
    fn temp_home_creates_expected_layout() {
        let home = TempHome::new().unwrap();
        assert!(home.config_dir().is_dir());
        assert!(home.data_dir().is_dir());
        assert!(home.cache_dir().is_dir());
        assert!(home.session_dir().is_dir());
        assert_eq!(home.envs().len(), 5);
    }

    #[test]
    fn write_helpers_create_parent_directories() {
        let home = TempHome::new().unwrap();
        let config = home
            .write_config("config.toml", "default_model = \"test/model\"\n")
            .unwrap();
        assert_eq!(config, home.config_dir().join("config.toml"));
        assert!(config.is_file());

        let nested = home.write_file("data/nested/file.txt", "hello").unwrap();
        assert_eq!(nested, home.data_dir().join("nested/file.txt"));
        assert_eq!(std::fs::read_to_string(nested).unwrap(), "hello");
    }

    #[test]
    fn write_helpers_reject_escaping_paths() {
        let home = TempHome::new().unwrap();
        assert!(home.write_file("../escape.txt", "nope").is_err());
        assert!(home.write_file("/tmp/escape.txt", "nope").is_err());
    }

    #[test]
    fn cli_binary_is_resolved_to_a_non_empty_path() {
        let binary = harness::cli_binary();
        assert!(!binary.as_os_str().is_empty());
    }
}
