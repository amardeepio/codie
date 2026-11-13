# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html). Before 1.0, breaking changes bump the minor version;
from 1.0 onward, they bump the major version. Session files written by a stable release remain readable across
that major series, with migrations applied on read.

## [Unreleased]

### Added

- Nothing yet.

### Changed

- Nothing yet.

### Fixed

- Nothing yet.

### Removed

- Nothing yet.

### Security

- Nothing yet.

## [0.7.0] - 2026-11-13

### Added

- Out-of-process extensions (`codie-ext`): a versioned JSON-RPC 2.0 protocol over stdio with process
  supervision, capability negotiation, hook and tool timeouts, and process-group termination.
- Hook points `session_start`, `session_shutdown`, `before_agent_start`, `context`, `before_tool_call`,
  `after_tool_call`, `turn_end`, and `user_submit`.
- Custom extension tools, slash commands, deferred command-line flags, and structured render hints
  (`text`, `markdown`, `code`, `diff`).
- Extension discovery from user config, trusted project roots, and `--extension`; `--no-extensions` and
  `[extensions] disabled` configuration.
- Generated extension manifest, protocol, and hook schemas.
- Python and TypeScript SDKs and `todo`, `plan-mode`, and `subagent-lite` examples.
- OpenRouter model picker backed by the live catalog: coding models are filtered by the `programming` category
  or a coding index of at least 70, carry their real context window, output limit, price, and reasoning efforts,
  and refresh in the background.
- `/provider` in the TUI switches between DeepSeek and OpenRouter in place, selects the provider default, and
  keeps the session running.

### Changed

- `codie models` lists the merged OpenRouter catalog from the local cache and warns when that cache is stale.
- A configured `default_model` is only used when it belongs to the selected provider; otherwise codie falls
  back to the provider default.
- `codie config get` and `codie config set` understand `extensions-disabled`.
- Project trust now covers `.codie/extensions`.

### Fixed

- The interactive TUI now refreshes the OpenRouter catalog instead of only headless runs doing so.
- The OpenRouter model picker no longer shows only `openrouter/auto`; it lists the merged coding catalog.

### Removed

- Nothing yet.

### Security

- Extension children inherit a sanitized environment with no provider API keys or `TOKEN` variables.
- Extension tools go through the same headless deny and interactive ask policy as built-in mutating tools.

## [0.6.1] - 2026-11-12

### Added

- Release workflow (`.github/workflows/release.yml`) that publishes checksum-verified archives for macOS
  arm64/x64 and Linux arm64/x64 on every `v*` tag. `install.sh` and `/update` now install a prebuilt binary
  instead of compiling from source. Linux archives are built in an Ubuntu 20.04 container so they support
  glibc 2.31 and newer.
- `codie update` now installs a newer release and reports the installed version; `--check` still only reports.
- A Linux build job in CI so published Linux artifacts stay tested.

### Changed

- Update checks probe for a prebuilt archive for the current platform before advertising a release. A tag
  without artifacts is reported with the installer command instead of an update that `/update` cannot install.
- The TUI `/update` banner, plain mode, and `codie update` print the installer command when no artifact exists.

### Fixed

- Nothing yet.

### Removed

- Nothing yet.

### Security

- Nothing yet.

## [0.6.0] - 2026-11-12

### Added

- OpenCode-inspired built-in themes: a near-black canvas, warm peach focus, purple headings, blue secondary
  text, green code, and distinct colors for warnings, emphasis, quotes, and lists. Light mode uses the same
  hues darkened for contrast.
- A `Codie!` block-art banner on the empty interactive screen, with a plain-text fallback on narrow terminals.
- A README screenshot of the interactive app with the command palette open.
- Semantic theme tokens for surfaces, interactive colors, borders, markdown, syntax, and diffs:
  `surface`, `surface_alt`, `primary`, `secondary`, `warning`, `info`, `border_active`, `border_subtle`,
  `heading`, `link`, `code`, `strong`, `emphasis`, `quote`, `list`, `list_enum`, and `rule`.

### Changed

- The built-in dark and light palettes now follow OpenCode's default theme.
- User messages render on a raised surface with a colored left bar; the editor, footer, overlays, selected
  rows, tool states, and diff output use the new palette.
- Markdown renders headings, strong text, emphasis, inline code, links, block quotes, list markers, and rules
  in distinct colors instead of a single accent.
- Fenced code highlighting now covers comments, strings, numbers, keywords, and function calls.
- Custom theme files remain compatible. Omitted semantic tokens fall back from `accent`, `primary`, `success`,
  `warning`, `info`, and `muted`.

### Fixed

- `install.sh` selects the highest semantic version from the tags API even when GitHub returns compact JSON,
  and warns when another `codie` earlier in `PATH` shadows the new install.

### Removed

- Nothing yet.

### Security

- Nothing yet.

## [0.5.0] - 2026-11-12

### Added

- RPC mode (`--mode rpc`): line-delimited JSON commands and events for editor integrations, with prompt, steer,
  follow-up, abort, session inspection, model/thinking changes, compaction, HTML export, and orderly shutdown.
- `codie-protocol` crate with published, generated JSON schemas for RPC commands and server records
  (`cargo xtask schemas [--check]`).
- Self-contained HTML transcript export: `codie sessions export --format html`, `/export html`, and the RPC
  `export_html` command.
- `/copy` copies the last assistant message to the clipboard; `/debug` prints diagnostics.
- `codie update` installs the latest checksum-verified prebuilt release; `--check` only reports it.
- `examples/rpc-client`, a minimal Python integration.
- The interactive app checks for a newer release in the background and reports it in the transcript. `/update`
  shows a checking banner, installs a newer release, or reports "No update found" with an OK prompt. The
  background check never installs automatically; disable it with `CODIE_NO_UPDATE_CHECK=1`.
- Panic crash reports under the data directory (`crashes/crash-<timestamp>.txt`); they contain the panic
  message, location, and backtrace, never environment variables or transcript content.

### Changed

- `codie sessions export` now takes `--format jsonl|html` (JSONL remains the default).
- The footer abbreviates token totals (`999`, `1k`, `70.2k`, `1.2M`) instead of printing raw digits.

### Fixed

- Assistant Markdown renders tables as width-aware grids with borders, header separators, and column
  alignment, falling back to stacked `Key: value` rows on narrow terminals. Headings no longer show `#`
  markers, ordered lists are numbered, and wide characters are measured by display width.

### Removed

- Nothing yet.

### Security

- Nothing yet.

## [0.4.0] - 2026-11-12

Phase 4: customization and trust. Skills, prompt templates, project trust, system instruction files, `@file`
attachments, image inputs, and a guarded `web_fetch` tool.

### Added

- Read-only search tools: `grep` (regex content search with `case_sensitive`, `glob`, and `max_results`),
  `find` (glob path search), and `ls` (non-recursive directory listing with sizes and symlink targets). All three
  respect `.gitignore`, skip hidden entries, cap output, and are permitted by the default headless policy.
- Skills: `.codie/skills/<name>/SKILL.md` and `~/.config/codie/skills/`, frontmatter (`name`, `description`,
  `model-invocable`), `/skill:<name> [args]`, and a `<skills>` prompt section for model-invocable skills.
- Prompt templates: `.codie/templates/<name>.md` and `~/.config/codie/templates/`, `$ARGUMENTS` and `$1`..`$9`
  substitution, and `/template:<name> [args]`.
- Project trust: `.codie/` resources load only for trusted projects; `--approve` records the canonical path in
  `trust.toml`; interactive runs prompt once; non-interactive runs default to untrusted.
- `SYSTEM.md` and `APPEND_SYSTEM.md` discovery from the working directory to the filesystem root, plus
  `--system-prompt` and `--append-system-prompt`.
- `@path` attachments: text files become `<file>` blocks and images become base64 image content. Missing and
  oversized files produce a visible warning.
- `web_fetch`: guarded HTTP(S) fetching with HTML-to-text conversion, redirect limits, byte and time caps, and
  refusals for loopback, private, and link-local addresses. Network egress requires `--allow web_fetch`, an
  interactive approval, or full-access mode.
- `/reload` refreshes skills and templates; `codie config set`, `codie config path`, and `codie config edit` join
  `codie config get`.

### Changed

- The built-in tool set is now eight tools; `--allow-all` and `--tools` cover `web_fetch`.
- Running tools show an animated marker, elapsed time, and the tool name in the editor border instead of the
  generic streaming label.
- Image blocks in user messages are sent as `image_url` data URLs (`PROV-023`); tool-result images remain text
  notes.
- Workspace version is `0.4.0`.

### Fixed

- `bash` no longer hangs on commands that leave a background process holding the output pipe open, and now
  applies a 120 second default timeout. Timeouts kill the command's process group.

### Removed

- Nothing yet.

### Security

- Project-controlled skills, templates, and system instructions are untrusted by default; a malformed or missing
  `trust.toml` fails closed, and trust entries are canonicalized before comparison.
- `web_fetch` blocks local and private destinations, re-validates every redirect, and caps response size and time.

## [0.3.0] - 2026-11-12

Phase 3: sessions at full fidelity. Resume, fork, clone, and compact without losing work, with session format
v1 frozen and announced as stable for the 1.x series.

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
- Command palette on `/` with filtering and scrolling; live model, theme, and reasoning pickers; tokens-per-second
  in the footer.
- Permission prompts accept `y`/`Y` and `n`/`N` (plus the existing allow-always key), and `Shift+Tab` toggles a
  visible full-access mode that skips prompts for the session.
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
- Session files written by `0.3.0` and later are stable for the 1.x series: they remain readable across that
  series, with migrations applied on read.
- Workspace version is `0.3.0`.

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
