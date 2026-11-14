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
        let token_start = index + 1;
        let after = &rest[token_start..];
        let end = after.find(char::is_whitespace).unwrap_or(after.len());
        let raw = &after[..end];
        let token = &rest[index..token_start + end];
        rest = &after[end..];

        if raw.is_empty() {
            rewritten.push('@');
            continue;
        }

        count += 1;
        if count > MAX_ATTACHMENTS {
            errors.push(format!(
                "Attachment limit reached: at most {MAX_ATTACHMENTS} files per message."
            ));
            rewritten.push_str(token);
            continue;
        }

        let path = resolve(raw, cwd);
        let canonical = path.canonicalize().unwrap_or_else(|_| path.clone());
        if !seen.insert(canonical.clone()) {
            if is_image(&path) {
                rewritten.push_str(&format!("[image: {raw}]"));
            } else {
                rewritten.push_str(&format!("<file path=\"{}\"/>", attribute(raw)));
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

        if is_image(&path) {
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
                    attribute(raw),
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

fn is_image(path: &Path) -> bool {
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
