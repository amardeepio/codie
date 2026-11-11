//! Opt-in provider fixture recording (`PROV-005`, `P1-T16`).
//!
//! Recording is compiled in only with the `fixtures` feature **and** enabled at
//! runtime with `CODIE_RECORD_FIXTURES=1`. CI never records and never sends
//! credentials. Recorded files are raw SSE bodies and contain no headers.

#[cfg(feature = "fixtures")]
mod imp {
    use std::fs::File;
    use std::io::Write;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    /// Returns true when the caller explicitly opted into recording.
    #[must_use]
    pub fn enabled() -> bool {
        std::env::var_os("CODIE_RECORD_FIXTURES").is_some()
    }

    /// Appends raw transport chunks to one recorded fixture file.
    #[derive(Debug)]
    pub struct Recorder {
        file: Option<File>,
        path: Option<PathBuf>,
    }

    impl Recorder {
        /// Creates a recorder for `provider` when recording is enabled.
        #[must_use]
        pub fn from_env(provider: &str) -> Self {
            if !enabled() {
                return Self {
                    file: None,
                    path: None,
                };
            }

            let directory = std::env::var_os("CODIE_FIXTURE_DIR")
                .map_or_else(|| PathBuf::from("target/fixtures"), PathBuf::from);
            if let Err(error) = std::fs::create_dir_all(&directory) {
                tracing::warn!(
                    path = %directory.display(),
                    %error,
                    "cannot create the fixture directory; recording is disabled"
                );
                return Self {
                    file: None,
                    path: None,
                };
            }

            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |duration| duration.as_millis());
            let path = directory.join(format!("{provider}-{stamp}.sse"));
            match File::create(&path) {
                Ok(file) => Self {
                    file: Some(file),
                    path: Some(path),
                },
                Err(error) => {
                    tracing::warn!(path = %path.display(), %error, "cannot start fixture recording");
                    Self {
                        file: None,
                        path: None,
                    }
                }
            }
        }

        /// Appends one transport chunk.
        pub fn write(&mut self, bytes: &[u8]) {
            if let Some(file) = self.file.as_mut() {
                let _ = file.write_all(bytes);
            }
        }

        /// Returns the recorded file path, when recording is active.
        #[must_use]
        pub fn path(&self) -> Option<&Path> {
            self.path.as_deref()
        }
    }
}

#[cfg(not(feature = "fixtures"))]
mod imp {
    use std::path::Path;

    /// Recording is unavailable without the `fixtures` feature.
    #[must_use]
    pub fn enabled() -> bool {
        false
    }

    /// A no-op recorder used when the `fixtures` feature is disabled.
    #[derive(Debug, Default)]
    pub struct Recorder;

    impl Recorder {
        /// Returns a no-op recorder.
        #[must_use]
        pub fn from_env(_provider: &str) -> Self {
            Self
        }

        /// Discards a chunk.
        pub fn write(&mut self, _bytes: &[u8]) {}

        /// Always returns `None`.
        #[must_use]
        pub fn path(&self) -> Option<&Path> {
            None
        }
    }
}

pub use imp::{Recorder, enabled};
