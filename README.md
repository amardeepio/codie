# codie

A fast, extensible, spec-driven coding agent for the terminal, written in Rust.

`codie` is a deliberate reimplementation of the architecture proven by [`pi`](https://github.com/earendil-works/pi).
It keeps the small event-driven core, append-only session tree, and provider abstraction, while changing the
implementation language, the safety posture, and the provider scope.

**Status: Phase 0, pre-alpha.** The workspace builds and the specification baseline is in place; the agent does
not yet perform coding tasks. Provider support, the agent loop, tools, sessions, and the TUI land in Phases 1
to 3. See [`ROADMAP.md`](ROADMAP.md).

## Why codie

- **One static binary.** No Node or Bun runtime, predictable memory, fast startup.
- **Safe by default.** Read-only tools are allowed; writes and shell commands ask interactively and are denied
  in headless mode unless explicitly allowed.
- **Provider discipline.** OpenRouter and DeepSeek at launch, behind one OpenAI-compatible adapter with
  recorded-fixture conformance. No live API calls in CI.
- **Sessions you can trust.** Append-only JSONL with resume, fork, and compaction. The file is the source of
  truth; the model-visible context is a projection.
- **Spec-first engineering.** Every normative requirement has an ID, a test path, and a traceability check.

## Principles

1. Behavior is specified before it is implemented. No feature without a spec and a traceable test.
2. The transcript is the prompt. System prompt and tool declarations are persisted and replayed, with no
   hidden state.
3. Tool calls execute only after the assistant message is final. Tool failures are results, not loop failures.
4. The core stays small. Skills, templates, themes, and extensions layer on top; foreign code never runs in the
   agent's address space.
5. Provider and platform claims are backed by tests, not aspirations.

## Architecture

```text
LLM wire adapter  ->  assistant event stream  ->  agent loop  ->  tool execution
                                                   |
                                           session (append-only JSONL tree)
                                                   |
                                    interactive TUI | print | JSON | RPC
```

Crate graph (target; crates are split out as consumers appear):

```text
                    ┌──────────────┐
                    │  codie-cli   │  binary, args, mode dispatch
                    └──────┬───────┘
       ┌───────────────────┼───────────────────┐
       ▼                   ▼                   ▼
┌────────────┐      ┌─────────────┐     ┌──────────────┐
│ codie-tui  │      │codie-proto  │     │ codie-prompt │
└─────┬──────┘      └──────┬──────┘     └──────┬───────┘
      └────────────────────┼───────────────────┘
                           ▼
                    ┌─────────────┐
                    │ codie-core  │  loop, messages, events
                    └──┬───────┬──┘
              ┌────────▼──┐  ┌─▼────────────┐
              │ codie-ai  │  │ codie-tools  │
              └────────┬──┘  └──────┬───────┘
              ┌────────▼────────────▼──────┐
              │      codie-context         │  compaction, tokens
              └────────────────────────────┘

codie-session: append-only JSONL store and projection
codie-testkit (dev only): fake provider, fixtures, #[spec] macro
```

The loop has no HTTP client. The provider adapter knows nothing about tool implementations. Only the CLI reads
environment variables and decides process exit codes. Details are in [`docs/src/architecture.md`](docs/src/architecture.md).

## Quickstart

Requirements: Rust 1.88, installed through `rust-toolchain.toml`. macOS 13+ is the tier-1 platform.

```bash
git clone https://github.com/your-org/codie
cd codie
cargo build
cargo run -- --help
cargo run -- --version
```

In Phase 0 the binary only supports `--help` and `--version`. Phase 1 adds:

```bash
export DEEPSEEK_API_KEY=...
cargo run -- -p "Fix the failing test in src/lib.rs"
cargo run -- --mode json -p "Summarize this repository"
```

Do not expect those commands to work before Phase 1. The planned command surface is documented in
[`PLAN.MD`](PLAN.MD) §4.2.

## Providers

| Provider | Status | API | Auth |
|---|---|---|---|
| DeepSeek | Planned, Phase 1 | OpenAI-compatible | `DEEPSEEK_API_KEY` |
| OpenRouter | Planned, Phase 1 | OpenAI-compatible | `OPENROUTER_API_KEY` |

Both are accessed through one adapter that handles SSE framing, tool-call accumulation, stop-reason and usage
mapping, retries, and per-provider compatibility flags. Anthropic-native passthrough, OAuth login, and
additional vendors are out of scope for v1.0. See [`docs/src/providers.md`](docs/src/providers.md) and
[`specs/provider/0001-openai-compatible.md`](specs/provider/0001-openai-compatible.md).

## Platforms

- **Tier 1:** macOS 13+ on arm64 and x64. Signed, notarized binaries; full CI suite; release-blocking.
- **Tier 2:** Linux glibc 2.31+ and WSL. Builds and core paths work; best-effort fixes.
- **Post-1.0:** Windows 10 21H2+ and other Unixes. No guarantees at 1.0.

Support claims and the macOS terminal matrix are in [`docs/src/platforms.md`](docs/src/platforms.md).

## Documentation

- [`PLAN.MD`](PLAN.MD) — scope, architecture, phases, decision log.
- [`specs/`](specs/) — normative requirements and the specification process.
- [`docs/src/`](docs/src/) — mdBook: introduction, architecture, development, specs, providers, platforms,
  security, roadmap.
- [`docs/adr/`](docs/adr/) — architecture decision records.
- [`docs/rfc/`](docs/rfc/) — change proposals for accepted or released behavior.
- [`CONTRIBUTING.md`](CONTRIBUTING.md) — development setup, spec-first workflow, PR checklist.
- [`GOVERNANCE.md`](GOVERNANCE.md) — decision making and release captains.
- [`CHANGELOG.md`](CHANGELOG.md) — release history.

Build the book with `mdbook build docs` and serve it with `mdbook serve docs`.

## Contributing

Contributions are welcome, but the process is strict: every feature needs an accepted spec and a named test.
Start with [`CONTRIBUTING.md`](CONTRIBUTING.md), then open an issue before a large change. Good first issues are
labeled in the tracker. Security reports follow the process in `SECURITY.md`.

## License

Licensed under either of:

- Apache License, Version 2.0 ([`LICENSE-APACHE`](LICENSE-APACHE))
- MIT License ([`LICENSE-MIT`](LICENSE-MIT))

at your option. Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in
this project shall be dual-licensed as above, without any additional terms or conditions.
