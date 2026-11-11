---
id: SPEC-SEC-001
title: Permissions and tool safety
status: accepted
version: 1.0.0
owners: ["@codie-maintainers"]
requirements: ["SEC-001", "SEC-002", "SEC-003", "SEC-004", "SEC-005", "SEC-006"]
---

# Context

Phase 1 is non-interactive. Read-only tools are always available; mutating tools (`write`, `edit`, `bash`) are
denied unless explicitly allowed with `--allow`, `--allow-all`, or configuration. A denial is a tool error the
model can read, not a run failure. Interactive approval prompts arrive in Phase 2.

Requirements `SEC-007` and `SEC-008` (project trust and approval prompts) are Phase 4 and are not allocated here.

# Requirements

**SEC-001 (MUST)** Read-only tools are permitted by default; `write`, `edit`, and `bash` are denied in headless mode unless they appear in the allow list.
Verification: `crates/codie-tools/src/registry.rs::read_is_always_allowed`

**SEC-002 (MUST)** An explicit deny wins over an allow for the same tool.
Verification: `crates/codie-tools/src/registry.rs::explicit_deny_wins`

**SEC-003 (MUST)** A denied call produces an error tool result, is not executed, and does not end the run.
Verification: `crates/codie-core/tests/agent_loop.rs::spec_sec_003_denial_is_an_error_result`

**SEC-004 (MUST)** Tool arguments are validated against the declared schema before policy and execution; invalid arguments never reach a tool.
Verification: `crates/codie-tools/tests/schema_validation.rs::spec_tool_002_invalid_arguments_never_execute`

**SEC-005 (MUST)** Request headers, request bodies, and response events logged at `trace` level never contain an API key.
Verification: `crates/codie-ai/tests/redaction.rs::spec_prov_005_trace_redaction_and_recording`

**SEC-006 (MUST)** The credential file is created with mode `0600` on Unix and is never written to a session.
Verification: `crates/codie-ai/tests/auth.rs::store_round_trips_and_is_private`

# Failure modes

- Denied tool: error result with the tool name and reason.
- Invalid arguments: schema error result before policy.
- Unknown tool: error result, not a crash.
- Panic in a tool: caught by the loop (`AGENT-012`).

# Security and performance

The policy is evaluated per call. It does not sandbox paths; `--allow bash` grants the same shell access the
user has. The default posture is read-only. No secret is ever part of a tool result.
