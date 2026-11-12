//! Shared helpers for the read-only search tools (`grep`, `find`, `ls`).

use std::path::{Path, PathBuf};

use serde_json::Value;

/// Default number of matches returned by `grep`.
pub(crate) const GREP_DEFAULT_MAX_RESULTS: usize = 100;

/// Upper bound accepted for `grep.max_results`.
pub(crate) const GREP_MAX_RESULTS: usize = 1_000;

/// Default number of paths returned by `find`.
pub(crate) const FIND_DEFAULT_MAX_RESULTS: usize = 1_000;

/// Upper bound accepted for `find.max_results`.
pub(crate) const FIND_MAX_RESULTS: usize = 5_000;

/// Files larger than this are skipped by content search.
pub(crate) const MAX_SEARCH_FILE_BYTES: u64 = 4 * 1024 * 1024;

/// Builds a deterministic, `.gitignore`-aware walker rooted at `root`.
pub(crate) fn walker(root: &Path) -> ignore::Walk {
    ignore::WalkBuilder::new(root)
        .hidden(true)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .follow_links(false)
        .sort_by_file_name(|left, right| left.cmp(right))
        .build()
}

/// Formats `path` relative to `cwd` when possible, with `/` separators.
pub(crate) fn display_path(path: &Path, cwd: &Path) -> String {
    let relative = path.strip_prefix(cwd).unwrap_or(path);
    if relative.as_os_str().is_empty() {
        return ".".to_owned();
    }
    relative.to_string_lossy().replace('\\', "/")
}

/// Returns `path` relative to `root` as a slash-separated string, if non-empty.
pub(crate) fn relative_to(path: &Path, root: &Path) -> Option<String> {
    let relative = path.strip_prefix(root).ok()?;
    if relative.as_os_str().is_empty() {
        return None;
    }
    Some(relative.to_string_lossy().replace('\\', "/"))
}

/// Reads an optional boolean argument.
pub(crate) fn optional_bool(arguments: &Value, name: &str) -> Result<Option<bool>, String> {
    match arguments.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_bool()
            .map(Some)
            .ok_or_else(|| format!("{name} must be a boolean")),
    }
}

/// Reads an optional positive integer argument clamped to `maximum`.
pub(crate) fn optional_positive_integer(
    arguments: &Value,
    name: &str,
    maximum: usize,
) -> Result<Option<usize>, String> {
    match arguments.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_u64()
            .and_then(|number| usize::try_from(number).ok())
            .filter(|number| *number > 0)
            .map(|number| Some(number.min(maximum)))
            .ok_or_else(|| format!("{name} must be a positive integer")),
    }
}

/// Reads an optional string argument, rejecting non-string values.
pub(crate) fn optional_string(arguments: &Value, name: &str) -> Result<Option<String>, String> {
    match arguments.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_str()
            .map(|text| Some(text.to_owned()))
            .ok_or_else(|| format!("{name} must be a string")),
    }
}

/// Resolves `path` relative to `cwd`, defaulting to `cwd` itself.
pub(crate) fn resolve_root(raw: Option<&str>, cwd: &Path) -> PathBuf {
    raw.map_or_else(
        || cwd.to_path_buf(),
        |value| crate::path::resolve_path(value, cwd),
    )
}

/// Builds a truncation notice for capped result sets.
pub(crate) fn cap_notice(limit: usize, noun: &str) -> String {
    format!("\n\n[Reached the {limit}-{noun} limit. Narrow the pattern or path to see more.]")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn display_path_is_relative_and_slash_separated() {
        let cwd = Path::new("/tmp/project");
        assert_eq!(display_path(&cwd.join("src/main.rs"), cwd), "src/main.rs");
        assert_eq!(display_path(cwd, cwd), ".");
        assert_eq!(display_path(Path::new("/etc/hosts"), cwd), "/etc/hosts");
    }

    #[test]
    fn optional_values_are_validated() {
        let arguments = json!({"all": true, "limit": 5, "name": "x"});
        assert_eq!(optional_bool(&arguments, "all"), Ok(Some(true)));
        assert_eq!(optional_bool(&arguments, "missing"), Ok(None));
        assert_eq!(
            optional_positive_integer(&arguments, "limit", 10),
            Ok(Some(5))
        );
        assert_eq!(
            optional_positive_integer(&arguments, "limit", 1),
            Ok(Some(1))
        );
        assert_eq!(
            optional_string(&arguments, "name"),
            Ok(Some("x".to_owned()))
        );
        assert!(optional_bool(&json!({"all": "yes"}), "all").is_err());
        assert!(optional_positive_integer(&json!({"limit": 0}), "limit", 10).is_err());
        assert!(optional_string(&json!({"name": 1}), "name").is_err());
    }
}
