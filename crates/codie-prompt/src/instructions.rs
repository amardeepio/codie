//! `SYSTEM.md` and `APPEND_SYSTEM.md` discovery.

use std::fs;
use std::path::{Path, PathBuf};

use crate::ContextFile;

/// System instruction files discovered for a working directory.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Instructions {
    /// Nearest `SYSTEM.md`, replacing the default preamble.
    pub system: Option<ContextFile>,
    /// Every `APPEND_SYSTEM.md`, topmost ancestor first.
    pub appends: Vec<ContextFile>,
}

/// Discovers and loads system instruction files between `cwd` and the root.
///
/// The nearest `SYSTEM.md` wins; all `APPEND_SYSTEM.md` files are retained in
/// root-to-leaf order. Unreadable files are skipped.
#[must_use]
pub fn load_instructions(cwd: &Path) -> Instructions {
    let cwd = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
    let mut directories: Vec<&Path> = cwd.ancestors().collect();
    directories.reverse();

    let mut instructions = Instructions::default();
    for directory in directories {
        if let Some(file) = read_context_file(&directory.join("SYSTEM.md")) {
            instructions.system = Some(file);
        }
        if let Some(file) = read_context_file(&directory.join("APPEND_SYSTEM.md")) {
            instructions.appends.push(file);
        }
    }
    instructions
}

fn read_context_file(path: &Path) -> Option<ContextFile> {
    if !path.is_file() {
        return None;
    }
    let content = fs::read_to_string(path).ok()?;
    Some(ContextFile {
        path: path.to_path_buf(),
        content: strip_bom(&content).to_owned(),
    })
}

fn strip_bom(content: &str) -> &str {
    content.strip_prefix('\u{feff}').unwrap_or(content)
}

/// Joins append blocks and one optional CLI override into a single string.
#[must_use]
pub fn combine_appends(appends: &[ContextFile], cli: Option<&str>) -> Option<String> {
    let mut parts = appends
        .iter()
        .map(|file| file.content.trim())
        .filter(|content| !content.is_empty())
        .collect::<Vec<_>>();
    if let Some(cli) = cli.map(str::trim).filter(|cli| !cli.is_empty()) {
        parts.push(cli);
    }
    (!parts.is_empty()).then(|| parts.join("\n\n"))
}

impl Instructions {
    /// Returns the append instructions joined with an optional CLI override.
    #[must_use]
    pub fn combined_append(&self, cli: Option<&str>) -> Option<String> {
        combine_appends(&self.appends, cli)
    }

    /// Returns the preamble, preferring the CLI override over `SYSTEM.md`.
    #[must_use]
    pub fn preamble(&self, cli: Option<&str>) -> Option<String> {
        cli.map(str::to_owned)
            .or_else(|| self.system.as_ref().map(|file| file.content.clone()))
    }

    /// Returns every loaded instruction path.
    #[must_use]
    pub fn paths(&self) -> Vec<&PathBuf> {
        self.system
            .iter()
            .chain(self.appends.iter())
            .map(|file| &file.path)
            .collect()
    }
}
