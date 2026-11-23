//! `@path` attachment expansion for user prompts.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use base64::Engine;
use codie_core::Content;

/// Maximum size of one attached file (10 MiB).
pub const MAX_ATTACHMENT_BYTES: u64 = 10 * 1024 * 1024;

/// Maximum number of attachments in one message.
pub const MAX_ATTACHMENTS: usize = 20;

/// The result of expanding `@path` references.
#[derive(Debug, Clone, PartialEq)]
pub struct Expansion {
    /// Message content: rewritten prompt text followed by image blocks.
    pub content: Vec<Content>,
    /// Problems that the caller should show to the user.
    pub errors: Vec<String>,
    /// Paths that were successfully attached, in order.
    pub attached: Vec<PathBuf>,
}

/// Expands `@path` tokens in `text` relative to `cwd`.
///
/// Text files become inline `<file path="...">` blocks. Images become
/// `Content::Image` blocks appended after the text. Duplicate references expand
/// once; the second occurrence keeps the path but not the payload. Missing or
/// unreadable paths keep their token and record an error.
#[must_use]
pub fn expand_attachments(text: &str, cwd: &Path) -> Expansion {
    let mut rewritten = String::with_capacity(text.len());
    let mut images = Vec::new();
    let mut errors = Vec::new();
    let mut attached = Vec::new();
    let mut seen = HashSet::new();
    let mut count = 0usize;
    let mut rest = text;

    while let Some(index) = find_token(rest) {
        rewritten.push_str(&rest[..index]);
        let after = &rest[index + 1..];
        let Some((raw, consumed)) = scan_attachment(after) else {
            rewritten.push('@');
            rest = after;
            continue;
        };
        let token = &rest[index..index + 1 + consumed];
        rest = &after[consumed..];

        count += 1;
        if count > MAX_ATTACHMENTS {
            errors.push(format!(
                "Attachment limit reached: at most {MAX_ATTACHMENTS} files per message."
            ));
            rewritten.push_str(token);
            continue;
        }

        let path = resolve(&raw, cwd);
        let canonical = path.canonicalize().unwrap_or_else(|_| path.clone());
        if !seen.insert(canonical.clone()) {
            if is_image_path(&path) {
                rewritten.push_str(&format!("[image: {raw}]"));
            } else {
                rewritten.push_str(&format!("<file path=\"{}\"/>", attribute(&raw)));
            }
            continue;
        }

        let metadata = match std::fs::metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) => {
                errors.push(format!("Cannot attach `{raw}`: {error}"));
                rewritten.push_str(token);
                continue;
            }
        };
        if metadata.is_dir() {
            errors.push(format!("Cannot attach `{raw}`: it is a directory."));
            rewritten.push_str(token);
            continue;
        }
        if metadata.len() > MAX_ATTACHMENT_BYTES {
            errors.push(format!(
                "Cannot attach `{raw}`: larger than {} MiB.",
                MAX_ATTACHMENT_BYTES / (1024 * 1024)
            ));
            rewritten.push_str(token);
            continue;
        }

        if is_image_path(&path) {
            match std::fs::read(&path) {
                Ok(bytes) => {
                    images.push(Content::Image {
                        data: base64::engine::general_purpose::STANDARD.encode(bytes),
                        mime_type: mime_type(&path).to_owned(),
                    });
                    attached.push(path);
                    rewritten.push_str(&format!("[image: {raw}]"));
                }
                Err(error) => {
                    errors.push(format!("Cannot attach `{raw}`: {error}"));
                    rewritten.push_str(token);
                }
            }
            continue;
        }

        match std::fs::read_to_string(&path) {
            Ok(content) => {
                attached.push(path);
                rewritten.push_str(&format!(
                    "<file path=\"{}\">\n{}\n</file>",
                    attribute(&raw),
                    content.trim_end()
                ));
            }
            Err(error) => {
                errors.push(format!("Cannot attach `{raw}`: {error}"));
                rewritten.push_str(token);
            }
        }
    }
    rewritten.push_str(rest);

    let mut content = Vec::new();
    if !rewritten.is_empty() || images.is_empty() {
        content.push(Content::text(rewritten));
    }
    content.extend(images);

    Expansion {
        content,
        errors,
        attached,
    }
}

/// Finds the next `@` that starts a word.
fn find_token(text: &str) -> Option<usize> {
    let mut previous = None;
    for (index, character) in text.char_indices() {
        if character == '@' && previous.is_none_or(|previous: char| previous.is_whitespace()) {
            return Some(index);
        }
        previous = Some(character);
    }
    None
}

/// Scans one attachment token after `@`.
///
/// Quoted tokens (`@"name with spaces.png"`) run to the matching quote.
/// Unquoted tokens run to whitespace, and a backslash escapes the next
/// character, so `Screenshot\ 2026.png` is one path.
fn scan_attachment(after: &str) -> Option<(String, usize)> {
    let first = after.chars().next()?;
    if first == '"' || first == '\'' {
        let close = after[first.len_utf8()..].find(first)?;
        let raw = &after[first.len_utf8()..first.len_utf8() + close];
        let consumed = close + 2 * first.len_utf8();
        return (!raw.is_empty()).then(|| (raw.to_owned(), consumed));
    }

    let mut raw = String::new();
    let mut consumed = 0;
    let mut escaped = false;
    for (index, character) in after.char_indices() {
        if escaped {
            raw.push(character);
            consumed = index + character.len_utf8();
            escaped = false;
            continue;
        }
        if character == '\\' {
            escaped = true;
            consumed = index + 1;
            continue;
        }
        if character.is_whitespace() {
            break;
        }
        raw.push(character);
        consumed = index + character.len_utf8();
    }
    if escaped {
        raw.push('\\');
    }
    (!raw.is_empty()).then_some((raw, consumed))
}

/// Converts a whole prompt that is a single existing file path into an
/// attachment token.
///
/// Copying a screenshot, dragging it from Finder, or pasting a path puts a bare
/// path on the line. Without this, the model receives text instead of the image.
/// Returns `None` when the text is not a path, or the path does not exist.
#[must_use]
pub fn attachment_token_for_path_text(text: &str, cwd: &Path) -> Option<String> {
    resolve_path_text(text, cwd).map(|path| quote_attachment_path(&path))
}

/// Resolves a whole-prompt file path to an existing absolute path.
///
/// Accepts quoted paths, backslash-escaped spaces, `~/`, absolute, and relative
/// forms. Returns `None` when the text is not path-like or the file is missing.
#[must_use]
pub fn resolve_path_text(text: &str, cwd: &Path) -> Option<PathBuf> {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.contains('\n') || !looks_like_path(trimmed) {
        return None;
    }
    let mut candidates = vec![trimmed.to_owned()];
    if let Some(unquoted) = strip_matching_quotes(trimmed) {
        candidates.push(unquoted.to_owned());
    }
    if trimmed.contains('\\') {
        candidates.push(trimmed.replace("\\ ", " ").replace("\\\\", "\\"));
    }
    candidates
        .into_iter()
        .map(|candidate| resolve(&candidate, cwd))
        .find(|path| path.is_file())
}

/// Returns true when a whole prompt looks like a filesystem path rather than a
/// command or ordinary sentence.
///
/// `/help` is not a path, `/var/folders/x.png` is, and so is an escaped
/// drag-and-drop path such as `Screenshot\ 2026.png`.
#[must_use]
pub fn looks_like_path_text(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.contains('\n') {
        return false;
    }
    if trimmed.starts_with("~/") || trimmed.starts_with("./") || trimmed.starts_with("../") {
        return true;
    }
    if trimmed.contains("\\ ") {
        return true;
    }
    // Windows drive paths (`C:\\shot.png`) are paths, not commands.
    let bytes = trimmed.as_bytes();
    if bytes.len() > 2 && bytes[1] == b':' && matches!(bytes[2], b'\\' | b'/') {
        return true;
    }
    // A colon in the first token marks a command argument (`skill:review`).
    if trimmed
        .split_whitespace()
        .next()
        .is_some_and(|token| token.contains(':'))
    {
        return false;
    }
    if let Some(rest) = trimmed.strip_prefix('/') {
        return rest.contains('/');
    }
    is_image_path(Path::new(trimmed))
}

fn looks_like_path(text: &str) -> bool {
    text.contains('/')
        || text.contains('\\')
        || text.starts_with('~')
        || is_image_path(Path::new(text))
}

fn strip_matching_quotes(text: &str) -> Option<&str> {
    let first = text.chars().next()?;
    if (first == '"' || first == '\'') && text.len() >= 2 {
        let inner = &text[first.len_utf8()..];
        return inner
            .strip_suffix(first)
            .filter(|inner| !inner.contains(first));
    }
    None
}

/// Validates a client-supplied base64 image and builds an image content block.
///
/// Used by RPC clients that paste or drag images rather than referencing a
/// path. The decoded size is bounded by [`MAX_ATTACHMENT_BYTES`].
///
/// # Errors
///
/// Returns a human-readable message when the MIME type is not `image/*`, the
/// payload is not valid base64, is empty, or exceeds the size limit.
pub fn inline_image(data: &str, mime_type: &str) -> Result<Content, String> {
    if !mime_type.starts_with("image/") {
        return Err(format!(
            "unsupported MIME type `{mime_type}`; expected image/*"
        ));
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data)
        .map_err(|error| format!("invalid base64 image: {error}"))?;
    if bytes.is_empty() {
        return Err("image attachment is empty".to_owned());
    }
    if bytes.len() as u64 > MAX_ATTACHMENT_BYTES {
        return Err(format!(
            "image attachment is larger than {} MiB",
            MAX_ATTACHMENT_BYTES / (1024 * 1024)
        ));
    }
    Ok(Content::Image {
        data: data.to_owned(),
        mime_type: mime_type.to_owned(),
    })
}

/// Formats a path as an attachment token, quoting it when it contains spaces.
#[must_use]
pub fn quote_attachment_path(path: &Path) -> String {
    let text = path.to_string_lossy();
    if !text.chars().any(char::is_whitespace) {
        return format!("@{text}");
    }
    if !text.contains('"') {
        return format!("@\"{text}\"");
    }
    if !text.contains('\'') {
        return format!("@'{text}'");
    }
    format!("@{text}")
}

/// Resolves `~`, absolute, and relative attachment paths.
fn resolve(raw: &str, cwd: &Path) -> PathBuf {
    if let Some(rest) = raw.strip_prefix("~/")
        && let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))
    {
        return PathBuf::from(home).join(rest);
    }
    let path = PathBuf::from(raw);
    if path.is_absolute() {
        path
    } else {
        cwd.join(path)
    }
}

/// Returns true for the image extensions codie can send to a model.
#[must_use]
pub fn is_image_path(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_lowercase)
            .as_deref(),
        Some("png" | "jpg" | "jpeg" | "gif" | "webp")
    )
}

fn mime_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_lowercase)
        .as_deref()
    {
        Some("png") => "image/png",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        _ => "image/jpeg",
    }
}

fn attribute(raw: &str) -> String {
    raw.replace('&', "&amp;").replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn email_addresses_are_not_attachments() {
        let directory = tempfile::tempdir().unwrap();
        let expansion = expand_attachments("mail me at a@b.com", directory.path());
        assert!(expansion.attached.is_empty());
        assert_eq!(expansion.content, vec![Content::text("mail me at a@b.com")]);
    }

    #[test]
    fn missing_files_keep_the_token_and_report_an_error() {
        let directory = tempfile::tempdir().unwrap();
        let expansion = expand_attachments("see @nope.txt", directory.path());
        assert_eq!(expansion.content, vec![Content::text("see @nope.txt")]);
        assert_eq!(expansion.errors.len(), 1);
    }

    #[test]
    fn quoted_paths_allow_spaces() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("my shot.txt"), "hello").unwrap();
        let expansion = expand_attachments("@\"my shot.txt\"", directory.path());
        assert_eq!(expansion.errors, Vec::<String>::new());
        assert_eq!(expansion.attached.len(), 1);
        assert!(
            expansion
                .content
                .iter()
                .filter_map(Content::as_text)
                .any(|text| text.contains("hello"))
        );
    }

    #[test]
    fn escaped_spaces_are_part_of_the_path() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("my shot.txt"), "hello").unwrap();
        let expansion = expand_attachments("@my\\ shot.txt", directory.path());
        assert_eq!(expansion.attached.len(), 1);
    }

    #[test]
    fn attachment_quoting_matches_the_tokenizer() {
        assert_eq!(
            quote_attachment_path(Path::new("/tmp/my shot.png")),
            "@\"/tmp/my shot.png\""
        );
        assert_eq!(
            quote_attachment_path(Path::new("/tmp/shot.png")),
            "@/tmp/shot.png"
        );
    }

    #[test]
    fn bare_path_text_becomes_an_attachment_token() {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("shot one.png");
        std::fs::write(&file, b"\x89PNG\r\n\x1a\nfake").unwrap();

        let token = attachment_token_for_path_text(&file.to_string_lossy(), directory.path())
            .expect("absolute path becomes a token");
        assert!(token.starts_with("@\""), "{token}");

        let relative = attachment_token_for_path_text("shot one.png", directory.path())
            .expect("relative path becomes a token");
        assert!(relative.starts_with("@\""), "{relative}");

        let quoted = format!("\"{}\"", file.display());
        assert!(
            attachment_token_for_path_text(&quoted, directory.path()).is_some(),
            "quoted paths are unwrapped"
        );
    }

    #[test]
    fn path_detection_separates_paths_from_commands() {
        assert!(looks_like_path_text("/var/folders/x/Screenshot 2026.png"));
        assert!(looks_like_path_text("/var/folders/x/Screenshot\\ 2026.png"));
        assert!(looks_like_path_text("~/shot.png"));
        assert!(looks_like_path_text("./shot.png"));
        assert!(!looks_like_path_text("/help"));
        assert!(!looks_like_path_text("/skill:review src/main.rs"));
        assert!(!looks_like_path_text("explain this"));
        assert!(!looks_like_path_text(""));
    }

    #[test]
    fn ordinary_text_is_not_a_path() {
        let directory = tempfile::tempdir().unwrap();
        assert!(attachment_token_for_path_text("explain this code", directory.path()).is_none());
        assert!(attachment_token_for_path_text("README.md", directory.path()).is_none());
        assert!(
            attachment_token_for_path_text("/definitely/missing.png", directory.path()).is_none()
        );
    }

    #[test]
    fn duplicate_text_references_expand_once() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("a.txt"), "hello").unwrap();
        let expansion = expand_attachments("@a.txt @a.txt", directory.path());
        let Content::Text { text, .. } = &expansion.content[0] else {
            panic!("text content");
        };
        assert!(text.contains("hello"));
        assert!(text.contains("<file path=\"a.txt\"/>"));
        assert_eq!(expansion.attached.len(), 1);
    }
}
