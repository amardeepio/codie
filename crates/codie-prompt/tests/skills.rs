//! Skills discovery and expansion (`SKILL-001`..`SKILL-004`).

use std::fs;
use std::path::Path;

use codie_prompt::{Customizations, PromptInput, Skill, build_system_prompt};
use codie_testkit::spec;

fn write(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent");
    }
    fs::write(path, contents).expect("write file");
}

fn skill(name: &str, description: &str, body: &str, model_invocable: bool) -> String {
    format!(
        "---\nname: {name}\ndescription: {description}\nmodel-invocable: {model_invocable}\n---\n{body}\n"
    )
}

#[test]
#[spec("SKILL-001")]
fn spec_skill_001_discovers_user_and_project_skills() {
    let config = tempfile::tempdir().expect("config");
    let project = tempfile::tempdir().expect("project");
    write(
        &config.path().join("skills/review/SKILL.md"),
        &skill("review", "Review a diff", "Review $ARGUMENTS", true),
    );
    write(
        &config.path().join("skills/shared/SKILL.md"),
        &skill("shared", "User version", "user body", true),
    );
    write(
        &project.path().join(".codie/skills/shared/SKILL.md"),
        &skill("shared", "Project version", "project body", true),
    );
    write(
        &project.path().join(".codie/skills/local/SKILL.md"),
        &skill("local", "Local only", "local body", true),
    );

    let customizations = Customizations::load(config.path(), Some(project.path()));
    let names = customizations
        .skills()
        .iter()
        .map(|skill| skill.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(names, ["local", "review", "shared"]);
    assert_eq!(
        customizations
            .skill("shared")
            .map(|skill| skill.body.as_str()),
        Some("project body"),
        "project resources override user resources"
    );
}

#[test]
#[spec("SKILL-002")]
fn spec_skill_002_parses_frontmatter_and_skips_invalid_files() {
    let config = tempfile::tempdir().expect("config");
    write(
        &config.path().join("skills/implicit/SKILL.md"),
        "Body without frontmatter\n",
    );
    write(
        &config.path().join("skills/broken/SKILL.md"),
        "---\nname: broken\nmissing close\n",
    );
    write(
        &config.path().join("skills/empty/SKILL.md"),
        "---\nname: empty\ndescription: nothing\n---\n",
    );

    let customizations = Customizations::load(config.path(), None);
    let names = customizations
        .skills()
        .iter()
        .map(|skill| skill.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(names, ["implicit"]);
    assert!(
        customizations
            .warnings()
            .iter()
            .any(|warning| warning.contains("broken")),
        "{:?}",
        customizations.warnings()
    );
}

#[test]
#[spec("SKILL-003")]
fn spec_skill_003_expands_arguments_and_rejects_unknown_names() {
    let config = tempfile::tempdir().expect("config");
    write(
        &config.path().join("skills/review/SKILL.md"),
        &skill(
            "review",
            "Review a diff",
            "Review $ARGUMENTS carefully.",
            true,
        ),
    );
    write(
        &config.path().join("skills/plain/SKILL.md"),
        &skill("plain", "No placeholder", "Follow the checklist.", true),
    );
    let customizations = Customizations::load(config.path(), None);

    assert_eq!(
        customizations.expand_command("/skill:review src/main.rs"),
        Some(Ok("Review src/main.rs carefully.".to_owned()))
    );
    assert_eq!(
        customizations.expand_command("/skill:plain now"),
        Some(Ok("Follow the checklist.\n\nnow".to_owned()))
    );
    assert!(
        customizations
            .expand_command("/skill:missing")
            .is_some_and(|result| result.is_err())
    );
}

#[test]
#[spec("SKILL-004")]
fn spec_skill_004_prompt_lists_only_model_invocable_skills() {
    let mut input = PromptInput::new("/work");
    input.skills = vec![
        Skill {
            name: "review".to_owned(),
            description: "Review a diff".to_owned(),
            body: "Review".to_owned(),
            model_invocable: true,
            path: Default::default(),
        },
        Skill {
            name: "secret".to_owned(),
            description: "Manual only".to_owned(),
            body: "Secret".to_owned(),
            model_invocable: false,
            path: Default::default(),
        },
    ];

    let prompt = build_system_prompt(&input);
    assert!(prompt.contains("<skills>\n- review: Review a diff\n</skills>"));
    assert!(!prompt.contains("secret"));
}
