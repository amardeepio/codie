# Extension examples

These examples implement the same `codie-ext` protocol as the Rust host. They are documentation and
integration tests, not built-in codie features.

| Example | Language | Demonstrates |
|---|---|---|
| [`todo`](todo) | Python | Tools, a command, markdown render hints, tool permissions |
| [`plan-mode`](plan-mode) | TypeScript | `before_tool_call` blocking, `context` injection, state, a command |
| [`subagent-lite`](subagent-lite) | Python | Custom command that expands a prompt |

The minimal SDKs live under [`_sdk/`](_sdk):

- Python: standard library only, import `codie_ext.py`.
- TypeScript: Node 22.6+ with `--experimental-strip-types`, import `codie-ext.ts`.

## Run an example

```bash
# From the repository root
codie --extension examples/extensions/todo -p "/todo"
codie --extension examples/extensions/todo --allow todo_add -p "add a todo: write docs"

codie --extension examples/extensions/plan-mode
# then use /plan on, /plan off, and try an edit while plan mode is on
```

`--extension` is explicit consent for one run and works even when the project is not trusted. Project
discovery (`<repo>/.codie/extensions/<name>/codie.toml`) requires `--approve` or an interactive trust
decision.

## Write your own

1. Create `<name>/codie.toml` with `name`, `version`, `protocol = 1`, an `entrypoint`, and
   `capabilities`.
2. Implement `initialize` and return your tools, commands, hooks, flags, and renderers.
3. Handle `tool.call`, `command.invoke`, and any declared hooks.
4. Keep stdout reserved for JSON-RPC. Write diagnostics to stderr or send a `log` notification.

See [`docs/src/extensions.md`](../../docs/src/extensions.md) for the full protocol, timeouts, security
model, and failure behavior.
