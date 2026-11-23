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

## [0.9.4] - 2026-11-23

### Added

- Subagent personas. `.codie/agents/<name>.md` (and `<config_dir>/agents/`) define a type's description, prompt,
  tool subset, model, turn budget, and timeout; `[subagents.types.<name>]` accepts the same fields. A definition
  overrides the matching built-in type field-by-field, and the names appear in the `task` schema and description
  (`AGENT-031`, `AGENT-032`).
- Live subagent activity. `task` rows show the agent, label, current turn and tool target, and elapsed time, then
  the status, turns, cost, and child session id on completion. `/subagents status` numbers runs, `open N` shows a
  read-only child transcript, and `stop N` cancels one child (`TUI-040`, `TUI-041`, `TUI-044`).
- A `question` tool for the parent and delegated children, with a one-line prompt in the TUI. Permission prompts
  now name the asking subagent and scope "Always allow" to that agent type (`AGENT-034`, `AGENT-035`, `TUI-045`).
- Subagent delegation (`SPEC-AGENT-003`). A host-registered `task` tool starts an in-process child run with its
  own transcript, model, turn budget, cancellation token, and tool subset. Children inherit the parent's
  permission policy, cannot escalate, and report progress through the existing tool-update events. Enable it with
  `[subagents] enabled = true`, `--subagents`, or `/subagents` in the TUI; prompt phrases such as "use subagents"
  are detected deterministically. See [`docs/src/subagents.md`](docs/src/subagents.md).
- RPC protocol v2. Clients can opt into interactive tool permissions (`permission_request` /
  `permission_response`), manage API keys over the private pipe, read the model catalog and effective
  configuration, write configuration values, and send path or inline-image prompt attachments. Version 1 clients
  keep the headless tool policy unchanged. See [`docs/src/rpc.md`](docs/src/rpc.md) and
  [`specs/protocol/0003-rpc-v2.md`](specs/protocol/0003-rpc-v2.md).
- `codie config set` and the RPC `set_config` command now share one writer, so both validate and normalize values
  identically.
- RPC `get_themes` returns the built-in TUI palettes as semantic hex colors so GUI clients reuse the same theme
  definitions (`PROTO-012`).
- TypeScript bindings for the RPC and agent-event types are generated from Rust with `cargo xtask bindings`
  (`--check` for CI), behind `typescript` features on `codie-core` and `codie-protocol`.

### Changed

- `rpc_ready` reports protocol version 2 plus the supported feature list (`PROTO-011`). The published
  `schemas/rpc-*.schema.json` files were regenerated.
- TUI list overlays now fit their filtered content, cap long lists at 16 visible rows, highlight the full selected
  row, and use normal foreground text for descriptions and keyboard hints. The footer drops secondary metadata at
  medium widths and uses separate location/status rows on narrow terminals; the original startup banner remains.
- Permission dialogs now hug their content horizontally, use rounded padded frames and a raised preview panel, and
  present clearer color-coded Allow once, Always allow, and Deny actions.
- The TUI lays out the transcript per entry and rebuilds only dirty or animated entries. A Fenwick row index
  keeps scroll lookup logarithmic, so streaming a long session updates only the changed tail instead of
  re-rendering all history.
- `project_memory` accepts entries up to 7 KiB instead of 1 KiB. The 32 KiB `CODIE.MD` and 128 KiB `PLAN.MD`
  file caps are unchanged (`MEM-013`).

### Fixed

- Subagents follow `/model` and `/provider`. The runtime kept the startup model, so switching providers (for
  example to OpenRouter) made every child fail with the previous model id at zero cost. The parent model and known
  model ids are now swappable and synced on every selection change (`AGENT-036`).
- Child failures are no longer swallowed. Provider and terminal errors are carried into the `task` result text
  and structured details instead of "completed without a final message" (`AGENT-039`).
- Children waiting for a concurrency slot are listed as `queued`, and `/subagents stop N` cancels them before
  they start (`AGENT-037`).
- A nested `task` (with `max_depth > 0`) now goes through the same permission gate as the root call instead of
  spawning silently (`AGENT-038`).
- `/reload` applies added or removed agent definitions without restarting, and `/subagents parallel N` applies
  live when no child is running or queued (`TUI-046`, `TUI-048`).
- An open child transcript refreshes once a second while it runs, and permission and question prompts name the
  asking run (`TUI-047`).
- Dropping the event stream now cancels the run and its tools, so exiting the UI cannot leave side effects
  running; each run uses a child token so a retry is unaffected (`AGENT-040`).
- Parallel tool calls from one assistant message are bounded by `max_parallel_tools` (default 8) instead of
  spawning every call at once (`AGENT-041`).
- A provider thought signature on a tool call survives streaming deltas, keeping Gemini-style multi-turn tool
  calling intact (`AGENT-042`).
- A tool that clones the progress sender into a detached task can no longer hang the run; late updates are
  dropped after a short drain.
- Sequential tool calls now report a measured `duration_ms` instead of zero, matching the parallel path
  (`AGENT-043`).
- Token and cost counters saturate instead of overflowing on hostile provider values, and additive fields
  accept older JSON that predates them (`AGENT-046`).
- A zero turn budget returns the caller transcript and prompt with `TurnLimit` instead of an empty history
  (`AGENT-044`).
- A unanimous `terminate` from a tool batch stops the run after those results, as the tool contract says
  (`AGENT-045`).
- `AgentEventStream::channel(0)` and `AssistantEventStream::channel(0)` fail with a named panic instead of the
  tokio assertion.
- A run that exhausts `max_turns` now makes one final tools-disabled request for a handoff summary before ending
  `TurnLimit` (`AGENT-047`).
- A child that exceeds its wall-clock limit returns its partial summary and usage promptly with an `error`
  naming the timeout, instead of waiting for the provider or discarding the transcript (`AGENT-048`).
- A forced delegation run that never calls `task` gets exactly one follow-up nudge, in the TUI, plain mode, and
  `codie -p` (`AGENT-030`).
- Child token and cost usage now reaches the parent footer, `/usage`, and a `subagent` session usage entry
  (`TUI-049`).
- Subagent summaries are labelled untrusted, and truncation keeps the beginning and the conclusion instead of
  only the head.

- Interactive runs now write tracing output to `<data>/logs/codie.log` instead of stderr. Background provider
  catalog warnings (for example, an offline Ollama or LM Studio server) no longer corrupt the alternate-screen TUI.
- Ctrl+C and Ctrl+D are no longer swallowed by Help, Tree, Usage, picker, or permission overlays. Ctrl+C still
  cancels a running turn and a second press quits; the connect overlay keeps its own handling.
- The footer context meter, compaction trigger, and session estimates now share one `chars / 4` estimator,
  including image costs and per-message rounding, so percentages and thresholds agree.
- Session write failures (model and thinking changes, subagent directives and settings, project-memory edits,
  branch pruning, and subagent usage) are surfaced in the transcript or returned in plain mode instead of being
  discarded.
- Collapsed turn summaries refresh when a hidden tool or reply changes.
- Plain mode's `/model` and `/provider` update the context window from the model catalog and persist the
  selection, matching the full TUI.

### Removed

- Nothing yet.

### Security

- Nothing yet.

## [0.9.3] - 2026-11-18

### Added

- Static provider registry: provider identity, environment variable, base URL, default model, compatibility,
  headers, bundled models, catalog kind, and key-probe kind now live in `crates/codie-ai/src/provider/`. The CLI,
  TUI, auth commands, model picker, and probes discover providers from this registry.
- Generic OpenAI-compatible model catalog for `CatalogKind::OpenAiModels`: per-provider `GET /models` refresh,
  ETag caching under `<provider>-models.json`, chat-model filtering, background refresh when a key is available,
  stale-on-failure behavior, and curated-first merging.
- Provider support for OpenAI, Groq, Cerebras, Mistral, xAI, Together AI, DeepInfra, Fireworks AI, Moonshot AI,
  Z.AI, NVIDIA NIM, Nebius, SambaNova, Novita AI, Alibaba DashScope, Baseten, Requesty, OpenCode, OpenCode Go,
  Ollama, LM Studio, and vLLM. Each includes curated coding models and a live `/models` catalog.
- Keyless local providers: Ollama, LM Studio, and vLLM start without credentials and switch without a key prompt.
- The TUI now reopens the connect overlay on a provider `401`/`403`, refuses provider switches while a run is
  streaming, and persists the last provider/model in `state.json`.
- TUI provider connect flow: `/connect` and `/login` open a picker and a masked API-key field, `/logout` removes a
  stored key, and `/auth` reports credential status. `/provider` now shows connection state and opens the connect
  flow for unconfigured providers.
- First interactive run without a credential opens the TUI on the connect overlay instead of exiting.
- `codie auth verify <provider>` and `codie auth status --verify` check a stored or environment key.

### Changed

- `codie auth login` now checks the key with the provider before storing it. A rejected key leaves any existing
  credential untouched. Use `--no-verify` for offline or proxy endpoints.
- The footer shows `provider/model`, and provider switches persist the provider in the session model change.

### Fixed

- Nothing yet.

### Removed

- Nothing yet.

### Security

- Keys entered through the TUI are masked, never rendered in the transcript, and never logged. Probe errors redact
  the key before display.

## [0.9.2] - 2026-11-17

### Added

- Project memory: `CODIE.MD` is loaded into every system prompt as `<project_memory>`, and `/init` scaffolds
  `PLAN.MD` plus `CODIE.MD` with the detected toolchain commands.
- `project_memory` tool for section-scoped, deduplicated memory writes with credential redaction and size caps.
- The prompt now instructs the model to create `PLAN.MD` and `CODIE.MD` when asked to start a new project.
- `/reload` refreshes project memory in place and records the change as a session context edit.
- Ctrl+V pastes a clipboard image as an attachment: images are saved as PNG and shown in the editor as compact
  `[Image 1]` / `[File 1]` labels that expand to `@path` on submit, with a text fallback. macOS supports PNG and TIFF clipboard flavours; Linux uses `wl-paste`/`xclip`, Windows PowerShell.
- Attachment tokens may be quoted (`@"a b.png"`) or escape spaces (`@a\ b.png`), and `Tab` completion reaches
  absolute and `~/` paths and quotes names with spaces.
- A path typed or pasted into the command palette now returns to the editor instead of being swallowed by the
  filter, and a dragged path becomes an attachment as soon as it resolves.
- A path-like prompt whose file is gone no longer reports `Unknown command`; codie falls back to the clipboard
  image and otherwise shows an attachment error.

### Changed

- Assistant markdown renders inline code without backticks or a chip background, tables span the transcript
  width with a rule between every row, and long cells wrap onto extra lines instead of truncating with `…`.
- The `/theme` picker previews the highlighted theme as the selection or filter moves, starts on the active
  theme, and restores the previous theme on `Esc`.

### Fixed

- Nothing yet.

### Removed

- Nothing yet.

### Security

- Nothing yet.

## [0.9.1] - 2026-11-15

### Added

- The permission overlay now renders as a content-sized modal with a dimmed transcript, a pinned footer, a
  scrolled and expandable preview (`↑↓`/`PgUp`/`PgDn`, `Ctrl+O`), per-tool wording, diff counts, and a
  destructive-command warning. Each decision has its own color: green for allow once, amber for always allow,
  and red for deny.
- Dragging with the mouse now selects rendered text and copies it to the clipboard on release, with an
  editor-title confirmation that leaves the transcript and scroll position untouched.
- Added the `dracula`, `material`, `gruvbox`, `nord`, `nightowl`, `orng`, and `tokyonight` themes, ported from
  their upstream palettes. `--theme`, `/theme`, and `CODIE_THEME` accept them, and `orange` is an alias for `orng`.
- Added Google Gemini as a provider (`--provider gemini`, `GEMINI_API_KEY`, `/provider` in the TUI) using
  Gemini's OpenAI-compatible endpoint. The bundled catalog includes Gemini 4 Carbon, Argon, and Hydrogen plus
  Gemini 3.8 Flash and 3.5 Flash-Lite, with `gemini-4-hydrogen` as the default.
- Gemini tool calls now preserve and replay `extra_content.google.thought_signature` so multi-turn tool use
  works, and parallel tool calls that reuse an index are no longer merged.
- Added `benchmark/`, a repeatable harness that compares codie and opencode startup plus a four-task coding
  suite (greenfield UI, brownfield UI, hidden-test bug fix, and JSON API) using the same prompts, scaffold, and
  pinned OpenRouter model. It records turns, tool calls, time to first edit, tokens, cost, and bootstrap
  confidence intervals, with browser, hidden-test, and HTTP verifiers.

### Changed

- Extra themes are labelled "Dark theme" in the picker, and the README documents the benchmark results and
  uses a refreshed TUI screenshot.

### Fixed

- Nothing yet.

### Removed

- Nothing yet.

### Security

- Nothing yet.

## [0.9.0] - 2026-11-14

### Changed

- Prebuilt macOS x86_64 archives are no longer published. Intel Macs install from source with `install.sh`.

### Fixed

- Windows session directories are now built from a filesystem-safe key: the verbatim `\\?\` prefix from
  `canonicalize` is stripped, characters Windows forbids in file names are percent-encoded, and overlong keys
  are truncated with a stable hash. `codie` and `codie sessions` normalize the working directory the same way.
- Session listings break timestamp ties by session id, so the order is deterministic even when files are
  written within the same Windows clock tick.

## [0.9.0-rc.1] - 2026-11-14

### Added

- Windows x64 preview build, packaged as `codie-v<version>-x86_64-pc-windows-msvc.zip` with a per-asset
  checksum and an entry in `sha256.sum`.
- One-command PowerShell installer, `install.ps1`, with checksum verification, `%LOCALAPPDATA%` installation,
  user `PATH` updates, and a pinned-version path through `CODIE_VERSION`.
- `powershell` tool for Windows, preferring PowerShell 7 and falling back to Windows PowerShell 5.1, with
  UTF-8 output, timeouts, cancellation, tail truncation, and `taskkill` process-tree termination.
- Windows clipboard copy through `clip`.
- Windows home-directory resolution through `USERPROFILE`.
- Windows-safe extension environments and process-tree termination.
- Prerelease-aware version comparison for `codie update`.
- Documentation: quickstart, Windows preview, terminal compatibility, Gatekeeper/quarantine, troubleshooting,
  and release checklist pages.

### Changed

- The tool registry exposes `bash` on macOS/Linux and `powershell` on Windows in the same slot; the declaration
  count remains eight.
- Windows is now documented as a tier-2 preview rather than post-1.0 only.
- Linux and Windows build jobs were added to CI; the release workflow publishes five archives.

### Security

- The Windows shell and extension processes terminate their process trees with `taskkill /T /F /PID` on
  timeout or abort.

## [0.8.0] - 2026-11-13

### Added

- Side-by-side `edit` diffs with line numbers on both sides, three lines of context, tinted added and removed
  rows, syntax highlighting, and word wrapping. Narrow terminals fall back to a unified diff.
- Numbered, syntax-highlighted `write` previews with a `# Wrote path` header.
- Collapsed reasoning by default with an animated thinking indicator. Clicking the indicator expands the live
  reasoning stream; `Esc` collapses it.
- `Esc` and `/stop` stop active model requests, tools, and compaction. Bash process groups are killed and queued
  steering or follow-up messages are dropped.
- Context meter in the footer with warning and compaction-threshold colors. `/usage` or a footer click opens a
  breakdown of context estimates, input, output, cache reads, cache writes, total tokens, cost, throughput, and
  session id.
- Rich permission review: edit and overwriting write calls show diffs, new files show numbered code, and bash
  shows the command before approval.
- Clickable tool headers, a `Ctrl+T` tool picker, `Ctrl+Shift+O` expand/collapse-all for tools, and
  `Ctrl+Shift+T` expand/collapse-all for turns.
- Automatic folding of older user turns into one-line summaries with tool, edit, error, and hidden-reply counts.
- Split session picker with a preview of each session's model, message count, context tokens, cumulative cost,
  working directory, age, last prompt, and last reply.

### Changed

- The footer is responsive: lower-priority fields drop out as the terminal narrows, and the working directory
  includes the git branch when available.
- Cumulative session usage now tracks input, output, cache-read, and cache-write tokens separately.
- Aborted runs return the editor to idle and report `Stopped.` instead of an error.
- Session summaries include the last model, context token estimate, cumulative cost, last prompt, and last reply.
- Transcript and rich tool bodies are cached by content revision, width, and theme; only visible rows are cloned
  while scrolling.

### Fixed

- Large edit diffs no longer make the TUI hang or scroll slowly. Diff parsing, syntax highlighting, wrapping, and
  transcript layout are reused instead of rebuilt on every frame.
- Permission prompts keep the action bar visible even when the preview is truncated.
- Compaction can be cancelled through `Esc` or `/stop` and does not run the pending prompt after it is stopped.

### Removed

- Nothing yet.

### Security

- Permission review now shows the actual edit diff or shell command before approval instead of a raw JSON
  argument blob.

## [0.7.1] - 2026-11-13

### Changed

- `/update` reports download progress as a percentage and byte count while it replaces the binary.

### Fixed

- `/update` selected from the command palette now starts the check instead of silently doing nothing. The event
  loop services the request after any input path, and a second request is ignored while one is in flight.

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
