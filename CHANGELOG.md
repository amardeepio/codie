# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html). Before 1.0, breaking changes bump the minor version;
from 1.0 onward, they bump the major version. Session files written by a stable release remain readable across
that major series, with migrations applied on read.

## [Unreleased]

### Added

- Interactive phase 2 foundation in the new `codie-tui` crate: alternate-screen transcript, fixed multiline
  editor, streaming assistant rendering, thinking and tool blocks, footer, and plain mode.
- Slash commands (`/help`, `/hotkeys`, `/new`, `/session`, `/model`, `/thinking`, `/compact`, `/quit`),
  steering and follow-up queues, `Ctrl+O` tool expansion, history, undo/redo, kill/yank, and large-paste markers.
- Permission prompts for mutating tools with allow-once, always, and deny; read-only tools never prompt.
- Themes: `system`, `dark`, `light`, and semantic TOML files with a 4.5:1 body contrast check.
- `--tui-mode full|inline|plain` and `--theme` on the `codie` CLI.
- Draft `SPEC-TUI-001` covering layout, streaming, editor, markdown, tool blocks, footer, themes, restore, and
  plain mode.

### Changed

- Running `codie` in a terminal now launches the interactive interface instead of erroring; `-p` and
  `--mode json` keep the headless paths.

### Fixed

- Nothing yet.

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
