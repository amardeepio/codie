//! `@path` attachment expansion (`PRMPT-010`, `PRMPT-011`).

use std::fs;
use std::path::Path;

use codie_core::Content;
use codie_prompt::expand_attachments;
use codie_testkit::spec;

fn write(path: impl AsRef<Path>, contents: &[u8]) {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent");
    }
    fs::write(path, contents).expect("write file");
}

fn text_of(expansion: &codie_prompt::Expansion) -> String {
    expansion
        .content
        .iter()
        .filter_map(Content::as_text)
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
#[spec("PRMPT-010")]
fn spec_prmpt_010_expands_files_and_images() {
    let directory = tempfile::tempdir().expect("dir");
    write(directory.path().join("src/main.rs"), b"fn main() {}\n");
    write(
        directory.path().join("diagram.png"),
        b"\x89PNG\r\n\x1a\nfake",
    );

    let expansion = expand_attachments(
        "Explain @src/main.rs and compare it with @diagram.png",
        directory.path(),
    );
    let text = text_of(&expansion);
    assert!(text.contains("<file path=\"src/main.rs\">\nfn main() {}\n</file>"));
    assert!(text.contains("[image: diagram.png]"));
    assert_eq!(expansion.errors, Vec::<String>::new());
    assert_eq!(expansion.attached.len(), 2);

    let image = expansion
        .content
        .iter()
        .find_map(|content| match content {
            Content::Image { data, mime_type } => Some((data, mime_type)),
            _ => None,
        })
        .expect("image block");
    assert_eq!(image.1, "image/png");
    assert!(!image.0.is_empty());
}

#[test]
#[spec("PRMPT-010")]
fn spec_prmpt_010_reports_missing_and_oversized_attachments() {
    let directory = tempfile::tempdir().expect("dir");
    let expansion = expand_attachments("@missing.txt", directory.path());
    assert_eq!(text_of(&expansion), "@missing.txt");
    assert_eq!(expansion.errors.len(), 1);
    assert!(expansion.errors[0].contains("missing.txt"));

    fs::create_dir(directory.path().join("folder")).expect("dir");
    let directory = expand_attachments("@folder", directory.path());
    assert!(directory.errors[0].contains("directory"));
}

#[test]
#[spec("PRMPT-011")]
fn spec_prmpt_011_duplicate_references_expand_once() {
    let directory = tempfile::tempdir().expect("dir");
    write(directory.path().join("a.txt"), b"payload");
    write(directory.path().join("b.png"), b"image");

    let expansion = expand_attachments("@a.txt @a.txt @b.png @b.png", directory.path());
    let text = text_of(&expansion);
    assert_eq!(text.matches("payload").count(), 1);
    assert!(text.contains("<file path=\"a.txt\"/>"));
    assert_eq!(
        expansion
            .content
            .iter()
            .filter(|content| matches!(content, Content::Image { .. }))
            .count(),
        1
    );
    assert_eq!(expansion.attached.len(), 2);
}
