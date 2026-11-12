# CODIE — A Rust Coding Agent Harness

**Implementation Plan & Specification Baseline**

| Field | Value |
|---|---|
| Status | Draft v0.4 — macOS-first scope locked |
| Date | 2026-11-11 |
| Name | **codie** (confirmed; binary `codie`) |
| Reference implementation | [`earendil-works/pi`](https://github.com/earendil-works/pi) (TypeScript, MIT) |
| Target language | Rust (edition 2024, MSRV pinned in `rust-toolchain.toml`) |
| Initial providers | **OpenRouter**, **DeepSeek** (HTTP API, API-key auth only) |
| License | Dual MIT OR Apache-2.0 |
| Development model | Spec-driven, phase-gated, trunk-based, every phase shippable |

> This document is the single source of truth for *what* we build and *why*. Normative behavior lives in
> `specs/` (created in Phase 0); this plan defines scope, architecture, phases, and acceptance gates.
> Requirement IDs used here (e.g. `AGENT-004`) become the traceability keys between specs, code, and tests.

---

## 0. How to read this document

1. **§1–§2** — context, goals, and the distilled findings from the pi codebase.
2. **§3** — the spec-driven engineering process we commit to (this is a hard requirement, not ceremony).
3. **§4–§5** — product surface and architecture.
4. **§6** — the normative specs, subsystem by subsystem, including provider wire contracts.
5. **§7–§8** — phase plan and task breakdown. **Nothing in §6 is built before its phase in §8.**
6. **§9–§13** — testing, performance, security, repo/OSS operations, risks.

A reader who only wants the schedule can jump to **§8**. A reader who only wants provider details can jump to **§6.2**.

---

## 1. Executive summary

`codie` is a terminal-first coding agent for macOS, written in Rust. It is a deliberate reimplementation of
the architecture proven by `pi`, with three changes:

1. **Rust instead of TypeScript** — single static binary, no Node runtime, predictable memory, low startup
   latency, and first-class concurrency.
2. **A stricter safety posture** — pi ships *no* permission system by design. codie keeps the same
   capability ceiling but ships a policy engine (allow / ask / deny) that is on by default in interactive
   mode and deny-by-default for destructive operations in headless mode.
3. **Provider scope discipline** — only OpenRouter and DeepSeek at launch, behind one
   OpenAI-compatible adapter with per-provider compatibility flags. Additional providers are added only
   when a conformance suite exists for them.

Everything else follows pi's proven decomposition:

```
LLM wire adapter  →  assistant event stream  →  agent loop  →  tool execution
                                                  ↓
                                          session (append-only JSONL tree)
                                                  ↓
                                   interactive TUI | print | JSON | RPC
```

The core is intentionally small. Customization (skills, prompt templates, themes, hooks) is layered on
top, not baked into the loop.

### 1.1 Goals

| ID | Goal | Measure |
|---|---|---|
| G-1 | A usable coding agent in one static binary | `codie` runs on macOS 13+ (arm64 + x64) with no runtime deps; Linux/Windows follow post-1.0 |
| G-2 | Correct, resumable sessions | byte-identical transcript replay in tests; resume/fork/compaction |
| G-3 | Fast and responsive | cold start < 50 ms; 60 fps streaming render; idle RSS < 80 MB |
| G-4 | Provider-correct | recorded-fixture conformance for OpenRouter + DeepSeek; no live API in CI |
| G-5 | Extensible without a plugin runtime in core | out-of-process extension protocol; declarative skills/templates/themes |
| G-6 | Professional OSS | specs, ADRs, changelog, semver, signed releases, security policy, CI gates |
| G-7 | Safe by default | explicit permission policy; no silent destructive execution |

### 1.2 Non-goals (for v1.0)

- Sub-agents, plan mode, multi-agent orchestration.
- A web UI, desktop app, or IDE plugin.
- OAuth/subscription login (API keys only at launch).
- Sandboxing/containerization built into the binary (we document external patterns instead).
- MCP, LSP, and code-mode execution (designed for, shipped post-1.0).
- Anthropic-native, Google-native, Bedrock, Azure adapters.
- Fine-tuning, embeddings, image generation.
- **Linux and Windows as tier-1 platforms at 1.0** — v1.0 targets macOS first. Linux builds are kept green
  as a portability signal (tier 2); Windows support lands post-1.0 (see §12.5).

### 1.3 Success criteria for v1.0

- A new user can `brew install codie`, run `codie auth login deepseek`, and complete a multi-step
  refactor in a repository using only the terminal.
- `codie -p "..."` and `codie --mode json` are stable enough to script in CI.
- The session file from any v1.0 run can be resumed by any later v1.x release.
- Every normative requirement in `specs/` has at least one automated test (checked in CI).
- Zero `unsafe` outside audited platform shims; `cargo deny` and `cargo audit` clean.

---

## 2. What we learned from pi (and what we copy)

### 2.1 pi's decomposition

| pi package | Responsibility | codie equivalent |
|---|---|---|
| `packages/ai` | Provider abstraction, wire adapters, model catalog, auth, streaming events | `codie-ai` |
| `packages/agent` | Agent loop, turn state machine, tool orchestration, event bus | `codie-core` |
| `packages/coding-agent` | Tools, sessions, compaction, prompt assembly, CLI modes, TUI app | `codie-tools`, `codie-session`, `codie-context`, `codie-prompt`, `codie-tui`, `codie-cli` |
| `packages/tui` | Terminal rendering primitives, editor, markdown, themes | `codie-tui` |
| `packages/durable` | Durable task runtime (checkpointed generations) | post-1.0 (`codie-durable`) |
| `packages/mcp` | Transport-neutral MCP client | post-1.0 |

### 2.2 Design decisions worth preserving

1. **Provider ≠ API ≠ Model.** A provider is a runtime unit (id, base URL, auth, catalog, model filters). An
   *API* is a wire protocol identifier (`openai-completions`, `anthropic-messages`, …). A *model* carries
   `api`, `provider`, limits, cost, and compat flags. This separation is what makes a multi-provider system
   maintainable. We preserve it.
2. **Assistant output is an event stream, not a promise.** `start → text_*/thinking_*/toolcall_* → done|error`,
   with a live partial message. UI and persistence consume the same events.
3. **Tool calls are executed only after the assistant message is final.** `message_end` is a barrier.
4. **Tool failures are tool results, never loop failures.** The model gets a chance to recover.
5. **Parallel tool execution with source-order results.** `tool_execution_end` is completion-ordered; the
   transcript order is source-ordered.
6. **The transcript *is* the prompt.** System prompt and tool declarations are persisted as system-message
   entries and replayed; there is no separate prompt state. Mid-conversation changes are patches (`sections`,
   `toolsAdded`, `toolsRemoved`).
7. **Append-only session tree.** Fork, clone, compaction, and context edits never rewrite history; a
   projection derives the model-visible context.
8. **Steering vs follow-up queues.** `Enter` while streaming steers the current run; `Alt+Enter` queues a
   follow-up for after the run. This is a better UX than "cancel and retype".
9. **Compaction is host logic, not loop logic.** The loop only exposes `prepare_request` / `prepare_next_turn`.
10. **Minimal core, extension surface at the edges.** Extensions are ordinary consumers, not privileged code.

### 2.3 Where we deliberately diverge

| pi behavior | codie behavior | Reason |
|---|---|---|
| No permission system | Policy engine, default `ask` for write/bash, `deny` headless unless allowlisted | OSS default safety; users can opt out |
| No max turns in core | `max_turns` in config (default 200), enforced by host | Prevents runaway loops |
| TS extensions loaded in-process (jiti) | Out-of-process extension protocol (JSON-RPC over stdio) + declarative resources | Rust has no stable in-process language plugin ABI; process isolation is safer |
| 10+ providers | 2 providers, one adapter, conformance suites | Shipping quality over breadth |
| Bun-compiled binary | `cargo-dist` native binaries | Smaller, reproducible, no runtime |
| JSON config + JSON Schema | TOML config, JSON state, generated schema | Hand-editability and Rust ecosystem fit |
| Custom line-diff TUI renderer | `ratatui` cell buffer + custom transcript widgets | Less code; ratatui already diffs cells |
| Image input | Text only at launch; image input ships in Phase 4 where the model supports it; inline TUI image rendering is Phase 5 | Scope discipline |
| No backward-compat policy | v1 session format is stable across 1.x; migrations on read | Users must trust their history |

### 2.4 pi files used as references (traceability anchors)

| codie spec | pi reference |
|---|---|
| `PROV-*` | `packages/ai/src/api/openai-completions.ts`, `src/providers/{openrouter,deepseek}.ts`, `src/providers/compat-schema.ts`, `catalog/providers/deepseek.ts` |
| `MSG-*` | `packages/ai/src/types.ts`, `src/utils/transcript.ts`, `src/utils/event-stream.ts` |
| `AGENT-*` | `packages/agent/src/agent-loop.ts`, `agent.ts`, `types.ts` |
| `TOOL-*` | `packages/coding-agent/src/core/tools/{read,write,edit,bash,grep,find,ls}.ts`, `truncate.ts`, `file-mutation-queue.ts` |
| `SESS-*` | `packages/coding-agent/src/core/session-manager.ts`, `docs/session-format.md` |
| `CTX-*` | `packages/coding-agent/src/core/compaction/*`, `docs/compaction.md` |
| `PRMPT-*` | `packages/coding-agent/src/core/{system-prompt,resource-loader,skills,prompt-templates}.ts` |
| `TUI-*` | `packages/tui/src/*`, `docs/{tui,themes,keybindings}.md` |
| `CLI-*` | `packages/coding-agent/src/{cli.ts,main.ts,cli/args.ts}`, `docs/cli.md` |
| `PROTO-*` | `packages/coding-agent/src/modes/{print-mode,json-event}.ts`, `modes/rpc/*`, `docs/{json,rpc,rpc-commands}.md` |

---

## 3. Spec-driven development process

This is mandatory. Code is not merged without a spec and a traceable test.

### 3.1 Artifact types

| Artifact | Location | Purpose |
|---|---|---|
| Product plan | `PLAN.MD` (this file) | Scope, phases, architecture |
| Spec | `specs/<area>/<NNNN>-<slug>.md` | Normative behavior with requirement IDs |
| ADR | `docs/adr/NNNN-<slug>.md` | Irreversible or cross-cutting decisions |
| RFC | `docs/rfc/NNNN-<slug>.md` | Proposed changes to a released spec |
| Conformance fixture | `crates/*/tests/fixtures/` | Recorded provider/wire data |
| Traceability report | `target/spec-report.md` (generated) | Requirement → test mapping |

### 3.2 Requirement ID scheme

```
<AREA>-<NNN>
```

Areas: `PROV` (providers), `MSG` (message model), `AGENT` (loop), `TOOL` (tools), `SESS` (sessions),
`CTX` (compaction/context), `PRMPT` (prompt/skills/templates), `TUI` (terminal UI), `CLI` (arguments/config),
`PROTO` (JSON/RPC), `EXT` (extensions), `SEC` (security/permissions), `PERF` (performance).

Rules:

- Every requirement is atomic and testable.
- Every requirement has `MUST` / `SHOULD` / `MAY` (RFC 2119 semantics).
- Every requirement names its acceptance test(s) in a `Verification:` line.
- Changing a released requirement requires an RFC; changing an unreleased one requires only a spec PR.

### 3.3 Spec template

```markdown
---
id: SPEC-AGENT-001
title: Agent turn loop
status: draft | accepted | implemented | deprecated
version: 1.0.0
owners: [@handle]
requirements: [AGENT-001, AGENT-002, ...]
---

# Context
Why this exists; what breaks if it is wrong.

# Normative behavior
AGENT-001 (MUST) ...
  Verification: `crates/codie-core/tests/agent_loop.rs::spec_agent_001_*`

# State machine / data shapes
...

# Edge cases
...

# Non-goals
...

# Reference implementation
pi: packages/agent/src/agent-loop.ts (behavioral reference only; no code copied)
```

### 3.4 Definition of Ready (before implementation)

1. Spec accepted; requirement IDs allocated.
2. Failure modes and edge cases enumerated.
3. Wire/format examples included (request/response/fixture).
4. Test plan names each acceptance test.
5. Security and performance impact assessed.

### 3.5 Definition of Done (before merge)

1. Code implements only specified behavior (plus explicit `MAY`s).
2. Every `MUST`/`SHOULD` has a test whose name contains the requirement ID.
3. `cargo test --workspace`, `cargo clippy -D warnings`, `cargo fmt --check`, `cargo deny check` pass.
4. Spec updated if behavior changed; ADR added if a decision was made.
5. Docs (`docs/`) updated for user-visible behavior.
6. `CHANGELOG.md` entry under `[Unreleased]`.

### 3.6 Traceability tooling (`xtask`)

- `cargo xtask spec-check` — parses `specs/**/*.md` for IDs; scans test names/attributes for IDs; fails on
  missing or orphaned requirements.
- `cargo xtask spec-report` — writes `target/spec-report.md` (matrix: requirement → tests → status).
- `cargo xtask fixtures` — regenerates provider fixtures from recorded JSON (offline).
- `cargo xtask dist` — local release build (see §12).

Test attribute convention:

```rust
#[test]
#[spec("AGENT-004")]
fn parallel_tool_results_are_source_ordered() { ... }
```

The `spec` attribute is a no-op proc macro in `codie-testkit` used only for discovery.

---

## 4. Product surface

### 4.1 Modes

| Mode | Trigger | stdout | Exit codes |
|---|---|---|---|
| Interactive | TTY in/out, no flags | TUI (alt screen) | 0 ok, 1 error, 130 SIGINT |
| Print | `-p/--print`, or non-TTY | final assistant text | 0 ok, 1 error/aborted/truncated |
| JSON | `--mode json` | JSONL event stream | 0 unless invocation throws |
| RPC | `--mode rpc` | JSONL responses + events | 0 on EOF, 143 SIGTERM, 129 SIGHUP |

Mode selection is explicit and documented: `--mode` wins, then `--print`, then TTY detection.

### 4.2 Command surface (v1.0)

```
codie [options] [--] [@files...] [message...]
codie auth login <provider> [--api-key] [--no-browser]
codie auth logout <provider>
codie auth status [--json]
codie models [--provider <p>] [--json]
codie sessions list [--json]
codie sessions show <id>
codie config get|set|edit|path
codie update [--check]
```

Core flags (Phase 1 subset marked ●):

| Flag | Phase | Meaning |
|---|---|---|
| `-p, --print` ● | 1 | One-shot print mode |
| `--mode text\|json\|rpc` | 1 (json), 5 (rpc) | Output interface |
| `-m, --model <provider/model>` ● | 1 | Exact model id |
| `--provider <p>` ● | 1 | Restrict model lookup |
| `--api-key <key>` ● | 1 | Ephemeral key; never persisted |
| `--thinking <off..max>` ● | 1 | Reasoning effort |
| `-t, --tools <list>` ● | 1 | Allowlist / `+`/`-` deltas |
| `-nt, --no-tools` ● | 1 | Disable all tools |
| `-c, --continue` | 3 | Resume most recent session for cwd |
| `-r, --resume` | 3 | Session picker |
| `--session <path\|id>` | 3 | Open a specific session |
| `--fork <path\|id>` | 3 | Fork into current cwd |
| `--no-session` | 3 | Ephemeral |
| `-a, --approve` / `-na, --no-approve` | 4 | Project trust |
| `--allow <pattern>` / `--deny <pattern>` | 4 | Permission policy overrides |
| `--no-context-files` | 4 | Skip AGENTS.md discovery |
| `--theme <name>` | 4 | TUI theme |
| `--offline` | 2 | Disable network catalog/version checks |
| `-V, --version`, `-h, --help` ● | 0 | |
| `-v, --verbose` ● | 0 | Increase log verbosity (repeatable) |

Unknown long flags become extension flags (validated after extensions load); unknown short flags are errors.

### 4.3 Configuration and data locations

XDG-compliant and overridable. macOS deliberately uses XDG-style locations (rather than
`~/Library/Application Support`) for consistency with the Unix CLI ecosystem; every path can be overridden.

| Purpose | Path (default) | Env override |
|---|---|---|
| Config | `~/.config/codie/config.toml` | `CODIE_CONFIG_DIR` |
| Credentials | `~/.local/share/codie/auth.json` (0600) | `CODIE_DATA_DIR` |
| Sessions | `~/.local/share/codie/sessions/<project-hash>/` | `CODIE_SESSION_DIR` |
| Cache (models) | `~/.cache/codie/` | `CODIE_CACHE_DIR` |
| Project config | `<repo>/.codie/config.toml` | — |
| Project context | `AGENTS.md`, `CLAUDE.md`, `.codie/AGENTS.md` | — |

Project-local `.codie/` resources are trusted only after an explicit trust decision.

### 4.4 Example config

```toml
# ~/.config/codie/config.toml
default_model = "deepseek/deepseek-flash"
thinking = "medium"

[ui]
theme = "system"
tui_mode = "fullscreen"        # fullscreen | inline
editor = "auto"

[agent]
max_turns = 200
tool_execution = "parallel"    # parallel | sequential
steering_mode = "one-at-a-time"
follow_up_mode = "one-at-a-time"

[compaction]
enabled = true
reserve_tokens = 16384
keep_recent_tokens = 20000

[retry]
enabled = true
max_retries = 3
base_delay_ms = 2000

[permissions]
mode = "ask"                   # ask | allow | deny
allow = ["read", "grep", "find", "ls"]
ask = ["write", "edit", "bash"]
deny = []

[providers.deepseek]
api_key_env = "DEEPSEEK_API_KEY"
base_url = "https://api.deepseek.com"

[providers.openrouter]
api_key_env = "OPENROUTER_API_KEY"
base_url = "https://openrouter.ai/api/v1"
referer = "https://github.com/amardeepio/codie"
title = "codie"
```

Precedence: CLI flag > env var > project config > user config > built-in default.

---

## 5. Architecture

### 5.1 Crate graph

```
                 ┌──────────────┐
                 │  codie-cli   │  binary, arg parsing, mode dispatch
                 └──────┬───────┘
        ┌───────────────┼────────────────┬──────────────┐
        ▼               ▼                ▼              ▼
 ┌────────────┐  ┌────────────┐   ┌────────────┐  ┌────────────┐
 │ codie-tui  │  │codie-proto │   │codie-prompt│  │codie-session│
 └─────┬──────┘  └─────┬──────┘   └─────┬──────┘  └─────┬──────┘
       │               │                │               │
       └───────────────┴────────┬───────┴───────────────┘
                                ▼
                        ┌───────────────┐
                        │  codie-core   │  agent loop, messages, events
                        └───┬───────┬───┘
                            │       │
                   ┌────────▼──┐  ┌─▼────────────┐
                   │ codie-ai  │  │ codie-tools  │
                   └────────┬──┘  └──────┬───────┘
                            │            │
                   ┌────────▼────────────▼──────┐
                   │      codie-context         │  compaction, tokens
                   └────────────────────────────┘

 codie-testkit (dev-only): fake provider, fixtures, spec macro, harness
```

Rules:

- `codie-core` has no internal dependencies and owns the shared vocabulary (messages, events, errors). `codie-ai`, `codie-tools`, `codie-context`, and `codie-session` depend on it, never the reverse. No I/O, terminal, or HTTP in core.
- `codie-ai` owns HTTP/SSE and provider quirks. It never knows about tools' implementations.
- `codie-tools` depends on `codie-core` (tool trait) and `codie-context` (truncation), never the reverse.
- `codie-cli` is the only crate that reads env vars or decides process exit codes.
- Public APIs are `pub(crate)` by default; crates expose narrow, documented surfaces.

Start with **five crates** (`core`, `ai`, `tools`, `session`, `cli`) and split only when a second consumer
appears ("crate graduation"). Premature crates are as harmful as monolithic modules.

### 5.2 Runtime model

- One `tokio` multi-thread runtime (small worker count; `worker_threads = min(4, cores)`).
- One **agent task** owns the loop and serializes state transitions.
- Events flow over a bounded `mpsc` channel (capacity 256) to the UI/protocol writer; backpressure is real.
- Tool execution uses a `JoinSet` + `Semaphore` (`max_parallel_tools = 8`).
- Cancellation uses `tokio_util::sync::CancellationToken`; every I/O boundary is cancellation-aware.
- Blocking filesystem work uses `spawn_blocking`; never block the reactor.
- Child processes are spawned in their own process group and killed as a group on abort/timeout.

### 5.3 Layering of state

```
SessionStore (append-only JSONL, source of truth)
        │  projection (branch + compaction + context edits)
        ▼
Transcript (Vec<Message>)  ──►  ProviderContext (normalized for wire)
        │
        ▼
AgentState (derived: messages, streaming partial, pending tool calls)
```

The loop works on a working copy; `AgentState` is reduced from events. Persistence happens on
`MessageEnd` only (never on partials).

### 5.4 Dependency policy

| Concern | Crate | Notes |
|---|---|---|
| Async runtime | `tokio` (rt-multi-thread, process, fs, signal, sync, time) | features minimal |
| HTTP | `reqwest` (rustls, stream, gzip) | no OpenSSL |
| SSE | hand-rolled parser over `bytes_stream` | must handle split frames + `[DONE]` |
| JSON | `serde`, `serde_json` | `preserve_order` disabled for determinism |
| CLI | `clap` (derive) | generated `--help` is authoritative |
| Config | `toml`, `figment` or hand-rolled layering | prefer explicit layering |
| TUI | `ratatui`, `crossterm` | crossterm for events/paste |
| Markdown | `pulldown-cmark` | streaming-friendly rendering |
| Syntax | `syntect` + `two-face` | lazy-loaded syntax set |
| Diff | `similar` | unified + inline diff |
| Grep | `ignore`, `regex`, `grep-searcher` | in-process; no external `rg` |
| Glob | `globset`, `ignore::WalkBuilder` | no external `fd` |
| Unicode | `unicode-width`, `unicode-segmentation` | grapheme-correct editing |
| Errors | `thiserror` (libraries), `anyhow` (binary) | typed at boundaries |
| Observability | `tracing`, `tracing-subscriber` | JSON logs to file, human logs to stderr |
| Testing | `insta`, `proptest`, `wiremock`, `assert_cmd` | deterministic |
| Audit | `cargo-deny` (advisories), `cargo-audit` locally, `cargo-machete` | Local and release gate |

Rules: exact versions in `Cargo.toml` for binaries where reproducibility matters; `Cargo.lock` committed;
no crate with `build.rs` network access; `#![forbid(unsafe_code)]` workspace-wide with a single audited
`codie-platform` exception for process-group kills.

---

## 6. Specifications

Each subsection is a *spec outline*: the full normative text is written in `specs/` during Phase 0/1.
IDs below are allocated now and must not be reused.

### 6.1 Message model (`MSG`)

**MSG-001 (MUST)** Messages are one of `system | user | assistant | toolResult`, plus host-only variants
(`bashExecution`, `custom`, `branchSummary`, `compactionSummary`) that are lowered before the wire.

**MSG-002 (MUST)** Content blocks preserve order and are one of:

```rust
pub enum Content {
    Text { text: String, signature: Option<String> },
    Thinking { thinking: String, signature: Option<String>, redacted: bool },
    ToolCall { id: String, name: String, arguments: serde_json::Value },
    Image { data: String, mime_type: String }, // phase 5
}
```

**MSG-003 (MUST)** `AssistantMessage` carries `api`, `provider`, `model`, `usage`, `stop_reason`, and
optional `response_model`, `response_id`, `thinking_level`, `provider_thinking_level`, `error_message`,
`raw_stop_reason`, `duration_ms`, `timestamp`.

**MSG-004 (MUST)** `StopReason = pending | stop | length | tool_use | error | aborted | deferred`.
`pending` exists only on partial messages and MUST NOT be persisted.

**MSG-005 (MUST)** `Usage` is normalized:

```rust
pub struct Usage {
    pub input: u64,        // uncached input tokens
    pub output: u64,       // includes reasoning tokens
    pub cache_read: u64,
    pub cache_write: u64,
    pub reasoning: Option<u64>,
    pub total_tokens: u64, // input + output + cache_read + cache_write
    pub cost: Cost,        // integer nano-USD, not f64
}
```

**MSG-006 (MUST)** Costs are tracked as integer nano-USD internally to avoid floating-point drift; they are
rendered with 6 decimal places. Provider-reported cost sets `cost_from_provider = true`.

**MSG-007 (MUST)** The provider-facing context contains the system prompt and tool declarations *only* as
leading system messages (`TranscriptContext`). No separate `system_prompt`/`tools` fields cross the adapter
boundary.

**MSG-008 (MUST)** `normalize_context` folds `system_prompt` + `tools` into a leading system message.
`collapse_system_messages` replays all system messages into one for providers that reject mid-conversation
system turns.

**MSG-009 (MUST)** Tool-call IDs are provider-assigned opaque strings; never regenerate them. If a provider
emits characters invalid for another hop, sanitize deterministically and record the mapping in diagnostics.

**MSG-010 (MUST)** Timestamps are Unix milliseconds for messages; session entries use RFC 3339 strings.

### 6.2 Provider layer (`PROV`) — OpenRouter + DeepSeek

#### 6.2.1 Abstractions

```rust
pub type ProviderId = Cow<'static, str>;
pub type ApiId = Cow<'static, str>;

pub struct Model {
    pub id: String,
    pub name: String,
    pub provider: ProviderId,
    pub api: ApiId,                 // "openai-completions" for both launch providers
    pub base_url: String,
    pub context_window: u32,
    pub max_output_tokens: u32,
    pub input: InputModalities,     // text | text+image
    pub reasoning: bool,
    pub thinking_levels: ThinkingLevelMap,
    pub cost: ModelCost,            // nano-USD per token, optional context tiers
    pub compat: Compat,             // provider quirks
    pub headers: HeaderMap,
}

pub trait Provider: Send + Sync {
    fn id(&self) -> &str;
    fn models(&self) -> &[Model];
    fn refresh_models(&self, ctx: &RefreshCtx) -> BoxFuture<'_, Result<(), Error>>;
    fn auth(&self, store: &CredentialStore) -> Result<Option<Auth>, Error>;
    fn stream(&self, model: &Model, ctx: &TranscriptContext, opts: &StreamOptions)
        -> AssistantEventStream;
}
```

**PROV-001 (MUST)** `stream` returns an event stream synchronously. Setup failures (auth, missing model,
network) are emitted as `error` events after an optional `start`; once a stream is returned, no panic or
synchronous throw is permitted.

**PROV-002 (MUST)** `stream` honors `CancellationToken`; abort produces `stop_reason = aborted`, never a
generic error.

**PROV-003 (MUST)** Every request goes through a shared retry layer: retry on network errors, 408, 409, 429,
5xx, and `x-should-retry: true`. Honor `retry-after-ms`, `retry-after` (seconds or HTTP date), else
exponential backoff `min(0.5 * 2^n, 8s) * jitter(0.75..1.0)`. Cap total delay at `max_retry_delay_ms`
(default 60s); a server-requested delay above the cap fails fast with a clear message. Never retry 4xx other
than the above, auth failures, or context overflow.

**PROV-004 (MUST)** Error normalization produces `"<status>: <body>"` (body truncated to 4000 chars) and
appends the deepest `cause` chain for transport errors.

**PROV-005 (MUST)** Provider request/response bodies are logged at `trace` level with `Authorization`
redacted; fixtures are captured with a `--record` test feature only.

#### 6.2.2 Shared OpenAI-compatible adapter

Endpoint: `POST {base_url}/chat/completions`, `Authorization: Bearer <key>`, `Content-Type: application/json`,
`Accept: text/event-stream`.

Request (fields emitted only when supported by `compat`):

```json
{
  "model": "deepseek-flash",
  "messages": [ ... ],
  "stream": true,
  "stream_options": { "include_usage": true },
  "max_tokens": 8192,
  "temperature": 0.7,
  "tools": [ { "type": "function", "function": { "name": "...", "description": "...", "parameters": { } } } ],
  "tool_choice": "auto",
  "reasoning_effort": "high",
  "thinking": { "type": "enabled" },
  "reasoning": { "effort": "high" },
  "provider": { "order": ["..."] },
  "prompt_cache_key": "session-uuid"
}
```

**PROV-010 (MUST)** `max_tokens` field selection is per-model: DeepSeek uses `max_tokens`; OpenRouter uses
`max_completion_tokens` unless the underlying model's compat says otherwise.

**PROV-011 (MUST)** Reasoning parameter selection is driven by `thinking_format`:

| Format | Provider | Body |
|---|---|---|
| `deepseek` | DeepSeek | `thinking: {type:"enabled"|"disabled"}` + optional `reasoning_effort` |
| `openrouter` | OpenRouter | `reasoning: {effort: "<mapped>"}`; off → `{effort:"none"}` |
| `openai` | future | `reasoning_effort` |

Thinking levels map: `off | minimal | low | medium | high | xhigh | max`. A level absent from a model's map
is clamped down to the nearest supported level; `xhigh`/`max` are unsupported unless explicitly mapped.

**PROV-012 (MUST)** SSE parsing accepts `data:` frames split across chunks, ignores comment/keepalive
lines, tolerates CRLF, and terminates on `data: [DONE]`. An EOF without a terminal chunk is an error
("stream ended without finish_reason") unless the provider is known to omit finish reasons.

**PROV-013 (MUST)** Tool-call deltas accumulate by `index` (primary) and `id` (secondary). An `id` arriving
after arguments is attached to the pending block. Arguments are concatenated as text and parsed once at the
end; invalid JSON sets `arguments_error` and produces a tool result the model can recover from.

**PROV-014 (MUST)** Stop-reason mapping:

```
null/stop/end_turn        → stop
length/max_tokens         → length
tool_calls/function_call  → tool_use
content_filter            → error (message)
anything unknown          → error (raw value preserved)
```

**PROV-015 (MUST)** Usage mapping:

```
DeepSeek:  input = prompt_tokens - prompt_cache_hit_tokens
           cache_read = prompt_cache_hit_tokens
           output = completion_tokens
           reasoning = completion_tokens_details.reasoning_tokens (if present)

OpenRouter: same as above, plus
           cost = usage.cost (USD) when present → cost_from_provider = true
           is_byok / cost_details preserved in diagnostics
```

**PROV-016 (MUST)** Assistant reasoning is replayed verbatim as `reasoning_content` when the model requires
it (DeepSeek) and dropped otherwise. Thinking blocks are never summarized into text.

**PROV-017 (MUST)** Tool results are sent as `{"role":"tool","tool_call_id":"...","content":"..."}`.
Image-bearing tool results are downgraded to a text note in v1 (image input ships in Phase 4).

**PROV-018 (MUST)** OpenRouter attribution headers are configurable and default to
`HTTP-Referer: https://github.com/amardeepio/codie`, `X-Title: codie`.

**PROV-019 (MUST)** Request fields are emitted only when the selected model's `compat` declarations
support them. Unsupported fields are omitted rather than sent as `null` or a default, and capability
resolution is per model, not per provider.

#### 6.2.3 DeepSeek specifics

| Item | Value |
|---|---|
| Base URL | `https://api.deepseek.com` (no `/v1`; accept `/v1` and normalize) |
| Auth | `Authorization: Bearer $DEEPSEEK_API_KEY` |
| Env | `DEEPSEEK_API_KEY` |
| Models (catalog, verify at release) | `deepseek-flash` (reasoning, text+image, 1M ctx, 384k out), `deepseek-v4-pro` (reasoning, text, 1M ctx, 384k out) |
| Compat | `supports_store=false`, `supports_developer_role=false`, `max_tokens_field=max_tokens`, `requires_reasoning_content_on_assistant_messages=true`, `thinking_format=deepseek`, `supports_strict_mode=true` |
| Cache | `prompt_cache_hit_tokens` → `cache_read`; prompt tokens include cached tokens |
| Notes | Send `thinking:{type:"disabled"}` when effort is off; replay `reasoning_content: ""` on assistant messages if reasoning was not preserved |

Model data is a bundled `catalog/deepseek.toml`; a `codie models --refresh` command updates OpenRouter only
in v1.

#### 6.2.4 OpenRouter specifics

| Item | Value |
|---|---|
| Base URL | `https://openrouter.ai/api/v1` |
| Auth | `Authorization: Bearer $OPENROUTER_API_KEY` |
| Env | `OPENROUTER_API_KEY` |
| Catalog | `GET /models` (public), `GET /models/user` (personalized; 401/403/404 → fall back to public) |
| Routing | `provider` object from model config (`order`, `only`, `allow_fallbacks`) |
| Reasoning | `reasoning: {effort}`; model list advertises supported efforts |
| Cost | Final usage chunk carries `cost`; treat as authoritative |
| Session affinity | `x-session-id: <session-id>` (helps provider-side caching) |
| Compat | `thinking_format=openrouter`, `supports_developer_role` only for `openai/*` and `anthropic/*` ids, `cache_control_format` only when using Anthropic-native adapter (not v1) |

**PROV-020 (MUST)** OpenRouter model IDs are used verbatim (`anthropic/claude-...`, `deepseek/deepseek-...`).
A model is selectable only if `architecture.input_modalities` includes `text` and it is a chat model.

**PROV-021 (MUST)** Catalog refresh is best-effort: 4s timeout, ETag support, 4-hour freshness window,
stale cache served on failure, and never blocks startup (runs in the background after first render).

**PROV-022 (MUST)** Because OpenRouter fronts heterogeneous models, we use only its OpenAI-compatible
endpoint in v1. Anthropic-native passthrough is explicitly out of scope and documented as such.

#### 6.2.5 Auth (`PROV-030..034`)

**PROV-030 (MUST)** Credentials resolve in this order: explicit `--api-key`, stored credential,
provider environment variable, error. Resolution never falls through to a different provider.

**PROV-031 (MUST)** Stored credentials live in `auth.json` with mode `0600`, written atomically
(temp file + rename + fsync). Malformed or unreadable files produce an actionable error, never a silent
fallback to an empty key.

**PROV-032 (MUST)** `codie auth login <provider> --api-key` reads the key without echo. Keys passed via
`--api-key` are ephemeral and are never persisted. `codie auth logout` removes only the selected
provider's credential.

**PROV-033 (MUST)** Secrets are redacted from logs, errors, JSON/RPC events, exports, crash reports, and
child-process environments. A key MUST NOT appear in process arguments for any child process.

**PROV-034 (MUST)** A missing or rejected key fails before any chat request with a provider-specific,
actionable message naming the accepted environment variable and the login command.

### 6.3 Assistant event stream (`MSG-040..049`)

```rust
pub enum AssistantDelta {
    Start { partial: AssistantMessage },
    TextStart { index: usize },
    TextDelta { index: usize, delta: String },
    TextEnd { index: usize, content: String },
    ThinkingStart { index: usize },
    ThinkingDelta { index: usize, delta: String },
    ThinkingEnd { index: usize, content: String },
    ToolCallStart { index: usize },
    ToolCallDelta { index: usize, delta: String },
    ToolCallEnd { index: usize, call: ToolCall },
    Done { reason: StopReason, message: AssistantMessage },
    Error { reason: StopReason, message: AssistantMessage },
}
```

**MSG-040 (MUST)** `start` precedes all content events. A setup failure may emit `error` without `start`.
**MSG-041 (MUST)** `partial` is a live value, not a snapshot; text/thinking blocks are empty at `*_start` and
authoritative at `*_end`.
**MSG-042 (MUST)** Exactly one terminal `done`/`error`, then the stream ends.
**MSG-043 (MUST)** Redacted thinking may be complete at `start` and emit no deltas.
**MSG-044 (MUST)** The consumer can either iterate events (UI) or await the final message (headless) without
racing; both are supported by a broadcast + oneshot pair.

### 6.4 Agent loop (`AGENT`)

The loop reproduces pi's semantics with an added turn budget.

```
run(prompt):
  emit AgentStart
  messages = [prompt]
  requests = 0
  loop:
    emit TurnStart
    deliver prepared + steering messages
    if requests >= max_turns: emit AgentEnd(status=TurnLimit); return
    requests += 1
    assistant = stream_llm(transcript)
    append assistant
    if assistant.stop_reason in {error, aborted}: end
    calls = tool_calls(assistant)
    if calls and stop_reason == length:
        results = synthetic_truncation_errors(calls)     # never execute
    else if calls:
        results = execute_batch(calls)                   # parallel or sequential
    append results in source order
    emit TurnEnd
    if finish_turn says end: break
    if no calls and no steering and no follow-up: break
    guard: if nothing changed this iteration → break with diagnostic
  emit AgentEnd
```

**AGENT-001 (MUST)** `AgentEnd` is always the last event, even on error/abort.
**AGENT-002 (MUST)** The assistant message is finalized (`MessageEnd` acknowledged by all subscribers) before
any tool starts.
**AGENT-003 (MUST)** Tool results are appended in assistant source order, exactly one per finalized call,
with `tool_call_id`/`tool_name` copied verbatim.
**AGENT-004 (MUST)** In parallel mode, `ToolExecutionEnd` is completion-ordered; transcript order is source
order.
**AGENT-005 (MUST)** A tool failure (unknown tool, schema error, block, panic-free error, abort) becomes
`is_error: true` tool result content. It never terminates the run.
**AGENT-006 (MUST)** `stop_reason == length` never executes tool calls; it synthesizes per-call error results
telling the model to re-issue.
**AGENT-007 (MUST)** Steering is polled before the first request and after each completed turn; follow-ups
are polled only when the loop would otherwise exit. `one-at-a-time` vs `all` modes are configurable.
**AGENT-008 (MUST)** Abort is cooperative: cancellation token propagates to the stream, hooks, and tools; the
run ends with an `aborted` assistant message and `AgentEnd`.
**AGENT-009 (MUST)** `max_turns` (default 200) bounds the number of assistant requests in one run. Before
starting a request that would exceed the budget, the run ends with `status = turn_limit` and a diagnostic,
queuing nothing. A value of zero is rejected at configuration time.
**AGENT-010 (MUST)** Hooks (`prepare_request`, `prepare_next_turn`, `finish_turn`, `before_tool`,
`after_tool`, `transform_context`, `convert_to_llm`) are fallible and may replace context/model; a hook error
fails the run with a synthesized assistant error message, never a panic.
**AGENT-011 (MUST)** The loop never mutates persisted state directly; persistence is event-driven.
**AGENT-012 (MUST)** A panic in a tool is caught (`catch_unwind`) and converted to an error result; panics in
the loop itself are fatal and reported with a crash log.

### 6.5 Tools (`TOOL`)

v1.0 tool set (macOS): `read`, `write`, `edit`, `bash`, `grep`, `find`, `ls`. The `powershell` tool and
Windows-specific tool behavior are post-1.0 (see §12.5).

Common contract:

```rust
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn schema(&self) -> &serde_json::Value;
    fn execution_mode(&self) -> ExecutionMode { ExecutionMode::Parallel }
    async fn execute(&self, call: ToolCall, ctx: &ToolCtx, updates: UpdateSink) -> ToolResult;
}
```

`ToolResult { content: Vec<Content>, details: Option<Value>, structured: Option<Value>, is_error: bool,
terminate: bool, usage: Option<Usage>, duration_ms: u64 }`.

**TOOL-001 (MUST)** All file paths support `~`, `@` prefix, `file://`, Unicode-space normalization, and
absolute/relative resolution against cwd. Paths are *not* sandboxed by default; the policy engine is the
control point.
**TOOL-002 (MUST)** Output truncation: 2000 lines / 50 KiB (head for read/grep/find/ls, tail for bash), never
splitting a UTF-8 sequence or a line. Notices use the exact pi wording (documented in spec) so model behavior
stays comparable.
**TOOL-003 (MUST)** Concurrent writes/edits to the same canonical path are serialized by a per-path async
mutex.
**TOOL-004 (MUST)** `edit` applies all `edits[]` against the same original content, requires unique and
non-overlapping matches, preserves BOM and line endings, and computes the diff before writing.
**TOOL-005 (MUST)** `bash` runs in its own process group; abort/timeout kills the whole group; timeout has no
default; exit codes map to `is_error` with the command output preserved.
**TOOL-006 (MUST)** `grep`/`find` respect `.gitignore`, `.ignore`, and `.fdignore`; `grep` caps at 100
matches and 500 chars/line; `find` caps at 1000 results.
**TOOL-007 (MUST)** Tool schemas are stable and documented; changing a schema is a breaking change to the
prompt contract and requires an RFC.
**TOOL-008 (MUST)** Every tool emits progress updates at most every 100 ms (bash) and supports cancellation.

Per-tool behavior is specified in `specs/tools/` with the same schema, output text, and error strings as pi
(see Appendix B). This matters: models are sensitive to tool wording, and matching pi gives us a free
behavioral baseline.

### 6.6 Session store (`SESS`)

**SESS-001 (MUST)** Sessions are append-only JSONL. Line 1 is a header; every subsequent entry has
`id`, `parent_id`, `timestamp`, `type`.

```rust
#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Entry {
    Message { message: Message },
    ModelChange { provider: String, model_id: String },
    ThinkingLevelChange { thinking_level: ThinkingLevel },
    Compaction { summary: String, first_kept_entry_id: String, tokens_before: u64,
                 details: Option<Value>, usage: Option<Usage> },
    BranchSummary { from_id: String, summary: String },
    ContextEdit { target_id: String, replacement: Option<Content> },
    Custom { custom_type: String, data: Value },
    CustomMessage { custom_type: String, content: Content, display: bool },
    Label { target_id: String, label: Option<String> },
    SessionInfo { name: Option<String> },
    Usage { kind: String, provider: String, model: String, usage: Usage },
}
```

**SESS-002 (MUST)** The file is created lazily, only when the first user or assistant message exists.
**SESS-003 (MUST)** Appends are atomic and ordered; a failed append rolls back the in-memory leaf so no
dangling parents exist.
**SESS-004 (MUST)** Session discovery is by encoded cwd; encoding is deterministic and documented.
**SESS-005 (MUST)** Resume = load header + path from leaf to root + latest compaction + context edits.
**SESS-006 (MUST)** Fork copies the source branch into a new file with `parent_session` set; the original is
never modified.
**SESS-007 (MUST)** `version` is written; unknown newer versions are refused with a clear error; older
versions migrate on read (migration table in spec).
**SESS-008 (MUST)** `codie sessions list/show` and `--mode rpc get_entries` are read-only and never rewrite.
**SESS-009 (MUST)** The store is safe under concurrent processes via an advisory lock file; a second writer
fails with "session is open in another process".

### 6.7 Context management (`CTX`)

**CTX-001 (MUST)** Compaction triggers when `context_tokens > context_window - reserve_tokens` (default
reserve 16384, keep recent 20000, per-model overrides).
**CTX-002 (MUST)** Token estimation uses the last valid provider usage plus `chars/4` for subsequent
messages, with images counted as 4800 chars. It is documented as an estimate and never used for billing.
**CTX-003 (MUST)** The cut point never separates a tool call from its result and never starts the retained
tail on a `toolResult`.
**CTX-004 (MUST)** Summaries use a fixed structure: Goal, Constraints, Progress, Key Decisions, Next Steps,
Critical Context, plus `<read-files>` / `<modified-files>`. Thinking blocks are omitted; tool outputs are
truncated to 2000 chars.
**CTX-005 (MUST)** Compaction is append-only: a `compaction` entry records the summary and
`first_kept_entry_id`; older entries remain in the file.
**CTX-006 (MUST)** One overflow recovery attempt is allowed per run: on a context-overflow error, compact
and retry once; then surface the error.
**CTX-007 (MUST)** Manual `/compact [instructions]` uses the same path with custom instructions.

### 6.8 Prompt assembly (`PRMPT`)

**PRMPT-001 (MUST)** The system prompt is an ordered set of named sections:
`preamble`, `tools`, `rules`, `docs` (optional), `addendum`, `project_context`, `skills`, `cwd`, then custom
sections. Non-preamble sections are XML-wrapped.
**PRMPT-002 (MUST)** Tool snippets come from each tool's `prompt_snippet`; hidden tools are omitted;
"additional tools may be available" appears when the declared set is incomplete.
**PRMPT-003 (MUST)** Project context discovery walks from cwd to the filesystem root, loading one file per
directory in the order `AGENTS.override.md`, `AGENTS.md`, `CLAUDE.md`, with the user-level file first and the
nearest project file last. Contents are wrapped in `<project_instructions path="...">`.
**PRMPT-004 (MUST)** Only sections that changed are persisted after the first request, as named patches;
`null` removes a section. Tool changes are persisted as `tools_added`/`tools_removed`.
**PRMPT-005 (MUST)** `SYSTEM.md` replaces the preamble; `APPEND_SYSTEM.md` appends; CLI flags override both.
**PRMPT-006 (MUST)** Skills are discovered from `~/.config/codie/skills`, `.codie/skills`,
`~/.agents/skills`, and ancestor `.agents/skills` up to the repo root. `SKILL.md` has YAML frontmatter with
`name`, `description`, optional `disable-model-invocation`. Only name/description/location enter the prompt.
**PRMPT-007 (MUST)** Prompt templates are `*.md` files with optional frontmatter and support `$1`, `$@`,
`$ARGUMENTS`, `${N:-default}`, `${@:N:L}`. They expand only when the input starts with `/name`.
**PRMPT-008 (MUST)** Prompt assembly is deterministic: the same inputs produce byte-identical output
(covered by snapshot tests).

### 6.9 TUI (`TUI`)

**TUI-001 (MUST)** Interactive mode uses the alternate screen with a scrolling transcript and a fixed input
dock. `--tui-mode inline` renders into normal scrollback instead.
**TUI-002 (MUST)** Streaming text renders incrementally at up to 60 fps; rendering is coalesced and never
blocks the agent task.
**TUI-003 (MUST)** The editor supports multiline input, grapheme-correct cursor movement, history, undo,
kill/yank, bracketed paste (large pastes collapsed to markers), and autocomplete for slash commands, skills,
and `@file` references.
**TUI-004 (MUST)** Markdown rendering supports headings, emphasis, lists, tables, blockquotes, code fences
with syntax highlighting, links (OSC 8 when supported), and horizontal rules.
**TUI-005 (MUST)** Tool calls render as stateful blocks (pending/success/error) with a compact summary and
expandable details (`Ctrl+O`); edits render an inline diff.
**TUI-006 (MUST)** A footer shows cwd, git branch, session name, token usage, cost, context percentage, model,
and thinking level.
**TUI-007 (MUST)** Keybindings are configurable in `keybindings.toml`; defaults match the table in
`docs/keybindings.md` (Appendix C). Unbinding is supported (`keys = []`).
**TUI-008 (MUST)** Themes are declarative TOML files with semantic tokens; `system` derives colors from the
terminal background and enforces a 4.5:1 body contrast ratio.
**TUI-009 (MUST)** A panic or terminal error restores the terminal (raw mode off, alt screen off, cursor
visible) before printing the error.
**TUI-010 (MUST)** Screen-reader-friendly "plain" mode (`--tui-mode plain`) emits append-only text without
cursor movement.

Implementation note: ratatui's cell-buffer diffing replaces pi's custom line-diff renderer. We keep pi's
*component* concepts (transcript, tool block, editor, footer, overlay) but not its rendering internals.

### 6.10 CLI and config (`CLI`)

**CLI-001 (MUST)** `--help` is generated from the same definitions as parsing and is authoritative for the
installed version.
**CLI-002 (MUST)** Config layering is explicit and testable; every key has a documented default and a
`codie config get <key>` path.
**CLI-003 (MUST)** Exit codes: 0 success, 1 usage/runtime error, 2 auth-check invalid, 130 SIGINT,
143 SIGTERM, 129 SIGHUP.
**CLI-004 (MUST)** `--mode json` and `--mode rpc` take over stdout exclusively; all diagnostics go to stderr.
**CLI-005 (MUST)** Unknown long flags are deferred to extensions; unknown short flags fail fast.
**CLI-006 (MUST)** `--api-key` requires an explicit model and is never written to disk.
**CLI-007 (MUST)** Startup performs no network I/O before the first render; catalog refresh and update checks
run in the background.

### 6.11 Automation protocols (`PROTO`)

**PROTO-001 (MUST)** JSONL is LF-framed; only LF splits records (U+2028/U+2029 are data). A trailing CR is
stripped.
**PROTO-002 (MUST)** JSON mode emits: session header, then agent events, then `agent_settled`. Event names and
payloads are specified in `specs/protocol/0001-json-events.md` and versioned by `schema_version`.
**PROTO-003 (MUST)** RPC mode accepts commands on stdin and emits responses/events on stdout, echoing an
optional `id`. `prompt` succeeds when accepted (`started|queued|handled`), not when complete.
**PROTO-004 (MUST)** `agent_settled` is the completion signal; `agent_end` is not (retries/compaction may
follow).
**PROTO-005 (MUST)** JSON schemas for all events and RPC commands are generated from Rust types
(`schemars`) and published in `schemas/`; a golden-schema test prevents accidental breakage.
**PROTO-006 (MUST)** RPC shutdown is orderly on stdin EOF or SIGTERM/SIGHUP; a second signal exits
immediately.

### 6.12 Extensions (`EXT`) — Phase 6

**EXT-001 (MUST)** Extensions are separate processes speaking JSON-RPC over stdio (`codie-ext` protocol).
The core never loads foreign code into its address space.
**EXT-002 (MUST)** Hook points: `session_start`, `session_shutdown`, `before_agent_start`, `context`,
`before_tool_call`, `after_tool_call`, `turn_end`, `user_submit`, plus custom tools and slash commands.
**EXT-003 (MUST)** A hook may block a tool call (`{block, reason}`) but cannot mutate the transcript directly.
**EXT-004 (MUST)** Extension failures are logged and isolated; the agent continues unless the hook explicitly
fails the run.
**EXT-005 (MUST)** Declarative resources (skills, templates, themes) need no process and are loaded first.
**EXT-006 (MUST)** An extension manifest (`codie.toml`) declares name, version, protocol version, entrypoint,
and capabilities.

### 6.13 Security and permissions (`SEC`)

**SEC-001 (MUST)** Default policy in interactive mode: read-only tools allowed; `write`, `edit`, `bash` ask.
**SEC-002 (MUST)** Default policy in headless/print/JSON mode: read-only allowed; mutating tools denied unless
`--allow` or config explicitly permits them. This prevents accidental destructive automation.
**SEC-003 (MUST)** Policy decisions are per tool call, match on tool name + normalized arguments (command
prefix, path glob), and are logged with the reason.
**SEC-004 (MUST)** Project-local resources (`.codie/`) load only after a trust decision; untrusted resources
are ignored with a visible warning.
**SEC-005 (MUST)** Secrets are redacted from logs, error messages, child environments, and `--mode json`.
**SEC-006 (MUST)** `bash` child processes do not inherit API keys (`*_API_KEY`, `CODIE_*TOKEN*`) unless the
user opts in.
**SEC-007 (MUST)** The security policy documents that prompt injection and OS-level sandboxing are out of
scope; containerization guidance is provided.
**SEC-008 (MUST)** Release artifacts are signed and accompanied by checksums and an SBOM.

### 6.14 Performance (`PERF`)

**PERF-001** Cold start to first render < 50 ms (p95) on a 2020-class laptop.
**PERF-002** Idle RSS < 80 MB; steady-state with a 100k-token session < 200 MB.
**PERF-003** Streaming render ≤ 16 ms per frame, no dropped frames under 5k tokens/s.
**PERF-004** Session append is O(1) and does not rewrite the file; resume of a 10k-entry session < 150 ms.
**PERF-005** No allocations proportional to transcript size per delta (streaming path is amortized).
**PERF-006** Release binary < 20 MB stripped (before optional syntax assets).

---

## 7. Data flow walkthrough (normative sequence)

A first prompt in interactive mode:

```
CLI parse → config load → auth resolve → model select → session open
  → prompt assembly (sections) → session append (system message #1)
  → AgentStart → TurnStart
  → MessageStart(user) → MessageEnd(user)
  → Provider request (SSE)
      → Start(partial) → MessageStart(assistant)
      → TextDelta* / ThinkingDelta* / ToolCallDelta*
      → Done(final) → MessageEnd(assistant) → persist
  → ToolExecutionStart* → ToolExecutionUpdate* → ToolExecutionEnd*
  → MessageStart(toolResult)* → MessageEnd(toolResult)* → persist
  → TurnEnd
  → (no more calls) → AgentEnd
  → agent_settled
```

Invariants checked in debug builds: no tool start before assistant `MessageEnd`; exactly one result per call;
no persisted `pending`; terminal event last.

---

## 8. Phased roadmap

Guiding rule: **each phase ends with a demoable, installable artifact and a tagged pre-release.** No phase
starts before the previous phase's exit criteria are met.

### Phase 0 — Foundations (1 week)

**Goal:** a repository that can accept code safely.

Deliverables:

- Workspace skeleton: `codie-core`, `codie-ai`, `codie-tools`, `codie-session`, `codie-cli`, `codie-testkit`.
- `rust-toolchain.toml`, `Cargo.toml` workspace lints, `rustfmt.toml`, `clippy.toml`.
- Error model (`CodieError` with stable `code` strings), tracing setup, feature flags.
- Spec framework: `specs/` tree, template, `xtask spec-check`, ADR/RFC templates.
- CI: one `cargo build --workspace --all-targets` job on macOS x64. Formatting, linting, tests,
  spec-check, docs, machete, and deny run locally via `cargo xtask ci` and before releases.
- OSS files: `README.md`, `LICENSE-MIT`, `LICENSE-APACHE`, `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md`,
  `SECURITY.md`, `CHANGELOG.md`, issue/PR templates, `docs/` skeleton.
- `codie-testkit`: `FakeProvider` (scripted SSE), temp-home harness, `#[spec]` macro, snapshot helpers.

Exit criteria:

- `cargo xtask ci` green on macOS x64; arm64 verified locally.
- A trivial `codie --version` binary builds and is < 5 MB.
- Spec template + 3 accepted specs (`SPEC-MSG-001`, `SPEC-AGENT-001`, `SPEC-PROV-001`).

### Phase 1 — Headless core, DeepSeek + OpenRouter (3 weeks)

**Goal:** `codie -p "…"` performs real multi-step coding work.

Scope:

- Provider adapter (OpenAI-compatible), DeepSeek + OpenRouter presets, SSE parser, retry, error mapping.
- Model catalog (bundled TOML) + `codie models`.
- Auth: env + `auth.json` + `--api-key`; `codie auth login/logout/status`.
- Message model, event stream, agent loop (single run, steering queue stub, max_turns).
- Tools: `read`, `write`, `edit`, `bash` (with truncation, process-group kill, timeout).
- Session store: create/append/resume (no fork yet), system-message persistence.
- Prompt assembly: preamble, tools, rules, project context (AGENTS.md), cwd.
- Print mode and `--mode json` (event stream).
- Permission policy v0: non-interactive deny for write/edit/bash unless `--allow`.

Tasks:

| ID | Task |
|---|---|
| P1-T01 | `codie-ai`: types, `Provider` trait, `Model`, `Usage`, `StopReason` |
| P1-T02 | SSE parser + chunk reassembly + `[DONE]` handling (unit + fuzz) |
| P1-T03 | OpenAI-compatible request builder + compat flags |
| P1-T04 | DeepSeek preset + reasoning/usage/replay quirks |
| P1-T05 | OpenRouter preset + attribution + cost + catalog refresh |
| P1-T06 | Retry/backoff with `retry-after` parsing + abort |
| P1-T07 | Event stream (`broadcast` + `oneshot`) and partial-message assembly |
| P1-T08 | Agent loop + turn budget + event ordering |
| P1-T09 | Tool trait, registry, schema validation (`jsonschema`) |
| P1-T10 | `read`, `write`, `edit` (+ per-path lock, BOM/EOL, diff) |
| P1-T11 | `bash` (+ process group, timeout, tail truncation, 100 ms updates) |
| P1-T12 | Session JSONL writer/reader v1 + projection v0 |
| P1-T13 | System prompt assembly + AGENTS.md discovery |
| P1-T14 | CLI parsing, config layering, mode selection |
| P1-T15 | Print/JSON output + exit codes |
| P1-T16 | Recorded fixtures for both providers + conformance tests |
| P1-T17 | E2E: fake provider runs a 3-turn refactor in a temp repo |

Exit criteria:

- Golden-path E2E with a fake provider: read → edit → bash test → final answer, transcript snapshot matches.
- Live smoke (manual, not CI) against both providers.
- JSON mode output validated against published schema.
- `codie -p` exit codes match spec.
- 80%+ line coverage in `core`/`ai`/`tools` (excluding generated code).

### Phase 2 — Interactive TUI (3 weeks)

**Goal:** the primary human experience.

Scope:

- ratatui app: alt screen, transcript scroll view, fixed editor dock, footer, overlays.
- Streaming assistant rendering, thinking blocks, tool blocks with states and expand/collapse.
- Markdown + syntax highlighting + inline diff.
- Editor: multiline, grapheme movement, history, undo, kill ring, paste markers, autocomplete
  (slash commands, `@file` via `ignore` walker).
- Steering (`Enter` while streaming) and follow-up (`Alt+Enter`) queues with dequeue.
- Status spinner in the editor border; compaction/retry states.
- Theme system (`system`/`dark`/`light` + TOML themes), hot reload.
- Keybinding config + `/hotkeys`.
- Slash commands: `/model`, `/thinking`, `/new`, `/session`, `/compact`, `/quit`, `/help`.
- Permission prompts (ask/allow/deny) with per-session "always allow".

Exit criteria:

- 60 fps streaming at 5k tokens/s (benchmark harness with a synthetic stream).
- Terminal restore verified on panic/SIGINT/SIGTERM (integration test using a PTY).
- No blocking call on the UI thread (assert via `tokio-console` trace in CI smoke).
- Accessibility: `--tui-mode plain` produces append-only output.

### Phase 3 — Sessions at full fidelity (2 weeks)

**Goal:** never lose work; branch and compact confidently.

Scope:

- Tree sessions: `--continue`, `--resume`, `--session`, `--fork`, `--clone`, `--no-session`.
- `/tree`, `/fork`, `/clone`, `/name`, `/export` (JSONL; HTML in Phase 5).
- Compaction: threshold, manual, overflow/length recovery, branch summaries.
- Token estimation + context percentage in footer.
- Session picker (search, sort, rename, delete).
- Session format v1 freeze + migration harness.

Exit criteria:

- Property test: random sequence of appends/branches/compactions always reconstructs the same projection.
- Resume after kill -9 mid-stream loses at most the in-flight partial message.
- 10k-entry session resume < 150 ms.
- Format documented and announced as stable for 1.x.

### Phase 4 — Customization & trust (2 weeks)

**Goal:** make it *the user's* agent.

Scope:

- Read-only search tools (`grep`, `find`, `ls`) with `.gitignore` semantics (`SPEC-TOOL-002`).
- Skills (discovery, frontmatter, `/skill:name`, model-invocation toggle).
- Prompt templates with argument substitution.
- Project trust (`--approve`, `trust.toml`, prompt), `.codie/` resources.
- `SYSTEM.md`, `APPEND_SYSTEM.md`, `--system-prompt`, `--append-system-prompt`.
- `@file` attachments and image inputs (provider permitting; DeepSeek Flash accepts images,
  OpenRouter depends on model).
- `/reload`, config hot reload, `codie config` subcommands.

Exit criteria:

- Skill/template/theme conformance fixtures.
- Trust bypass attempts (untrusted `.codie`) covered by tests.
- Image path tested end-to-end with a recorded fixture.

### Phase 5 — Protocols, export, polish (2 weeks)

**Goal:** automation and sharing.

Scope:

- RPC mode (`--mode rpc`) with the full command set needed for editor integrations.
- JSON schema publication + versioning.
- HTML transcript export (self-contained, themed, no external JS).
- `/export`, `/copy`, `/debug`, crash reports.
- `codie update --check` (no self-update execution by default).
- Performance pass: allocation audit, startup profile, binary size.

Exit criteria:

- RPC conformance suite (command matrix) green.
- An example integration (`examples/rpc-client`) in the repo.
- Startup and memory budgets met on CI runners.

### Phase 6 — Extensions (3 weeks)

**Goal:** customization without core bloat.

Scope:

- `codie-ext` JSON-RPC protocol (versioned), process supervision, timeouts, capability declaration.
- Hook surface (see `EXT-002`), custom tools, custom slash commands, custom renderers (structured, not ANSI).
- Extension discovery and manifest.
- Extension test kit + example extensions: `todo`, `plan-mode`, `subagent-lite` (documentation examples,
  not core features).
- Security: extension permissions, no secret inheritance by default.

Exit criteria:

- Protocol spec + conformance tests.
- A TypeScript and a Python example extension run against the same protocol.
- Killing a hung extension never hangs the agent.

### Phase 7 — Release hardening (2 weeks)

**Goal:** v1.0.0.

Scope:

- macOS terminal matrix (Terminal.app, iTerm2, Ghostty, kitty, WezTerm, VS Code terminal).
- macOS packaging: arm64 + x64 (or universal2), Developer ID signing, notarization, hardened runtime,
  Homebrew tap, install script, quarantine/Gatekeeper behavior documented.
- macOS platform hardening: POSIX process-group kill, APFS case-insensitivity and Unicode normalization
  tests, Option-as-Meta, clipboard (`pbcopy`/`pbpaste`, OSC 52 fallback), PTY test harness.
- Fuzzing: SSE parser, JSONL reader, session migration, edit matcher.
- Dependency and supply-chain review; SBOM; signed artifacts.
- Docs: mdBook site, quickstart, provider guide, spec index, troubleshooting, platform support matrix.
- Release automation (`cargo-dist`), Homebrew tap, install script, Docker image (Linux best-effort).
- Load/soak test: 8-hour session, 1M-token transcript.

Exit criteria:

- All `PERF` budgets met.
- Zero P0/P1 issues open.
- macOS tier-1 matrix green: full CI suite passes on arm64 and x64; notarized binaries launch cleanly on a
  fresh macOS 13+ machine.
- `v1.0.0` tagged, signed, notarized, reproducible, installable via Homebrew and the install script.

### 8.1 Timeline summary

| Phase | Duration | Cumulative | Shippable artifact |
|---|---|---|---|
| 0 Foundations | 1 wk | 1 wk | repo + CI |
| 1 Headless core | 3 wk | 4 wk | `0.1.0` — `codie -p` works |
| 2 TUI | 3 wk | 7 wk | `0.2.0` — interactive agent |
| 3 Sessions | 2 wk | 9 wk | `0.3.0` — resume/fork/compact |
| 4 Customization | 2 wk | 11 wk | `0.4.0` — skills/templates/trust |
| 5 Protocols | 2 wk | 13 wk | `0.5.0` — RPC/export |
| 6 Extensions | 3 wk | 16 wk | `0.6.0` — extension protocol |
| 7 Hardening | 2 wk | 18 wk | `1.0.0` |

Estimates assume one full-time engineer. They are planning bounds, not commitments; each phase's exit
criteria are the real gate.

---

## 9. Testing strategy

### 9.1 Test pyramid

| Layer | Tool | What |
|---|---|---|
| Unit | `cargo test` | parsers, truncation, diff, config layering, token math |
| Property | `proptest` | session projection, edit matching, SSE chunk boundaries, JSONL framing |
| Snapshot | `insta` | system prompt, rendered markdown, JSON events, session files |
| Conformance | fixtures | provider request/response, tool output text, RPC matrix |
| Integration | `assert_cmd` + PTY | CLI modes, TUI lifecycle, signals |
| E2E | fake provider | multi-turn coding scenario in a temp repo |
| Live (manual/nightly) | real APIs | smoke only, credential-gated, never on PRs |
| Fuzz | `cargo-fuzz` | SSE, JSONL, session migration, edit matcher |
| Bench | `criterion` / `divan` | startup, parse throughput, render frame, projection |

### 9.2 Fixtures and the fake provider

- `FakeProvider` replays scripted SSE chunks with configurable chunk boundaries (byte-level split points
  chosen by proptest to stress reassembly).
- Recorded fixtures are sanitized: no keys, no account IDs, timestamps normalized.
- `--record` requires an env opt-in and writes to a scratch dir; CI never records.

### 9.3 Regression discipline

- Every bug fix adds a failing test first.
- Snapshot changes require a reviewer-visible `insta` diff in the PR.
- Wire-format changes require a fixture update + changelog entry.
- Session-format changes require a migration test from every prior version.

### 9.4 Coverage policy

Line coverage is a smoke signal, not a target. The real gate is **requirement coverage**: every `MUST` has a
named test, verified by `cargo xtask spec-check`.

---

## 10. Performance and optimization plan

### 10.1 Budgets (enforced where practical)

| Metric | Budget | How measured |
|---|---|---|
| Cold start → first paint | < 50 ms p95 | `hyperfine` + PTY timestamp |
| Idle RSS | < 80 MB | `/proc`/`task_info` sampler |
| Frame time while streaming | < 16 ms p99 | synthetic 5k tok/s stream |
| Session append | < 1 ms p99 | criterion |
| 10k-entry resume | < 150 ms | criterion |
| Binary size | < 20 MB | release CI |

### 10.2 Concrete techniques

1. **Zero-copy where it matters.** SSE frames are parsed from `Bytes` slices; text deltas are `Arc<str>` +
   append to a `String` buffer per block, not re-allocated per delta.
2. **Incremental markdown.** Cache rendered blocks keyed by source hash and width; only the last block
   re-renders while streaming (pi does this; it is the difference between smooth and janky).
3. **Coalesced rendering.** UI drains the event channel and renders at most once per 16 ms; input triggers an
   immediate frame.
4. **Bounded channels.** Backpressure protects memory when the terminal is slow; the agent stalls rather than
   buffering unbounded output.
5. **Lazy syntax sets.** `syntect` syntax/theme sets are loaded once on first code block, in a `OnceLock`.
6. **No per-token cloning.** The partial message is mutated in place; events carry indices and deltas, and the
   UI reconstructs; the final message is sent once.
7. **In-process grep/find.** Avoid spawning `rg`/`fd`: use `ignore` + `grep-searcher` + `globset`. Saves
   process startup and makes cancellation immediate.
8. **Streaming file reads.** `read` streams lines and stops at the truncation limit instead of loading huge
   files.
9. **Append-only session writes.** `OpenOptions::append` + `BufWriter`, flush per entry; no rewrites, no
   full-file parse on append.
10. **Avoid `serde_json::Value` in hot paths.** Typed structs for messages; `Value` only at tool-argument
    boundaries.
11. **Profile before optimizing.** `tracing` spans + `cargo flamegraph` recipes in `docs/perf.md`; a
    `--trace` flag writes a Chrome trace.

### 10.3 What not to optimize

- Startup network calls (they run in the background by design).
- Model catalog completeness over correctness.
- Binary size at the cost of rustls or Unicode correctness.

---

## 11. Security, privacy, and trust

### 11.1 Threat model (documented in `SECURITY.md`)

In scope:

- Accidental destructive tool execution.
- Secret leakage via logs, child processes, session files, exports.
- Untrusted project resources (`.codie/`, `AGENTS.md`, skills) influencing the agent.
- Supply-chain compromise of dependencies or release artifacts.
- Malicious extension processes.

Out of scope (documented, with guidance):

- OS-level sandboxing (use containers/VMs; we publish recipes).
- Prompt injection from repository content (mitigations, not guarantees).
- A user who intentionally runs `--allow-all` on an untrusted repo.

### 11.2 Controls

- Permission policy (`SEC-001..007`) with an audit log line per decision.
- Secret redaction filter applied to logs, errors, JSON events, and exports.
- `auth.json` 0600, atomic writes, never committed; `.gitignore` guidance.
- Project trust gate for `.codie/` resources.
- `cargo deny` (licenses, advisories, bans, sources), `cargo audit`, locked dependencies, committed lockfile.
- Signed releases (minisign/Sigstore), SHA256SUMS, SBOM (CycloneDX).
- Coordinated disclosure: private GitHub advisories + security email; 90-day window.

### 11.3 Privacy

- No telemetry by default. If added later, it is opt-in, documented, and off in CI.
- Session files are local; `codie export` warns when output may contain secrets.
- Provider requests go only to the configured base URL; a `codie doctor` check prints the effective endpoints.

---

## 12. Repository, tooling, and release engineering

### 12.1 Layout

```
codie/
├── Cargo.toml                 # workspace, shared lints
├── rust-toolchain.toml
├── Cargo.lock                 # committed
├── PLAN.MD                    # this document
├── README.md
├── CHANGELOG.md
├── CONTRIBUTING.md
├── CODE_OF_CONDUCT.md
├── SECURITY.md
├── LICENSE-MIT
├── LICENSE-APACHE
├── crates/
│   ├── codie-core/
│   ├── codie-ai/
│   ├── codie-tools/
│   ├── codie-session/
│   ├── codie-context/
│   ├── codie-prompt/
│   ├── codie-tui/
│   ├── codie-protocol/
│   ├── codie-cli/
│   └── codie-testkit/
├── specs/
│   ├── README.md
│   ├── provider/
│   ├── agent/
│   ├── tools/
│   ├── session/
│   ├── context/
│   ├── prompt/
│   ├── tui/
│   ├── cli/
│   ├── protocol/
│   ├── extensions/
│   └── security/
├── docs/
│   ├── book.toml              # mdBook
│   ├── src/
│   ├── adr/
│   └── rfc/
├── schemas/                   # generated JSON Schemas
├── catalog/                   # bundled model data
├── examples/
├── xtask/
└── .github/
    ├── workflows/
    ├── ISSUE_TEMPLATE/
    └── PULL_REQUEST_TEMPLATE.md
```

### 12.2 Toolchain and gates

- Edition 2024, MSRV pinned, `rust-toolchain.toml` with `rustfmt` + `clippy`.
- Workspace lints: `unsafe_code = "forbid"`, `missing_docs = "warn"`, `unreachable_pub = "warn"`.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`.
- `cargo deny check`, `cargo audit`, `cargo machete`.
- `cargo xtask spec-check`, `cargo xtask spec-report`, `cargo xtask ci`. (`schema-check` arrives with the
  generated protocol schemas in Phase 5.)
- Docs build with `-D warnings` (broken intra-doc links fail).
- CI matrix: one `check` job on `macos-15-intel` (x64) runs `cargo build --workspace --all-targets` and
  is release-blocking. Formatting, linting, tests, spec-check, docs, machete, and deny stay in
  `cargo xtask ci` for local and release gates. macOS arm64 is verified locally and in release builds.
- PR preview: `cargo-dist` artifacts for the branch (optional).

### 12.3 Versioning and releases

- Semver. Before 1.0: `0.MINOR.PATCH`; breaking changes bump MINOR.
- Conventional Commits; `git-cliff` generates `CHANGELOG.md`.
- Release tags `vX.Y.Z`; release notes grouped Added/Changed/Fixed/Removed/Security.
- `cargo-dist` produces macOS arm64/x64 (or universal2) archives, a Homebrew formula, and checksums;
  Linux archives are best-effort; Windows is post-1.0.
- Reproducible builds: locked deps, `--remap-path-prefix`, documented toolchain.
- Publish order: `codie-*` libraries (if published) then CLI; crates.io trusted publishing via OIDC.
- Release checklist in `docs/release.md`; a release captain rotates.

### 12.4 Community and governance

- `CONTRIBUTING.md`: spec-first contribution flow, "good first issue" label, review SLA, no drive-by refactors.
- `CODE_OF_CONDUCT.md`: Contributor Covenant 2.1.
- `GOVERNANCE.md`: BDFL until 5 maintainers, then consensus; roadmap RFC process.
- `ROADMAP.md` mirrors §8 and is updated at each phase boundary.
- Discussion forum (GitHub Discussions) with categories: Q&A, Showcase, Ideas, Development.
- `docs/book` published to GitHub Pages; versioned docs per minor release.

### 12.5 Platform support tiers

Support claims are explicit so users know what to expect and maintainers know what blocks a release.
**v1.0 is macOS-first.**

| Tier | Meaning | Release-blocking? | Platform |
|---|---|---|---|
| **1** | Fully supported. CI builds the workspace on real hardware; the full suite runs locally and before releases; P0/P1 bugs block release. | Yes | **macOS 13+ (arm64 and x64)** |
| **2** | Builds and core paths work; best-effort fixes; known gaps documented. | No | Linux glibc 2.31+ (x64/arm64), WSL |
| **3** | Community-supported; no guarantees. Post-1.0 roadmap. | No | Windows 10 21H2+, other Unixes |

**Tier-1 macOS scope (D-11).** The following are in scope and tested on every release:

- Universal distribution: native arm64 and x64 binaries (or a universal2 binary), signed with a Developer ID
  and notarized so Gatekeeper opens it without workarounds.
- `print`, `json`, `rpc`, and interactive TUI on Terminal.app, iTerm2, Ghostty, kitty, WezTerm, and the
  VS Code integrated terminal.
- crossterm raw mode, bracketed paste, truecolor/256-color fallback, resize, Option-as-Meta handling,
  Cmd-key behavior (Cmd is left to the terminal; we bind Ctrl/Alt only).
- Process control with POSIX process groups: abort/timeout kills the whole tree; no orphaned children.
- Paths: `~` expansion, APFS case-insensitivity, Unicode normalization (NFD/NFC) on filenames, `/Volumes`
  and symlink traversal, filenames with spaces and emoji.
- Clipboard via `pbcopy`/`pbpaste` with an OSC 52 fallback over SSH.
- No unsigned-binary quarantine prompts; `xattr`/quarantine behavior documented.
- PTY-based integration tests using the macOS pseudo-terminal API.

**Post-1.0 platform roadmap.** Linux is promoted to tier 1 when a maintainer-owned CI runner and a terminal
compatibility pass exist. Windows tier-1 work (Job Objects, ConPTY, PowerShell tool, long/UNC paths) is
tracked as a post-1.0 epic (§12.6) and is **not** release-blocking for 1.0.

**Tier-1 requirement:** every release is tested on macOS x64 in CI and smoke-tested on arm64 locally or in
the release pipeline; a red `check` job blocks the tag.

### 12.6 Post-1.0 platform roadmap

**Linux → tier 1 (0.x minor after 1.0).** A maintainer-owned Linux runner, terminal matrix
(alacritty/kitty/wezterm), musl static builds, and a green full-suite job promote Linux. The codebase stays
portable from day one (`std::path`, no macOS-only assumptions outside `codie-platform`), so this is a
testing and packaging effort, not a rewrite.

**Windows → tier 1 (post-1.0 epic).** The work is already scoped so it does not surprise us later:

- MSVC toolchain build; no Git Bash/MSYS2 required to run `codie`; signed `.exe`.
- `print`, `json`, `rpc`, and interactive TUI in Windows Terminal and modern conhost.
- Process-tree termination via **Job Objects** (`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`), including
  grandchildren; no reliance on `taskkill`.
- Path handling: drive letters, UNC paths, long paths (>260 chars), junctions, case-insensitive
  comparisons, `\\?\` prefixes; CRLF and BOM preservation in `read`/`edit`/`write`.
- `powershell` tool (`powershell.exe` / `pwsh.exe`); `bash` uses Git Bash only when present.
- Console control events instead of POSIX signals; ConPTY-based integration tests.

None of this is in the 1.0 critical path. macOS-only code must sit behind `cfg(target_os = "macos")` and
must not leak into shared modules.

---

## 13. Risks, mitigations, and open decisions

### 13.1 Risk register

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Rust TUI scope creep (editor is a project in itself) | High | High | Phase 2 timebox; start with ratatui `TextArea`-class behavior; defer images/Kitty |
| Provider drift breaks parsing | Medium | High | Fixtures + tolerant parser + nightly live smoke; versioned compat flags |
| OpenRouter model heterogeneity | High | Medium | v1 uses only the OpenAI-compatible surface; explicit compat overrides per model |
| Tool output wording changes alter model behavior | Medium | Medium | Freeze tool text in specs; snapshot tests |
| Session format mistakes become permanent | Medium | High | Freeze in Phase 3; migration tests; version field from day one |
| Extension protocol over-design | Medium | Medium | Phase 6 only; start with 6 hooks and 2 capabilities |
| Performance regressions | Medium | Medium | Budgets in CI; criterion baselines; flamegraph recipes |
| Cross-platform process termination | Low (macOS) / Medium (Linux) | Medium | Dedicated `codie-platform` module; POSIX process groups; deferred Windows Job Objects |
| Security incident via `bash` | Medium | High | Deny-by-default headless; ask-by-default interactive; clear docs |
| Maintainer burnout / scope | High | High | Phase gates, explicit non-goals, "no" as a feature |

### 13.2 Decision log (proposed defaults — confirm during Phase 0)

**Locked 2026-11-11:** D-1 (name `codie`), D-5 (out-of-process JSON-RPC extensions), D-6
(`ask` interactive / `deny` mutating headless), D-11 (macOS tier 1 for v1.0; Linux tier 2, Windows post-1.0).
The remaining rows are proposed defaults and may be challenged with an ADR before the phase that depends on
them.

| # | Decision | Proposed | Alternatives |
|---|---|---|---|
| D-1 | Project name | **`codie` — confirmed 2026-11-11** | `tenon`, `relay`, `forge` |
| D-2 | License | MIT OR Apache-2.0 | MIT only |
| D-3 | Config format | TOML | JSON + schema |
| D-4 | TUI framework | ratatui + crossterm | custom renderer (pi-style) |
| D-5 | Extension model | **out-of-process JSON-RPC — confirmed 2026-11-11** | embedded Rhai/Lua/WASM |
| D-6 | Permission defaults | **`ask` interactive, `deny` mutating headless — confirmed 2026-11-11** | allow (pi behavior) |
| D-7 | Session scope | tree from day one | linear first, tree later |
| D-8 | Provider scope | OpenRouter + DeepSeek only | add Anthropic native early |
| D-9 | Binary distribution | cargo-dist + Homebrew | npm-style installer |
| D-10 | Telemetry | none | opt-in crash reports |
| D-11 | Platform support | **macOS tier 1 for v1.0 (arm64 + x64); Linux tier 2; Windows post-1.0 — confirmed 2026-11-11** | Windows tier 1 at launch |

### 13.3 Open questions

1. Should `codie` support a `--yolo`/`--allow-all` flag for CI, or require explicit per-tool allowlists?
2. Do we want a built-in `plan` mode and `todo` tool at 1.0, or keep them as example extensions (pi's choice)?
3. Is an in-process embedding API (`codie-sdk`) a 1.0 requirement, or is RPC sufficient?
4. How much of pi's exact tool wording do we keep? (Recommendation: keep it; add a conformance fixture.)
5. ~~Windows support level at 1.0: tier 1 (full) or tier 2 (best effort)?~~ Resolved by D-11: **macOS is
   the only tier-1 platform for v1.0.** Linux stays tier 2 (build/test signal); Windows tier-1 is a
   post-1.0 epic (§12.6). This keeps the 1.0 critical path focused and testable.

---

## Appendix A — Reference request/response shapes

### A.1 DeepSeek streaming request

```http
POST /chat/completions HTTP/1.1
Host: api.deepseek.com
Authorization: Bearer $DEEPSEEK_API_KEY
Content-Type: application/json

{
  "model": "deepseek-flash",
  "messages": [
    {"role": "system", "content": "You are a coding assistant..."},
    {"role": "user", "content": "Fix the failing test in src/lib.rs"}
  ],
  "tools": [{"type": "function", "function": {"name": "read", "description": "...", "parameters": {}}}],
  "stream": true,
  "stream_options": {"include_usage": true},
  "max_tokens": 8192,
  "thinking": {"type": "enabled"},
  "reasoning_effort": "high"
}
```

### A.2 DeepSeek streaming response (abridged)

```
data: {"id":"...","choices":[{"index":0,"delta":{"role":"assistant","reasoning_content":"We need to "}}]}
data: {"id":"...","choices":[{"index":0,"delta":{"reasoning_content":"inspect the test."}}]}
data: {"id":"...","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"read","arguments":"{\"path\":"}}]}}]}
data: {"id":"...","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"src/lib.rs\"}"}}]}}]}
data: {"id":"...","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}
data: {"id":"...","choices":[],"usage":{"prompt_tokens":1200,"completion_tokens":80,"prompt_cache_hit_tokens":900,"completion_tokens_details":{"reasoning_tokens":40}}}
data: [DONE]
```

### A.3 OpenRouter streaming request

```http
POST /api/v1/chat/completions HTTP/1.1
Host: openrouter.ai
Authorization: Bearer $OPENROUTER_API_KEY
HTTP-Referer: https://github.com/amardeepio/codie
X-Title: codie
x-session-id: 0192f0c4-...
Content-Type: application/json

{
  "model": "deepseek/deepseek-flash",
  "messages": [...],
  "stream": true,
  "stream_options": {"include_usage": true},
  "max_completion_tokens": 8192,
  "reasoning": {"effort": "high"},
  "provider": {"order": ["deepinfra", "together"]},
  "tools": [...]
}
```

### A.4 OpenRouter usage chunk

```json
{"choices":[],"usage":{"prompt_tokens":1200,"completion_tokens":80,"total_tokens":1280,
 "cost":0.00042,"cost_details":{"upstream_inference_cost":0.00040},
 "prompt_tokens_details":{"cached_tokens":900},"is_byok":false}}
```

---

## Appendix B — Tool contracts (summary)

| Tool | Schema | Output | Limits |
|---|---|---|---|
| `read` | `{path, offset?, limit?}` | file text, no line numbers | 2000 lines / 50 KiB; offset notices |
| `write` | `{path, content}` | `Successfully wrote to <path>` | creates parent dirs |
| `edit` | `{path, edits:[{oldText,newText}]}` | `Successfully replaced N block(s) in <path>` | unique, non-overlapping; BOM/EOL preserved |
| `bash` | `{command, timeout?}` | combined stdout+stderr + exit notice | tail 2000 lines / 50 KiB; temp file for full output; no default timeout |
| ~~`powershell`~~ | `{command, timeout?}` | post-1.0 (Windows tier 1) | — |
| `grep` | `{pattern, path?, glob?, ignoreCase?, literal?, context?, limit?}` | `path:line: text` | 100 matches, 500 chars/line, 50 KiB |
| `find` | `{pattern, path?, limit?}` | relative paths | 1000 results, 50 KiB |
| `ls` | `{path?, limit?}` | sorted entries, `/` for dirs | 500 entries, 50 KiB |

Exact strings, edge cases, and error text are normative in `specs/tools/*.md` and mirrored from pi's
implementations.

---

## Appendix C — Default keybindings (interactive)

| Action | Default |
|---|---|
| Submit | `Enter` |
| Newline | `Shift+Enter`, `Ctrl+J` |
| Interrupt / abort | `Esc` |
| Clear input | `Ctrl+C` (twice to exit when empty) |
| Exit | `Ctrl+D` (empty) |
| Suspend | `Ctrl+Z` |
| External editor | `Ctrl+G` |
| Model picker | `Ctrl+L` |
| Cycle model | `Ctrl+P` / `Shift+Ctrl+P` |
| Cycle thinking | `Shift+Tab` |
| Toggle thinking block | `Ctrl+T` |
| Expand tool output | `Ctrl+O` |
| Follow-up (queue) | `Alt+Enter` |
| Dequeue | `Alt+Up` |
| Paste image | `Ctrl+V` |
| Transcript search | `Ctrl+Shift+F` |
| Page up/down | `PageUp` / `PageDown` |

---

## Appendix D — Spec index

| Spec | Requirements | Phase |
|---|---|---|
| `specs/message/0001-message-model.md` | MSG-001..010, MSG-040..044 | 1 |
| `specs/provider/0001-openai-compatible.md` | PROV-001..005, PROV-010..022 (canonical) | 1 |
| `specs/provider/0002-deepseek.md` | profile of PROV-010..019; no new IDs | 1 |
| `specs/provider/0003-openrouter.md` | profile of PROV-010..022; no new IDs | 1 |
| `specs/provider/0004-auth.md` | PROV-030..034 | 1 |
| `specs/agent/0001-turn-loop.md` | AGENT-001..012 | 1 |
| `specs/tools/0001-tool-contract.md` | TOOL-001..008 | 1 |
| `specs/tools/0002..0008-*.md` | per tool | 1–2 |
| `specs/session/0001-jsonl-format.md` | SESS-001..009 | 1, frozen in 3 |
| `specs/context/0001-compaction.md` | CTX-001..007 | 3 |
| `specs/prompt/0001-system-prompt.md` | PRMPT-001..008 | 1, 4 |
| `specs/tui/0001-layout-and-rendering.md` | TUI-001..010 | 2 |
| `specs/cli/0001-cli-surface.md` | CLI-001..007 | 1 |
| `specs/protocol/0001-json-events.md` | PROTO-001, PROTO-002, PROTO-004, PROTO-005 | 1 |
| `specs/protocol/0002-rpc.md` | PROTO-003, PROTO-006 | 5 |
| `specs/extensions/0001-jsonrpc-hooks.md` | EXT-001..006 | 6 |
| `specs/security/0001-permissions.md` | SEC-001..008 | 1, 4 |

---

## Appendix E — Phase 1 acceptance script (concrete)

```bash
# 1. Build
cargo build --release

# 2. Auth (key never echoed, never persisted with --api-key)
export DEEPSEEK_API_KEY=...
./target/release/codie auth status --json

# 3. Headless run in a scratch repo
mkdir -p /tmp/codie-demo && cd /tmp/codie-demo && git init -q
printf 'def add(a, b):\n    return a - b\n' > math.py
cat > test_math.py <<'EOF'
from math import add
assert add(2, 3) == 5
EOF

./target/release/codie -p --allow edit,bash \
  "Make the test pass. Read the files, fix the bug, run the test."

# Expected: exit 0, test passes, session file exists, JSON mode replay matches

# 4. Deterministic replay with fixtures
cargo test -p codie-ai --features fixtures
cargo xtask spec-check
```

---

## Appendix F — Definition of "done" for this plan

The plan is considered executed when:

1. Every requirement ID in Appendix D has an accepted spec.
2. Every accepted spec's `MUST` has at least one named test.
3. Phases 0–5 are complete and tagged.
4. `v1.0.0` artifacts are published, signed, notarized, and installable on macOS (arm64 + x64).
5. The project has an external contributor who merged a non-trivial PR (bus-factor signal).

---

*End of plan. Changes to scope require an RFC; changes to phases require maintainer approval recorded in
`docs/adr/`.*
