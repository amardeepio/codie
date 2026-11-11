# codie-prompt

Deterministic system prompt assembly for codie.

The prompt is an ordered set of sections: preamble, tools, rules, project
context (when present), and cwd. Non-preamble sections are XML-wrapped and
sections are joined by a blank line. `discover_project_context` walks from the
working directory to the filesystem root, loading one instruction file per
directory in the order `AGENTS.override.md`, `AGENTS.md`, `CLAUDE.md`, with the
topmost ancestor first.

- Plan: [../../PLAN.MD](../../PLAN.MD)
- Specs: [../../specs/](../../specs/)

See `PLAN.MD` §6.8 for the normative prompt contract.
