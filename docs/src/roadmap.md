# Roadmap

codie is developed in gated phases. Each phase ends with a demoable, installable artifact and a tagged
pre-release. No phase starts before the previous phase's exit criteria are met. Durations are planning bounds,
not commitments; the exit criteria are the real gate.

## Phases

| Phase | Focus | Duration | Cumulative | Artifact | Status |
|---|---|---|---|---|---|
| 0 | Foundations: workspace, CI, spec framework, OSS files | 1 wk | 1 wk | repository + CI | Complete |
| 1 | Headless core: providers, loop, tools, sessions, print/JSON | 3 wk | 4 wk | `0.1.0` — `codie -p` works | Complete |
| 2 | Interactive TUI: streaming render, editor, themes, steering | 3 wk | 7 wk | `0.2.0` — interactive agent | Complete |
| 3 | Sessions at full fidelity: resume, fork, compaction, picker | 2 wk | 9 wk | `0.3.0` — resume/fork/compact | Complete |
| 4 | Customization and trust: skills, templates, `SYSTEM.md`, images | 2 wk | 11 wk | `0.4.0` — skills/templates/trust | Complete |
| 5 | Protocols and polish: RPC, schemas, export, performance | 2 wk | 13 wk | `0.5.0` — RPC/export | Complete |
| 6 | Extensions: JSON-RPC protocol, hooks, manifests, examples | 3 wk | 16 wk | `0.7.0` — extension protocol | Planned |
| 7 | Release hardening: macOS matrix, signing, notarization, fuzzing | 2 wk | 18 wk | `1.0.0` | Planned |

## Phase 0 exit criteria (met 2026-11-11)

- Workspace skeleton and pinned toolchain build on macOS x64 in CI and arm64 locally.
- CI runs `cargo build --workspace --all-targets` on macOS x64; the full format/lint/test/spec/docs/
  machete/deny suite runs locally via `cargo xtask ci`.
- A trivial `codie --version` binary builds (1.22 MiB).
- Spec template plus three accepted specs (45 requirements) exist.
- OSS and documentation baseline is complete.

## Phase 1 exit criteria (met 2026-11-11)

- A golden-path end-to-end test with a fake provider performs read, edit, bash test, and final answer, and the
  transcript snapshot matches.
- Manual smoke tests against DeepSeek and OpenRouter pass with real credentials; CI stays offline.
- JSON mode output validates against `schemas/json-events.schema.json`.
- Print-mode exit codes match the CLI spec, including 130/143/129 for signals.
- Line coverage is 89.07 percent in `core`, 88.89 percent in `ai`, and 86.73 percent in `tools`;
  `cargo xtask coverage` enforces the 80 percent floor.
- 12 accepted specs, 92 requirements, and 92 requirements with named tests (`spec-check --strict` is green).

## Phase 2 exit criteria (met 2026-11-11)

- Streaming renders 5,000 tokens over 60 frames in under 40 ms total (0.4–0.6 ms per frame, over 140k tokens/s) via
  `cargo bench -p codie-tui --bench streaming`.
- PTY integration tests verify terminal restore on SIGINT and SIGTERM.
- Rendering stays far below the 16.7 ms frame budget; the benchmark asserts both the total and slowest frame.
- Accessibility: `--tui-mode plain` produces append-only output, verified in a PTY.
- `SPEC-TUI-001` is accepted and all ten requirements have named tests.

## Phase 3 exit criteria (met 2026-11-12)

- Session tree operations, persisted leaf markers, fork, clone, export, and read-only `codie sessions` commands.
- `/tree`, `/fork`, `/clone`, `/name`, `/export`, and a searchable picker with rename and delete.
- Compaction with a fixed summary structure, a tool-call-safe cut point, manual `/compact [instructions]`,
  automatic threshold triggering, and one-shot overflow recovery.
- Token estimates and context percentage, a frozen format v1, a migration harness, a property test, a 10k-entry
  resume budget, and a SIGKILL recovery test.
- Random append/branch/compaction sequences reopen to the same projection; resume after `kill -9` loses at most
  the in-flight partial message; 10k-entry resume stays within 150 ms.
- Session format v1 is documented and announced as stable for the 1.x series; released as `0.3.0`.

## Phase 4 exit criteria (met 2026-11-12)

- Skills, templates, `SYSTEM.md`/`APPEND_SYSTEM.md`, and `@file` attachments are implemented with discovery,
  override, and expansion fixtures.
- Project trust is fail-closed: untrusted `.codie/` resources contribute nothing, `--approve` persists the
  canonical path, and malformed trust files grant no access.
- `web_fetch` rejects local and private destinations, re-validates redirects, and caps bytes and time.
- User-message images are sent as `image_url` data URLs (`PROV-023`), covered by a fake-provider request-body test; tool-result images stay text notes.
- Released as `0.4.0`.

## Phase 5 exit criteria (met 2026-11-12)

- RPC mode covers the documented command matrix, streams agent events, echoes command ids, and shuts down
  cleanly on EOF and signals.
- RPC schemas are generated from Rust types and verified in CI (`cargo xtask schemas --check`).
- HTML export is self-contained and escapes transcript content.
- `codie update` installs a checksum-verified prebuilt release; `--check` only reports.
- `examples/rpc-client` demonstrates an integration.
- Release budgets pass: 10.3 MB binary, 8 ms startup p95, 5.5 MB idle RSS.
- Released as `0.5.0`.

## 0.6.0 TUI polish (released 2026-11-12)

An interim release after Phase 5; Phase 6 now targets `0.7.0`.

- OpenCode-inspired dark and light palettes and semantic theme tokens.
- A `Codie!` startup banner and a README screenshot.
- Colored markdown for headings, strong text, emphasis, code, links, quotes, lists, and rules.
- Syntax highlighting for comments, strings, numbers, keywords, and function calls.

Later phase criteria are listed in [`PLAN.MD`](../../PLAN.MD) §8. This page mirrors `ROADMAP.md` and is updated at
each phase boundary. Scope changes require an RFC; phase changes require maintainer approval recorded in
[`docs/adr/`](../adr/).
