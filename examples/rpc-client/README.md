# RPC client example

A minimal integration that drives `codie --mode rpc` over stdin/stdout.

```bash
# From the repository root, after `cargo build`
python3 examples/rpc-client/rpc_client.py "Summarize this repository"
```

Environment:

- `CODIE_BIN` — binary to run (defaults to `target/debug/codie`, then `codie` on PATH).
- `CODIE_ARGS` — extra arguments, for example `--provider openrouter --model ...`.

The script sends a `prompt`, prints finalized assistant text, and sends `shutdown` after `agent_settled`.
See [docs/src/rpc.md](../../docs/src/rpc.md) for the full command set and
[`schemas/rpc-commands.schema.json`](../../schemas/rpc-commands.schema.json) for the generated schema.
