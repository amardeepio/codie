# Pull request

## Summary

<!-- What changes, and why? Link the issue or discussion. -->

## Spec / requirement tracking

<!--
Every behavior change must point at the normative text it implements or
changes. Use requirement IDs from specs/ (for example MSG-001, AGENT-001,
PROV-010, SEC-003). If no requirement exists yet, say which spec or ADR this
PR should add or amend.
-->

- Spec: `specs/...`
- Requirement IDs:
- ADR / RFC (if any):

## Type of change

- [ ] Bug fix (no behavior change beyond the defect)
- [ ] New feature (spec updated first)
- [ ] Refactor (no behavior change; justified)
- [ ] Documentation
- [ ] CI, tooling, or repository hygiene
- [ ] Security hardening

## Tests

<!-- Name the tests added or updated. Snapshot updates must be explained. -->

- [ ] Unit tests
- [ ] Integration / E2E tests (fake provider where possible)
- [ ] Snapshot tests reviewed (`cargo insta review`)
- [ ] Manual verification (describe below)

Manual steps / evidence:

## Changelog

- [ ] `CHANGELOG.md` updated under `Unreleased`, or
- [ ] No user-visible change (explain why):

## Security checklist

- [ ] No secrets, tokens, credentials, session files, or private paths in code,
      tests, fixtures, logs, or snapshots.
- [ ] Tool execution still goes through the permission policy; no new bypass.
- [ ] Untrusted input (repo files, `AGENTS.md`, `.codie/`, provider responses)
      is validated or explicitly treated as untrusted.
- [ ] Secret-redaction paths were considered for any new log, error, event, or
      export surface.
- [ ] New dependencies are justified, licensed compatibly, and pass `cargo deny`.
- [ ] No new `unsafe` code (`unsafe_code = "forbid"`).
- [ ] Platform-specific code is behind `cfg(...)` and has a tier-2 fallback
      where applicable.

## Review checklist

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo test --workspace`
- [ ] `cargo run -p xtask -- spec-check`
- [ ] Docs build without warnings (`cargo doc --workspace --no-deps`)
- [ ] Changes are scoped; no drive-by refactors
