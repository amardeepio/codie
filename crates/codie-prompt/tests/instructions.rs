//! `SYSTEM.md` and `APPEND_SYSTEM.md` discovery (`PRMPT-009`).

use std::fs;
use std::path::Path;

use codie_prompt::load_instructions;
use codie_testkit::spec;

fn write(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent");
    }
    fs::write(path, contents).expect("write file");
}

#[test]
#[spec("PRMPT-009")]
fn spec_prmpt_009_system_and_append_instructions() {
    let root = tempfile::tempdir().expect("root");
    let nested = root.path().join("project");
    write(&root.path().join("SYSTEM.md"), "root system");
    write(&root.path().join("APPEND_SYSTEM.md"), "root append");
    write(&nested.join("SYSTEM.md"), "project system");
    write(&nested.join("APPEND_SYSTEM.md"), "project append");

    let instructions = load_instructions(&nested);
    assert_eq!(
        instructions
            .system
            .as_ref()
            .map(|file| file.content.as_str()),
        Some("project system"),
        "the nearest SYSTEM.md wins"
    );
    assert_eq!(
        instructions
            .appends
            .iter()
            .map(|file| file.content.as_str())
            .collect::<Vec<_>>(),
        ["root append", "project append"]
    );
    assert_eq!(
        instructions.preamble(None).as_deref(),
        Some("project system")
    );
    assert_eq!(
        instructions.preamble(Some("cli system")).as_deref(),
        Some("cli system")
    );
    assert_eq!(
        instructions.combined_append(Some("cli append")).as_deref(),
        Some("root append\n\nproject append\n\ncli append")
    );
}
