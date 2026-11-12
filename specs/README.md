# codie specifications

This directory is the normative source of truth for codie's behavior. `PLAN.MD` defines scope, architecture, and
phases; a spec defines the observable contract that code and tests must satisfy. When the two disagree about
behavior, the accepted spec wins and `PLAN.MD` is corrected by a documentation PR.

Specs are not design notes. A requirement that cannot be verified by an automated test does not belong here.

## Artifact types

| Artifact | Location | Purpose |
|---|---|---|
| Product plan | [`PLAN.MD`](../PLAN.MD) | Scope, phases, architecture, decision log |
| Spec | `specs/<area>/<NNNN>-<slug>.md` | Normative behavior with traceable requirement IDs |
| ADR | [`docs/adr/`](../docs/adr/) | Irreversible or cross-cutting decisions |
| RFC | [`docs/rfc/`](../docs/rfc/) | Proposed changes to an accepted or released spec |
| Fixture | `crates/*/tests/fixtures/` | Recorded provider and wire data |
| Report | `target/spec-report.md` | Generated requirement-to-test matrix |

## Requirement IDs

The format is `<AREA>-<NNN>`, for example `AGENT-004` or `PROV-012`.

Allocated areas are: `PROV` (providers), `MSG` (message model), `AGENT` (agent loop), `TOOL` (tools),
`SESS` (sessions), `CTX` (compaction and context), `PRMPT` (prompt, skills, templates), `TUI` (terminal UI),
`CLI` (arguments and configuration), `PROTO` (JSON and RPC), `EXT` (extensions), `SEC` (security and
permissions), and `PERF` (performance).

Rules:

1. An ID names exactly one requirement in exactly one canonical spec. IDs are never reused, renumbered, or
   copied into a second spec.
2. IDs are allocated in the canonical spec. Ranges mentioned in `PLAN.MD` or Appendix D are reservations, not
   permission to duplicate a requirement.
3. Requirements are atomic, observable, and testable. "Be robust" is not a requirement.
4. Every requirement uses the RFC 2119 keyword that matches its weight: `MUST`, `SHOULD`, or `MAY`.
5. Every requirement names at least one acceptance test on the following `Verification:` line.
6. Changing a released requirement requires an RFC. Changing an unreleased requirement requires a spec PR and
   maintainer review.

### Requirement line format

The parser expects one requirement per line, in exactly this shape:

```markdown
**MSG-001 (MUST)** Messages are one of `system`, `user`, `assistant`, or `toolResult` ...
Verification: `crates/codie-core/tests/message_model.rs::spec_msg_001_message_kinds_lower_before_wire`
```

The `Verification:` line MUST follow immediately. It names a future or existing test path; wildcard suffixes
such as `::spec_msg_001_*` are permitted only while the spec is `draft`.

Do not wrap requirement text onto a second line. Long requirements are split into additional IDs.

## Statuses

| Status | Meaning | Exit condition |
|---|---|---|
| `draft` | Under review; behavior may change without an RFC. | Definition of Ready is met and maintainers accept it. |
| `accepted` | Normative and stable for its unreleased scope; implementation may lag. | All `MUST`/`SHOULD` requirements have passing named tests, then move to `implemented`. |
| `implemented` | Accepted behavior is implemented and covered in CI. | The spec is deprecated or a bug reopens coverage. |
| `deprecated` | Superseded; retained for traceability. | Never reused for new work. |

Versioning is semantic: a clarification is a patch, additive requirements are a minor version, and removing or
contradicting a requirement is a major version plus an RFC.

## Definition of Ready

Before implementation starts, a spec MUST have:

1. An accepted status and allocated requirement IDs.
2. Enumerated failure modes and edge cases.
3. Concrete wire, file-format, or event examples where the behavior crosses a boundary.
4. A test plan naming each acceptance test.
5. A security and performance impact note, even when the answer is "none".

## Definition of Done

Before merge, a change MUST have:

1. Code that implements only specified behavior, plus explicit `MAY`s.
2. One test per `MUST`/`SHOULD` whose name contains the requirement ID.
3. `cargo test --workspace`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, and
   `cargo deny check` passing.
4. The spec updated when behavior changed, and an ADR when a decision was made.
5. User-visible documentation updated under `docs/`.
6. A `CHANGELOG.md` entry under `[Unreleased]`.

## Traceability tooling

`cargo xtask spec-check` performs four steps:

1. Parse every `specs/**/*.md` file and collect requirement IDs, keywords, and frontmatter.
2. Scan `crates/**/*.rs` and `xtask/**/*.rs` for `#[spec("AREA-NNN")]` attributes.
3. Fail when an `implemented` spec has an untested `MUST`/`SHOULD`, when a test attribute references an
   unknown ID, when an ID is malformed, or when the same ID is declared by two specs.
4. Print a per-area summary and exit non-zero on any violation. `--strict` extends coverage enforcement to
   `accepted` specs as well.

`cargo xtask spec-report` writes `target/spec-report.md`, a matrix of requirement, keyword, spec status, and
the tests currently annotated with that ID. `cargo xtask ci` runs formatting, linting, tests, spec-check, and
documentation build in that order; `cargo deny` and `cargo audit` run as separate CI jobs.

The `Verification:` line is the human-readable acceptance target. Automated coverage is discovered from the
`#[spec]` attribute, so the two must agree. The `#[spec]` attribute is a no-op proc macro from
`codie-testkit`; it exists only for discovery. The test name must still contain the requirement ID in
snake_case so failures are readable without the macro.

## Index

| Spec | Requirement IDs | Status | Phase |
|---|---|---|---|
| [`SPEC-MSG-001`](message/0001-message-model.md) — Message model | `MSG-001`..`MSG-010`, `MSG-040`..`MSG-044` | accepted | 1 |
| [`SPEC-AGENT-001`](agent/0001-turn-loop.md) — Agent turn loop | `AGENT-001`..`AGENT-012` | accepted | 1 |
| [`SPEC-AGENT-002`](agent/0002-continuation.md) — Transcript continuation | `AGENT-013` | implemented | 3 |
| [`SPEC-PROV-001`](provider/0001-openai-compatible.md) — OpenAI-compatible provider | `PROV-001`..`PROV-005`, `PROV-010`..`PROV-022` | accepted | 1 |
| [`SPEC-PROV-002`](provider/0002-deepseek.md) — DeepSeek profile | none (profile) | accepted | 1 |
| [`SPEC-PROV-003`](provider/0003-openrouter.md) — OpenRouter profile | none (profile) | accepted | 1 |
| [`SPEC-PROV-004`](provider/0004-auth.md) — Authentication | `PROV-030`..`PROV-034` | accepted | 1 |
| [`SPEC-TOOL-001`](tools/0001-tool-contract.md) — Tool contract | `TOOL-001`..`TOOL-008` | accepted | 1 |
| [`SPEC-TOOL-002`](tools/0002-search-tools.md) — Read-only search tools | `TOOL-010`..`TOOL-014` | implemented | 4 |
| [`SPEC-TOOL-003`](tools/0003-web-fetch.md) — Web fetch | `TOOL-020`..`TOOL-022` | implemented | 4 |
| [`SPEC-SESS-001`](session/0001-jsonl-format.md) — Session JSONL v1 | `SESS-001`..`SESS-009` | accepted | 1 |
| [`SPEC-SESS-002`](session/0002-tree-operations.md) — Session tree operations | `SESS-010`..`SESS-025` | implemented | 3 |
| [`SPEC-CTX-001`](context/0001-compaction.md) — Context compaction | `CTX-001`..`CTX-007` | implemented | 3 |
| [`SPEC-PRMPT-001`](prompt/0001-system-prompt.md) — System prompt | `PRMPT-001`..`PRMPT-008` | accepted | 1 |
| [`SPEC-PRMPT-002`](prompt/0004-instructions-and-attachments.md) — System instructions and attachments | `PRMPT-009`..`PRMPT-011` | implemented | 4 |
| [`SPEC-SKILL-001`](prompt/0002-skills.md) — Skills | `SKILL-001`..`SKILL-004` | implemented | 4 |
| [`SPEC-TMPL-001`](prompt/0003-templates.md) — Prompt templates | `TMPL-001`..`TMPL-003` | implemented | 4 |
| [`SPEC-CLI-001`](cli/0001-cli-surface.md) — CLI surface | `CLI-001`..`CLI-007` | accepted | 1 |
| [`SPEC-PROTO-001`](protocol/0001-json-events.md) — JSON events | `PROTO-001`, `PROTO-002`, `PROTO-004`, `PROTO-005` | accepted | 1 |
| [`SPEC-PROTO-002`](protocol/0002-rpc.md) — RPC mode | `PROTO-003`, `PROTO-006` | implemented | 5 |
| [`SPEC-SEC-001`](security/0001-permissions.md) — Permissions | `SEC-001`..`SEC-006` | accepted | 1 |
| [`SPEC-SEC-002`](security/0002-trust.md) — Project trust | `TRUST-001`..`TRUST-004` | implemented | 4 |
| [`SPEC-TUI-001`](tui/0001-layout-and-rendering.md) — Interactive terminal | `TUI-001`..`TUI-010` | accepted | 2 |
| [`SPEC-TUI-002`](tui/0002-session-controls.md) — Session controls and picker | `TUI-011`..`TUI-013` | implemented | 3 |
| [`SPEC-TUI-003`](tui/0003-command-palette.md) — Command palette and live pickers | `TUI-014`..`TUI-017` | implemented | 3 |
| [`SPEC-TUI-004`](tui/0004-permission-mode.md) — Permission choices and full access | `TUI-018`, `TUI-019` | implemented | 3 |
| [`SPEC-TUI-005`](tui/0005-running-indicators.md) — Running indicators and scrolling | `TUI-020`, `TUI-021` | implemented | 4 |

`SEC-007`/`SEC-008` (project trust and approval prompts) are canonical in `specs/security/0002-trust.md`. The
full planned index is in `PLAN.MD` Appendix D.

New specs start from [`template.md`](template.md). Terminology is defined in [`glossary.md`](glossary.md).
