---
id: SPEC-PROTO-001
title: JSON event stream
status: accepted
version: 1.0.0
owners: ["@codie-maintainers"]
requirements: ["PROTO-001", "PROTO-002", "PROTO-004", "PROTO-005"]
---

# Context

`--mode json` is the machine interface for Phase 1. It emits a session header, then agent events, then a final
`agent_settled` record. The published schema lives at `schemas/json-events.schema.json`.

Requirements `PROTO-003` (RPC) and `PROTO-006` (RPC shutdown) are reserved for Phase 5 and are not part of the
Phase 1 surface.

# Record order

```text
session header
agent_start
turn_start ...
message_start / message_update / message_end ...
tool_execution_start / tool_execution_end ...
turn_end
agent_end
agent_settled
```

# Requirements

**PROTO-001 (MUST)** Records are LF-framed JSON objects. Only LF splits records, a trailing CR is stripped, and every record is valid JSON on its own line.
Verification: `crates/codie-cli/tests/e2e.rs::golden_path_end_to_end_with_a_fake_provider`

**PROTO-002 (MUST)** The stream begins with the session header and ends with `agent_settled`; agent events use the `type` discriminator defined by the message and event models.
Verification: `crates/codie-cli/tests/e2e.rs::golden_path_end_to_end_with_a_fake_provider`

**PROTO-004 (MUST)** `agent_settled` is the completion signal. It carries `aborted`, and consumers must not treat `agent_end` as final.
Verification: `crates/codie-cli/tests/e2e.rs::golden_path_end_to_end_with_a_fake_provider`

**PROTO-005 (MUST)** Every emitted record validates against the published schema. The schema and the Rust serialization are kept in sync by the end-to-end validation test.
Verification: `crates/codie-cli/tests/e2e.rs::golden_path_end_to_end_with_a_fake_provider`

# Failure modes

- A run error still emits `agent_end` with `status: "error"` and a final `agent_settled`.
- An interrupted run emits `agent_settled` with `aborted: true` and exits with the signal code.
- Diagnostics never appear on stdout.

# Security and performance

Events can contain full message content and tool output. They are not redacted, so consumers must treat the
stream as sensitive. Writing one record per line keeps the stream flushable and streamable.
