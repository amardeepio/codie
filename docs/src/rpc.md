# RPC mode

`codie --mode rpc` is a long-lived, line-delimited JSON interface for editor integrations and scripts. It reads
commands on stdin and writes records on stdout; diagnostics go to stderr.

```bash
codie --mode rpc --provider deepseek
```

The first two records are the version handshake and the session header:

```json
{"type":"rpc_ready","version":1}
{"type":"session","version":1,"id":"...","timestamp":"...","cwd":"/path/to/project"}
```

Send one JSON object per line. Every command produces exactly one `response` with your `id` echoed:

```json
{"id":"1","command":"prompt","message":"Summarize this repository"}
```

```json
{"type":"response","version":1,"id":"1","command":"prompt","success":true,"data":{"started":true,"session":"..."}}
```

Agent events follow the response and end with `agent_settled`, the completion signal. Event names and shapes are
the same as [`--mode json`](../../schemas/json-events.schema.json).

## Commands

| Command | Fields | Result |
|---|---|---|
| `prompt` | `message` | Starts a run, or queues a follow-up while one is active |
| `steer` | `message` | Adds a steering message to the active run |
| `follow_up` | `message` | Queues a message after the active run |
| `abort` | — | Cancels the active run |
| `get_state` | — | Model, thinking, session, activity, tokens |
| `get_messages` | — | Transcript messages, excluding the system prompt |
| `get_entries` | — | Session entry tree |
| `get_session_stats` | — | Counts, tokens, context, last usage |
| `new_session` | — | Starts a new session |
| `set_model` | `model`, optional `provider` | Changes the model; provider changes require a restart |
| `set_thinking` | `level` | Changes the reasoning level |
| `compact` | optional `instructions` | Compacts the context once |
| `export_html` | optional `path` | Writes a self-contained HTML transcript |
| `shutdown` | — | Cancels the run and exits after it settles |

Failures return `success: false` and a stable `error.code`, for example `no_active_run`, `run_active`,
`invalid_argument`, or `bad_request`.

## Shutdown

Closing stdin exits the process with code 0 after any active run settles. SIGTERM and SIGHUP cancel the run and
exit with 143 or 129. A second signal exits immediately.

## Schemas

The wire types live in `codie-protocol`. Published schemas are generated from those Rust types:

```bash
cargo xtask schemas           # write schemas/rpc-*.schema.json
cargo xtask schemas --check   # verify the checked-in copies are current
```

## Example client

[`examples/rpc-client`](../../examples/rpc-client) is a minimal Python integration:

```bash
python3 examples/rpc-client/rpc_client.py "Summarize this repository"
```
