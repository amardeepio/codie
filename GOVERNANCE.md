# Governance

This document describes how codie makes decisions, who can make them, and how the process changes. It applies to
the repository, the specification baseline, releases, and the community spaces listed in `CONTRIBUTING.md`.

## Principles

1. **Specs over preferences.** Decisions are argued against the normative requirements, not personal taste.
2. **Consensus where possible, a tie-breaker where necessary.** Discussion is public and recorded; silence is
   not consent for a change to released behavior.
3. **Reversibility matters.** Cross-cutting or hard-to-reverse decisions require an ADR. Released behavior
   changes require an RFC.
4. **One release captain per release.** Responsibility for a tag is explicit, not diffuse.
5. **No hidden authority.** Decisions and their rationale live in the repository, not in private chat.

## Current model: BDFL

Until the project has five maintainers, codie operates under a benevolent dictator for life (BDFL). The BDFL:

- Accepts or rejects RFCs and ADRs.
- Resolves deadlocks and settles disputes that cannot reach consensus.
- Appoints maintainers and the release captain.
- Can veto a change that violates the project's scope, security posture, or license.

The BDFL is expected to follow the same spec and review process as everyone else and to record decisions in the
decision log. A veto must include a written rationale.

## Transition to consensus

When the project has five or more active maintainers, governance moves to consensus:

- Routine changes use lazy consensus: a proposal is accepted if no maintainer objects within five business days.
- Significant changes (RFCs, new maintainers, governance changes) require explicit approval from at least
  two-thirds of maintainers, with a minimum of three approvals.
- A maintainer who objects must state a technical reason and a path to resolution. An objection without a
  reason is recorded but does not block.
- If consensus fails after two review rounds, any maintainer may call a vote. A simple majority of active
  maintainers decides, and the result is recorded in the RFC or issue.
- The transition itself requires an RFC approved by the BDFL and every active maintainer.

## Roles

| Role | Responsibilities |
|---|---|
| Contributor | Opens issues and PRs, follows the spec-first workflow, reviews where able. |
| Reviewer | Reviews for correctness against specs and tests; no merge rights. |
| Maintainer | Merges PRs, triages issues, accepts specs, participates in RFC decisions, upholds the code of conduct. |
| Release captain | Owns one release cycle: gates, changelog, artifacts, signing, and the tag. |
| BDFL | Final tie-breaker and guardian of scope until the consensus transition. |

### Becoming a maintainer

A contributor becomes a maintainer through sustained, high-quality work: multiple merged changes, careful
reviews, and demonstrated judgment about scope and compatibility. Any maintainer may nominate a candidate. The
BDFL appoints before the transition; afterward, two-thirds of maintainers must approve. Maintainers who are
inactive for six months move to emeritus status and stop counting toward quorum; they may return by request.

## Decision process

- **Routine change:** issue, spec PR where needed, code PR, review, merge.
- **Released behavior change:** RFC under `docs/rfc/`, seven-day public review, decision recorded, then spec and
  implementation PRs.
- **Irreversible or cross-cutting decision:** ADR under `docs/adr/` with context, decision, consequences, and
  alternatives. The ADR is merged before the dependent phase begins.
- **Security report:** private handling per `SECURITY.md`; public disclosure only after a fix and coordination.
- **Scope change:** RFC approved by maintainers and recorded in `PLAN.MD` and `ROADMAP.md`.

An RFC is accepted by rough consensus, with the BDFL breaking ties while the BDFL model applies. The decision,
date, and dissenting positions are recorded in the RFC. Accepted RFCs are immutable except for status, decision
date, and links; a later RFC supersedes them.

## Release captain

The release captain rotates by release and is appointed by the BDFL (or, after the transition, by maintainer
consensus). The captain:

1. Confirms every phase exit criterion and the release checklist.
2. Runs the full CI matrix and blocks the tag on a red macOS arm64 or x64 job.
3. Verifies `cargo xtask spec-check` and the changelog entry.
4. Builds, signs, notarizes, and publishes artifacts with checksums and an SBOM.
5. Tags `vX.Y.Z`, publishes release notes grouped as Added, Changed, Fixed, Removed, and Security, and announces
   the release.
6. Ensures the Homebrew formula and install script point at the new artifacts.

The captain cannot waive a spec requirement, a security gate, or a failing platform job. The release checklist
lives in `docs/release.md` once that document is created; until then, `PLAN.MD` §12.3 is authoritative.

## Decision log

Decisions are recorded in three places:

- `PLAN.MD` §13.2 for locked project decisions (name, extensions, permissions, platforms) and proposed defaults.
- `docs/adr/` for accepted architecture decisions with consequences and alternatives.
- `docs/rfc/` for proposed changes to accepted or released behavior, including rejected proposals.

Locked decisions as of 2026-11-11:

| ID | Decision |
|---|---|
| D-1 | Project name is `codie`. |
| D-5 | Extensions are out-of-process JSON-RPC processes. |
| D-6 | Permissions default to `ask` interactively and `deny` for mutating tools in headless mode. |
| D-11 | macOS 13+ arm64/x64 is the only tier-1 platform for v1.0; Linux is tier 2; Windows is post-1.0. |

Changing a locked decision requires an RFC and an ADR that supersedes the original record. The decision log is
append-only; superseded entries are marked, not deleted.

## Conduct and conflicts

All participants follow `CODE_OF_CONDUCT.md`. Maintainers disclose conflicts of interest before voting on a
change that affects their employer, a competing product, or a dependency they maintain. A conflicted maintainer
may participate in discussion but does not count toward quorum on that decision.

## Amendments

Amendments to this document require an RFC and the approval threshold of the current model: the BDFL before the
transition, two-thirds of maintainers afterward. The amendment takes effect when the RFC is merged and this file
is updated in the same pull request.
