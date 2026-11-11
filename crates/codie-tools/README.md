# codie-tools

The built-in tool vocabulary and Phase 1 implementations for codie.

- `read`: UTF-8 text files with `offset`/`limit` and head truncation (2000
  lines / 50 KiB).
- `write`: create or overwrite files and parent directories.
- `edit`: exact-text replacements with uniqueness, overlap, BOM, and line-ending
  preservation.
- `bash`: `/bin/bash -c` execution with optional timeout and tail truncation.
- `truncate`: shared head/tail/line truncation helpers.
- `path`: `~`, `@`, `file://`, and Unicode-space path normalization.
- `registry`: built-in registry plus a `Policy`/`PolicyToolbox` permission
  wrapper.

`grep`, `find`, and `ls` follow in Phase 2.

- Plan: [../../PLAN.MD](../../PLAN.MD)
- Specs: [../../specs/](../../specs/)

See `PLAN.MD` §6.5 and Appendix B for the normative tool contract.
