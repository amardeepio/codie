---
id: SPEC-PRMPT-001
title: System prompt assembly
status: accepted
version: 1.0.0
owners: ["@codie-maintainers"]
requirements: ["PRMPT-001", "PRMPT-002", "PRMPT-003", "PRMPT-004", "PRMPT-005", "PRMPT-006", "PRMPT-007", "PRMPT-008"]
---

# Context

The system prompt is built once per session and persisted with the tool declarations effective at that point.
Assembly is deterministic: the same inputs produce the same bytes. Project instructions are discovered from
the working directory and its ancestors.

# Requirements

**PRMPT-001 (MUST)** Sections are emitted in this order: preamble, tools, rules, project instructions (omitted when empty), and cwd, separated by one blank line with no trailing newline.
Verification: `crates/codie-prompt/tests/prompt.rs::spec_prmpt_001_section_order`

**PRMPT-002 (MUST)** Every section except the preamble is wrapped in a lowercase XML tag (`<tools>`, `<rules>`, `<project_instructions>`, `<cwd>`).
Verification: `crates/codie-prompt/tests/prompt.rs::non_preamble_sections_are_xml_wrapped`

**PRMPT-003 (MUST)** Project instruction discovery walks from the filesystem root to the working directory; per directory the first existing file among `AGENTS.override.md`, `AGENTS.md`, and `CLAUDE.md` wins.
Verification: `crates/codie-prompt/tests/prompt.rs::spec_prmpt_003_discovery_order`

**PRMPT-004 (MUST)** Missing, unreadable, or non-UTF-8 instruction files are skipped; a leading UTF-8 BOM is stripped.
Verification: `crates/codie-prompt/tests/prompt.rs::missing_files_are_ignored`

**PRMPT-005 (MUST)** Prompt assembly is deterministic and matches the accepted snapshot.
Verification: `crates/codie-prompt/tests/prompt.rs::prompt_assembly_is_deterministic_and_matches_snapshot`

**PRMPT-006 (MUST)** Without an override, the default preamble is used; a replacement preamble and an append block are honored when supplied.
Verification: `crates/codie-prompt/tests/prompt.rs::default_preamble_is_used_when_none_is_supplied`

**PRMPT-007 (MUST)** The system message carries the assembled prompt and the tool declarations in registry order.
Verification: `crates/codie-prompt/tests/prompt.rs::system_message_carries_prompt_and_tools`

**PRMPT-008 (MUST)** An empty tool list still emits an empty `<tools>` section rather than omitting the tag.
Verification: `crates/codie-prompt/tests/prompt.rs::empty_tools_still_produce_a_wrapped_section`

# Failure modes

- Unreadable file: skipped with a debug log.
- No context files: no `<project_instructions>` section.
- Non-canonical cwd: ancestor traversal uses the canonical path when it resolves.

# Security and performance

Project files are untrusted input. They are quoted inside a tagged section and never interpreted as instructions
to codie itself. Discovery reads at most one file per ancestor directory.
