//! Prompt template discovery and expansion (`TMPL-001`..`TMPL-003`).

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

#[test]
#[spec("TMPL-001")]
fn spec_tmpl_001_discovers_user_and_project_templates() {
    let config = tempfile::tempdir().expect("config");
    let project = tempfile::tempdir().expect("project");
    write(
        &config.path().join("templates/review.md"),
        "User $ARGUMENTS",
    );
    write(&config.path().join("templates/release.md"), "User release");
    write(
        &project.path().join(".codie/templates/review.md"),
        "Project $ARGUMENTS",
    );
    write(&project.path().join(".codie/templates/fix.md"), "Fix $1");

    let customizations = Customizations::load(config.path(), Some(project.path()));
    let names = customizations
        .templates()
        .iter()
        .map(|template| template.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(names, ["fix", "release", "review"]);
    assert_eq!(
        customizations
            .template("review")
            .map(|template| template.body.as_str()),
        Some("Project $ARGUMENTS")
    );
}

#[test]
#[spec("TMPL-002")]
fn spec_tmpl_002_substitutes_arguments() {
    let config = tempfile::tempdir().expect("config");
    write(
        &config.path().join("templates/compare.md"),
        "---\ndescription: Compare two files\n---\nCompare $1 with $2. Notes: $ARGUMENTS. Literal $$1.",
    );
    let customizations = Customizations::load(config.path(), None);
    let template = customizations.template("compare").expect("template");

    assert_eq!(
        template.expand("a.rs b.rs"),
        "Compare a.rs with b.rs. Notes: a.rs b.rs. Literal $1."
    );
    assert_eq!(
        customizations.expand_command("/template:compare a.rs b.rs"),
        Some(Ok(
            "Compare a.rs with b.rs. Notes: a.rs b.rs. Literal $1.".to_owned()
        ))
    );
}

#[test]
#[spec("TMPL-003")]
fn spec_tmpl_003_expands_and_rejects_unknown_names() {
    let customizations = Customizations::load(std::path::Path::new("/nonexistent"), None);
    assert!(
        customizations
            .expand_command("/template:missing")
            .is_some_and(|result| result.is_err())
    );
    assert_eq!(customizations.expand_command("not a command"), None);
}
