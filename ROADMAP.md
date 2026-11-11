# Roadmap

codie is built in gated phases. Each phase ends with a demoable, installable artifact and a tagged pre-release.
No phase starts before the previous phase's exit criteria are met. Durations are planning bounds for one
full-time engineer, not commitments; the exit criteria are the real gate.

**Current status: Phase 2 (Interactive TUI), complete.** `0.1.0` shipped the headless core and `0.2.0` ships
the interactive app: streaming transcript, editor dock, keybindings, themes, slash commands, steering,
permission prompts, plain mode, PTY restore tests, and the streaming benchmark. Phase 3 is next.

## Phase summary

| Phase | Focus | Duration | Cumulative | Artifact | Status |
|---|---|---|---|---|---|
| 0 | Foundations: workspace, CI, spec framework, OSS files | 1 week | 1 week | repository + CI | Complete |
| 1 | Headless core: providers, loop, tools, sessions, print/JSON | 3 weeks | 4 weeks | `0.1.0` — `codie -p` works | Complete |
| 2 | Interactive TUI: streaming render, editor, themes, steering | 3 weeks | 7 weeks | `0.2.0` — interactive agent | Complete |
| 3 | Sessions at full fidelity: resume, fork, compaction, picker | 2 weeks | 9 weeks | `0.3.0` — resume/fork/compact | Planned |
| 4 | Customization and trust: skills, templates, `SYSTEM.md`, images | 2 weeks | 11 weeks | `0.4.0` — skills/templates/trust | Planned |
| 5 | Protocols and polish: RPC, schemas, export, performance | 2 weeks | 13 weeks | `0.5.0` — RPC/export | Planned |
| 6 | Extensions: JSON-RPC protocol, hooks, manifests, examples | 3 weeks | 16 weeks | `0.6.0` — extension protocol | Planned |
| 7 | Release hardening: macOS matrix, signing, notarization, fuzzing | 2 weeks | 18 weeks | `1.0.0` | Planned |

## Phase 0: Foundations

Goal: a repository that can accept code safely.

- Workspace skeleton, pinned toolchain, shared lints, formatting and clippy configuration.
- Error model with stable codes, tracing setup, and feature flags.
- Spec framework: tree, template, `xtask spec-check`, ADR and RFC templates.
- CI: one workspace build job on macOS x64; the full format/lint/test/spec/docs/machete/deny suite runs
  locally via `cargo xtask ci` and before releases.
- OSS files and documentation skeleton.
- `codie-testkit`: fake provider, temporary-home harness, `#[spec]` macro, snapshot helpers.

Exit criteria (met 2026-11-11): `cargo xtask ci` green on macOS x64; arm64 verified locally; a 1.22 MiB `codie --version`
binary builds; the spec template and three accepted specs (45 requirements) exist.

## Phase 1: Headless core

Goal: `codie -p "..."` performs real multi-step coding work.

Status: complete 2026-11-11.

- OpenAI-compatible adapter for DeepSeek and OpenRouter, including SSE, retries, and error mapping.
- Bundled model catalog and `codie models`.
- Auth through environment variables, `auth.json`, and `--api-key`.
- Message model, event stream, agent loop with a turn budget.
- Tools: `read`, `write`, `edit`, `bash`, with truncation, process-group kill, and timeout.
- Session create, append, and resume; system-message persistence.
- Prompt assembly with preamble, tools, rules, project context, and cwd.
- Print mode and JSON event mode.
- Permission policy v0: non-interactive deny for mutating tools unless allowed.

Exit criteria (met 2026-11-11): a fake-provider end-to-end refactor scenario with a transcript snapshot; live
smoke runs against DeepSeek and OpenRouter; JSON output validated against `schemas/json-events.schema.json`;
documented exit codes including signals; 89.07 / 88.89 / 86.73 percent line coverage in `core`, `ai`, and
`tools`; 12 accepted specs and 92 requirements covered by named tests.

## Phase 2: Interactive TUI

Goal: the primary human experience.

Status: complete 2026-11-11.

- ratatui app with an alternate-screen transcript, fixed editor dock, and footer.
- Streaming assistant text, thinking blocks, and stateful tool blocks with inline diff colors.
- Multiline editor with grapheme movement, history, undo/redo, kill/yank, paste markers, and completion.
- Steering and follow-up queues, permission prompts, slash commands, themes, keybindings, and plain mode.
- PTY restore tests and a 5,000-token streaming benchmark.

Exit criteria (met 2026-11-11): 5,000 tokens over 60 frames render in under 40 ms (0.4–0.6 ms per frame); PTY tests
verify terminal restore on SIGINT and SIGTERM; plain mode is append-only; `SPEC-TUI-001` is accepted with all ten
requirements tested.

## Later phases

- **Phase 3:** session tree operations, fork and clone, compaction, token estimation, picker, format freeze.
- **Phase 4:** skills, prompt templates, project trust, `SYSTEM.md`, config reload, `@file` and image inputs.
- **Phase 5:** RPC mode, published JSON schemas, HTML export, update checks, performance pass.
- **Phase 6:** out-of-process extension protocol, hooks, custom tools and commands, example extensions.
- **Phase 7:** macOS terminal and packaging matrix, signing, notarization, fuzzing, SBOM, 1.0 release.

Full scope, tasks, and exit criteria are in [`PLAN.MD`](PLAN.MD) §8. Platform tiers are in
[`docs/src/platforms.md`](docs/src/platforms.md). Scope changes require an RFC; phase changes require maintainer
approval recorded in [`docs/adr/`](docs/adr/).
