---
rfc: 0000
title: Short imperative title
status: draft
authors: ["@handle"]
created: YYYY-MM-DD
targets: ["AREA-001"]
---

# Summary

One paragraph. State the change, who benefits, and what happens if the RFC is rejected. Avoid implementation
detail here; a reader should be able to decide whether to keep reading.

# Motivation

What problem exists today? Cite the affected requirement IDs and the user-visible or maintainer-visible
consequence. Include concrete examples, failure modes, or measurements. If the change is cosmetic, say so.

# Guide-level explanation

Explain the change as it appears to a user or extension author. Show commands, configuration, and before/after
behavior. Define new terms or link them in `specs/glossary.md`.

# Reference-level explanation

Describe the exact behavior change. For each affected requirement, give the current text and the proposed text.
Include wire formats, file formats, event ordering, and error behavior. Normative requirements use RFC 2119
keywords and remain one line per ID in the eventual spec.

| Requirement | Current | Proposed |
|---|---|---|
| `AREA-001` | ... | ... |
| `AREA-002` | New | ... |

# Compatibility and migration

- Is this a breaking change? For which versions?
- What happens to existing sessions, configs, credentials, or extensions?
- Is a migration required on read? Is it reversible?
- What is the deprecation window and the removal version?

# Drawbacks

Why might this be a bad idea? Include implementation cost, maintenance burden, performance impact, and effects
on the support matrix. Do not list only strawmen.

# Rationale and alternatives

Compare at least two alternatives, including "do nothing". Explain why the proposed design wins. Record
constraints from `PLAN.MD`, prior ADRs, and provider or platform behavior.

# Prior art

Reference `pi`, other coding agents, protocol specifications, or upstream crates. Distinguish behavior we copy
from code we do not copy.

# Unresolved questions

List questions that must be answered before acceptance. Mark each as blocking or non-blocking. Remove this
section's blocking items before changing the status to `accepted`.

# Test plan

Name the acceptance tests that will carry each requirement ID, following the `Verification:` convention in
`specs/README.md`. Include fixtures, snapshot changes, property tests, and manual checks. State what cannot be
tested automatically and why.

# Security and performance impact

Assess new trust boundaries, secret handling, permission interactions, dependency additions, and effects on the
budgets in `PERF-001`..`PERF-006`. State "none" with a reason when there is no impact.

# Future possibilities

What does this unlock, and what is deliberately deferred? Keep this section short; speculative scope is not
part of the proposal.
