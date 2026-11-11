---
id: SPEC-AREA-001
title: Short imperative title
status: draft
version: 1.0.0
owners: ["@handle"]
requirements: ["AREA-001", "AREA-002"]
---

<!--
Copy this file to specs/<area>/<NNNN>-<slug>.md and replace every placeholder.
Keep `status: draft` until the Definition of Ready in specs/README.md is met.
Every requirement is one line in the form `**AREA-001 (MUST)** text...` and is
followed immediately by a `Verification:` line naming a test path.
-->

# Context

Why this exists; what breaks if it is wrong. Name the consumers of the behavior and the failure mode that the
spec prevents. Link the relevant sections of `PLAN.MD`.

# Normative behavior

**AREA-001 (MUST)** Replace with one atomic, observable requirement. State the condition, the required behavior, and the observable result.
Verification: `crates/codie-example/tests/example.rs::spec_area_001_descriptive_name`

**AREA-002 (SHOULD)** Replace with the next requirement. Use `SHOULD` only when a documented, justified exception is acceptable.
Verification: `crates/codie-example/tests/example.rs::spec_area_002_descriptive_name`

# State machine / data shapes

Describe states, transitions, and the Rust types that cross a boundary. Include invariants that are not
expressible in the type system. Keep wire examples minimal but complete enough to implement from.

# Edge cases

- Empty input, duplicate identifiers, cancellation at each await point.
- Ordering violations and how they are detected.
- Behavior at configured limits and at integer or size boundaries.
- What is persisted, what is discarded, and what is reported to the user.

# Non-goals

State what this spec deliberately does not cover, with a pointer to the spec that does. Non-goals prevent
scope creep and duplicate requirements.

# Reference implementation

pi: `packages/agent/src/agent-loop.ts` (behavioral reference only; no code copied).
