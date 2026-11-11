# ADR 0001: Rust and edition 2024

- Status: Accepted
- Date: 2026-11-11
- Deciders: codie maintainers
- Related: `PLAN.MD` §1, §5.4, §12.2

## Context

codie is a terminal coding agent that must start quickly, stream reliably, and ship as a single installable
artifact on macOS. The reference implementation, `pi`, is TypeScript running on Bun. Reusing that stack would
carry a runtime, a package manager, and a larger startup and memory footprint into a tool that users invoke
many times per day.

The language choice is cross-cutting: it determines the concurrency model, the dependency policy, the error
model, the extension boundary, the release pipeline, and the contributor pool. Changing it after Phase 1 would
invalidate the crate graph and the public spec surface, so it is recorded as an ADR rather than left implicit
in `PLAN.MD`.

## Decision

Implement codie in Rust, pinned to edition 2024 with MSRV 1.88 in `rust-toolchain.toml`.

- One statically linked binary with no Node or JVM runtime dependency.
- Async I/O on a `tokio` multi-thread runtime with a small worker count.
- Workspace-wide `unsafe_code = "forbid"`, with no exception crate at Phase 0.
- Edition 2024 language rules apply from the first commit; no migration from an earlier edition is planned.
- Release builds use LTO, a single codegen unit, and symbol stripping, and are produced by `cargo-dist`.
- Libraries use `thiserror`; the binary uses `anyhow` at the outer boundary; errors crossing a protocol or
  process boundary carry stable string codes.

## Consequences

Positive:

- Cold start and idle memory budgets in `PERF-001` and `PERF-002` are achievable without a runtime.
- `tokio` cancellation, `JoinSet`, and bounded channels map directly to the runtime model in `PLAN.MD` §5.2.
- The type system makes the message and event invariants in `MSG-*` and `AGENT-*` enforceable.
- A single binary simplifies signing, notarization, and Homebrew distribution.

Negative:

- Compile times are higher than the TypeScript reference; CI caching and a small dependency set are required.
- The contributor pool is smaller than for TypeScript, so contribution docs and "good first issue" work matter.
- Some pi reference code must be re-derived behaviorally; no code is copied.
- Rust's lack of a stable in-process plugin ABI directly motivates ADR 0002.

## Alternatives considered

- **TypeScript on Bun (copy pi).** Rejected: runtime dependency, larger binary, weaker process and memory
  control, and a different security posture.
- **Go.** Rejected: viable concurrency and binaries, but weaker type-level modeling of message variants and a
  less mature terminal UI ecosystem for the chosen `ratatui` design.
- **Rust on an older edition.** Rejected: no compatibility benefit for a new codebase, and edition 2024
  improves lifetime and match ergonomics that the loop code uses.

## References

- `PLAN.MD` §1 (executive summary), §5.4 (dependency policy), §12.2 (toolchain and gates).
- `rust-toolchain.toml`, workspace lints in `Cargo.toml`.
