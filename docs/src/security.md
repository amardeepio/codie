# Security

codie executes model-proposed commands on the user's machine. The security posture is therefore part of the
product, not an afterthought: destructive operations are gated by an explicit policy, secrets are redacted, and
project-local resources are untrusted until approved.

## Permission defaults

The confirmed default (D-6) is:

- **Interactive:** read-only tools (`read`, `grep`, `find`, `ls`) are allowed; `write`, `edit`, `bash`, and
  `web_fetch` ask.
- **Headless, print, and JSON modes:** read-only tools are allowed; mutating and network tools are denied unless
  `--allow` or configuration explicitly permits them. `web_fetch` is network egress and requires
  `--allow web_fetch` or `--allow-all`.
- **Policy decisions** are made per tool call, match on tool name plus normalized arguments (command prefix,
  path glob), and are logged with the reason.
- **Project trust:** `.codie/` resources load only after an explicit trust decision. Non-interactive runs default
  to untrusted; `--approve` records the canonical project path in `trust.toml`. A missing or malformed trust file
  grants nothing.
- **Network:** `web_fetch` refuses non-HTTP(S) schemes, loopback, private, link-local, and unspecified addresses,
  re-validates every redirect, and caps response size and time. It sends no credentials or cookies.

The policy engine is the control point. Paths are not sandboxed by default; users who need filesystem
isolation should run codie in a container or VM, and the documentation provides patterns for that.

## Secrets

- API keys are resolved from `--api-key`, the stored credential, or the provider environment variable.
- `auth.json` is mode `0600`, written atomically (temp, fsync, rename), and never committed.
- Keys are redacted from logs, error messages, JSON events, exports, and child environments.
- `bash` children do not inherit `*_API_KEY` or `CODIE_*TOKEN*` variables unless the user opts in.
- Provider request and response bodies are logged only at `trace` level with `Authorization` redacted.
- Recorded fixtures require an explicit `--record` feature and environment opt-in; CI never records.

## Threat model

In scope:

- Accidental destructive tool execution.
- Secret leakage through logs, child processes, session files, or exports.
- Untrusted project resources (`.codie/`, `AGENTS.md`, skills) influencing the agent.
- Supply-chain compromise of dependencies or release artifacts.
- Malicious extension processes.

Out of scope, with documented guidance:

- OS-level sandboxing. Use containers or VMs; codie does not claim to be a sandbox.
- Prompt injection from repository content. Mitigations exist, guarantees do not.
- A user who intentionally runs with an allow-all policy on an untrusted repository.

## Supply chain and releases

Dependencies are locked and audited with `cargo deny` (including RustSec advisories), `cargo audit` locally,
and `cargo machete`. The workspace forbids
`unsafe` outside audited platform shims. Release artifacts are signed, accompanied by SHA256 checksums and an
SBOM, and built from the pinned toolchain. Coordinated disclosure uses private GitHub advisories and the
security contact documented in `SECURITY.md`, with a 90-day window.

## Privacy

There is no telemetry by default. Session files are local. Provider requests go only to the configured base URL,
and a diagnostic command prints the effective endpoints. Exports warn when output may contain secrets.

## Extensions

Extensions are separate processes speaking JSON-RPC over stdio. They cannot mutate the transcript directly,
must go through the permission policy for tool calls, and inherit no secrets by default. A hung or crashing
extension is isolated and killed without hanging the agent.

The normative permission requirements are `SEC-001`..`SEC-008` in [`PLAN.MD`](../../PLAN.MD) §6.13 and will be
specified in `specs/security/0001-permissions.md`. See [`docs/rfc/`](../rfc/) for changes to released behavior.
