# Introduction

`codie` is a terminal-first coding agent written in Rust. It reads and edits files, runs commands, and works
through multi-step tasks with a language model, while keeping a durable, append-only record of everything that
happened. It is a deliberate reimplementation of the architecture proven by [`pi`](https://github.com/earendil-works/pi),
with a Rust core, a stricter safety posture, and a deliberately narrow provider scope.

## Status

**Phase 0: pre-alpha.** The repository currently contains the workspace skeleton, the error model, the initial
test harness, and this documentation and specification baseline. The agent does not yet perform coding tasks.
Provider support, the turn loop, tools, sessions, and the TUI land in Phases 1 through 3. See the
[roadmap](roadmap.md) and [`ROADMAP.md`](../../ROADMAP.md).

Phase 1 is complete: `codie -p "..."` runs a headless coding session and `--mode json` emits the event stream.
Phase 2 is complete (`0.2.0`); run `codie` in a terminal for the interactive interface.

## What it will do

- Run headless with `codie -p "..."` for scripts and CI, or interactively in a terminal UI.
- Use OpenRouter or DeepSeek through one OpenAI-compatible adapter with per-provider compatibility flags.
- Execute a small, well-specified tool set: `read`, `write`, `edit`, `bash`, `grep`, `find`, and `ls`.
- Persist sessions as an append-only JSONL tree that supports resume, fork, and compaction.
- Stream assistant output as events so the UI and the session store consume the same source of truth.
- Support out-of-process extensions over JSON-RPC without loading foreign code into the core.

## Design principles

1. **Spec-first.** Behavior is defined by requirement IDs in `specs/`, and every `MUST` has a named test. No
   feature without a spec.
2. **Safe by default.** Read-only tools are allowed; mutating tools ask interactively and are denied in
   headless mode unless explicitly allowed. The policy engine is the control point, not path sandboxing.
3. **Provider discipline.** Two providers at launch, one adapter, recorded-fixture conformance. Breadth is
   added only when it can be tested.
4. **Small core, extension surface at the edges.** The loop, message model, and tool contract stay minimal;
   skills, templates, themes, and hooks layer on top.
5. **The transcript is the prompt.** System prompt and tool declarations are persisted and replayed. There is
   no hidden prompt state that can drift from history.
6. **Sessions are truth.** History is append-only. Fork, compaction, and context edits never rewrite the file;
   a projection derives what the model sees.

## Non-goals for v1.0

Sub-agents, plan mode, multi-agent orchestration, a web or desktop UI, OAuth login, built-in sandboxing, MCP and
LSP, additional native provider adapters, image generation, and Windows as a tier-1 platform. These are scoped
in `PLAN.MD` §1.2 and §12.6.

## Where to go next

- [Architecture](architecture.md) for the crate graph and runtime model.
- [Specifications](specs.md) for the requirement process and the accepted specs.
- [`PLAN.MD`](../../PLAN.MD) for scope, phases, and the decision log.
- [`CONTRIBUTING.md`](../../CONTRIBUTING.md) to build and contribute.
