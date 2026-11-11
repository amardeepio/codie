# Sessions

A session is an append-only JSONL tree under the per-project session directory. Phase 3 adds resume on any
branch, fork, clone, export, and compaction. The normative format is
[`SPEC-SESS-001`](../../specs/session/0001-jsonl-format.md); tree operations are
[`SPEC-SESS-002`](../../specs/session/0002-tree-operations.md).

## Starting and resuming

| Command | Behavior |
|---|---|
| `codie` | Start a fresh session |
| `codie --continue` / `-c` | Resume the newest session for this directory |
| `codie --resume` / `-r` | Open the interactive session picker |
| `codie --resume <id>` | Resume a session by identifier or unique prefix |
| `codie --session <id\|path>` | Open a specific session file or identifier |
| `codie --fork <id\|path>` | Copy the source branch into a new session here |
| `codie --clone <id\|path>` | Copy the whole source tree into a new session here |
| `codie --no-session` | Run without persistence |

Session identifiers are stable and sortable; a unique prefix is accepted. An ambiguous prefix is refused rather
than guessed. Fork and clone set `parent_session` on the new header and never modify the source.

## Inspecting and exporting

```bash
codie sessions list                # newest first, for the current directory
codie sessions list --json
codie sessions show <id> --tree    # metadata plus the entry tree
codie sessions export <id>         # JSONL to stdout
codie sessions export <id> -o out.jsonl
```

These commands are read-only.

## Pickers and slash commands

`--resume` with no identifier opens a picker scoped to the current directory. Type to filter, `s` toggles
recency/name sorting, `r` renames, `d` then `y` deletes, and `Enter` resumes.

Inside the interactive app:

| Command | Behavior |
|---|---|
| `/tree` | Show entries with branch markers, depth, and a preview |
| `/fork <entry>` | Move the leaf to an entry and continue from there |
| `/fork` | Show the tree and a hint |
| `/clone` | Copy the current session and switch to the copy |
| `/name [text]` | Set or clear the session name |
| `/export [path]` | Write canonical JSONL (default `codie-session-<id>.jsonl`) |
| `/compact [instructions]` | Summarize older context and continue |

## Compaction

`/compact` asks the active model for a fixed-structure summary (Goal, Constraints, Progress, Key Decisions, Next
Steps, Critical Context, read/modified files). The summary is appended as a `compaction` entry; earlier lines
stay in the file. The projection keeps the system message, inserts the summary, and retains the recent tail
within a 20,000-token budget without starting at a tool result.

Compaction also runs automatically before a request when the estimated history plus prompt exceeds the model's
context window minus a 16,384-token reserve. If a provider still reports a context-overflow error, codie hides
the failed message with a context edit, compacts, and retries once with the same transcript. Both interactive and
headless runs use this path.

Token estimates use `chars / 4` (images count as 4,800 characters) and are shown as a context percentage in the
footer. Estimates are never used for billing.
