# codie-session

Append-only JSONL session storage for codie.

Each session file starts with a `session` header (format version 1) and records
one entry per line. Entries form a tree through `parent_id`; appending never
rewrites history. The file is created lazily when the first message is
appended, and the projection walks the current leaf back to its root.

- Plan: [../../PLAN.MD](../../PLAN.MD)
- Specs: [../../specs/](../../specs/)

See `PLAN.MD` §6.6 for the normative session contract. Compaction and context
edits are Phase 3 work; Phase 1 projects the full branch.
