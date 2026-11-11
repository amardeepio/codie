# Roadmap

codie is developed in gated phases. Each phase ends with a demoable, installable artifact and a tagged
pre-release. No phase starts before the previous phase's exit criteria are met. Durations are planning bounds,
not commitments; the exit criteria are the real gate.

## Phases

| Phase | Focus | Duration | Cumulative | Artifact | Status |
|---|---|---|---|---|---|
| 0 | Foundations: workspace, CI, spec framework, OSS files | 1 wk | 1 wk | repository + CI | Complete |
| 1 | Headless core: providers, loop, tools, sessions, print/JSON | 3 wk | 4 wk | `0.1.0` — `codie -p` works | In progress |
| 2 | Interactive TUI: streaming render, editor, themes, steering | 3 wk | 7 wk | `0.2.0` — interactive agent | Planned |
| 3 | Sessions at full fidelity: resume, fork, compaction, picker | 2 wk | 9 wk | `0.3.0` — resume/fork/compact | Planned |
| 4 | Customization and trust: skills, templates, `SYSTEM.md`, images | 2 wk | 11 wk | `0.4.0` — skills/templates/trust | Planned |
| 5 | Protocols and polish: RPC, schemas, export, performance | 2 wk | 13 wk | `0.5.0` — RPC/export | Planned |
| 6 | Extensions: JSON-RPC protocol, hooks, manifests, examples | 3 wk | 16 wk | `0.6.0` — extension protocol | Planned |
| 7 | Release hardening: macOS matrix, signing, notarization, fuzzing | 2 wk | 18 wk | `1.0.0` | Planned |

## Phase 0 exit criteria (met 2026-11-11)

- Workspace skeleton and pinned toolchain build on macOS arm64 and x64.
- CI runs formatting, linting, tests, deny, audit, machete, docs, and MSRV checks; the Linux job is green or
  explicitly quarantined.
- A trivial `codie --version` binary builds (1.22 MiB).
- Spec template plus three accepted specs (45 requirements) exist.
- OSS and documentation baseline is complete.

## Phase 1 exit criteria

- A golden-path end-to-end test with a fake provider performs read, edit, bash test, and final answer, and the
  transcript snapshot matches.
- Manual smoke tests against DeepSeek and OpenRouter pass with real credentials; CI stays offline.
- JSON mode output validates against the published schema.
- Print-mode exit codes match the CLI spec.
- Line coverage in `core`, `ai`, and `tools` is at least 80 percent, excluding generated code.

Later phase criteria are listed in [`PLAN.MD`](../../PLAN.MD) §8. This page mirrors `ROADMAP.md` and is updated at
each phase boundary. Scope changes require an RFC; phase changes require maintainer approval recorded in
[`docs/adr/`](../adr/).
