# ADR 0002: Out-of-process extensions over JSON-RPC

- Status: Accepted
- Date: 2026-11-11
- Deciders: codie maintainers
- Related: `PLAN.MD` §2.3, §6.12, §13.2 (D-5, D-6)

## Context

Users need to add tools, slash commands, hooks, and renderers without forking the core. The reference
implementation loads TypeScript extensions in-process through `jiti`. Rust has no stable in-process language
plugin ABI, and embedding a scripting runtime would put foreign code in the agent's address space, complicate
cancellation and panic isolation, and expand the trusted computing base.

The extension boundary also interacts with the permission model. The confirmed D-6 decision makes `write`,
`edit`, and `bash` ask by default interactively and deny by default in headless mode. An extension must not be
able to bypass that policy by calling a tool directly.

## Decision

Extensions are separate processes that speak versioned JSON-RPC over stdio using the `codie-ext` protocol.

- The core never loads foreign code into its address space.
- Declarative resources (skills, prompt templates, themes) are loaded first and require no process.
- An extension manifest (`codie.toml`) declares name, version, protocol version, entrypoint, and capabilities.
- Hook points are `session_start`, `session_shutdown`, `before_agent_start`, `context`, `before_tool_call`,
  `after_tool_call`, `turn_end`, and `user_submit`, plus custom tools and slash commands.
- A hook may block a tool call with `{block, reason}` but cannot mutate the transcript directly.
- Extension failures are logged and isolated. A hung extension is killed without hanging the agent.
- Extension processes inherit no API keys by default and remain subject to the host permission policy.
- Protocol changes are versioned; incompatible changes require a new protocol version and an RFC.

## Consequences

Positive:

- Process isolation contains crashes, memory bugs, and runaway extensions.
- Any language can implement an extension; the repository ships TypeScript and Python examples.
- Cancellation, timeouts, and capability checks have a clear process boundary.
- The permission engine remains the single control point for destructive actions.

Negative:

- Every hook is an IPC round trip; hooks on the hot path need timeouts and batching.
- Extension authors must manage a process lifecycle and a protocol version.
- Structured renderers are less expressive than in-process UI code; the protocol intentionally allows only
  structured output, not ANSI.
- Supervision, health checks, and protocol conformance tests become core work in Phase 6.

## Alternatives considered

- **Embedded Rhai/Lua/WASM.** Rejected for v1: still expands the trusted core, fragments the extension
  ecosystem across runtimes, and does not solve language neutrality.
- **In-process dynamic libraries.** Rejected: no stable Rust ABI, unsafe loading, and poor crash isolation.
- **No extension system.** Rejected: customization is a stated goal, and declarative resources alone cannot
  express tool hooks.

## References

- `PLAN.MD` §6.12 (`EXT-001`..`EXT-006`), §13.2 (D-5, D-6).
- Future normative spec: `specs/extensions/0001-jsonrpc-hooks.md`.
