//! Project trust: `.codie/` resources load only after approval.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// A per-user list of trusted project directories.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrustStore {
    path: PathBuf,
    trusted: Vec<PathBuf>,
}

impl TrustStore {
    /// Loads `trust.toml` from `config_dir`, failing closed on errors.
    #[must_use]
    pub fn load(config_dir: &Path) -> Self {
        let path = config_dir.join("trust.toml");
        let trusted = fs::read_to_string(&path)
            .ok()
            .and_then(|contents| toml::from_str::<toml::Value>(&contents).ok())
            .and_then(|value| {
                value
                    .get("trusted")
                    .and_then(toml::Value::as_array)
                    .cloned()
            })
            .map(|entries| {
                entries
                    .iter()
                    .filter_map(toml::Value::as_str)
                    .map(|entry| canonicalize(Path::new(entry)))
                    .collect()
            })
            .unwrap_or_default();
        Self { path, trusted }
    }

    /// Returns the trust file path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns true when `project` or one of its ancestors was approved.
    #[must_use]
    pub fn is_trusted(&self, project: &Path) -> bool {
        let project = canonicalize(project);
        self.trusted
            .iter()
            .any(|entry| project == *entry || project.starts_with(entry))
    }

    /// Approves `project`, persisting the canonical path atomically.
    pub fn approve(&mut self, project: &Path) -> io::Result<()> {
        let project = canonicalize(project);
        if self.trusted.contains(&project) {
            return Ok(());
        }
        self.trusted.push(project);
        self.trusted.sort();
        self.persist()
    }

    /// Returns true when a project contains any `.codie/` resource we load.
    #[must_use]
    pub fn has_project_resources(project: &Path) -> bool {
        let codie = project.join(".codie");
        codie.join("skills").is_dir()
            || codie.join("templates").is_dir()
            || codie.join("extensions").is_dir()
            || codie.join("SYSTEM.md").is_file()
            || codie.join("APPEND_SYSTEM.md").is_file()
    }

    fn persist(&self) -> io::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut rendered = String::from(
            "# codie project trust — paths approved with `codie --approve`\ntrusted = [\n",
        );
        for entry in &self.trusted {
            rendered.push_str(&format!("  \"{}\",\n", escape(&entry.to_string_lossy())));
        }
        rendered.push_str("]\n");

        let temporary = self.path.with_extension("toml.tmp");
        fs::write(&temporary, rendered)?;
        fs::rename(&temporary, &self.path)
    }
}

fn canonicalize(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

/// Escapes a path for a TOML basic string.
fn escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_are_escaped_for_toml() {
        assert_eq!(escape(r#"C:\a"b"#), r#"C:\\a\"b"#);
    }
}
