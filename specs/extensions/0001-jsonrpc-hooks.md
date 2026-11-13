---
id: SPEC-EXT-001
title: Out-of-process extensions
status: implemented
version: 1.0.0
owners: ["@codie-maintainers"]
requirements: ["EXT-001", "EXT-002", "EXT-003", "EXT-004", "EXT-005", "EXT-006", "EXT-007", "EXT-008", "EXT-009", "EXT-010", "EXT-011", "EXT-012", "EXT-013", "EXT-014", "EXT-015", "EXT-016"]
---

# Context

Users need custom tools, commands, and hooks without forking codie. Loading foreign code
in-process would put it inside the agent's address space, complicate cancellation and panic
isolation, and let an extension bypass the host permission policy. Extensions are therefore
separate processes that speak a versioned JSON-RPC 2.0 protocol over stdio. Declarative
resources (skills, templates, and themes) remain process-free and load first.

The extension process is not a sandbox. It runs with the user's privileges, so installing or
trusting an extension is the security boundary. Host tool permissions protect host tools, not
code the extension chooses to run itself. This limitation is documented for users.

# Normative behavior

**EXT-001 (MUST)** Extensions are separate processes. The host never loads extension code into its address space, and a crashed or hung extension never crashes or hangs the agent.
Verification: `crates/codie-ext/tests/extensions.rs::spec_ext_001_process_boundary`

**EXT-002 (MUST)** The host exposes exactly these hook points: `session_start`, `session_shutdown`, `before_agent_start`, `context`, `before_tool_call`, `after_tool_call`, `turn_end`, and `user_submit`, plus declared tools and slash commands.
Verification: `crates/codie-ext/tests/extensions.rs::spec_ext_002_hook_surface`

**EXT-003 (MUST)** `before_tool_call` can block a call with `{block: true, reason}`. A block becomes an error tool result. Hooks cannot write directly into the persisted transcript; context returned by a hook is applied to the in-memory request only and stripped before the next turn.
Verification: `crates/codie-ext/tests/extensions.rs::spec_ext_003_before_tool_can_block`

**EXT-004 (MUST)** Extension failures are logged with the extension name and isolated. A malformed manifest, failed handshake, dead process, hook error, or timeout is skipped; the agent continues. A failed custom tool becomes an error tool result. A failed command produces a visible error.
Verification: `crates/codie-ext/tests/extensions.rs::spec_ext_004_failure_is_isolated`

**EXT-005 (MUST)** Declarative resources load before any extension process starts and require no process. An empty extension set is valid and adds no startup work or failure.
Verification: `crates/codie-ext/tests/extensions.rs::spec_ext_005_declarative_resources_are_optional`

**EXT-006 (MUST)** An extension manifest named `codie.toml` declares a non-empty name, version, protocol version, entrypoint, capabilities, and optional timeout limits. Unsupported protocol versions, unsafe names, empty entrypoints, and zero limits are rejected before the process starts.
Verification: `crates/codie-ext/tests/extensions.rs::spec_ext_006_manifest_round_trip`

**EXT-007 (MUST)** Discovery considers user extensions and, only for trusted projects, project extensions, plus explicit `--extension` paths. Explicit paths win over project paths, and project paths win over user paths for the same manifest name. Disabled names are removed before any process starts.
Verification: `crates/codie-ext/src/manifest.rs::spec_ext_007_discovery_precedence_and_trust`

**EXT-008 (MUST)** Every extension performs an `initialize` handshake carrying the host protocol version, cwd, session id, deferred flags, and host capabilities. The extension returns its protocol version, capabilities, hooks, tools, commands, flags, and renderers. A protocol mismatch disables the extension with a warning.
Verification: `crates/codie-ext/tests/extensions.rs::spec_ext_008_handshake_and_versioning`

**EXT-009 (MUST)** Messages are LF-framed JSON-RPC 2.0 objects. Requests carry unique numeric IDs, responses echo the ID, notifications omit IDs, errors carry a numeric code and message, and a line larger than one mebibyte is rejected.
Verification: `crates/codie-protocol/src/ext.rs::spec_ext_009_jsonrpc_framing`

**EXT-010 (MUST)** Extension tools declare a valid lowercase wire name, a non-empty description, an object JSON Schema, and a scheduling mode. Arguments are validated by the host before the process is called. Extension tools pass through the same headless deny and interactive ask policy as built-in mutating tools.
Verification: `crates/codie-ext/tests/extensions.rs::spec_ext_010_extension_tools`

**EXT-011 (MUST)** Extensions can declare slash commands. A command is invocable as `/ext:<extension>:<command>`, and as `/<command>` when the name is unambiguous and does not collide with a built-in. A command returns either a prompt to submit or a structured display.
Verification: `crates/codie-ext/tests/extensions.rs::spec_ext_011_extension_commands`

**EXT-012 (MUST)** Structured render hints use one of `text`, `markdown`, `code`, or `diff` and never contain ANSI escape sequences. The host may fall back to raw tool output when a hint is absent or invalid.
Verification: `crates/codie-ext/tests/extensions.rs::spec_ext_012_structured_renderers`

**EXT-013 (MUST)** Every hook and tool call has a timeout. A timeout kills the extension process group, fails pending calls, and returns a safe default. Killing a hung extension never hangs the agent.
Verification: `crates/codie-ext/tests/extensions.rs::spec_ext_013_hung_extension_is_killed`

**EXT-014 (MUST)** Extension children receive a sanitized environment. Variables matching `*_API_KEY` or containing `TOKEN`, and all other unlisted variables, are removed by default. The protocol version and extension name are passed explicitly.
Verification: `crates/codie-ext/src/process.rs::spec_ext_014_no_secret_environment`

**EXT-015 (MUST)** Unknown long command-line flags are deferred until extensions load. A flag claimed by an extension is passed in the initialize handshake; an unclaimed long flag, an unknown short flag, or a value-shape mismatch fails with a usage error and exit 1. Management subcommands never defer flags.
Verification: `crates/codie-ext/tests/extensions.rs::spec_ext_015_extension_flags_are_validated`

**EXT-016 (MUST)** Tool, command, and flag declarations are ordered deterministically by extension name and declaration order. Invalid or duplicate names are skipped with a warning, and declaration limits are enforced.
Verification: `crates/codie-ext/tests/extensions.rs::spec_ext_016_duplicate_tool_names_are_skipped`

# Wire shapes

Manifest:

```toml
name = "todo"
version = "0.1.0"
protocol = 1
entrypoint = ["python3", "todo.py"]
capabilities = ["tools", "commands"]

[limits]
hook_timeout_ms = 2000
tool_timeout_ms = 120000
```

Handshake methods:

| Method | Direction | Purpose |
|---|---|---|
| `initialize` | host → extension | Version, cwd, session, flags, capabilities |
| `session_start` / `session_shutdown` | host → extension | Session lifecycle |
| `before_agent_start` / `context` | host → extension | Context injection |
| `before_tool_call` / `after_tool_call` | host → extension | Blocking and result inspection |
| `turn_end` / `user_submit` | host → extension | Turn and input hooks |
| `tool.call` | host → extension | Execute a declared tool |
| `command.invoke` | host → extension | Execute a declared command |
| `log` | extension → host | Notification only |

A tool result carries `content` blocks, optional `details`, `is_error`, `terminate`, and an
optional structured `render` hint. A command result carries an optional `prompt` or `display`.

# Edge cases

- A malformed initialize result disables only that extension.
- A hook timeout kills the extension; later hooks from that extension are skipped.
- A duplicate tool name from a later extension is skipped with a warning.
- Context injection is replaced each request and never written to the session.
- An extension that exits during a request fails that request and is skipped for the rest of the session.
- A command display-only result does not start a provider request.
- Extension context is escaped so transcript text cannot forge a closing tag.

# Non-goals

- In-process dynamic loading or embedded scripting runtimes.
- OS-level sandboxing of extension processes.
- Extensions calling host tools directly. They must declare tools and rely on the host policy.
- Interactive extension UI beyond structured render hints.

# Security and performance

Extensions inherit no provider keys by default. The host applies its permission policy to every
extension tool call and never lets a hook bypass `SEC-001`/`SEC-002`. Hooks run on the agent task
with timeouts; a hung extension is killed with its process group. RPC lines and tool output are
bounded to prevent memory exhaustion.
