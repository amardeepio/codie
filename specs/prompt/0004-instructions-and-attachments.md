---
id: SPEC-PRMPT-002
title: System instructions and attachments
status: implemented
version: 1.0.0
owners: ["@codie-maintainers"]
requirements: ["PRMPT-009", "PRMPT-010", "PRMPT-011"]
---

# Context

`SPEC-PRMPT-001` assembles the system prompt from a preamble, tool declarations, rules, project instructions, and
cwd. Users need to replace or extend the preamble without editing source, and they need to point the model at
files and images from the prompt line. Both surfaces must stay deterministic and keep untrusted project content
behind the trust boundary (`SPEC-TRUST-001`).

# Normative behavior

**PRMPT-009 (MUST)** `SYSTEM.md` replaces the default preamble and `APPEND_SYSTEM.md` appends to it. Both are
discovered from the working directory to the filesystem root; the nearest `SYSTEM.md` wins and every
`APPEND_SYSTEM.md` from the topmost ancestor to the working directory is appended in order. `--system-prompt`
overrides file discovery for the preamble, and `--append-system-prompt` is appended last.
Verification: `crates/codie-prompt/tests/instructions.rs::spec_prmpt_009_system_and_append_instructions`

**PRMPT-010 (MUST)** `@path` tokens in a submitted prompt are expanded before the message is sent. Text files are
inserted as `<file path="...">...</file>` blocks; image files (`.png`, `.jpg`, `.jpeg`, `.gif`, `.webp`) become
base64 image content with the matching MIME type. Missing or unreadable paths produce a visible error notice and
the token is left in place. Directories, files over 10 MiB, and attachments beyond 20 per message are refused.
Verification: `crates/codie-prompt/tests/attachments.rs::spec_prmpt_010_expands_files_and_images`

**PRMPT-011 (MUST)** Attachment expansion is deterministic for unchanged files, preserves prompt text order, and
never evaluates file contents. Duplicate references are expanded once per message and the second occurrence
carries only the path, not the content.
Verification: `crates/codie-prompt/tests/attachments.rs::spec_prmpt_011_duplicate_references_expand_once`

# Data shapes

```text
Explain @src/main.rs and compare it with @diagram.png
```

becomes a user message with the original text plus one `<file path="src/main.rs">` block and one image block.

# Edge cases

- `@` inside an email address or a path segment is not an attachment token unless it starts a word.
- An `@file` reference in a template or skill body expands the same way.
- Attachments are recorded in the session like any other message content.
- Running headless without an image-capable model keeps the image block; the provider decides how to present it
  (`PROV-017`).

# Non-goals

No URL attachments (use `web_fetch`), no glob expansion, and no directory trees.
