//! Subagent definition discovery (`.codie/agents/*.md`).

use std::fs;
use std::path::Path;

use codie_prompt::Customizations;
use codie_testkit::spec;

fn write(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent");
    }
    fs::write(path, contents).expect("write file");
}

fn agent(fields: &str, body: &str) -> String {
    format!("---\n{fields}\n---\n{body}\n")
}

#[test]
#[spec("AGENT-031")]
fn discovers_user_and_project_agents_with_project_precedence() {
    let config = tempfile::tempdir().expect("config");
    let project = tempfile::tempdir().expect("project");
    write(
        &config.path().join("agents/shared.md"),
        &agent("description: User version", "user prompt"),
    );
    write(
        &config.path().join("agents/reviewer.md"),
        &agent("description: Reviews code", "review prompt"),
    );
    write(
        &project.path().join(".codie/agents/shared.md"),
        &agent("description: Project version", "project prompt"),
    );

    let customizations = Customizations::load(config.path(), Some(project.path()));
    let names = customizations
        .agents()
        .iter()
        .map(|agent| agent.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(names, ["reviewer", "shared"]);
    assert_eq!(
        customizations
            .agent("shared")
            .map(|agent| agent.prompt.as_str()),
        Some("project prompt"),
        "project definitions override user definitions"
    );
}

#[test]
#[spec("AGENT-031")]
fn parses_routing_fields_and_defaults() {
    let config = tempfile::tempdir().expect("config");
    write(
        &config.path().join("agents/ui-reviewer.md"),
        &agent(
            "description: Reviews rendered UI\nmodel: provider/model\ntools: read, grep, bash\nmax-turns: 12\ntimeout-secs: 90",
            "You review interfaces.",
        ),
    );
    write(
        &config.path().join("agents/plain.md"),
        &agent("description: Inherits everything", "Do the thing."),
    );

    let customizations = Customizations::load(config.path(), None);
    let reviewer = customizations.agent("ui-reviewer").expect("ui-reviewer");
    assert_eq!(reviewer.description, "Reviews rendered UI");
    assert_eq!(reviewer.model.as_deref(), Some("provider/model"));
    assert_eq!(
        reviewer.tools.as_deref(),
        Some(["read".to_owned(), "grep".to_owned(), "bash".to_owned()].as_slice())
    );
    assert_eq!(reviewer.max_turns, Some(12));
    assert_eq!(reviewer.timeout_secs, Some(90));
    assert_eq!(reviewer.prompt, "You review interfaces.");

    let plain = customizations.agent("plain").expect("plain");
    assert_eq!(plain.tools, None);
    assert_eq!(plain.model, None);
    assert_eq!(plain.max_turns, None);
}

#[test]
fn inherit_tools_word_means_no_override() {
    let config = tempfile::tempdir().expect("config");
    write(
        &config.path().join("agents/explore.md"),
        &agent(
            "description: Override without tools\ntools: inherit",
            "Explore.",
        ),
    );

    let customizations = Customizations::load(config.path(), None);
    assert_eq!(
        customizations.agent("explore").expect("explore").tools,
        None
    );
}

#[test]
fn skips_invalid_and_empty_definitions() {
    let config = tempfile::tempdir().expect("config");
    write(
        &config.path().join("agents/broken.md"),
        "---\ndescription: broken\n",
    );
    write(
        &config.path().join("agents/empty.md"),
        "---\ndescription: x\n---\n",
    );
    write(
        &config.path().join("agents/bad-number.md"),
        &agent("description: x\nmax-turns: many", "Prompt."),
    );
    write(&config.path().join("agents/notes.txt"), "not an agent");

    let customizations = Customizations::load(config.path(), None);
    assert_eq!(customizations.agents().len(), 1);
    assert_eq!(
        customizations.agent("bad-number").expect("kept").max_turns,
        None
    );
    assert!(
        customizations
            .warnings()
            .iter()
            .any(|warning| warning.contains("max-turns"))
    );
}
