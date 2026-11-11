# Development

codie uses a pinned Rust toolchain and a spec-first workflow. The repository is expected to build and test
offline once dependencies are cached; no live provider credentials are required for the default test suite.

## Prerequisites

- Rust 1.88, installed through `rust-toolchain.toml` (rustup selects it automatically).
- `rustfmt` and `clippy` components, also pinned by the toolchain file.
- macOS 13+ for the primary development path. Linux works for portability work. Windows is post-1.0.
- No Node, Bun, or Python runtime is required to build or run codie.

## Setup

```bash
git clone https://github.com/amardeepio/codie
cd codie
cargo build
cargo test --workspace
cargo run -- --help
```

The binary is `target/debug/codie`. `cargo run -- --version` should print the workspace version. In Phase 0 the
binary intentionally does little; features land in Phase 1.

## Quality gates

The full gate is:

```bash
cargo xtask ci
```

Until the xtask implementation lands, run the equivalent commands directly:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo deny check
cargo audit
cargo machete
```

`cargo xtask ci` runs formatting, linting, tests, `cargo xtask spec-check`, and documentation builds in a
fixed order. Deny, machete, and docs run in the same CI job. `cargo xtask spec-check` fails
when an `implemented` spec has an untested `MUST`/`SHOULD`, a test names an unknown requirement, an ID is
malformed, or two specs declare the same ID. `--strict` extends coverage enforcement to `accepted` specs.

## Spec-first workflow

1. Open an issue describing the behavior and the failure mode it prevents.
2. Add or update a spec under `specs/`, following [`specs/template.md`](../../specs/template.md).
3. Meet the Definition of Ready in [`specs/README.md`](../../specs/README.md): IDs, edge cases, wire examples,
   test plan, security and performance impact.
4. Implement only the specified behavior. Add tests named after requirement IDs and tag them with
   `#[spec("AREA-NNN")]`.
5. Update user docs and add a `CHANGELOG.md` entry under `[Unreleased]`.
6. Open a pull request; the PR template repeats the checklist.

No feature is merged without an accepted spec and a traceable test. Changing an accepted or released
requirement requires an RFC under [`docs/rfc/`](../rfc/).

## Tests and fixtures

- Unit tests live beside the code; integration tests live in `crates/*/tests/`.
- Snapshot tests use `insta`; snapshot changes must be reviewed as diffs, and `*.snap.new` files are never
  committed.
- Property tests use `proptest` for session projection, edit matching, SSE chunk boundaries, and JSONL framing.
- Provider tests use recorded fixtures and `FakeProvider`; live APIs are never called in CI.
- Recording is opt-in through the `--record` test feature and an environment variable, and writes to a scratch
  directory.

## Documentation

The book is built with mdBook:

```bash
mdbook build docs
mdbook serve docs
```

`docs/book/` is generated and ignored. Documentation builds treat broken links as errors. The book covers
users and contributors; `specs/` remains the normative source for behavior.

## Commits and pull requests

Use Conventional Commits (`feat:`, `fix:`, `docs:`, `spec:`, `test:`, `refactor:`, `chore:`). Keep pull requests
focused: no drive-by refactors, no unrelated formatting churn. A pull request should state the requirement IDs
it implements, the tests that cover them, and any spec or ADR it changes.

See [`CONTRIBUTING.md`](../../CONTRIBUTING.md) for the full checklist and code style.
