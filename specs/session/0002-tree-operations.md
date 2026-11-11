---
id: SPEC-SESS-002
title: Session tree operations, fork, clone, and inspection
status: implemented
version: 1.0.0
owners: ["@codie-maintainers"]
requirements: ["SESS-010", "SESS-011", "SESS-012", "SESS-013", "SESS-014", "SESS-015", "SESS-016", "SESS-017", "SESS-018", "SESS-019", "SESS-020", "SESS-021", "SESS-022", "SESS-023"]
---

# Context

`SPEC-SESS-001` defines the append-only JSONL file and its projection. Phase 3 adds the operations that make the
tree usable: resuming on a branch, forking a branch into a new file, cloning a whole tree, inspecting the tree in
the terminal, exporting it, and keeping the v1 format stable. Every operation is append-only with respect to the
source file; nothing here rewrites history.

# Normative behavior

**SESS-010 (MUST)** `fork_at` copies the branch ending at a selected entry, or at the current leaf, into a new
session file whose header records `parent_session`; the source file is never modified.
Verification: `crates/codie-session/tests/tree.rs::spec_sess_010_fork_copies_branch_without_touching_source`

**SESS-011 (MUST)** `clone_session` copies every entry and the current leaf into a new file with a new identifier,
preserving the whole tree and its projection.
Verification: `crates/codie-session/tests/tree.rs::spec_sess_011_clone_preserves_entire_tree`

**SESS-012 (MUST)** `tree` returns every entry in file order with its depth and an `on_branch` flag relative to
the current leaf, including persisted leaf markers.
Verification: `crates/codie-session/tests/tree.rs::spec_sess_012_tree_marks_branch_and_depth`

**SESS-013 (MUST)** A session is resolvable by file path, exact identifier, or a unique identifier prefix; an
ambiguous prefix is an error naming the number of matches.
Verification: `crates/codie-session/tests/tree.rs::spec_sess_013_resolve_by_id_and_prefix`

**SESS-014 (MUST)** `export` writes the canonical header followed by every entry as JSONL, byte-identical to the
backing file for a persisted session, and never writes to the session file.
Verification: `crates/codie-session/tests/tree.rs::spec_sess_014_export_matches_backing_file`

**SESS-015 (MUST)** `list` returns readable sessions newest first with identifier, name, cwd, creation time,
message count, update time, and parent session; `list_for_cwd` filters by recorded working directory.
Verification: `crates/codie-session/tests/tree.rs::spec_sess_015_list_reports_newest_first`

**SESS-016 (MUST)** A crash that leaves a partial final line loses only that partial entry; every complete entry
before it is recovered on read.
Verification: `crates/codie-session/tests/tree.rs::spec_sess_016_truncated_final_line_loses_only_partial_message`

**SESS-017 (MUST)** Any sequence of appends, branch selections, model changes, and compactions reopens to
byte-equivalent entries, the same leaf, and the same projection.
Verification: `crates/codie-session/tests/tree.rs::spec_sess_017_random_tree_operations_reopen_identically`

**SESS-018 (MUST)** `append_context_edit` replaces the content of the targeted message, or removes it when the
replacement is `null`, without removing the original entry from the file.
Verification: `crates/codie-session/tests/compaction.rs::spec_sess_018_context_edit_replaces_or_removes_a_message`

**SESS-019 (MUST)** A branch-summary entry appears in the projection as a synthetic user message after the
selected branch point; entries on the abandoned branch remain on disk and are not projected.
Verification: `crates/codie-session/tests/compaction.rs::spec_sess_019_branch_summary_is_inserted_on_the_branch`

**SESS-020 (MUST)** A 10,000-entry session opens and projects in under 150 ms on the reference development
machine.
Verification: `crates/codie-session/tests/format.rs::spec_sess_020_ten_thousand_entries_resume_within_budget`

**SESS-021 (MUST)** Every v1 entry type round-trips through JSON with a stable `type` tag: `message`,
`model_change`, `thinking_level_change`, `session_info`, `usage`, `compaction`, `branch_summary`,
`context_edit`, `label`, `custom`, and `leaf`.
Verification: `crates/codie-session/tests/format.rs::spec_sess_021_every_entry_type_roundtrips_with_a_stable_tag`

**SESS-022 (MUST)** The reader accepts format version 1 only and refuses versions 0 and 2 with an error naming
the version.
Verification: `crates/codie-session/tests/format.rs::spec_sess_022_unsupported_versions_are_refused`

**SESS-023 (MUST)** `codie sessions list`, `show`, and `export` are read-only: they never modify a session file,
and export reproduces it exactly.
Verification: `crates/codie-cli/tests/cli.rs::sessions_list_show_and_export_are_read_only`

# State machine / data shapes

A branch selection is persisted as a `leaf` entry whose `parent_id` and `target_id` both name the selected entry.
The marker is skipped by `projection`; later appends use it as their parent. This makes `--resume` land on the
same branch even when the branch point was chosen without sending a follow-up message.

Fork and clone headers add an optional field:

```json
{"type":"session","version":1,"id":"...","timestamp":"...","cwd":"...","parent_session":"..."}
```

# Edge cases

- Forking an empty session creates a lazy file with the new header and no entries.
- Cloning preserves branch markers, so the clone's leaf matches the source.
- A fork at an unknown entry is an error and does not create a file.
- A prefix matching more than one session is refused rather than guessed.
- Listing skips unreadable files instead of failing the whole picker.
- The 10,000-entry budget is measured in a debug build and includes projection and token estimation.

# Non-goals

The on-disk tree encoding is unchanged from `SPEC-SESS-001`; this spec does not introduce a new version. RPC
access to sessions is deferred to Phase 5. Cross-process locking is deferred to the release-hardening phase.

# Security and performance

Fork and clone write a new file under the session directory and never touch the source. Export is read-only.
Resume is linear in file size; the measured budget is in SESS-020. Session files may contain prompts and tool
output, so exports inherit the user's umask and are never written to the config directory automatically.
