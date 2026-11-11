//! Path normalization and resolution for tool arguments.
//!
//! Paths are intentionally not sandboxed. The policy layer decides which
//! operations are allowed; this module only turns the model's spelling into a
//! filesystem path.

use std::path::{Path, PathBuf};

/// Resolves a user-supplied path against `cwd`.
///
/// The input supports:
///
/// - a leading `@` (removed),
/// - a leading `~` or `~/` (expanded with `HOME`),
/// - `file://` URLs (including `file://localhost/...` and percent escapes),
/// - Unicode space variants, normalized to ASCII spaces,
/// - relative and absolute paths.
///
/// No sandboxing is applied.
#[must_use]
pub fn resolve_path(raw: &str, cwd: impl AsRef<Path>) -> PathBuf {
    let mut value = normalize_unicode_spaces(raw);
    if let Some(stripped) = value.strip_prefix('@') {
        value = stripped.to_owned();
    }

    let expanded = if value == "~" {
        home_dir().unwrap_or_else(|| PathBuf::from(value))
    } else if let Some(rest) = value
        .strip_prefix("~/")
        .or_else(|| value.strip_prefix("~\\"))
    {
        home_dir().map_or_else(|| PathBuf::from(&value), |home| home.join(rest))
    } else if let Some(url_path) = file_url_path(&value) {
        url_path
    } else {
        PathBuf::from(value)
    };

    if expanded.is_absolute() {
        expanded
    } else {
        cwd.as_ref().join(expanded)
    }
}

/// Returns the current user's home directory from the environment.
fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// Replaces Unicode space variants with an ASCII space.
fn normalize_unicode_spaces(input: &str) -> String {
    input
        .chars()
        .map(|character| match character {
            '\u{00A0}' | '\u{2000}'..='\u{200A}' | '\u{202F}' | '\u{205F}' | '\u{3000}' => ' ',
            other => other,
        })
        .collect()
}

/// Converts a `file://` URL into a path when the input has that scheme.
fn file_url_path(input: &str) -> Option<PathBuf> {
    let rest = input.strip_prefix("file://")?;
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    Some(PathBuf::from(percent_decode(rest)))
}

/// Decodes percent escapes as UTF-8, leaving malformed escapes untouched.
fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && index + 2 < bytes.len()
            && let (Some(high), Some(low)) =
                (hex_value(bytes[index + 1]), hex_value(bytes[index + 2]))
        {
            output.push((high << 4) | low);
            index += 3;
        } else {
            output.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8_lossy(&output).into_owned()
}

/// Returns the value of one hexadecimal digit.
fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_paths_resolve_against_cwd() {
        assert_eq!(
            resolve_path("src/main.rs", "/tmp/project"),
            PathBuf::from("/tmp/project/src/main.rs")
        );
    }

    #[test]
    fn absolute_paths_are_unchanged() {
        assert_eq!(
            resolve_path("/etc/hosts", "/tmp"),
            PathBuf::from("/etc/hosts")
        );
    }

    #[test]
    fn at_prefix_is_removed() {
        assert_eq!(
            resolve_path("@notes.txt", "/tmp"),
            PathBuf::from("/tmp/notes.txt")
        );
    }

    #[test]
    fn unicode_spaces_are_normalized() {
        assert_eq!(
            resolve_path("my\u{00A0}file.txt", "/tmp"),
            PathBuf::from("/tmp/my file.txt")
        );
        assert_eq!(
            resolve_path("my\u{202F}file.txt", "/tmp"),
            PathBuf::from("/tmp/my file.txt")
        );
    }

    #[test]
    fn file_urls_become_paths() {
        assert_eq!(
            resolve_path("file:///tmp/a%20b.txt", "/"),
            PathBuf::from("/tmp/a b.txt")
        );
        assert_eq!(
            resolve_path("file://localhost/tmp/c.txt", "/"),
            PathBuf::from("/tmp/c.txt")
        );
    }

    #[test]
    fn tilde_expands_with_home() {
        if let Some(home) = home_dir() {
            assert_eq!(resolve_path("~/notes.txt", "/tmp"), home.join("notes.txt"));
            assert_eq!(resolve_path("~", "/tmp"), home);
        }
    }
}
