# Subagents

A subagent is a child codie run started by the `task` tool. It has its own transcript, model, turn budget,
cancellation token, tool subset, and optional session file. The parent sees one tool result, not the child's
conversation.

Delegation helps when work splits into independent pieces — search three modules, review two packages, summarize
several logs. It does not make a single-file edit faster: children share the working directory, and the per-path
lock serializes writes.

The normative behavior is in [`SPEC-AGENT-003`](../../specs/agent/0003-subagent-delegation.md).

## Enable it

In the interactive app, `task` is always declared, so you can enable and disable delegation at runtime without
editing configuration or restarting:

```text
/subagents on
```

`[subagents] enabled` in `config.toml` only sets the **initial** mode for a new session. If it is `false`, the
session starts with the tool present but in `off` mode; `/subagents on` enables it immediately.

```toml
[subagents]
enabled = true
max_parallel = 4
max_depth = 0
```

For a one-shot run, the flag enables it without configuration:

```bash
codie -p --subagents auto --allow task "use subagents to audit the provider adapters"
```

In the interactive app, use the slash command:

| Command | Effect |
|---|---|
| `/subagents` | Show mode, parallelism, budgets, and agent types |
| `/subagents on` | `auto`: the model decides |
| `/subagents off` | Refuse `task` calls for this session |
| `/subagents force` | Keep telling the model to delegate |
| `/subagents once` | Force delegation for the next prompt only |
| `/subagents parallel N` | Set live parallelism; refused while children run or queue |
| `/subagents types` | List every type with its description, tools, and budgets |
| `/subagents status` | Status block with live and finished runs |
| `/subagents open N` | Open run `N`'s read-only child transcript |
| `/subagents stop N` | Cancel run `N` without stopping the parent |

You can also ask in plain language:

```text
use subagents to compare the three retry paths, then summarize the differences
```

The host detects the request deterministically and applies it to that run. `make it faster` on its own is not a
delegation request. `don't use subagents` always wins.

## Agent types

| Type | Tools | Use |
|---|---|---|
| `general` | Inherits the parent's permitted tools | Independent implementation or review work |
| `explore` | `read`, `grep`, `find`, `ls`, `web_fetch` | Parallel search and reading |

Projects can add personas without rebuilding the binary. A markdown file in `.codie/agents/` (or
`<config_dir>/agents/`) becomes a type:

```markdown
---
description: Reviews rendered interfaces and accessibility
tools: read, grep, bash
max-turns: 12
timeout-secs: 120
---

You review interfaces for contrast, focus order, empty states, and narrow viewports.
Report `file:line` evidence and do not edit files.
```

The file name is the type name. `description` is what the model sees when choosing a type, and the body is
appended to the child's system prompt. Omitted fields keep the value from the built-in or configured type, so
overriding `explore` without a `tools` line stays read-only. Project definitions override user definitions with
the same name; `[subagents.types.<name>]` in `config.toml` still works and can override a file. Run `/reload`
after editing a persona; the new types apply without a restart.

A child can never use a tool the parent policy denies. In headless mode, `task` itself needs `--allow task`:

```bash
codie -p --allow task --subagents auto "delegate the audit to two explore agents"
```

In the interactive app, `task` prompts like any other non-read-only tool. `--allow-all` covers it.

## Configuration

```toml
[subagents]
enabled = true
max_parallel = 4          # 1-8 concurrent children
max_depth = 0             # 0 = children cannot delegate further
max_turns = 50            # per child
timeout_secs = 600        # per child
persist_sessions = true
result_max_bytes = 20000
max_total_tokens = 200000
max_total_cost_usd = 2.00

[subagents.types.explore]
model = "deepseek-chat"
tools = ["read", "grep", "find", "ls", "web_fetch"]
description = "Read-only search"
max_turns = 20
timeout_secs = 120

[subagents.types.ui-reviewer]
tools = ["read", "grep", "bash"]
description = "Reviews rendered interfaces"
prompt = "Check contrast, focus order, empty states, and narrow viewports."
max_turns = 12

[subagents.types.general]
tools = "inherit"
```

`max_parallel` bounds the process, not one batch. Aggregate token and cost caps apply to all children in the
process; when a cap is reached, queued children return a budget error. A child that times out, hits its turn
limit, or fails returns an error tool result; the parent continues.

## Sessions

A persisted child session is a normal session file with `parent_session` set to the parent id and a `custom`
entry of type `subagent`. The `task` result includes the child session id, so you can inspect it with
`codie --session <id>` or the session picker. The child session is not a fork: it starts empty and contains only
the child prompt and its own turns.

Set `persist_sessions = false` for throwaway work; the result reports `persisted = false`.

## Progress and stopping

Child progress reaches the parent as throttled tool updates (at most once per second per child). The `task` row
shows the latest activity, for example:

```text
⠋ Task · general · News feeds: US market selloff   turn 2 · grep markets  1m 51s
✓ Task · general · News feeds: US market selloff   completed · 3 turns · $0.0123 · ses_019f…
```

While the pool is full, extra calls show as `queued · waiting for a slot` and `/subagents stop N` works before
they start. A child that fails reports the provider's message in the result instead of only `error`, so a bad
model id or a rate limit is visible in the row and in `/subagents status`.

A child that runs out of turns still hands off: the loop makes one final request with tools disabled and uses
that text as the summary. A timeout returns the partial summary it already collected with an `error` naming the
limit. Summaries are labelled as untrusted data, and a long one keeps both the beginning and the conclusion.
Completed child usage is added to the parent footer and recorded as a `subagent` usage entry, so `/usage` and
session cost include delegated work.

When delegation is forced and the first run does not call `task`, the host asks once more, then stops asking.

Child messages never appear in the parent transcript. `/subagents status` numbers every run, `/subagents open N`
shows that child's transcript read-only (refreshing once a second while it runs), and `/subagents stop N` cancels
one child. `/stop` or `Ctrl+C` cancels the parent and every running child. Switching `/model` or `/provider`
updates the model children inherit for later calls.

A child can ask a question instead of guessing. The prompt names the asking persona when the call comes from a
delegated run, and "Always allow" applies only to that persona:

```text
? question  Which breakpoint matters for the header?
> 768px▌
Enter send · Esc dismiss (the model continues without an answer)
```

## Limitations

- Children run in the parent process and share its working directory; there is no sandbox or worktree isolation.
- Extension loop hooks do not run inside children in this release. Extension tools remain available.
- `max_depth` defaults to `0`. Grandchildren require an explicit higher value and a deliberate cost review.
- Delegation mode is advisory. `/subagents force` adds an instruction and a one-shot follow-up; it never
  fabricates a tool call.
