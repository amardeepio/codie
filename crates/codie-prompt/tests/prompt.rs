//! Integration tests for deterministic prompt assembly (PRMPT-001, PRMPT-003,
//! PRMPT-008).

use std::fs;
use std::path::{Path, PathBuf};

use codie_core::{Message, ToolDecl};
use codie_prompt::{
    ContextFile, DEFAULT_PREAMBLE, PromptInput, build_system_message, build_system_prompt,
    discover_project_context, load_project_context,
};
use codie_testkit::spec;

fn tool(name: &str, description: &str) -> ToolDecl {
    ToolDecl {
        name: name.to_string(),
        description: description.to_string(),
        parameters: Default::default(),
    }
}

fn write(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent");
    }
    fs::write(path, contents).expect("write file");
}

fn sample_input() -> PromptInput {
    PromptInput {
        cwd: PathBuf::from("/work/project"),
        tools: vec![tool("read", "Read a file.\nMore detail.")],
        preamble: Some("Custom preamble.".to_string()),
        append: Some("Appended instruction.".to_string()),
        skills: Vec::new(),
        project_context: vec![ContextFile {
            path: PathBuf::from("/work/AGENTS.md"),
            content: "Use tabs.\n".to_string(),
        }],
    }
}

#[spec("PRMPT-001")]
#[test]
fn sections_are_ordered_preamble_tools_rules_context_cwd() {
    let prompt = build_system_prompt(&sample_input());

    assert!(prompt.starts_with("Custom preamble."));
    let tools = prompt.find("<tools>").expect("tools section");
    let rules = prompt.find("<rules>").expect("rules section");
    let context = prompt.find("<project_context>").expect("context section");
    let cwd = prompt.find("<cwd>").expect("cwd section");

    assert!(tools < rules, "tools must precede rules");
    assert!(rules < context, "rules must precede project context");
    assert!(context < cwd, "project context must precede cwd");
    assert!(prompt.ends_with("</cwd>"));
}

#[spec("PRMPT-002")]
#[test]
fn non_preamble_sections_are_xml_wrapped() {
    let prompt = build_system_prompt(&sample_input());

    assert!(prompt.contains("<tools>\n- read: Read a file. More detail.\n</tools>"));
    assert!(prompt.contains("<rules>\n"));
    assert!(prompt.contains("Be concise in your responses."));
    assert!(prompt.contains("Show file paths clearly when working with files."));
    assert!(prompt.contains(
        "<project_instructions path=\"/work/AGENTS.md\">\nUse tabs.\n</project_instructions>"
    ));
    assert!(prompt.contains("<cwd>/work/project</cwd>"));
    assert!(!prompt.starts_with("<preamble>"));
}

#[spec("PRMPT-005")]
#[test]
fn prompt_assembly_is_deterministic_and_matches_snapshot() {
    let input = sample_input();
    let first = build_system_prompt(&input);
    let second = build_system_prompt(&input);
    assert_eq!(first, second);

    let expected = concat!(
        "Custom preamble.\n",
        "\n",
        "Appended instruction.\n",
        "\n",
        "<tools>\n",
        "- read: Read a file. More detail.\n",
        "</tools>\n",
        "\n",
        "<rules>\n",
        "- Use the available tools instead of guessing about files or commands.\n",
        "- Read a file before editing it, and verify changes after writing them.\n",
        "- Prefer small, focused edits over broad rewrites.\n",
        "- Be concise in your responses.\n",
        "- Show file paths clearly when working with files.\n",
        "</rules>\n",
        "\n",
        "<project_context>\n",
        "<project_instructions path=\"/work/AGENTS.md\">\n",
        "Use tabs.\n",
        "</project_instructions>\n",
        "</project_context>\n",
        "\n",
        "<cwd>/work/project</cwd>",
    );
    assert_eq!(first, expected);
}

#[spec("PRMPT-006")]
#[test]
fn default_preamble_is_used_when_none_is_supplied() {
    let mut input = PromptInput::new("/work");
    input.append = Some("Extra rule.".to_string());
    let prompt = build_system_prompt(&input);

    assert!(prompt.starts_with(DEFAULT_PREAMBLE));
    assert!(prompt.contains(&format!("{DEFAULT_PREAMBLE}\n\nExtra rule.")));
    assert!(!prompt.contains("<project_context>"));
}

#[spec("PRMPT-003")]
#[test]
fn discovery_walks_to_root_topmost_first() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().canonicalize().expect("canonical tempdir");
    let nested = root.join("a").join("b").join("c");

    let root_file = root.join("AGENTS.md");
    let middle_file = root.join("a").join("b").join("CLAUDE.md");
    let nearest_file = nested.join("AGENTS.md");
    write(&root_file, "root instructions");
    write(&middle_file, "middle instructions");
    write(&nearest_file, "nearest instructions");

    let discovered = discover_project_context(&nested);
    assert!(
        discovered.ends_with(&[root_file, middle_file, nearest_file]),
        "unexpected discovery order: {discovered:?}"
    );
}

#[spec("PRMPT-003")]
#[test]
fn override_file_wins_inside_the_same_directory() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().canonicalize().expect("canonical tempdir");
    let project = root.join("project");

    let override_file = project.join("AGENTS.override.md");
    let agents_file = project.join("AGENTS.md");
    let claude_file = project.join("CLAUDE.md");
    write(&override_file, "override");
    write(&agents_file, "agents");
    write(&claude_file, "claude");

    let discovered = discover_project_context(&project);
    assert!(discovered.ends_with(std::slice::from_ref(&override_file)));
    assert!(!discovered.contains(&agents_file));
    assert!(!discovered.contains(&claude_file));
}

#[spec("PRMPT-004")]
#[test]
fn missing_files_are_ignored() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().canonicalize().expect("canonical tempdir");
    let empty = root.join("empty");
    fs::create_dir_all(&empty).expect("create empty dir");

    let discovered = discover_project_context(&empty);
    assert!(
        discovered.iter().all(|path| !path.starts_with(&root)),
        "no files should come from the empty tree: {discovered:?}"
    );
}

#[test]
fn loaded_context_strips_a_bom_and_skips_non_utf8() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().canonicalize().expect("canonical tempdir");
    let agents = root.join("AGENTS.md");
    write(&agents, "\u{feff}# Guide\nUse tabs.\n");

    let loaded = load_project_context(&root);
    let entry = loaded
        .iter()
        .find(|file| file.path == agents)
        .expect("AGENTS.md should be loaded");
    assert_eq!(entry.content, "# Guide\nUse tabs.\n");

    let invalid_dir = root.join("invalid");
    fs::create_dir_all(&invalid_dir).expect("create invalid dir");
    let invalid = invalid_dir.join("AGENTS.md");
    fs::write(&invalid, [0xff_u8, 0xfe, 0xfd]).expect("write invalid utf-8");
    let loaded = load_project_context(&invalid_dir);
    assert!(
        !loaded.iter().any(|file| file.path == invalid),
        "non-utf8 files must be skipped"
    );
    // Ancestor context is still discovered for the nested directory.
    assert!(loaded.iter().any(|file| file.path == agents));
}

#[spec("PRMPT-007")]
#[test]
fn system_message_carries_prompt_and_tools() {
    let input = sample_input();
    let tools = input.tools.clone();
    let message = build_system_message(&input, tools.clone());

    assert_eq!(message.role(), "system");
    match message {
        Message::System(system) => {
            assert_eq!(system.content, build_system_prompt(&input));
            assert_eq!(system.tools, tools);
        }
        other => panic!("expected a system message, got {}", other.role()),
    }
}

#[spec("PRMPT-008")]
#[test]
fn empty_tools_still_produce_a_wrapped_section() {
    let prompt = build_system_prompt(&PromptInput::new("/work"));
    assert!(prompt.contains("<tools>\n</tools>"));
    assert!(prompt.ends_with("<cwd>/work</cwd>"));
}
