# codie-tools

The built-in tool vocabulary for codie. Phase 0 fixes the stable `ToolName`
spellings used on the wire, in prompts, and in permission policies, plus
their `Display`/`FromStr` conversions.

Status: **Phase 0 scaffold** — the `Tool` trait and the `read`, `write`,
`edit`, and `bash` implementations are Phase 1 work; `grep`, `find`, and
`ls` follow in Phase 2.

- Plan: [../../PLAN.MD](../../PLAN.MD)
- Specs: [../../specs/](../../specs/)

See `PLAN.MD` §6.5 for the normative tool contract.
