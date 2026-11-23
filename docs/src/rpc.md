# RPC mode

`codie --mode rpc` is a long-lived, line-delimited JSON interface for editor integrations, desktop clients, and
scripts. It reads commands on stdin and writes records on stdout; diagnostics go to stderr.

```bash
codie --mode rpc --provider deepseek
```

The first two records are the version handshake and the session header:

```json
{"type":"rpc_ready","version":2,"features":["permissions","auth","catalog","config","attachments"]}
{"type":"session","version":1,"id":"...","timestamp":"...","cwd":"/path/to/project"}
```

Send one JSON object per line. Every command produces exactly one `response` with your `id` echoed:

```json
{"id":"1","command":"prompt","message":"Summarize this repository"}
```

```json
{"type":"response","version":2,"id":"1","command":"prompt","success":true,"data":{"started":true,"session":"..."}}
```

Agent events follow the response and end with `agent_settled`, the completion signal. Event names and shapes are
the same as [`--mode json`](../../schemas/json-events.schema.json).

## Commands

| Command | Fields | Result |
|---|---|---|
| `prompt` | `message`, optional `attachments` | Starts a run, or queues a follow-up while one is active. Extension commands such as `/ext:todo:todo` are expanded first. |
| `steer` | `message` | Adds a steering message to the active run |
| `follow_up` | `message` | Queues a message after the active run |
| `abort` | — | Cancels the active run |
| `set_permission_mode` | `mode`: `headless`, `ask`, `allow` | Changes how mutating tools are authorized |
| `permission_response` | `request_id`, `decision` | Answers a pending `permission_request` |
| `get_state` | — | Model, thinking, session, activity, tokens |
| `get_catalog` | — | Selectable models, providers, and the current selection |
| `get_themes` | — | Built-in palettes as semantic hex colors |
| `get_config` | — | Effective configuration and paths, without secrets |
| `set_config` | `key`, `value` | Writes one `config.toml` value; reports `restart_required` |
| `auth_status` | — | Credential state for every provider, without keys |
| `auth_set_key` | `provider`, `key`, optional `verify` | Stores a key for one provider |
| `auth_logout` | `provider` | Removes a stored key |
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
`invalid_argument`, `bad_request`, `unknown_permission`, `unknown_provider`, `no_key_required`, `invalid_key`, or
`auth_failed`.

## Permissions

Version 2 defaults to `headless`: mutating tools are denied unless they were pre-approved with `--allow` or
configuration. A GUI opts into prompts:

```json
{"command":"set_permission_mode","mode":"ask"}
```

Each mutating call that is not read-only and not pre-approved then emits a request:

```json
{"type":"permission_request","version":2,"id":"perm-1","tool_call_id":"call_7","tool":"edit","arguments":{"path":"src/main.rs","edits":[...]}}
```

Answer with `allow_once`, `allow_always`, or `deny`:

```json
{"id":"answer","command":"permission_response","request_id":"perm-1","decision":"allow_once"}
```

`allow_always` covers the tool for the rest of the session. Aborting, shutting down, closing stdin, or dropping the
answer denies every pending request. `mode: "allow"` approves everything without asking, matching the TUI's
full-access mode; use it only with a deliberate user action.

## Credentials

`auth_set_key` is the only way to supply a key without putting it in `argv` or a terminal prompt. The key travels
over the private stdio pipe, is written atomically to `auth.json` with mode `0600`, and never appears in a
response or log. `verify: true` performs one provider probe before storing; a throttled probe still stores the key.

```json
{"command":"auth_set_key","provider":"deepseek","key":"sk-...","verify":true}
{"type":"response","command":"auth_set_key","success":true,"data":{"provider":"deepseek","configured":true,"verified":true,"restart_required":true}}
```

`restart_required` is true when the stored provider is the one the process is already using; start a new process to
pick up a different key.

## Themes

`get_themes` returns every built-in palette except `system`, with the 29 semantic color names used by the TUI
theme spec:

```json
{"type":"response","command":"get_themes","success":true,"data":{"themes":[{"name":"dark","colors":{"background":"#0a0a0a","foreground":"#eeeeee","primary":"#fab283"}}]}}
```

`system` is resolved by the client from its appearance setting. User theme files are not exposed over RPC yet.

## Configuration

`get_config` reports the effective values and the paths (`config_path`, `auth_path`, `session_dir`) without
secrets. `set_config` accepts the same keys as `codie config set`: `provider`, `model`, `thinking`, `max-turns`,
`allow`, `base-url`, and `extensions-disabled`. Thinking, max turns, and the allow list apply immediately;
`allow` replaces the configured pre-approval list while `allow_always` grants from the current session survive.
Provider, model, base URL, and extension changes report `restart_required: true`.

## Attachments

```json
{
  "command": "prompt",
  "message": "review this",
  "attachments": [
    {"type": "path", "path": "src/main.rs"},
    {"type": "image", "data": "<base64>", "mime_type": "image/png", "name": "clipboard.png"}
  ]
}
```

Path attachments expand exactly like `@path` references. Inline images must be base64 with an `image/*` MIME type.
At most 20 attachments and 10 MiB per file are accepted; rejected entries appear in the prompt response's
`warnings` and do not fail the run.

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
