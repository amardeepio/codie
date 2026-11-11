# Specifications

Behavior in codie is defined by requirement IDs in `specs/`, not by implementation accident. A spec is
normative: if code and an accepted spec disagree, the code is wrong until the spec is changed through the
documented process.

## Process at a glance

1. `PLAN.MD` sets scope, architecture, phases, and decisions.
2. A spec under `specs/<area>/` defines testable behavior with allocated requirement IDs.
3. An ADR records an irreversible or cross-cutting decision.
4. An RFC proposes a change to accepted or released behavior.
5. Tests carry requirement IDs, and `cargo xtask spec-check` proves coverage.

Unreleased specs can change through a normal spec PR. Changing a released requirement requires an RFC, a spec
version bump, and a changelog entry.

## Requirement IDs

IDs use `<AREA>-<NNN>[-<SUB>]`. Allocated areas are `PROV`, `MSG`, `AGENT`, `TOOL`, `SESS`, `CTX`, `PRMPT`,
`TUI`, `CLI`, `PROTO`, `EXT`, `SEC`, and `PERF`. An ID names exactly one requirement in exactly one canonical
spec and is never reused.

Each requirement is one line:

```markdown
**MSG-001 (MUST)** Every message is exactly one of `system`, `user`, `assistant`, or `toolResult` ...
Verification: `crates/codie-core/tests/message_model.rs::spec_msg_001_message_kinds_lower_before_wire`
```

`MUST`, `SHOULD`, and `MAY` carry their RFC 2119 meanings. A `Verification:` line names the acceptance test; a
future path is acceptable only while the spec is `draft`.

## Statuses

| Status | Meaning |
|---|---|
| `draft` | Under review; behavior may change without an RFC. |
| `accepted` | Normative and stable for its unreleased scope. |
| `implemented` | Implemented and covered by passing named tests in CI. |
| `deprecated` | Superseded; retained for traceability. |

The Definition of Ready and Definition of Done are defined in [`specs/README.md`](../../specs/README.md).
In short: no implementation before edge cases, examples, and a test plan exist; no merge without a named test
per `MUST`/`SHOULD`, a changelog entry, and updated docs.

## Tooling

- `cargo xtask spec-check` parses specs, scans tests and `#[spec]` attributes, and fails on missing or orphaned
  requirements or duplicate declarations.
- `cargo xtask spec-report` writes `target/spec-report.md`, a requirement-to-test matrix.
- `cargo xtask fixtures` regenerates recorded provider fixtures offline.
- `cargo xtask ci` runs the full gate.

The `#[spec]` attribute is a no-op proc macro used only for discovery. The test name must also contain the
snake-case requirement ID so failures are readable.

## Accepted specs

| Spec | Requirements | Status | Phase |
|---|---|---|---|
| [Message model](../../specs/message/0001-message-model.md) | `MSG-001`..`MSG-010`, `MSG-040`..`MSG-044` | accepted | 1 |
| [Agent turn loop](../../specs/agent/0001-turn-loop.md) | `AGENT-001`..`AGENT-012` | accepted | 1 |
| [OpenAI-compatible provider](../../specs/provider/0001-openai-compatible.md) | `PROV-001`..`PROV-005`, `PROV-010`..`PROV-022` | accepted | 1 |

Auth requirements `PROV-030`..`PROV-034` are reserved for a future `specs/provider/0004-auth.md`. The full
planned index is in [`PLAN.MD`](../../PLAN.MD) Appendix D.

## Writing a spec

Start from [`specs/template.md`](../../specs/template.md). Keep the Context section short and concrete, put the
normative requirements in one section, include Rust type sketches and wire examples under data shapes, and list
edge cases that a reviewer could turn into tests. Non-goals are mandatory: they prevent duplicate requirements
and scope creep. Terminology belongs in [`specs/glossary.md`](../../specs/glossary.md), not in each spec.
