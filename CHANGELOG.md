# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html). Before 1.0, breaking changes bump the minor version;
from 1.0 onward, they bump the major version. Session files written by a stable release remain readable across
that major series, with migrations applied on read.

## [Unreleased]

### Added

- Phase 0 foundations: Rust workspace (`codie-core`, `codie-ai`, `codie-tools`, `codie-session`,
  `codie-cli`), `codie-testkit` harness, and `#[spec]` attribute macro.
- Specification framework with three accepted specs (`SPEC-MSG-001`, `SPEC-AGENT-001`, `SPEC-PROV-001`),
  45 tracked requirements, and `cargo xtask spec-check` / `spec-report`.
- macOS-first CI (arm64 + x64), cargo-deny and RustSec audit jobs, and OSS project scaffolding.

### Changed

- Repository automation runs through `cargo xtask` (`ci`, `spec-check`, `spec-report`, `dist`).

### Fixed

- Nothing yet.

### Removed

- Nothing yet.

### Security

- Nothing yet.

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
