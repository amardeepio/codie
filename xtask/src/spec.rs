//! Specification discovery, validation, and traceability reporting.
//!
//! The checker is deliberately parser-light. Specs use YAML frontmatter and
//! requirement definitions of the form `AREA-001 (MUST) ...`; a full YAML
//! parser would add dependencies without buying anything for the fields we
//! need. Requirement definitions inside fenced code blocks are ignored.

use std::collections::{BTreeSet, HashMap};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use anyhow::{Context, Result};
use regex::Regex;
use walkdir::WalkDir;

const STATUSES: [&str; 4] = ["draft", "accepted", "implemented", "deprecated"];

/// RFC 2119 level attached to a requirement definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Level {
    Must,
    Should,
    May,
}

impl Level {
    fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_uppercase().as_str() {
            "MUST" => Some(Self::Must),
            "SHOULD" => Some(Self::Should),
            "MAY" => Some(Self::May),
            _ => None,
        }
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::Must => "MUST",
            Self::Should => "SHOULD",
            Self::May => "MAY",
        }
    }
}

/// Parsed spec frontmatter.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Frontmatter {
    id: String,
    title: String,
    status: String,
    version: String,
    requirements: Vec<String>,
}

/// One requirement definition found in a spec body.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Requirement {
    id: String,
    level: Level,
    line: usize,
}

/// One `#[spec("...")]` annotation found in a Rust source file.
#[derive(Debug, Clone, PartialEq, Eq)]
struct TestRef {
    id: String,
    file: String,
    line: usize,
}

/// A spec file with its parsed metadata and definitions.
#[derive(Debug, Clone)]
struct SpecFile {
    rel: String,
    frontmatter: Frontmatter,
    requirements: Vec<Requirement>,
}

/// Result of scanning the whole repository.
#[derive(Debug, Default)]
struct Analysis {
    specs: Vec<SpecFile>,
    tests: Vec<TestRef>,
    errors: Vec<String>,
    warnings: Vec<String>,
}

impl Analysis {
    fn test_count(&self, id: &str) -> usize {
        self.tests.iter().filter(|test| test.id == id).count()
    }

    fn requirement_count(&self) -> usize {
        self.specs.iter().map(|spec| spec.requirements.len()).sum()
    }

    fn level_count(&self, level: Level) -> usize {
        self.specs
            .iter()
            .flat_map(|spec| &spec.requirements)
            .filter(|requirement| requirement.level == level)
            .count()
    }

    fn tested_count(&self) -> usize {
        self.specs
            .iter()
            .flat_map(|spec| &spec.requirements)
            .filter(|requirement| self.test_count(&requirement.id) > 0)
            .count()
    }
}

/// Runs `spec-check`, printing a human-readable summary.
///
/// Returns `true` when validation produced no errors.
pub(crate) fn check(root: &Path, strict: bool) -> Result<bool> {
    let analysis = analyze(root, strict)?;
    print_summary(&analysis, strict);
    Ok(analysis.errors.is_empty())
}

/// Runs `spec-report`, writing `target/spec-report.md`.
///
/// Returns `true` when validation produced no errors.
pub(crate) fn report(root: &Path) -> Result<bool> {
    let analysis = analyze(root, false)?;
    let path = root.join("target").join("spec-report.md");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating report directory {}", parent.display()))?;
    }
    fs::write(&path, render_report(&analysis))
        .with_context(|| format!("writing {}", path.display()))?;
    println!("spec-report: wrote {}", path.display());
    print_summary(&analysis, false);
    Ok(analysis.errors.is_empty())
}

fn analyze(root: &Path, strict: bool) -> Result<Analysis> {
    let mut analysis = Analysis::default();
    let spec_paths = discover_specs(root)?;

    let mut requirement_owners: HashMap<String, String> = HashMap::new();
    let mut spec_owners: HashMap<String, String> = HashMap::new();

    for path in spec_paths {
        let rel = relative_path(root, &path);
        let source = match fs::read_to_string(&path) {
            Ok(source) => source,
            Err(error) => {
                analysis
                    .errors
                    .push(format!("{rel}: cannot read spec: {error}"));
                continue;
            }
        };
        // `specs/template.md` is a scaffold with placeholder IDs, and
        // root-level markdown without frontmatter (README, glossary) is
        // reference material. Normative specs are validated either way.
        if is_template(&rel) {
            continue;
        }
        if !is_normative_spec(&rel) && !starts_with_frontmatter(&source) {
            continue;
        }
        let frontmatter = match parse_frontmatter(&source) {
            Ok(frontmatter) => frontmatter,
            Err(message) => {
                analysis.errors.push(format!("{rel}: {message}"));
                continue;
            }
        };

        if let Some(previous) = spec_owners.get(&frontmatter.id) {
            analysis.errors.push(format!(
                "{rel}: duplicate spec id `{}` (already declared by {previous})",
                frontmatter.id
            ));
        } else {
            spec_owners.insert(frontmatter.id.clone(), rel.clone());
        }

        let mut requirements = Vec::new();
        for raw in extract_requirements(&source) {
            if !is_valid_id(&raw.id) {
                analysis.errors.push(format!(
                    "{rel}:{}: malformed requirement id `{}` (expected AREA-001)",
                    raw.line, raw.id
                ));
            }
            if let Some(previous) = requirement_owners.get(&raw.id) {
                analysis.errors.push(format!(
                    "{rel}:{}: duplicate requirement id `{}` (already defined in {previous})",
                    raw.line, raw.id
                ));
            } else {
                requirement_owners.insert(raw.id.clone(), rel.clone());
            }
            requirements.push(Requirement {
                id: raw.id,
                level: raw.level,
                line: raw.line,
            });
        }

        check_frontmatter_list(
            &rel,
            &frontmatter.requirements,
            &requirements,
            &mut analysis,
        );
        analysis.specs.push(SpecFile {
            rel,
            frontmatter,
            requirements,
        });
    }

    if analysis.specs.is_empty() {
        analysis.errors.push(
            "no specs found: expected at least one `specs/<area>/<NNNN>-<slug>.md`".to_owned(),
        );
    }

    analysis.tests = discover_tests(root)?;
    for test in &analysis.tests {
        if !is_valid_id(&test.id) {
            analysis.errors.push(format!(
                "{}:{}: malformed spec id `{}` in #[spec]",
                test.file, test.line, test.id
            ));
        } else if !requirement_owners.contains_key(&test.id) {
            analysis.errors.push(format!(
                "{}:{}: unknown requirement `{}` in #[spec]",
                test.file, test.line, test.id
            ));
        }
    }

    analysis
        .errors
        .extend(coverage_errors(&analysis.specs, &analysis.tests, strict));
    Ok(analysis)
}

/// Compares the frontmatter `requirements:` list with definitions in the body.
fn check_frontmatter_list(
    rel: &str,
    listed: &[String],
    defined: &[Requirement],
    analysis: &mut Analysis,
) {
    let mut listed_set = BTreeSet::new();
    for entry in listed {
        if !is_valid_id(entry) {
            analysis.errors.push(format!(
                "{rel}: malformed requirement id `{entry}` in frontmatter `requirements`"
            ));
        } else if !listed_set.insert(entry.clone()) {
            analysis.warnings.push(format!(
                "{rel}: duplicate `{entry}` in frontmatter `requirements`"
            ));
        }
    }

    let defined_set: BTreeSet<String> = defined
        .iter()
        .filter(|requirement| is_valid_id(&requirement.id))
        .map(|requirement| requirement.id.clone())
        .collect();

    for id in defined_set.difference(&listed_set) {
        analysis.warnings.push(format!(
            "{rel}: `{id}` is defined but missing from frontmatter `requirements`"
        ));
    }
    for id in listed_set.difference(&defined_set) {
        analysis.warnings.push(format!(
            "{rel}: `{id}` is listed in frontmatter `requirements` but not defined"
        ));
    }
}

/// Enforces test coverage rules.
///
/// A spec with status `implemented` needs a test for every MUST/SHOULD. With
/// `--strict`, every requirement in every spec needs a test.
fn coverage_errors(specs: &[SpecFile], tests: &[TestRef], strict: bool) -> Vec<String> {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for test in tests {
        *counts.entry(test.id.as_str()).or_default() += 1;
    }

    let mut errors = Vec::new();
    for spec in specs {
        let implemented = spec.frontmatter.status == "implemented";
        if !implemented && !strict {
            continue;
        }

        let mut missing: Vec<&Requirement> = spec
            .requirements
            .iter()
            .filter(|requirement| is_valid_id(&requirement.id))
            .filter(|requirement| {
                strict || matches!(requirement.level, Level::Must | Level::Should)
            })
            .filter(|requirement| counts.get(requirement.id.as_str()).copied().unwrap_or(0) == 0)
            .collect();
        missing.sort_by(|left, right| left.id.cmp(&right.id));

        for requirement in missing {
            let rule = if strict {
                "strict mode requires a test for every requirement"
            } else {
                "implemented specs require a test for every MUST/SHOULD"
            };
            errors.push(format!(
                "{}:{}: requirement `{}` ({}) has no #[spec] test ({rule})",
                spec.rel,
                requirement.line,
                requirement.id,
                requirement.level.as_str()
            ));
        }
    }
    errors
}

fn discover_specs(root: &Path) -> Result<Vec<PathBuf>> {
    let dir = root.join("specs");
    if !dir.is_dir() {
        return Ok(Vec::new());
    }

    let mut paths = Vec::new();
    for entry in WalkDir::new(&dir).follow_links(false) {
        let entry = entry.with_context(|| format!("walking {}", dir.display()))?;
        if entry.file_type().is_file() && entry.path().extension().is_some_and(|ext| ext == "md") {
            paths.push(entry.into_path());
        }
    }
    paths.sort();
    Ok(paths)
}

fn discover_tests(root: &Path) -> Result<Vec<TestRef>> {
    let mut tests = Vec::new();
    for base in ["crates", "xtask"] {
        let dir = root.join(base);
        if !dir.is_dir() {
            continue;
        }
        let walker = WalkDir::new(&dir)
            .follow_links(false)
            .into_iter()
            .filter_entry(|entry| !matches!(entry.file_name().to_str(), Some("target" | ".git")));
        for entry in walker {
            let entry = entry.with_context(|| format!("walking {}", dir.display()))?;
            if !entry.file_type().is_file()
                || entry.path().extension().is_none_or(|ext| ext != "rs")
            {
                continue;
            }
            let source = fs::read_to_string(entry.path())
                .with_context(|| format!("reading {}", entry.path().display()))?;
            let rel = relative_path(root, entry.path());
            tests.extend(extract_test_refs(&source, &rel));
        }
    }
    tests.sort_by(|left, right| (&left.file, left.line).cmp(&(&right.file, right.line)));
    Ok(tests)
}

fn relative_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Normative specs live at `specs/<area>/<NNNN>-<slug>.md`. A markdown file
/// directly under `specs/` is reference material unless it carries frontmatter.
fn is_normative_spec(rel: &str) -> bool {
    rel.split('/').count() >= 3
}

/// Returns `true` for the copied-spec scaffold, which contains placeholder IDs.
fn is_template(rel: &str) -> bool {
    matches!(rel.rsplit('/').next(), Some("template.md" | "_template.md"))
}

/// Returns `true` when the document starts with a `---` frontmatter fence.
fn starts_with_frontmatter(source: &str) -> bool {
    source
        .lines()
        .next()
        .is_some_and(|line| line.trim_start_matches('\u{feff}').trim() == "---")
}

fn id_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^[A-Z][A-Z0-9_]{1,8}-\d{3}$").expect("valid id pattern"))
}

fn is_valid_id(value: &str) -> bool {
    id_re().is_match(value)
}

fn requirement_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"^[ \t]*(?:>[ \t]*)*(?:(?:[-*+]|\d+\.)[ \t]+)?(?:#{1,6}[ \t]+)?(?:`|\*\*)?(?P<id>[A-Za-z][A-Za-z0-9_]*(?:-[A-Za-z0-9_]+)+)(?:`|\*\*)?[ \t]*(?:\((?P<level>[A-Za-z]+)\)|\*\*(?P<bold_level>[A-Za-z]+)\*\*)",
        )
        .expect("valid requirement pattern")
    })
}

fn spec_attr_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r#"#\[\s*(?:[A-Za-z_][A-Za-z0-9_]*\s*::\s*)*spec\s*\(\s*"(?P<id>[^"]*)"\s*\)\s*\]"#,
        )
        .expect("valid spec attribute pattern")
    })
}

/// Extracts requirement definitions from a spec body.
///
/// Accepted shapes include `**MSG-001 (MUST)**`, `AGENT-002 (SHOULD)`,
/// bullet/heading variants, and bold levels. Fenced code blocks are skipped so
/// examples and templates are not mistaken for definitions.
fn extract_requirements(source: &str) -> Vec<Requirement> {
    let pattern = requirement_re();
    let mut requirements = Vec::new();
    let mut fence: Option<&'static str> = None;

    for (index, line) in source.lines().enumerate() {
        let trimmed = line.trim_start();
        if let Some(marker) = fence {
            if trimmed.starts_with(marker) {
                fence = None;
            }
            continue;
        }
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fence = Some(if trimmed.starts_with("```") {
                "```"
            } else {
                "~~~"
            });
            continue;
        }

        let Some(captures) = pattern.captures(line) else {
            continue;
        };
        let level = captures
            .name("level")
            .or_else(|| captures.name("bold_level"))
            .and_then(|value| Level::parse(value.as_str()));
        if let Some(level) = level {
            requirements.push(Requirement {
                id: captures["id"].to_owned(),
                level,
                line: index + 1,
            });
        }
    }
    requirements
}

/// Finds `#[spec("ID")]` annotations in code, ignoring comments and string
/// literals (including multi-line raw strings used in fixtures).
fn extract_test_refs(source: &str, file: &str) -> Vec<TestRef> {
    let pattern = spec_attr_re();
    let masks = code_mask(source);
    source
        .lines()
        .enumerate()
        .filter_map(|(index, line)| {
            let captures = pattern.captures(line)?;
            let start = captures.get(0)?.start();
            let in_code = masks
                .get(index)
                .and_then(|mask| mask.get(start))
                .copied()
                .unwrap_or(false);
            if !in_code {
                return None;
            }
            Some(TestRef {
                id: captures["id"].to_owned(),
                file: file.to_owned(),
                line: index + 1,
            })
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LexState {
    Code,
    LineComment,
    BlockComment(usize),
    Str,
    RawStr(usize),
}

/// Returns a per-line, per-byte mask where `true` means "ordinary Rust code".
///
/// Comments and string literals are marked `false`; newline bytes are not
/// represented so each mask lines up with the corresponding `str::lines` entry.
/// This keeps attribute discovery from matching examples embedded in test
/// fixtures, doc comments, or raw strings.
fn code_mask(source: &str) -> Vec<Vec<bool>> {
    let bytes = source.as_bytes();
    let mut masks = vec![Vec::new()];
    let mut state = LexState::Code;
    let mut index = 0;

    while index < bytes.len() {
        let byte = bytes[index];
        if byte == b'\n' {
            masks.push(Vec::new());
            if state == LexState::LineComment || state == LexState::Str {
                state = LexState::Code;
            }
            index += 1;
            continue;
        }

        let line = masks.len() - 1;
        match state {
            LexState::Code => {
                if byte == b'/' && bytes.get(index + 1) == Some(&b'/') {
                    masks[line].extend([false, false]);
                    state = LexState::LineComment;
                    index += 2;
                    continue;
                }
                if byte == b'/' && bytes.get(index + 1) == Some(&b'*') {
                    masks[line].extend([false, false]);
                    state = LexState::BlockComment(1);
                    index += 2;
                    continue;
                }
                if byte == b'"' {
                    masks[line].push(false);
                    state = LexState::Str;
                    index += 1;
                    continue;
                }
                if byte == b'r' {
                    let mut hashes = 0;
                    while bytes.get(index + 1 + hashes) == Some(&b'#') {
                        hashes += 1;
                    }
                    if bytes.get(index + 1 + hashes) == Some(&b'"') {
                        for _ in 0..=hashes {
                            masks[line].push(false);
                        }
                        masks[line].push(false);
                        state = LexState::RawStr(hashes);
                        index += hashes + 2;
                        continue;
                    }
                }
                masks[line].push(true);
            }
            LexState::LineComment => masks[line].push(false),
            LexState::BlockComment(depth) => {
                if byte == b'*' && bytes.get(index + 1) == Some(&b'/') {
                    masks[line].extend([false, false]);
                    state = if depth == 1 {
                        LexState::Code
                    } else {
                        LexState::BlockComment(depth - 1)
                    };
                    index += 2;
                    continue;
                }
                if byte == b'/' && bytes.get(index + 1) == Some(&b'*') {
                    masks[line].extend([false, false]);
                    state = LexState::BlockComment(depth + 1);
                    index += 2;
                    continue;
                }
                masks[line].push(false);
            }
            LexState::Str => {
                if byte == b'\\' {
                    masks[line].push(false);
                    if bytes.get(index + 1) == Some(&b'\n') {
                        masks.push(Vec::new());
                        index += 2;
                        continue;
                    }
                    masks[line].push(false);
                    index += 2;
                    continue;
                }
                masks[line].push(false);
                if byte == b'"' {
                    state = LexState::Code;
                }
            }
            LexState::RawStr(hashes) => {
                if byte == b'"'
                    && (0..hashes).all(|offset| bytes.get(index + 1 + offset) == Some(&b'#'))
                {
                    for _ in 0..=hashes {
                        masks[line].push(false);
                    }
                    state = LexState::Code;
                    index += hashes + 1;
                    continue;
                }
                masks[line].push(false);
            }
        }
        index += 1;
    }
    masks
}

/// Parses YAML-ish frontmatter delimited by `---` lines.
///
/// Only the fields the tooling needs are modelled: `id`, `title`, `status`,
/// `version`, and `requirements`. `requirements` accepts inline arrays and
/// block lists. Unknown keys (for example `owners`) are ignored.
fn parse_frontmatter(source: &str) -> Result<Frontmatter, String> {
    let mut lines = source.lines();
    let Some(first) = lines.next() else {
        return Err("empty document".to_owned());
    };
    if first.trim_start_matches('\u{feff}').trim() != "---" {
        return Err("missing frontmatter: expected `---` on the first line".to_owned());
    }

    let mut values: HashMap<String, String> = HashMap::new();
    let mut requirements = Vec::new();
    let mut in_requirements_list = false;
    let mut closed = false;

    for (offset, raw) in lines.enumerate() {
        let line_number = offset + 2;
        let line = raw.trim();
        if line == "---" {
            closed = true;
            break;
        }
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        if in_requirements_list {
            if let Some(item) = line.strip_prefix("- ").or_else(|| line.strip_prefix("-\t")) {
                requirements.push(unquote(item));
                continue;
            }
            in_requirements_list = false;
        }

        let Some((key, value)) = line.split_once(':') else {
            return Err(format!("line {line_number}: expected `key: value`"));
        };
        let key = key.trim().to_ascii_lowercase();
        let value = value.trim();
        if key == "requirements" {
            if value.is_empty() {
                in_requirements_list = true;
            } else if let Some(inner) = value
                .strip_prefix('[')
                .and_then(|rest| rest.strip_suffix(']'))
            {
                requirements.extend(
                    inner
                        .split(',')
                        .map(str::trim)
                        .filter(|item| !item.is_empty())
                        .map(unquote),
                );
            } else {
                return Err(format!(
                    "line {line_number}: requirements must be an inline array or a block list"
                ));
            }
        } else {
            values.insert(key, unquote(value));
        }
    }

    if !closed {
        return Err("unterminated frontmatter: missing closing `---`".to_owned());
    }

    let id = required(&mut values, "id")?;
    let title = required(&mut values, "title")?;
    let status = required(&mut values, "status")?;
    let version = required(&mut values, "version")?;
    if !STATUSES.contains(&status.as_str()) {
        return Err(format!(
            "invalid status `{status}`: expected one of {}",
            STATUSES.join(", ")
        ));
    }

    Ok(Frontmatter {
        id,
        title,
        status,
        version,
        requirements,
    })
}

fn required(values: &mut HashMap<String, String>, key: &str) -> Result<String, String> {
    values
        .remove(key)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("missing required frontmatter field `{key}`"))
}

fn unquote(value: &str) -> String {
    let trimmed = value.trim();
    for quote in ['"', '\''] {
        if let Some(inner) = trimmed
            .strip_prefix(quote)
            .and_then(|rest| rest.strip_suffix(quote))
        {
            return inner.trim().to_owned();
        }
    }
    trimmed.to_owned()
}

fn print_summary(analysis: &Analysis, strict: bool) {
    let total = analysis.requirement_count();
    let label = if strict {
        "spec-check (strict)"
    } else {
        "spec-check"
    };
    println!(
        "{label}: {} specs, {total} requirements ({} MUST, {} SHOULD, {} MAY), {} test annotations, {} requirements tested",
        analysis.specs.len(),
        analysis.level_count(Level::Must),
        analysis.level_count(Level::Should),
        analysis.level_count(Level::May),
        analysis.tests.len(),
        analysis.tested_count(),
    );

    for spec in &analysis.specs {
        let tested = spec
            .requirements
            .iter()
            .filter(|requirement| analysis.test_count(&requirement.id) > 0)
            .count();
        println!(
            "  {:<48} {:<12} {:>3} requirements, {:>3} tested",
            spec.rel,
            spec.frontmatter.status,
            spec.requirements.len(),
            tested
        );
    }

    for warning in &analysis.warnings {
        eprintln!("warning: {warning}");
    }
    for error in &analysis.errors {
        eprintln!("error: {error}");
    }

    if analysis.errors.is_empty() {
        println!("{label}: OK ({} warning(s))", analysis.warnings.len());
    } else {
        eprintln!(
            "{label}: FAILED ({} error(s), {} warning(s))",
            analysis.errors.len(),
            analysis.warnings.len()
        );
    }
}

fn render_report(analysis: &Analysis) -> String {
    let mut out = String::new();
    out.push_str("# Specification traceability report\n\n");
    out.push_str("Generated by `cargo xtask spec-report`.\n\n");

    if analysis.specs.is_empty() {
        out.push_str("_No specs found under `specs/**/*.md`._\n\n");
    }

    out.push_str("| Requirement | Level | Spec | Status | Tests |\n");
    out.push_str("|---|---|---|---|---|\n");
    for spec in &analysis.specs {
        for requirement in &spec.requirements {
            let tests = test_locations(analysis, &requirement.id);
            let _ = writeln!(
                out,
                "| `{}` | {} | `{}` | {} | {} |",
                requirement.id,
                requirement.level.as_str(),
                spec.rel,
                spec.frontmatter.status,
                tests
            );
        }
    }

    out.push_str("\n## Totals\n\n");
    out.push_str("| Metric | Count |\n|---|---|\n");
    let total = analysis.requirement_count();
    let tested = analysis.tested_count();
    let _ = writeln!(out, "| Specs | {} |", analysis.specs.len());
    let _ = writeln!(out, "| Requirements | {total} |");
    let _ = writeln!(out, "| MUST | {} |", analysis.level_count(Level::Must));
    let _ = writeln!(out, "| SHOULD | {} |", analysis.level_count(Level::Should));
    let _ = writeln!(out, "| MAY | {} |", analysis.level_count(Level::May));
    let _ = writeln!(out, "| Test annotations | {} |", analysis.tests.len());
    let _ = writeln!(out, "| Requirements with tests | {tested} |");
    let _ = writeln!(out, "| Requirements without tests | {} |", total - tested);

    if !analysis.warnings.is_empty() {
        out.push_str("\n## Warnings\n\n");
        for warning in &analysis.warnings {
            let _ = writeln!(out, "- {warning}");
        }
    }
    if !analysis.errors.is_empty() {
        out.push_str("\n## Errors\n\n");
        for error in &analysis.errors {
            let _ = writeln!(out, "- {error}");
        }
    }

    out
}

fn test_locations(analysis: &Analysis, id: &str) -> String {
    let locations: Vec<String> = analysis
        .tests
        .iter()
        .filter(|test| test.id == id)
        .map(|test| format!("`{}:{}`", test.file, test.line))
        .collect();
    if locations.is_empty() {
        "-".to_owned()
    } else {
        locations.join("<br>")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec_file(rel: &str, status: &str, requirements: &[(&str, Level)]) -> SpecFile {
        SpecFile {
            rel: rel.to_owned(),
            frontmatter: Frontmatter {
                id: format!("SPEC-{rel}"),
                title: "test".to_owned(),
                status: status.to_owned(),
                version: "1.0.0".to_owned(),
                requirements: requirements
                    .iter()
                    .map(|(id, _)| (*id).to_owned())
                    .collect(),
            },
            requirements: requirements
                .iter()
                .map(|(id, level)| Requirement {
                    id: (*id).to_owned(),
                    level: *level,
                    line: 1,
                })
                .collect(),
        }
    }

    fn test_ref(id: &str) -> TestRef {
        TestRef {
            id: id.to_owned(),
            file: "crates/example/src/lib.rs".to_owned(),
            line: 1,
        }
    }

    #[test]
    fn frontmatter_parses_inline_requirements() {
        let source = r#"---
id: SPEC-MSG-001
title: Message model
status: draft
version: 1.0.0
owners: [@alice]
requirements: [MSG-001, "MSG-002"]
---

# Context
"#;
        let frontmatter = parse_frontmatter(source).unwrap();
        assert_eq!(frontmatter.id, "SPEC-MSG-001");
        assert_eq!(frontmatter.title, "Message model");
        assert_eq!(frontmatter.status, "draft");
        assert_eq!(frontmatter.version, "1.0.0");
        assert_eq!(frontmatter.requirements, ["MSG-001", "MSG-002"]);
    }

    #[test]
    fn frontmatter_parses_block_requirements() {
        let source = "\
---
id: SPEC-AGENT-001
title: Agent loop
status: accepted
version: 0.1.0
requirements:
  - AGENT-001
  - AGENT-002
---
";
        let frontmatter = parse_frontmatter(source).unwrap();
        assert_eq!(frontmatter.requirements, ["AGENT-001", "AGENT-002"]);
    }

    #[test]
    fn frontmatter_requires_core_fields() {
        let source = "\
---
id: SPEC-X-001
title: Missing status
version: 1.0.0
---
";
        let error = parse_frontmatter(source).unwrap_err();
        assert!(error.contains("status"), "unexpected error: {error}");
    }

    #[test]
    fn frontmatter_rejects_unknown_status() {
        let source = "\
---
id: SPEC-X-001
title: Bad status
status: review
version: 1.0.0
---
";
        let error = parse_frontmatter(source).unwrap_err();
        assert!(
            error.contains("invalid status"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn frontmatter_requires_delimiters() {
        assert!(parse_frontmatter("# no frontmatter\n").is_err());
        let unterminated = "---\nid: SPEC-X-001\ntitle: X\nstatus: draft\nversion: 1.0.0\n";
        assert!(parse_frontmatter(unterminated).is_err());
    }

    #[test]
    fn extraction_supports_documented_shapes() {
        let source = "\
**MSG-001 (MUST)** Messages are typed.
AGENT-002 (SHOULD) The assistant message is finalized.
PROV-010 (MUST) Choose the token field.
- **TOOL-003 (MAY)** Serialize writes.
### `SESS-001` (MUST) Sessions are JSONL.
**CTX-004** (MUST) Summaries use a fixed structure.
";
        let requirements = extract_requirements(source);
        let ids: Vec<&str> = requirements
            .iter()
            .map(|requirement| requirement.id.as_str())
            .collect();
        assert_eq!(
            ids,
            [
                "MSG-001",
                "AGENT-002",
                "PROV-010",
                "TOOL-003",
                "SESS-001",
                "CTX-004"
            ]
        );
        assert_eq!(requirements[0].level, Level::Must);
        assert_eq!(requirements[1].level, Level::Should);
        assert_eq!(requirements[3].level, Level::May);
        assert_eq!(requirements[0].line, 1);
    }

    #[test]
    fn extraction_skips_fenced_code_blocks() {
        let source = "\
**MSG-001 (MUST)** Real requirement.

```markdown
AGENT-001 (MUST) This is a template example.
```

~~~text
PROV-002 (MUST) Another example.
~~~
";
        let requirements = extract_requirements(source);
        assert_eq!(requirements.len(), 1);
        assert_eq!(requirements[0].id, "MSG-001");
    }

    #[test]
    fn extraction_ignores_unknown_levels() {
        let requirements = extract_requirements("MSG-001 (optional) Not normative.\n");
        assert!(requirements.is_empty());
    }

    #[test]
    fn extraction_surfaces_malformed_ids_for_validation() {
        let requirements =
            extract_requirements("MSG-001-A (MUST) Sub-ids are not in the scheme.\n");
        assert_eq!(requirements.len(), 1);
        assert_eq!(requirements[0].id, "MSG-001-A");
        assert!(!is_valid_id(&requirements[0].id));
    }

    #[test]
    fn ids_are_validated_against_the_scheme() {
        for valid in ["MSG-001", "AGENT-999", "PROV-010", "A_1-001"] {
            assert!(is_valid_id(valid), "{valid} should be valid");
        }
        for invalid in [
            "msg-001",
            "M-001",
            "TOOLONGAREA-001",
            "MSG-01",
            "MSG-0001",
            "MSG001",
            "MSG-001-EXTRA",
            "1MSG-001",
            "MSG-001 ",
        ] {
            assert!(!is_valid_id(invalid), "{invalid} should be invalid");
        }
    }

    #[test]
    fn spec_attributes_are_discovered() {
        let source = "\
//! Example docs: #[spec(\"IGNORED-001\")]

#[test]
#[spec(\"AGENT-004\")]
fn parallel() {}

#[test]
#[codie_testkit::spec(\"MSG-001\")]
fn message() {}
";
        let tests = extract_test_refs(source, "crates/example/src/lib.rs");
        assert_eq!(tests.len(), 2);
        assert_eq!(tests[0].id, "AGENT-004");
        assert_eq!(tests[0].line, 4);
        assert_eq!(tests[1].id, "MSG-001");
        assert_eq!(tests[1].line, 8);
    }

    #[test]
    fn spec_attributes_in_strings_and_comments_are_ignored() {
        let source = r##"fn plain() {}

#[spec("AGENT-001")]
fn tagged() {}

const EXAMPLE: &str = r#"
#[spec("MSG-001")]
"#;

// #[spec("MSG-002")]
/* #[spec("MSG-003")] */
"##;
        let tests = extract_test_refs(source, "crates/example/src/lib.rs");
        assert_eq!(tests.len(), 1);
        assert_eq!(tests[0].id, "AGENT-001");
        assert_eq!(tests[0].line, 3);
    }

    #[test]
    fn normative_specs_are_nested_in_area_directories() {
        assert!(is_normative_spec("specs/agent/0001-turn-loop.md"));
        assert!(!is_normative_spec("specs/README.md"));
        assert!(!is_normative_spec("specs/template.md"));
        assert!(!is_normative_spec("specs/glossary.md"));
    }

    #[test]
    fn templates_and_frontmatter_are_recognized() {
        assert!(is_template("specs/template.md"));
        assert!(is_template("specs/_template.md"));
        assert!(!is_template("specs/agent/0001-turn-loop.md"));

        assert!(starts_with_frontmatter("---\nid: SPEC-X-001\n"));
        assert!(starts_with_frontmatter("\u{feff}---\nid: SPEC-X-001\n"));
        assert!(!starts_with_frontmatter("# Glossary\n"));
        assert!(!starts_with_frontmatter("\n---\n"));
    }

    #[test]
    fn implemented_specs_require_tests_for_must_and_should() {
        let specs = [spec_file(
            "specs/agent/0001-agent.md",
            "implemented",
            &[
                ("AGENT-001", Level::Must),
                ("AGENT-002", Level::Should),
                ("AGENT-003", Level::May),
            ],
        )];
        let tests = [test_ref("AGENT-001")];
        let errors = coverage_errors(&specs, &tests, false);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("AGENT-002"));
        assert!(!errors[0].contains("AGENT-003"));
    }

    #[test]
    fn draft_specs_are_only_checked_in_strict_mode() {
        let specs = [spec_file(
            "specs/agent/0001-agent.md",
            "draft",
            &[("AGENT-001", Level::Must), ("AGENT-002", Level::May)],
        )];
        assert!(coverage_errors(&specs, &[], false).is_empty());

        let errors = coverage_errors(&specs, &[], true);
        assert_eq!(errors.len(), 2);
        assert!(errors[0].contains("strict mode"));
    }

    #[test]
    fn strict_mode_checks_may_in_implemented_specs() {
        let specs = [spec_file(
            "specs/tools/0001-tools.md",
            "implemented",
            &[("TOOL-001", Level::Must), ("TOOL-002", Level::May)],
        )];
        let tests = [test_ref("TOOL-001")];
        let errors = coverage_errors(&specs, &tests, true);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("TOOL-002"));
    }

    #[test]
    fn frontmatter_mismatches_are_warnings() {
        let mut analysis = Analysis::default();
        let frontmatter = Frontmatter {
            id: "SPEC-X-001".to_owned(),
            title: "X".to_owned(),
            status: "draft".to_owned(),
            version: "1.0.0".to_owned(),
            requirements: vec!["MSG-001".to_owned(), "MSG-009".to_owned()],
        };
        let requirements = [Requirement {
            id: "MSG-001".to_owned(),
            level: Level::Must,
            line: 10,
        }];
        check_frontmatter_list(
            "specs/x.md",
            &frontmatter.requirements,
            &requirements,
            &mut analysis,
        );
        assert!(analysis.errors.is_empty());
        assert_eq!(analysis.warnings.len(), 1);
        assert!(analysis.warnings[0].contains("MSG-009"));
    }

    fn write(root: &Path, relative: &str, contents: &str) {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    #[test]
    fn analysis_reports_missing_specs() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "specs/README.md", "# Reference only\n");
        let analysis = analyze(dir.path(), false).unwrap();
        assert!(analysis.specs.is_empty());
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.contains("no specs found"))
        );
    }

    #[test]
    fn analysis_skips_docs_and_template_and_flags_unknown_tests() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(
            root,
            "specs/README.md",
            "# Reference\n\n**MSG-001 (MUST)** Example only.\n",
        );
        write(
            root,
            "specs/template.md",
            r#"---
id: SPEC-AREA-001
title: Template
status: draft
version: 1.0.0
requirements: ["AREA-001"]
---

**AREA-001 (MUST)** Placeholder.
"#,
        );
        write(
            root,
            "specs/agent/0001-agent.md",
            r#"---
id: SPEC-AGENT-001
title: Agent loop
status: accepted
version: 1.0.0
requirements: ["AGENT-001"]
---

**AGENT-001 (MUST)** The loop ends with AgentEnd.
Verification: `crates/codie-core/tests/agent.rs::spec_agent_001`
"#,
        );
        write(
            root,
            "crates/example/src/lib.rs",
            r#"#[test]
#[spec("AGENT-001")]
fn a() {}

#[test]
#[spec("AGENT-999")]
fn b() {}
"#,
        );

        let analysis = analyze(root, false).unwrap();
        assert_eq!(analysis.specs.len(), 1);
        assert_eq!(analysis.specs[0].requirements.len(), 1);
        assert_eq!(analysis.tests.len(), 2);
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.contains("AGENT-999") && error.contains("unknown")),
            "unexpected errors: {:?}",
            analysis.errors
        );
    }

    #[test]
    fn duplicate_requirement_ids_across_specs_are_errors() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for (area, spec_id) in [("agent", "SPEC-AGENT-001"), ("tools", "SPEC-TOOL-001")] {
            write(
                root,
                &format!("specs/{area}/0001-spec.md"),
                &format!(
                    "---\nid: {spec_id}\ntitle: Spec\nstatus: draft\nversion: 1.0.0\nrequirements: [\"MSG-001\"]\n---\n\n**MSG-001 (MUST)** Duplicated requirement.\n"
                ),
            );
        }
        let analysis = analyze(root, false).unwrap();
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.contains("duplicate requirement id `MSG-001`")),
            "unexpected errors: {:?}",
            analysis.errors
        );
    }
}
