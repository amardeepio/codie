# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html). Before 1.0, breaking changes bump the minor version;
from 1.0 onward, they bump the major version. Session files written by a stable release remain readable across
that major series, with migrations applied on read.

## [Unreleased]

### Added

- Session tree operations: persisted branch markers, `Session::fork_at`, `Session::clone_session`, and
  `Session::tree`, with `--resume`/`-r`, `--fork`, `--clone`, and `--session` accepting an identifier or path.
- Interactive session controls: `/tree`, `/fork [entry]`, `/clone`, `/name [text]`, `/export [path]`, and a
  `--resume` picker with search, sorting, rename, and delete.
- Manual compaction via `/compact [instructions]` with a fixed summary structure, a tool-call-safe cut point, and
  append-only `compaction` entries.
- Automatic compaction before a request when the estimated context exceeds the model window minus reserve, plus
  one-shot overflow recovery using `Agent::resume`.
- Token estimation (`chars / 4`, images as 4,800 characters) and a context percentage in the footer.
- Read-only `codie sessions list/show/export` commands.
- Session format v1 is frozen: new `compaction`, `branch_summary`, `context_edit`, `label`, `custom`, and `leaf`
  entry types, `parent_session` header provenance, and tests for round-tripping and refusing unknown versions.
- Property test for random append/branch/compaction sequences, a 10,000-entry resume benchmark, and a SIGKILL
  mid-stream recovery test.
- Read-time migration harness with a monotonic table and unsupported-version refusal.

### Changed

- `Agent::resume` continues a transcript without duplicating the user prompt, used by overflow recovery.
- `Session::set_leaf` now appends a persisted leaf marker, so resuming lands on the selected branch.
- `Session::projection` honors compaction, branch summaries, and context edits, and always retains the system
  message.
- Workspace version remains `0.2.0` until the Phase 3 release.

### Fixed

- Nothing yet.

### Removed

- Nothing yet.

### Security

- Nothing yet.

## [0.2.0] - 2026-11-11

Phase 2: the interactive terminal experience.

### Added

- Interactive app in the new `codie-tui` crate: alternate-screen transcript, fixed multiline editor, streaming
  assistant rendering, thinking and tool blocks, footer, and plain mode.
- Slash commands (`/help`, `/hotkeys`, `/new`, `/session`, `/model`, `/thinking`, `/compact`, `/quit`),
  steering and follow-up queues, `Ctrl+O` tool expansion, history, undo/redo, kill/yank, and large-paste markers.
- Permission prompts for mutating tools with allow-once, always, and deny; read-only tools never prompt.
- Themes: `system`, `dark`, `light`, and semantic TOML files with a 4.5:1 body contrast check and hot reload.
- Configurable keybindings in `keybindings.toml`, including unbinding an action with an empty list.
- `@file` completion using the `ignore` walker, in addition to slash-command completion.
- `--tui-mode full|inline|plain` and `--theme` on the `codie` CLI.
- PTY integration tests for SIGINT/SIGTERM terminal restoration and append-only plain mode.
- `cargo bench -p codie-tui --bench streaming`: 5,000 tokens over 60 frames in under 40 ms (0.4–0.6 ms per frame).
- Accepted `SPEC-TUI-001` with all ten requirements covered by named tests.

### Changed

- Running `codie` in a terminal now launches the interactive interface instead of erroring; `-p` and
  `--mode json` keep the headless paths.
- Workspace version is `0.2.0`.

### Fixed

- Running `codie` with no arguments in a terminal now launches the interactive interface instead of printing
  help and exiting. Non-interactive invocations still read the prompt from stdin and fail with an actionable
  message when it is empty.

### Removed

- Nothing yet.

### Security

- Nothing yet.

## [0.1.0] - 2026-11-11

Phase 1: headless core. `codie -p "..."` performs real multi-step coding work against DeepSeek and OpenRouter.

### Added

- Phase 0 foundations: Rust workspace (`codie-core`, `codie-ai`, `codie-tools`, `codie-session`,
  `codie-cli`), `codie-testkit` harness, and `#[spec]` attribute macro.
- OpenAI-compatible provider for DeepSeek and OpenRouter: SSE reassembly, retries with `retry-after`,
  reasoning replay, usage normalization, integer nano-USD cost, and a compatibility matrix.
- Agent loop with steering and follow-up queues, a turn budget, cooperative abort, loop hooks, and panic-safe
  tool execution.
- Tools `read`, `write`, `edit`, and `bash`, with JSON Schema argument validation, per-path mutation locks,
  output truncation, process-group kill, timeouts, and progress updates.
- Append-only JSONL sessions with `--continue`, model and thinking-level entries, system-message persistence,
  and projection from the current leaf.
- System-prompt assembly with preamble, tools, rules, `AGENTS.override.md`/`AGENTS.md`/`CLAUDE.md` discovery,
  and cwd.
- Print mode and `--mode json`, the published event schema at `schemas/json-events.schema.json`, and
  schema-validated JSONL output.
- Documented exit codes: 0 success, 1 usage/runtime error, 2 unconfigured auth check, 130/143/129 for
  SIGINT/SIGTERM/SIGHUP.
- Auth through `--api-key`, `auth.json`, and provider environment variables, plus `codie auth
  login/logout/status`, `codie models`, and `codie config get`.
- OpenRouter catalog refresh: four-hour cache, ETag revalidation, four-second timeout, stale-on-failure, and
  background execution; text-only chat models are filtered for selection.
- `--base-url` and `CODIE_BASE_URL` for proxies and deterministic tests.
- Fake-provider golden-path end-to-end test, recorded DeepSeek/OpenRouter fixtures, an SSE libFuzzer target,
  and a deterministic malformed-input test.
- Nine additional accepted specs (12 total, 92 requirements), all covered by named tests.
- `cargo xtask coverage` and an 80 percent line-coverage gate for `core`, `ai`, and `tools`.

### Changed

- Repository automation runs through `cargo xtask` (`ci`, `coverage`, `spec-check`, `spec-report`, `dist`).
- Workspace version is `0.1.0`.

### Fixed

- Nothing yet.

### Removed

- Nothing yet.

### Security

- API keys are redacted from trace logs and never persisted by `--api-key`; `auth.json` is written atomically
  with mode `0600`.

## [0.0.0] - 2026-11-11

Initial repository skeleton. No user-facing agent functionality is included in this release.

### Added

- Cargo workspace with `codie-core`, `codie-ai`, `codie-tools`, `codie-session`, `codie-cli`,
  `codie-testkit`, and the `codie-spec-macro` helper.
- Pinned Rust 1.88 toolchain, workspace lints, formatting and clippy configuration, and a committed lockfile.
- Stable error model with machine-readable codes (`config_error`, `io_error`, `provider_error`, `tool_error`,
  `session_error`, `internal_error`).
- `codie-testkit` with an isolated temporary home, a scripted SSE fixture helper, and the `#[spec]` attribute.
- Specification framework: process documentation, template, glossary, and three accepted specs covering the
  message model, the agent turn loop, and the OpenAI-compatible provider adapter.
- Documentation baseline: architecture decision records for Rust and edition 2024, out-of-process extensions,
  and macOS-first platform support; RFC process and template; mdBook skeleton.
- Project baseline: README, contributing guide, roadmap, governance, changelog, and issue/PR templates.
