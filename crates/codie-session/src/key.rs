//! Filesystem-safe keys for per-project session directories.
//!
//! A session directory name is derived from the absolute working directory. The
//! key must stay stable so existing sessions keep resolving, and it must be a
//! legal single path component on every platform — in particular on Windows,
//! where `?`, `*`, `"`, `<`, `>`, `|`, control characters, and the verbatim
//! `\\?\` prefix are either invalid or leak into names.

use std::borrow::Cow;

/// Longest key, in characters, kept before truncating and hashing.
///
/// Windows limits a path component to 255 UTF-16 code units. The key is wrapped
/// as `--{key}--`, and a truncated key gains a `-` plus sixteen hex digits. A
/// hundred characters can occupy at most 200 UTF-16 units even when every
/// character is outside the BMP, leaving margin under the limit.
const MAX_KEY_CHARS: usize = 100;

/// Returns a stable, filesystem-safe session directory key for `cwd`.
///
/// Existing POSIX keys are preserved: `/Users/me/project` becomes
/// `Users-me-project`. Windows verbatim prefixes (`\\?\` and `\\?\UNC\`) are
/// removed, path separators and drive colons become `-`, characters Windows
/// forbids in file names are percent-encoded (`?` becomes `%3F`), and overlong
/// keys are truncated with a stable hash suffix.
#[must_use]
pub fn session_dir_key(cwd: &str) -> String {
    let cwd = strip_verbatim(cwd);
    let mut key = String::with_capacity(cwd.len());
    for character in cwd.trim_start_matches('/').chars() {
        match character {
            '/' | '\\' | ':' => key.push('-'),
            '<' | '>' | '"' | '|' | '?' | '*' => push_encoded(&mut key, character),
            character if character.is_control() => push_encoded(&mut key, character),
            character => key.push(character),
        }
    }
    if key.chars().count() > MAX_KEY_CHARS {
        let hash = fnv1a(cwd.as_bytes());
        let truncated: String = key.chars().take(MAX_KEY_CHARS).collect();
        key = format!("{truncated}-{hash:016x}");
    }
    key
}

/// Removes a Windows verbatim prefix from a path string.
///
/// `\\?\C:\dir` becomes `C:\dir` and `\\?\UNC\server\share` becomes
/// `\\server\share`. Other strings are returned unchanged.
fn strip_verbatim(path: &str) -> Cow<'_, str> {
    if let Some(rest) = path.strip_prefix(r"\\?\UNC\") {
        Cow::Owned(format!(r"\\{rest}"))
    } else if let Some(rest) = path.strip_prefix(r"\\?\") {
        Cow::Borrowed(rest)
    } else {
        Cow::Borrowed(path)
    }
}

/// Appends the percent-encoded UTF-8 bytes of one character.
fn push_encoded(output: &mut String, character: char) {
    use std::fmt::Write as _;

    let mut buffer = [0_u8; 4];
    for byte in character.encode_utf8(&mut buffer).bytes() {
        // Writing to a `String` cannot fail.
        let _ = write!(output, "%{byte:02X}");
    }
}

/// FNV-1a, used only to keep truncated keys distinct and stable.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Characters that Windows rejects in file and directory names.
    const FORBIDDEN: [char; 9] = ['<', '>', ':', '"', '/', '\\', '|', '?', '*'];

    #[test]
    fn posix_keys_are_unchanged() {
        assert_eq!(session_dir_key("/Users/me/project"), "Users-me-project");
        assert_eq!(session_dir_key("/home/me"), "home-me");
    }

    #[test]
    fn windows_drive_paths_become_legal_keys() {
        assert_eq!(session_dir_key(r"C:\Users\amard"), "C--Users-amard");
        assert_eq!(session_dir_key("C:/Users/amard"), "C--Users-amard");
    }

    #[test]
    fn verbatim_prefixes_are_removed() {
        assert_eq!(session_dir_key(r"\\?\C:\Users\amard"), "C--Users-amard");
        assert_eq!(
            session_dir_key(r"\\?\UNC\server\share\project"),
            "--server-share-project"
        );
    }

    #[test]
    fn forbidden_characters_are_encoded() {
        let key = session_dir_key(r"\\?\C:\Users\a b\w?e*i<r>d|name");
        for character in FORBIDDEN {
            assert!(!key.contains(character), "{character:?} survived in {key}");
        }
        assert!(
            key.contains("%3F"),
            "question mark should be encoded: {key}"
        );
    }

    #[test]
    fn control_characters_are_encoded() {
        let key = session_dir_key("a\u{1}b");
        assert_eq!(key, "a%01b");
    }

    #[test]
    fn long_keys_are_truncated_and_hashed() {
        let prefix = "a".repeat(300);
        let key = session_dir_key(&prefix);
        assert_eq!(key.chars().count(), MAX_KEY_CHARS + 17);
        assert!(key.starts_with(&"a".repeat(MAX_KEY_CHARS)));
        assert!(key[MAX_KEY_CHARS..].starts_with('-'));
        assert!(
            key[MAX_KEY_CHARS + 1..]
                .chars()
                .all(|character| character.is_ascii_hexdigit())
        );

        // A one-character difference at the end still yields a distinct key.
        let other = session_dir_key(&format!("{prefix}b"));
        assert_ne!(key, other);
    }

    #[test]
    fn keys_never_exceed_the_windows_component_limit() {
        let key = session_dir_key(&"é".repeat(300));
        let utf16_len: usize = key.encode_utf16().count();
        assert!(
            utf16_len + 4 <= 255,
            "key occupies {utf16_len} UTF-16 units"
        );
    }
}
