---
id: SPEC-PRMPT-005
title: Attachment token paths
status: accepted
version: 1.0.0
owners: ["@codie-maintainers"]
requirements: ["PRMPT-012"]
---

# Context

`SPEC-PRMPT-002` defines `@path` expansion. Unquoted tokens end at the first whitespace, so a path that contains
spaces — the default name of every macOS screenshot — cannot be attached. Finder drag-and-drop and the terminal
also emit backslash-escaped spaces. Both forms must resolve to the same file.

# Normative behavior

**PRMPT-012 (MUST)** An attachment token may be quoted with `"` or `'`, in which case it runs to the matching
quote and may contain spaces. An unquoted token may escape a space or backslash with `\`. Both forms attach the
same file as an unquoted token, keep `[image: ...]` or `<file>` rendering identical, and count toward the same
attachment limit. `quote_attachment_path` produces the canonical token: unquoted when the path has no whitespace,
double-quoted otherwise. In addition, a prompt whose whole text is a single existing file path — absolute, `~/`,
relative, quoted, or with backslash-escaped spaces — is converted to that canonical token before submission, so a
pasted or dragged path attaches the file instead of reaching the model as text.
Verification: `crates/codie-prompt/tests/attachments.rs::spec_prmpt_012_quoted_and_escaped_attachment_paths`

# Data shapes

```text
@"Screenshot 2026-11-16 at 3.24.22 PM.png"
@Screenshot\ 2026-11-16\ at\ 3.24.22\ PM.png
@src/main.rs
```

All three are single attachment tokens.

# Edge cases

- A quote with no closing quote leaves the `@` in the text and records no attachment.
- An empty quoted token (`@""`) attaches nothing and leaves the text untouched.
- A trailing backslash is kept as a literal backslash in the path.
- An email address is still not a token: the `@` must start a word.
- Windows separators are not rewritten; the token is resolved as written.
- A whole-prompt path conversion requires the file to exist; a missing path stays plain text.
- A single word such as `README.md` that is not path-like (no separator, no image extension) is not converted.
