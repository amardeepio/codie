# Extensions

Extensions add tools, slash commands, and hooks without loading foreign code into codie. Each
extension is a separate process that speaks line-delimited JSON-RPC 2.0 over stdin/stdout. A
crashed, hung, or malformed extension is isolated and skipped; the agent keeps running.

The process boundary is an isolation boundary for bugs and hangs, **not a security sandbox**. An
extension runs with your user privileges. Treat installing or trusting an extension like installing
a small program.

## Discovery

codie looks for extensions in three places:

| Location | Loaded when |
|---|---|
| `~/.config/codie/extensions/<name>/codie.toml` | Always |
| `<trusted-project>/.codie/extensions/<name>/codie.toml` | Only after the project is trusted |
| `--extension <path>` | Explicitly, for this run |

Project manifests override user manifests with the same `name`; explicit paths override both.
`--no-extensions` disables every process. To disable selected names, set:

```toml
# ~/.config/codie/config.toml
[extensions]
disabled = ["plan-mode"]
```

or run:

```bash
codie config set extensions-disabled plan-mode,todo
```

Declarative resources such as skills, templates, and themes are loaded before any extension process
starts and never require one.

## Manifest

```toml
name = "todo"
version = "0.1.0"
protocol = 1
description = "Example todo tools and a /todo command."
entrypoint = ["python3", "todo.py"]
capabilities = ["tools", "commands"]

[limits]
hook_timeout_ms = 2000
tool_timeout_ms = 120000
```

- `entrypoint` is an executable plus arguments, run with the manifest directory as the working
  directory. It is not passed through a shell.
- `protocol` must equal the host protocol version. Today that is `1`.
- `capabilities` is a hint; the initialize result is authoritative.
- Limits are clamped by the host. A timed-out extension is killed.

Extension children receive a sanitized environment. Provider API keys, `*_API_KEY` variables,
`TOKEN` variables, and anything outside a small allowlist (`PATH`, `HOME`, locale, and terminal
basics) are removed. `CODIE_EXTENSION_PROTOCOL`, `CODIE_EXTENSION_NAME`, and
`CODIE_EXTENSION_VERSION` are set explicitly.

## Handshake

The host sends `initialize` with the protocol version, cwd, session id, deferred flags, and host
capabilities. The extension replies with:

```json
{
  "protocol": 1,
  "name": "todo",
  "version": "0.1.0",
  "capabilities": ["tools", "commands"],
  "hooks": [],
  "tools": [
    {
      "name": "todo_add",
      "description": "Add one todo item.",
      "parameters": {
        "type": "object",
        "properties": {"text": {"type": "string"}},
        "required": ["text"],
        "additionalProperties": false
      },
      "execution": "sequential"
    }
  ],
  "commands": [{"name": "todo", "description": "Show the todo list"}],
  "flags": [],
  "renderers": ["markdown"]
}
```

A protocol mismatch disables that extension with a warning. Tool and command names are validated;
duplicates across extensions are skipped. Each extension may declare up to 64 tools, 64 commands, and
64 flags. Extension tools pass through the same permission policy as built-in mutating tools:
headless runs deny them unless `--allow` names them, and the interactive app asks before running them.

## Hooks

| Hook | When | Returns |
|---|---|---|
| `session_start` | A session is opened | Empty result |
| `session_shutdown` | A session is closing | Empty result |
| `before_agent_start` | Before an agent run | `context`, `prompt`, or `block` |
| `context` | Before every provider request | `context` for this request only |
| `before_tool_call` | Before a tool runs | `{block, reason}` |
| `after_tool_call` | After a tool settles | Optional replacement `result` |
| `turn_end` | After a turn ends | Optional `context` for the next request |
| `user_submit` | When the user submits input | `prompt` rewrite or `{block, reason}` |

`before_tool_call` is the normal way to enforce a policy: return
`{"block": true, "reason": "..."}` and the model sees an error tool result. A hook cannot write
directly into the persisted transcript. Context returned by `context` is injected into an in-memory
copy of the system prompt and stripped before the next turn.

Hooks run with a timeout. On timeout the host kills the extension process group and continues with
the safe default: context is skipped, a tool block is not applied, and the agent keeps running.

## Tools

A tool result is:

```json
{
  "content": [{"type": "text", "text": "Added t1."}],
  "details": {"id": "t1"},
  "is_error": false,
  "terminate": false,
  "render": {
    "kind": "markdown",
    "title": "Todo",
    "text": "- [ ] **t1** write the docs"
  }
}
```

`render` is an optional structured hint. Kinds are `text`, `markdown`, `code`, and `diff`; ANSI
escape sequences are stripped. Tool output is capped at one mebibyte.

## Commands

Declare commands in the initialize result. Invoke them as:

```text
/ext:<extension>:<command> [arguments]
```

A bare `/<command>` also works when it is unambiguous and does not collide with a built-in. A
command can return a prompt to submit:

```json
{"prompt": "Act as a focused subagent for this task: ..."}
```

or structured output without starting a provider request:

```json
{"display": {"kind": "markdown", "title": "Todo", "text": "_No todos._"}}
```

## Flags

An extension can claim unknown long flags by returning them in its `flags` list:

```json
{"name": "todo-file", "description": "Todo file", "takes_value": true}
```

Use `--todo-file=path`. A bare `--todo-file` fails because the flag requires a value. Unclaimed long
flags, unknown short flags, and flags used on management subcommands (`auth`, `models`, `config`,
`sessions`, `update`) fail with exit code 1.

## Examples

The repository ships working examples:

- [`examples/extensions/todo`](../../examples/extensions/todo) — Python tools, a command, and
  markdown rendering.
- [`examples/extensions/plan-mode`](../../examples/extensions/plan-mode) — TypeScript hooks that
  block mutating tools and inject plan-mode context.
- [`examples/extensions/subagent-lite`](../../examples/extensions/subagent-lite) — Python custom
  command that expands a scoped prompt.

Minimal SDKs live in [`examples/extensions/_sdk`](../../examples/extensions/_sdk). The Python SDK
needs only the standard library. The TypeScript SDK runs on Node 22.6+ with
`--experimental-strip-types`.

```bash
# Load the todo example for one run
codie --extension examples/extensions/todo --allow todo_add -p "add a todo: write docs"

# Use the todo command without starting a provider request
codie --extension examples/extensions/todo -p "/todo"
```

## Protocol reference

The generated schemas are checked into the repository:

- [`schemas/ext-manifest.schema.json`](../../schemas/ext-manifest.schema.json)
- [`schemas/ext-protocol.schema.json`](../../schemas/ext-protocol.schema.json)
- [`schemas/ext-hooks.schema.json`](../../schemas/ext-hooks.schema.json)

The normative requirements are `EXT-001`..`EXT-016` in
[`specs/extensions/0001-jsonrpc-hooks.md`](../../specs/extensions/0001-jsonrpc-hooks.md).
