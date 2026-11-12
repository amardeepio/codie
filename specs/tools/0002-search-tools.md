---
id: SPEC-TOOL-002
title: Read-only search tools: grep, find, and ls
status: implemented
version: 1.0.0
owners: ["@codie-maintainers"]
requirements: ["TOOL-010", "TOOL-011", "TOOL-012", "TOOL-013", "TOOL-014"]
---

# Context

`SPEC-TOOL-001` defines the tool contract and the four tools that mutate or read a single file. Navigating an
unfamiliar repository through `read` and `bash` alone forces the model to guess paths and to shell out for
searches, which produces unstructured output and bypasses the `.gitignore` rules the rest of the tooling honors.
This spec adds the read-only search surface: `grep` for content, `find` for paths, and `ls` for orientation.

All three tools are read-only in the permission model (`SEC-001`): a headless policy with no allow list permits
them, while `write`, `edit`, and `bash` remain denied.

# Normative behavior

**TOOL-010 (MUST)** `grep` searches file contents with a regular expression and returns one line per match in
`path:line:text` form, where `path` is relative to the working directory when possible. It honors
`case_sensitive` (default `true`) and an optional `glob` filter on file paths, accepts a file or directory as
`path` (default: the working directory), and returns an error result for an invalid regular expression, an
invalid glob, or a missing path.
Verification: `crates/codie-tools/tests/search.rs::spec_tool_010_grep_reports_path_line_and_text`

**TOOL-011 (MUST)** `grep` and `find` respect `.gitignore` and git exclude rules, skip hidden entries, and skip
files whose content is not valid UTF-8 during content search. Both cap their output with `max_results`
(`grep` defaults to 100 and is capped at 1,000; `find` defaults to 1,000 and is capped at 5,000) and state in the
output when the cap was reached.
Verification: `crates/codie-tools/tests/search.rs::spec_tool_011_grep_respects_ignore_rules_and_skips_binaries`

**TOOL-012 (MUST)** `find` matches files by glob relative to `path` (default: the working directory). A pattern
without a slash matches the file name at any depth (`*.rs`); a pattern with a slash matches the path relative to
the search root (`src/*.rs`, `**/*.toml`). Results are sorted lexicographically and a missing root is an error
result.
Verification: `crates/codie-tools/tests/search.rs::spec_tool_012_find_matches_globs_at_any_depth`

**TOOL-013 (MUST)** `ls` lists one directory without recursing. Directories are presented with a trailing `/`,
files with their size, and symlinks with their target. Hidden entries are omitted unless `all` is `true`. A
missing path or a path that is not a directory is an error result, and an empty directory yields
`(empty directory)`.
Verification: `crates/codie-tools/tests/search.rs::spec_tool_013_ls_lists_directories_files_and_symlinks`

**TOOL-014 (MUST)** `Registry::builtin()` declares `read`, `write`, `edit`, `bash`, `grep`, `find`, and `ls` in
that order, and `grep`, `find`, and `ls` are read-only: a headless `Policy` with an empty allow list permits them
while still denying `write`, `edit`, and `bash`.
Verification: `crates/codie-tools/tests/search.rs::spec_tool_014_search_tools_are_read_only`

# Data shapes

Each tool takes a flat object and rejects unknown properties at the schema layer (`TOOL-002`):

```json
{"pattern": "TODO|FIXME", "path": "src", "glob": "*.rs", "case_sensitive": false, "max_results": 50}
{"pattern": "**/*.toml", "path": ".", "max_results": 500}
{"path": "src", "all": true}
```

Results carry host-only details for tests and hosts: `grep` reports `matches`, `files`, `capped`, and `truncated`;
`find` reports `count`, `capped`, and `truncated`; `ls` reports `entries` and `truncated`.

# Edge cases

- A path that names a file is searched (or listed) directly instead of walking its parent.
- Output is bounded by the shared truncation caps (2,000 lines, 50 KiB) in addition to `max_results`; long
  matched lines are shortened with an explicit marker.
- Symlinked directories are not followed, so a search cannot escape the root through a link.
- Cancellation between files returns an aborted error result, matching the other tools.
- Missing, unreadable, and non-UTF-8 files are skipped rather than failing the whole search; an unreadable root is
  reported as an error.

# Non-goals

No content search inside archives, no binary or PDF indexing, no semantic search, and no network access.
`web_fetch` and web search are separate specs. Extension tools remain Phase 6.

# Security and performance

Paths are resolved but not sandboxed; the permission policy remains the control point (`SEC-001`). These tools
only read. The walker is deterministic and single-threaded, so results and truncation behave identically across
runs and platforms. Content search skips files larger than 4 MiB and stops at `max_results`, keeping memory and
latency bounded on large repositories.

# Reference implementation

riprep-style walker semantics from the `ignore` and `globset` crates; behavioral reference only.
