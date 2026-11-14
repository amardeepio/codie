# Development

codie uses a pinned Rust toolchain and a spec-first workflow. The repository is expected to build and test
offline once dependencies are cached; no live provider credentials are required for the default test suite.

## Prerequisites

- Rust 1.88, installed through `rust-toolchain.toml` (rustup selects it automatically).
- `rustfmt` and `clippy` components, also pinned by the toolchain file.
- macOS 13+ for the primary development path. Linux works for portability work. Windows x64 is a preview;
  see [Windows (preview)](windows.md).
- No Node, Bun, or Python runtime is required to build or run codie.

## Setup

```bash
git clone https://github.com/amardeepio/codie
cd codie
cargo build
cargo test --workspace
cargo run -- --help
```

The binary is `target/debug/codie`. `cargo run -- --version` prints the workspace version.

On Windows x64, build and test with:

```powershell
cargo build --workspace --all-targets
cargo test -p codie-tools --test powershell
```

The `bash` tool is POSIX-only; Windows registers `powershell` in the same registry slot.

## Quality gates

The full gate is:

```bash
cargo xtask ci
```

The equivalent commands, plus the network-dependent checks that CI owns:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo xtask spec-check --strict
cargo xtask schemas --check
cargo xtask perf
cargo xtask coverage
cargo deny check
cargo audit
cargo machete
```

`cargo xtask ci` runs formatting, linting, tests, strict `cargo xtask spec-check`, the 80 percent coverage gate,
and documentation builds in a fixed order. Coverage requires `cargo install cargo-llvm-cov` and the
`llvm-tools-preview` component. CI only builds the workspace; run `cargo xtask ci` locally before pushing.
`cargo xtask spec-check` fails
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

## Crash reports

A panic writes `crash-<timestamp>.txt` under `$CODIE_DATA_DIR/crashes` (default
`~/.local/share/codie/crashes`) and prints the path to stderr. A report contains the codie version, OS and
architecture, the panic message, its location, and a backtrace. Environment variables, credentials, and
transcript content are never included. Review the file before attaching it to an issue.

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
