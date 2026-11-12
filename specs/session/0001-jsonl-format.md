---
id: SPEC-SESS-001
title: Session JSONL format v1
status: accepted
version: 1.0.0
owners: ["@codie-maintainers"]
requirements: ["SESS-001", "SESS-002", "SESS-003", "SESS-004", "SESS-005", "SESS-006", "SESS-007", "SESS-008", "SESS-009"]
---

# Context

A session is an append-only JSONL file. The first line is a header; every later line is one entry. Entries form
a tree through `parent_id`, so appending never rewrites history. The model-visible transcript is a projection of
the branch from the header to the current leaf.

The format is version 1, frozen in Phase 3 and stable for the 1.x series. Branch operations, compaction, and
migration extend it without breaking existing files.

# Requirements

**SESS-001 (MUST)** Entries are append-only nodes with `id`, `parent_id`, and `timestamp`; the first entry has a null parent and later entries point at the previous leaf. Existing lines are never rewritten.
Verification: `crates/codie-session/tests/session.rs::spec_sess_001_appends_preserve_parent_links`

**SESS-002 (MUST)** The file is created lazily on the first appended message; the first line is the session header with `type`, `version`, `id`, `timestamp`, and `cwd`.
Verification: `crates/codie-session/tests/session.rs::spec_sess_002_file_is_created_lazily_with_a_header`

**SESS-003 (MUST)** Each append is one JSON object followed by one LF and is flushed before the call returns; reopening reproduces the same order.
Verification: `crates/codie-session/tests/session.rs::create_append_reopen_and_project`

**SESS-004 (MUST)** The projection follows the parent chain from the current leaf and contains only message entries on that branch.
Verification: `crates/codie-session/tests/session.rs::set_leaf_moves_the_projection_and_rejects_unknown_ids`

**SESS-005 (MUST)** A malformed or truncated entry line is skipped; a malformed header or a newer format version is an error.
Verification: `crates/codie-session/tests/session.rs::malformed_entry_line_is_skipped`

**SESS-006 (MUST)** Model and thinking-level change entries persist across reopen and apply to later requests.
Verification: `crates/codie-session/tests/session.rs::model_and_thinking_changes_persist_across_reopen`

**SESS-007 (MUST)** Entry identifiers are eight lowercase hexadecimal characters and unique within a file.
Verification: `crates/codie-session/tests/session.rs::entry_identifiers_are_eight_lowercase_hex_characters`

**SESS-008 (MUST)** `continue_recent` selects the newest session whose header cwd matches the requested working directory, ordered by header timestamp and then file modification time.
Verification: `crates/codie-session/tests/session.rs::continue_recent_returns_the_matching_working_directory`

**SESS-009 (MUST)** The system message, including its tool declarations, is persisted as the first message and replayed unchanged on resume.
Verification: `crates/codie-session/tests/session.rs::spec_sess_009_system_message_is_persisted_and_replayed`

# Failure modes

- Missing directory: created on `create`.
- Empty file: error, because a header is mandatory.
- Unknown newer version: error with both versions named.
- Partial final line (crash): skipped on read.

# Security and performance

Sessions contain prompts, file contents, and tool output. They are written under the user's data directory with
the process umask; they never contain API keys. Appends are O(1); resume reads one file.
