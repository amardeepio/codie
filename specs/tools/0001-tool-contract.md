---
id: SPEC-TOOL-001
title: Tool contract and built-in tools
status: accepted
version: 1.0.0
owners: ["@codie-maintainers"]
requirements: ["TOOL-001", "TOOL-002", "TOOL-003", "TOOL-004", "TOOL-005", "TOOL-006", "TOOL-007", "TOOL-008", "TOOL-030"]
---

# Context

Tools are the only way the model observes or changes the workspace. Every tool exposes a stable wire name, a
one-paragraph description, and a self-contained JSON Schema. Tool failures are results the model can read, never
loop failures. Phase 1 ships `read`, `write`, `edit`, and `bash`.

# Requirements

**TOOL-001 (MUST)** The registry declares `read`, `write`, `edit`, the platform shell, `grep`, `find`, `ls`, and `web_fetch` in that order, each with an object schema; unknown names resolve to no tool. The platform shell is `bash` on macOS and Linux and `powershell` on Windows.
Verification: `crates/codie-tools/tests/schema_validation.rs::spec_tool_001_registry_declares_schemas`

**TOOL-002 (MUST)** Arguments are validated against the tool's JSON Schema before execution. Invalid arguments produce an error result and the tool is not invoked.
Verification: `crates/codie-tools/tests/schema_validation.rs::spec_tool_002_invalid_arguments_never_execute`

**TOOL-003 (MUST)** `read` returns numbered-free text, supports 1-indexed `offset` and `limit`, reports offsets beyond EOF, rejects non-positive values, marks binary files as errors, and truncates at 2,000 lines or the byte cap with an explicit notice.
Verification: `crates/codie-tools/tests/tools.rs::spec_tool_003_read_semantics`

**TOOL-004 (MUST)** `write` creates parent directories, replaces the file atomically enough that concurrent writers serialize per path, and reports the byte count written.
Verification: `crates/codie-tools/tests/tools.rs::spec_tool_004_write_semantics`

**TOOL-005 (MUST)** `edit` applies one or more exact-text replacements against the original file, rejects missing, duplicate, overlapping, and no-op edits, preserves CRLF outside changed ranges, and reports the first changed line.
Verification: `crates/codie-tools/tests/tools.rs::spec_tool_005_edit_semantics`

**TOOL-006 (MUST)** `bash` runs the command in its own process group, streams output with updates, defaults to a 120 second timeout, kills the whole group on timeout or abort, truncates the tail with an explicit notice, and surfaces the exit code. When the shell exits but a background child keeps the output pipe open, the tool returns the buffered output after a short grace period instead of waiting forever.
Verification: `crates/codie-tools/tests/tools.rs::spec_tool_006_bash_semantics`

**TOOL-007 (MUST)** A tool error, denial, missing tool, or panic becomes an error `tool_result` message; the agent loop continues and the model may recover.
Verification: `crates/codie-core/tests/agent_loop.rs::spec_tool_007_failures_are_results`

**TOOL-008 (MUST)** Mutations to one path are serialized by a per-path lock so two concurrent writes or edits cannot interleave.
Verification: `crates/codie-tools/tests/tools.rs::write_serializes_concurrent_updates_to_one_file`

**TOOL-030 (MUST)** On Windows, the registry exposes `powershell` in the shell slot. It runs `pwsh.exe` when present and otherwise `powershell.exe`, forces UTF-8 output, uses the same 120-second default timeout and tail truncation as `bash`, and terminates the process tree with `taskkill /T /F /PID` on timeout or abort.
Verification: `crates/codie-tools/tests/powershell.rs::powershell_times_out`

# Failure modes

- Invalid JSON arguments: schema validation error result.
- Missing file, permission denied, binary content: error result with the path.
- Timeout: process group terminated, partial output retained, error result.
- Panic inside a tool: caught by the loop and converted to an error result (`AGENT-012`).

# Security and performance

Paths are resolved against the working directory but are not sandboxed; the permission policy is the control.
Output is bounded so a runaway command cannot exhaust memory. `bash` emits progress at most every 100 ms.

# Out of scope

The read-only search tools are defined by `SPEC-TOOL-002`. Extension tools are Phase 6.
