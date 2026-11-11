# Requests for Comments

An RFC proposes a change to accepted or released behavior, scope, or process. It is the only path to changing
a requirement after a spec is `accepted`; unreleased drafts and clarifications can move through a normal spec
PR. See `PLAN.MD` §3.1 and §3.2.

## When an RFC is required

- Changing or removing a released requirement.
- Adding a provider, a public protocol version, or a platform tier.
- Changing the session format in a way that requires migration.
- Changing the extension protocol in an incompatible way.
- Changing governance, licensing, or the release process.

An ADR records a decision that has already been made; an RFC proposes and debates a change before it is
accepted. A change may need both: an accepted RFC that changes a spec, followed by an ADR if the decision is
cross-cutting and hard to reverse.

## Lifecycle

| Status | Meaning |
|---|---|
| `draft` | Open for discussion; the author is still shaping the proposal. |
| `accepted` | Approved; the linked specs and implementation work may proceed. |
| `rejected` | Not accepted; the rationale is retained. |
| `withdrawn` | The author retired the proposal. |
| `superseded` | Replaced by a later RFC, which is linked. |

An RFC is accepted by rough consensus of maintainers, with the BDFL breaking ties while the project is under
BDFL governance. Silence is not consent for a change to a released requirement. The decision, date, and
dissenting positions are recorded in the RFC before the status changes.

## Process

1. Copy [`template.md`](template.md) to `NNNN-<slug>.md`, using the next free number.
2. Open a pull request that adds only the RFC. Discussion happens on the PR.
3. Announce the RFC in GitHub Discussions under Development for a minimum review window of seven days.
4. Address review comments by editing the RFC; do not rewrite history in review threads.
5. On acceptance, update the linked specs and the changelog in follow-up PRs. The RFC itself is immutable
   except for status, decision date, and links.
6. If implementation reveals a problem, file a new RFC that supersedes the old one.

## Numbering and ownership

Numbers are allocated in order and never reused. The author owns the proposal through acceptance; a maintainer
owns the follow-up spec change. RFCs live in this directory for the life of the project, even when rejected,
because the rationale is part of the design record.

## Index

| RFC | Title | Status | Targets |
|---|---|---|---|
| — | No RFCs have been filed yet. | — | — |
