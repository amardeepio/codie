# codie-session

Append-only JSONL session storage for codie. Phase 0 pins the on-disk format
version so that later work cannot silently change it.

Status: **Phase 0 scaffold** — the session header, reader, writer, and
projection are Phase 1 work; the format is frozen in Phase 3.

- Plan: [../../PLAN.MD](../../PLAN.MD)
- Specs: [../../specs/](../../specs/)

See `PLAN.MD` §6.6 for the normative session contract.
