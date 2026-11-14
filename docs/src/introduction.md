# Introduction

`codie` is a terminal-first coding agent written in Rust. It reads and edits files, runs commands, and works
through multi-step tasks with a language model, while keeping a durable, append-only record of everything that
happened. It is a deliberate reimplementation of the architecture proven by [`pi`](https://github.com/earendil-works/pi),
with a Rust core, a stricter safety posture, and a deliberately narrow provider scope.

## Status

**Release candidate (`0.9.0-rc.1`).** The workspace, the error model, the test harness, and the
documentation and specification baseline are in place, and the agent runs end to end. Phase 1 added the headless
core (`codie -p "..."`, `--mode json`), Phase 2 the interactive terminal app, Phase 3 full-fidelity sessions,
Phase 4 customization and trust, Phase 5 RPC mode, generated schemas, HTML export, update checks, and the
performance gate, and Phase 6 out-of-process extensions with hooks, custom tools and commands, and generated
schemas. The `0.6.0` release refreshed the terminal UI, `0.7.0` added the extension protocol plus an
OpenRouter coding-model catalog and in-session `/provider` switching, `0.8.0` added rich edit/write previews,
collapsed reasoning, `Esc`/`/stop` cancellation, a context meter and `/usage`, permission review previews, tool
navigation, folded turns, and session picker previews, and `0.9.0-rc.1` adds Windows x64 preview packaging, a
PowerShell tool, the one-command Windows installer, and the release-candidate documentation set. See the
[roadmap](roadmap.md), [RPC mode](rpc.md), [configuration](configuration.md), and
[`ROADMAP.md`](../../ROADMAP.md).

## What it will do

- Run headless with `codie -p "..."` for scripts and CI, or interactively in a terminal UI.
- Use OpenRouter or DeepSeek through one OpenAI-compatible adapter with per-provider compatibility flags.
- Execute a small, well-specified tool set: `read`, `write`, `edit`, `bash` (or `powershell` on Windows),
  `grep`, `find`, `ls`, and `web_fetch`.
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
