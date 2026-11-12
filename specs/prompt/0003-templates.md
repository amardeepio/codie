---
id: SPEC-TMPL-001
title: Prompt templates
status: implemented
version: 1.0.0
owners: ["@codie-maintainers"]
requirements: ["TMPL-001", "TMPL-002", "TMPL-003"]
---

# Context

Prompt templates are short, named prompts with argument placeholders. They are the project-local equivalent of a
shell alias for a repeated request ("fix the failing test", "write a changelog entry"). Unlike skills, templates
are always user-invoked and contain no model-invocation policy.

# Normative behavior

**TMPL-001 (MUST)** Templates are discovered as Markdown files from `<project>/.codie/templates/*.md` and
`<config_dir>/templates/*.md`. The template name is the file stem; when both locations define a name, the project
template wins deterministically.
Verification: `crates/codie-prompt/tests/templates.rs::spec_tmpl_001_discovers_user_and_project_templates`

**TMPL-002 (MUST)** A template supports `$ARGUMENTS` for the complete argument string and `$1`..`$9` for
individual whitespace-separated arguments. Missing positional arguments expand to an empty string, and
`$$` produces a literal `$`. An optional `---` frontmatter block may declare a `description`.
Verification: `crates/codie-prompt/tests/templates.rs::spec_tmpl_002_substitutes_arguments`

**TMPL-003 (MUST)** `/template:<name> [arguments]` expands the template and submits it as the next user message.
An unknown template produces a visible notice and sends nothing. Templates are never listed in the system prompt.
Verification: `crates/codie-prompt/tests/templates.rs::spec_tmpl_003_expands_and_rejects_unknown_names`

# Edge cases

- A template without placeholders ignores arguments rather than appending them.
- `$ARGUMENTS` inside a larger word still expands; `$$` is never treated as a placeholder.
- Invalid UTF-8 files are skipped with a warning; files over 64 KiB are skipped.
- Untrusted projects contribute no templates (`TRUST-001`).

# Non-goals

No template inheritance, includes, conditionals, or shell evaluation. No model-invoked templates.
