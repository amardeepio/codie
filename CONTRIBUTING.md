# Contributing to codie

Thanks for helping. codie is built spec-first: behavior is agreed in `specs/`, implemented behind traceable
tests, and documented before merge. Contributions that skip that process will be asked to restart from a spec,
not merged on an exception.

By participating you agree to the project's [`CODE_OF_CONDUCT.md`](CODE_OF_CONDUCT.md).

## Ways to contribute

- Report a bug with a minimal reproduction and the version from `codie --version`.
- Propose or review a spec under `specs/`.
- Implement an accepted requirement and its named tests.
- Improve documentation, fixtures, or test coverage.
- Report a security issue privately through the process in `SECURITY.md`; do not open a public issue.

For a large change, open an issue first. For a change to accepted or released behavior, start an RFC under
[`docs/rfc/`](docs/rfc/). For an irreversible or cross-cutting decision, expect an ADR.

## Development setup

Requirements:

- Rust 1.88. `rust-toolchain.toml` pins the exact toolchain; rustup selects it automatically.
- macOS 13+ for the primary development path. Linux works for portability work. Windows is post-1.0.
- No Node, Bun, or Python runtime is needed to build or run codie.
- `mdbook` only if you are editing the documentation book.

```bash
git clone https://github.com/your-org/codie
cd codie
cargo build
cargo test --workspace
cargo run -- --help
```

The full gate is:

```bash
cargo xtask ci
```

The xtask implementation is part of Phase 0. Until it is complete, run the equivalent commands directly:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo deny check
cargo audit
cargo machete
```

A change is not ready for review while any of these fail. CI runs on macOS arm64 and x64; the Linux job is a
portability signal and must stay green or be explicitly quarantined.

## Spec-first workflow

The rule is simple: **no feature without a spec.**

1. **Issue.** Describe the behavior, the user-visible problem, and the failure mode the change prevents.
2. **Spec.** Add or update a file under `specs/<area>/` using [`specs/template.md`](specs/template.md). Allocate
   requirement IDs from the reserved ranges; never reuse an ID or declare one in two specs.
3. **Ready.** Meet the Definition of Ready in [`specs/README.md`](specs/README.md): edge cases, wire or file
   examples, a test plan naming each acceptance test, and a security and performance impact note.
4. **Implement.** Write only the specified behavior plus explicit `MAY`s. Add tests named after requirement
   IDs and tag them with `#[spec("AREA-NNN")]`.
5. **Document.** Update `docs/` for user-visible behavior and add a `CHANGELOG.md` entry under `[Unreleased]`.
6. **Review.** Open a pull request using the template. State the requirement IDs, the tests that cover them, and
   any spec, ADR, or RFC change.

Changing an unreleased requirement requires a spec PR. Changing a released requirement requires an RFC. A
release is not a license to skip the spec; it is the point at which the spec becomes harder to change.

## Code style

- Run `cargo fmt`; formatting is not a review topic.
- `cargo clippy --all-targets -- -D warnings` must pass. Do not add `#[allow]` without a comment explaining
  why the lint is wrong in that location.
- `unsafe` is forbidden workspace-wide. Platform exceptions require an audited `codie-platform` shim and an ADR.
- Public APIs are `pub(crate)` by default. Expose a narrow, documented surface and add rustdoc for public
  items; missing docs are a warning, and documentation builds treat warnings as errors.
- Prefer typed structs over `serde_json::Value` outside tool-argument boundaries.
- Errors crossing a process or protocol boundary carry a stable string code. Do not change a code without a
  changelog entry and a compatibility note.
- No drive-by refactors, dependency additions, or formatting churn in a feature PR. Dependency additions need a
  justification in the PR and must pass `cargo deny` and `cargo machete`.

## Testing expectations

- Every `MUST` and `SHOULD` has at least one test whose name contains the requirement ID, for example
  `spec_agent_004_parallel_completion_order`.
- Add the failing test before the fix for a bug. A bug fix without a regression test is incomplete.
- Prefer deterministic tests. Provider tests use recorded fixtures and `FakeProvider`; live APIs are never
  called in CI and credentials are never committed.
- Snapshot changes use `insta` and must be reviewed as a diff. `*.snap.new` files are not committed.
- Property tests cover session projection, edit matching, SSE chunk boundaries, and JSONL framing.
- Wire-format changes require a fixture update, a changelog entry, and a spec change.
- Session-format changes require a migration test from every prior version.
- Performance-sensitive changes should include a benchmark or a clear argument against the budgets in `PERF-*`.

## Commit style

Use [Conventional Commits](https://www.conventionalcommits.org/):

```text
feat(ai): add DeepSeek reasoning replay
fix(core): preserve tool result order under abort
spec(agent): clarify follow-up polling
docs(adr): record macOS-first platform decision
test(tools): cover edit BOM preservation
chore(ci): cache cargo registry
```

Scopes are crate or area names (`ai`, `core`, `tools`, `session`, `cli`, `tui`, `spec`, `docs`, `ci`). The
summary is imperative, lower-case, and under 72 characters. Breaking changes use `!` and a `BREAKING CHANGE:`
footer. Reference issues in the footer, not the summary.

## Pull request checklist

- [ ] The change is covered by an accepted spec, or it is a documentation/test-only change.
- [ ] Requirement IDs are listed in the PR description.
- [ ] Every new `MUST`/`SHOULD` has a named test and `#[spec]` attribute.
- [ ] `cargo xtask ci` passes, or the equivalent commands above pass and the gap is noted.
- [ ] `CHANGELOG.md` has an `[Unreleased]` entry for user-visible changes.
- [ ] Docs are updated for user-visible behavior; the book builds.
- [ ] No secrets, credentials, session files, or generated snapshots are committed.
- [ ] The diff is focused; unrelated refactors are split out.
- [ ] An ADR or RFC is included when the change requires one.

## Review and merge

Maintainers review for correctness against the spec, test quality, and operational risk. The review SLA is
best-effort; a ping after five business days is appropriate. Pull requests are squash-merged with a
Conventional Commit title. The author is responsible for resolving conflicts and keeping the branch current.
