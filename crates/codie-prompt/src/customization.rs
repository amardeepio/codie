//! Skills and prompt templates loaded from user and project directories.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::frontmatter;

/// Maximum size of one skill or template file.
pub const MAX_RESOURCE_BYTES: u64 = 64 * 1024;

/// Maximum number of skills loaded from both locations.
pub const MAX_SKILLS: usize = 100;

/// Maximum number of templates loaded from both locations.
pub const MAX_TEMPLATES: usize = 100;

/// A reusable instruction block invoked with `/skill:<name>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skill {
    /// Command name.
    pub name: String,
    /// One-line description shown in the prompt.
    pub description: String,
    /// Markdown body injected into the conversation.
    pub body: String,
    /// Whether the system prompt advertises the skill to the model.
    pub model_invocable: bool,
    /// File the skill was loaded from.
    pub path: PathBuf,
}

impl Skill {
    /// Expands the skill body with slash-command arguments.
    #[must_use]
    pub fn prompt(&self, arguments: &str) -> String {
        let arguments = arguments.trim();
        if self.body.contains("$ARGUMENTS") {
            return self.body.replace("$ARGUMENTS", arguments);
        }
        if arguments.is_empty() {
            return self.body.clone();
        }
        format!("{}\n\n{arguments}", self.body.trim_end())
    }
}

/// A named prompt with argument placeholders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Template {
    /// Command name, taken from the file stem.
    pub name: String,
    /// Optional description from frontmatter.
    pub description: String,
    /// Markdown body with `$ARGUMENTS` and `$1`..`$9` placeholders.
    pub body: String,
    /// File the template was loaded from.
    pub path: PathBuf,
}

impl Template {
    /// Expands placeholders with slash-command arguments.
    #[must_use]
    pub fn expand(&self, arguments: &str) -> String {
        let positional = arguments.split_whitespace().collect::<Vec<_>>();
        let mut output = String::with_capacity(self.body.len());
        let mut characters = self.body.char_indices().peekable();

        while let Some((_, character)) = characters.next() {
            if character != '$' {
                output.push(character);
                continue;
            }
            let Some(&(index, next)) = characters.peek() else {
                output.push('$');
                continue;
            };
            match next {
                '$' => {
                    characters.next();
                    output.push('$');
                }
                '1'..='9' => {
                    characters.next();
                    let slot = next as usize - '1' as usize;
                    output.push_str(positional.get(slot).copied().unwrap_or(""));
                }
                'A' if self.body[index..].starts_with("ARGUMENTS") => {
                    for _ in 0.."ARGUMENTS".len() {
                        characters.next();
                    }
                    output.push_str(arguments);
                }
                _ => output.push('$'),
            }
        }

        output
    }
}

/// Skills and templates resolved from user and project locations.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Customizations {
    skills: Vec<Skill>,
    templates: Vec<Template>,
    warnings: Vec<String>,
    user_dir: PathBuf,
    project_root: Option<PathBuf>,
}

impl Customizations {
    /// Loads user resources and, when trusted, project resources.
    ///
    /// Project resources override a user resource with the same name.
    #[must_use]
    pub fn load(config_dir: &Path, project_root: Option<&Path>) -> Self {
        let mut customizations = Self {
            user_dir: config_dir.to_path_buf(),
            project_root: project_root.map(Path::to_path_buf),
            ..Self::default()
        };
        customizations.refresh();
        customizations
    }

    /// Re-reads every resource from disk, keeping the configured roots.
    pub fn refresh(&mut self) {
        let mut skills = BTreeMap::new();
        let mut templates = BTreeMap::new();
        let mut warnings = Vec::new();

        load_skills(&self.user_dir.join("skills"), &mut skills, &mut warnings);
        load_templates(
            &self.user_dir.join("templates"),
            &mut templates,
            &mut warnings,
        );
        if let Some(root) = self.project_root.clone() {
            let codie = root.join(".codie");
            load_skills(&codie.join("skills"), &mut skills, &mut warnings);
            load_templates(&codie.join("templates"), &mut templates, &mut warnings);
        }

        self.skills = skills.into_values().take(MAX_SKILLS).collect();
        self.templates = templates.into_values().take(MAX_TEMPLATES).collect();
        self.warnings = warnings;
    }

    /// Returns every loaded skill, sorted by name.
    #[must_use]
    pub fn skills(&self) -> &[Skill] {
        &self.skills
    }

    /// Returns every loaded template, sorted by name.
    #[must_use]
    pub fn templates(&self) -> &[Template] {
        &self.templates
    }

    /// Returns non-fatal problems encountered while loading.
    #[must_use]
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// Looks up a skill by name.
    #[must_use]
    pub fn skill(&self, name: &str) -> Option<&Skill> {
        self.skills.iter().find(|skill| skill.name == name)
    }

    /// Looks up a template by name.
    #[must_use]
    pub fn template(&self, name: &str) -> Option<&Template> {
        self.templates.iter().find(|template| template.name == name)
    }

    /// Expands `/skill:<name>` or `/template:<name>` into a prompt.
    ///
    /// Returns `None` when `text` is not a customization command.
    #[must_use]
    pub fn expand_command(&self, text: &str) -> Option<Result<String, String>> {
        let text = text.trim();
        if let Some(rest) = text.strip_prefix("/skill:") {
            let (name, arguments) = split_name(rest);
            return Some(match self.skill(name) {
                Some(skill) => Ok(skill.prompt(arguments)),
                None => Err(format!(
                    "Unknown skill `{name}`. Add it under .codie/skills/{name}/SKILL.md."
                )),
            });
        }
        if let Some(rest) = text.strip_prefix("/template:") {
            let (name, arguments) = split_name(rest);
            return Some(match self.template(name) {
                Some(template) => Ok(template.expand(arguments)),
                None => Err(format!(
                    "Unknown template `{name}`. Add it under .codie/templates/{name}.md."
                )),
            });
        }
        None
    }
}

/// Splits `name arguments` at the first whitespace boundary.
fn split_name(input: &str) -> (&str, &str) {
    input
        .trim()
        .split_once(char::is_whitespace)
        .map_or((input.trim(), ""), |(name, arguments)| {
            (name, arguments.trim())
        })
}

fn load_skills(dir: &Path, skills: &mut BTreeMap<String, Skill>, warnings: &mut Vec<String>) {
    let entries = match sorted_entries(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
        Err(error) => {
            warnings.push(format!("cannot read {}: {error}", dir.display()));
            return;
        }
    };

    for entry in entries {
        if !entry.is_dir() {
            continue;
        }
        let path = entry.join("SKILL.md");
        let Some(content) = read_resource(&path, warnings) else {
            continue;
        };
        let parsed = match frontmatter::parse(&content) {
            Ok(parsed) => parsed,
            Err(error) => {
                warnings.push(format!("skipping {}: {error}", path.display()));
                continue;
            }
        };
        let name = parsed
            .fields
            .get("name")
            .filter(|name| !name.is_empty())
            .cloned()
            .or_else(|| {
                entry
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
            });
        let Some(name) = name.filter(|name| !name.contains(char::is_whitespace)) else {
            warnings.push(format!("skipping {}: invalid skill name", path.display()));
            continue;
        };
        let body = parsed.body.trim().to_owned();
        if body.is_empty() {
            warnings.push(format!("skipping {}: empty skill body", path.display()));
            continue;
        }
        let model_invocable = parsed
            .fields
            .get("model-invocable")
            .and_then(|value| parse_bool(value))
            .unwrap_or(true);
        skills.insert(
            name.clone(),
            Skill {
                name,
                description: parsed
                    .fields
                    .get("description")
                    .cloned()
                    .unwrap_or_default(),
                body,
                model_invocable,
                path,
            },
        );
    }
}

fn load_templates(
    dir: &Path,
    templates: &mut BTreeMap<String, Template>,
    warnings: &mut Vec<String>,
) {
    let entries = match sorted_entries(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
        Err(error) => {
            warnings.push(format!("cannot read {}: {error}", dir.display()));
            return;
        }
    };

    for entry in entries {
        if entry.extension().and_then(|extension| extension.to_str()) != Some("md") {
            continue;
        }
        let Some(stem) = entry
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
        else {
            continue;
        };
        let Some(content) = read_resource(&entry, warnings) else {
            continue;
        };
        let parsed = match frontmatter::parse(&content) {
            Ok(parsed) => parsed,
            Err(error) => {
                warnings.push(format!("skipping {}: {error}", entry.display()));
                continue;
            }
        };
        let body = parsed.body.trim().to_owned();
        if body.is_empty() {
            warnings.push(format!("skipping {}: empty template body", entry.display()));
            continue;
        }
        templates.insert(
            stem.clone(),
            Template {
                name: stem,
                description: parsed
                    .fields
                    .get("description")
                    .cloned()
                    .unwrap_or_default(),
                body,
                path: entry,
            },
        );
    }
}

/// Returns directory entries sorted by name.
fn sorted_entries(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut entries = fs::read_dir(dir)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    entries.sort();
    Ok(entries)
}

/// Reads a resource as UTF-8 within the size cap.
fn read_resource(path: &Path, warnings: &mut Vec<String>) -> Option<String> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) => {
            warnings.push(format!("cannot read {}: {error}", path.display()));
            return None;
        }
    };
    if !metadata.is_file() {
        return None;
    }
    if metadata.len() > MAX_RESOURCE_BYTES {
        warnings.push(format!(
            "skipping {}: larger than {} bytes",
            path.display(),
            MAX_RESOURCE_BYTES
        ));
        return None;
    }
    match fs::read_to_string(path) {
        Ok(content) => Some(content),
        Err(error) => {
            warnings.push(format!("cannot read {}: {error}", path.display()));
            None
        }
    }
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.trim().to_lowercase().as_str() {
        "true" | "yes" | "1" | "on" => Some(true),
        "false" | "no" | "0" | "off" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn template(body: &str) -> Template {
        Template {
            name: "t".to_owned(),
            description: String::new(),
            body: body.to_owned(),
            path: PathBuf::new(),
        }
    }

    #[test]
    fn templates_expand_arguments_and_positionals() {
        assert_eq!(template("Fix $1 in $2").expand("a b"), "Fix a in b");
        assert_eq!(template("All: $ARGUMENTS").expand("a b"), "All: a b");
        assert_eq!(template("$$1").expand("x"), "$1");
        assert_eq!(template("$1 $9").expand("x"), "x ");
        assert_eq!(template("plain").expand("ignored"), "plain");
    }

    #[test]
    fn skills_append_arguments_without_a_placeholder() {
        let skill = Skill {
            name: "s".to_owned(),
            description: String::new(),
            body: "Do the thing.".to_owned(),
            model_invocable: true,
            path: PathBuf::new(),
        };
        assert_eq!(skill.prompt("now"), "Do the thing.\n\nnow");
        assert_eq!(skill.prompt(""), "Do the thing.");
    }

    #[test]
    fn command_expansion_reports_unknown_names() {
        let customizations = Customizations::default();
        assert_eq!(
            customizations.expand_command("/skill:missing"),
            Some(Err(
                "Unknown skill `missing`. Add it under .codie/skills/missing/SKILL.md.".to_owned()
            ))
        );
        assert_eq!(customizations.expand_command("hello"), None);
    }
}
