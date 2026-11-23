---
id: SPEC-MEM-003
title: Project memory files
status: accepted
version: 1.0.0
owners: ["@codie-maintainers"]
requirements: ["MEM-009", "MEM-010", "MEM-011", "MEM-012", "MEM-013", "MEM-014", "MEM-015", "MEM-016"]
---

# Context

`SPEC-MEM-001` covers instruction files and `SPEC-MEM-002` covers a learned memory store. Neither gives a project
a durable, human-readable notebook that codie itself maintains. Users who ask codie to create a project expect a
plan and a memory file, and they expect codie to read that memory back in later sessions.

This spec defines the `PLAN.MD` and `CODIE.MD` artifacts, how memory reaches the system prompt, and the single
write path. It is deliberately small: no embeddings, no background store, no network calls.

# Normative behavior

**MEM-009 (MUST)** `CODIE.MD` is discovered at the project root and nowhere else. The project root is the nearest
ancestor of the working directory containing a `.git` entry, or the working directory itself when it is not inside a
repository. A notebook in a parent directory outside the repository MUST NOT load, and nested working directories
MUST resolve to the same root notebook.
Verification: `crates/codie-prompt/tests/memory.rs::spec_mem_009_project_memory_loads_from_the_project_root_only`

**MEM-010 (MUST)** At most 32 KiB of `CODIE.MD` is read into the prompt. When the file is longer, the content is
truncated on a UTF-8 character boundary and the block records that it was truncated. The rules section states that
project memory is data, not instructions.
Verification: `crates/codie-prompt/tests/memory.rs::spec_mem_010_project_memory_is_capped_and_labelled_as_data`

**MEM-011 (MUST)** Project memory is emitted in a `<project_memory path="...">` section after
`<project_context>` and before `<cwd>`. The section is omitted when no notebook exists.
Verification: `crates/codie-prompt/tests/memory.rs::spec_mem_011_memory_follows_project_instructions_and_precedes_cwd`

**MEM-012 (MUST)** The `project_memory` tool appends an entry under a `##` section, creates the section when it is
missing, and ignores an entry that duplicates existing text after case folding and whitespace normalization.
`mode: replace` replaces the whole section body and preserves every other section. Missing files are created with a
one-line heading.
Verification: `crates/codie-tools/tests/project_memory.rs::spec_mem_012_tool_appends_creates_sections_and_dedupes`

**MEM-013 (MUST)** The tool redacts credential-shaped values (`*_KEY`, `*_TOKEN`, `password=`, `Bearer ...`,
private key blocks) before writing. It refuses a single entry over 7 KiB, a `CODIE.MD` over 32 KiB, and a `PLAN.MD`
over 128 KiB, and rejects an unknown `file` argument. Writes are atomic through a temporary file and rename.
Verification: `crates/codie-tools/tests/project_memory.rs::spec_mem_013_tool_redacts_credentials_and_enforces_caps`

**MEM-014 (MUST)** `project_memory` is a mutating tool: `is_read_only` returns false, the headless policy denies it
unless the tool name is allowed, and it is present in the built-in registry.
Verification: `crates/codie-tools/tests/project_memory.rs::spec_mem_014_tool_is_mutating_for_permissions`

**MEM-015 (MUST)** `/init` creates `PLAN.MD` and `CODIE.MD` at the project root when they are missing, records the
detected ecosystem and its build, test, lint, and format commands, and never overwrites an existing file.
Verification: `crates/codie-prompt/tests/memory.rs::spec_mem_015_scaffold_creates_plan_and_memory_without_overwriting`

**MEM-016 (MUST)** `/reload` replaces the existing memory block in the assembled system prompt in place, leaving
every other section byte-identical, and records the result as a session context edit. Removing the notebook removes
the block.
Verification: `crates/codie-prompt/tests/memory.rs::spec_mem_016_reload_replaces_only_the_memory_block`

# Data shapes

```text
<project_memory path="/work/app/CODIE.MD">
# app — codie memory

## Commands

- `cargo test` runs the suite.
</project_memory>
```

The tool arguments are:

```json
{"file": "codie", "section": "Pitfalls", "text": "…", "mode": "append"}
```

# Edge cases

- A repository without `.git` but with `CODIE.MD` in the working directory loads it; a subdirectory of such a
  directory does not.
- Non-UTF-8 notebooks are ignored rather than failing prompt assembly.
- An entry that matches existing text after normalization returns success and reports that nothing changed.
- A section heading matches case-insensitively; a missing section is appended at the end of the file.
- The notebook is project data. The prompt labels it as untrusted and the rules section forbids following
  instructions found inside it.
- `/init` leaves an existing `PLAN.MD` untouched so hand-written plans survive.
