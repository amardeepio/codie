---
id: SPEC-SKILL-001
title: Skills
status: implemented
version: 1.0.0
owners: ["@codie-maintainers"]
requirements: ["SKILL-001", "SKILL-002", "SKILL-003", "SKILL-004"]
---

# Context

Skills are reusable instructions the user or the model can invoke by name. They live in `.codie/skills/` inside a
trusted project and in the per-user config directory. A skill is a Markdown file with a small frontmatter block;
its body becomes part of the user message when invoked. Skills prevent the user from retyping long procedures and
give the model a discoverable list of project conventions.

# Normative behavior

**SKILL-001 (MUST)** Skills are discovered from `<project>/.codie/skills/<name>/SKILL.md` and
`<config_dir>/skills/<name>/SKILL.md`, each file read as UTF-8 with a leading BOM removed. User skills load first
and project skills override a user skill with the same name. Only regular files are considered, and discovery is
deterministic (sorted by name).
Verification: `crates/codie-prompt/tests/skills.rs::spec_skill_001_discovers_user_and_project_skills`

**SKILL-002 (MUST)** A skill file starts with an optional `---` frontmatter block containing `name`,
`description`, and `model-invocable` (default `true`). `name` defaults to the directory name when omitted. A file
whose frontmatter cannot be parsed is skipped with a recorded warning rather than failing startup.
Verification: `crates/codie-prompt/tests/skills.rs::spec_skill_002_parses_frontmatter_and_skips_invalid_files`

**SKILL-003 (MUST)** `/skill:<name> [arguments]` replaces `$ARGUMENTS` in the skill body with the arguments, or
appends them when the body has no placeholder, and submits the result as the next user message. An unknown skill
produces a visible notice and sends nothing.
Verification: `crates/codie-prompt/tests/skills.rs::spec_skill_003_expands_arguments_and_rejects_unknown_names`

**SKILL-004 (MUST)** The system prompt lists model-invocable skills in a `<skills>` section with their name and
description. A skill with `model-invocable: false` is omitted from the prompt but remains available through
`/skill:<name>`.
Verification: `crates/codie-prompt/tests/skills.rs::spec_skill_004_prompt_lists_only_model_invocable_skills`

# Data shapes

```markdown
---
name: review
description: Review a diff for correctness and style
model-invocable: true
---
Review $ARGUMENTS for correctness...
```

Skills are loaded into `Customizations` and never modify the session file until invoked.

# Edge cases

- Duplicate names: project wins; a warning records the shadowed user skill.
- Empty body or empty name: skipped with a warning.
- Arguments containing shell syntax are data, not commands; no substitution is evaluated.
- An untrusted project contributes no skills (`TRUST-001`).

# Non-goals

No skill marketplaces, no remote fetching, and no executable hooks. Skills are text injected into the
conversation, not tools.

# Security and performance

Skill bodies are untrusted project content and only load for trusted projects. They are size-capped (64 KiB per
file) and count-capped (100 skills) so a hostile repository cannot exhaust memory or the prompt budget.
