---
id: SPEC-TRUST-001
title: Project trust
status: implemented
version: 1.0.0
owners: ["@codie-maintainers"]
requirements: ["TRUST-001", "TRUST-002", "TRUST-003", "TRUST-004"]
---

# Context

A repository can ship instructions (`AGENTS.md`), skills, templates, and system prompt fragments. Treating those
as trusted input would let any cloned repository influence the agent's behavior without the user's consent. Project
trust makes that boundary explicit: repository-controlled `.codie/` resources load only after approval, while the
absence of approval keeps the agent usable with built-in behavior.

# Normative behavior

**TRUST-001 (MUST)** Project resources under `<project>/.codie/` — skills, templates, `SYSTEM.md`, and
`APPEND_SYSTEM.md` — load only when the canonical project directory is trusted. Untrusted projects contribute no
project resources; user-level configuration still loads.
Verification: `crates/codie-prompt/tests/trust.rs::spec_trust_001_untrusted_projects_contribute_no_resources`

**TRUST-002 (MUST)** `--approve` records the canonical project directory in `trust.toml` in the config directory
and trusts it for the current run. Writes are atomic, and the file uses a stable, sorted list of paths.
Verification: `crates/codie-prompt/tests/trust.rs::spec_trust_002_approve_persists_the_canonical_path`

**TRUST-003 (MUST)** In an interactive terminal, an untrusted project with `.codie/` resources prompts once to
trust, approve, or decline. Declining or EOF continues the run without project resources and does not persist
trust. Non-interactive runs never prompt and default to untrusted.
Verification: `crates/codie-prompt/tests/trust.rs::spec_trust_003_declining_keeps_project_untrusted`

**TRUST-004 (MUST)** Trust entries are compared after canonicalization; a missing, unreadable, or malformed
`trust.toml` yields no trusted projects rather than an error, and an entry that does not exist cannot match a
different project.
Verification: `crates/codie-prompt/tests/trust.rs::spec_trust_004_trust_file_errors_fail_closed`

# Data shapes

```toml
# trust.toml
trusted = ["/Users/me/src/project"]
```

# Edge cases

- Symlinked project paths trust the canonical target, not the link.
- A project without `.codie/` is never prompted and needs no trust.
- `--approve` is idempotent.
- Trust is per project root; nested directories inherit trust from the approved ancestor.

# Security and performance

This is the phase's security boundary. Trust grants no execution by itself; it only allows text resources to enter
the prompt. Resource sizes are capped (`SKILL-002`, `TMPL-002`) and `.codie/` paths are resolved inside the
project root.
