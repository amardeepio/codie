# Interactive terminal

Phase 2 adds the primary human experience. Running `codie` with a terminal attached (and without `-p`,
`--mode json`, or piped input) starts the interactive interface.

## Modes

| Mode | Flag | Behavior |
|---|---|---|
| Full | `--tui-mode full` (default) | Alternate screen, scrolling transcript, fixed editor dock |
| Inline | `--tui-mode inline` | Same layout without entering the alternate screen |
| Plain | `--tui-mode plain` | Append-only text with no cursor movement (screen readers, logs) |

`-p/--print` and `--mode json` keep the Phase 1 headless behavior.

## Tool output

`edit` calls show a side-by-side diff with line numbers on both sides, three lines of context, and tinted
removed/added rows. `write` calls show the written file with line numbers. Both views are syntax-highlighted
and visible without expanding the block. Long bodies are capped at 80 rows; `Ctrl+O` toggles the full body.
Narrow terminals fall back to a single-column unified diff.

## Reasoning

Reasoning is collapsed by default to an animated `Thinking…` line while the model is still planning. Click that
line to expand the live reasoning stream, and press `Esc` (or click the header again) to collapse it. Once the
answer starts streaming, the collapsed form reads `Thought  (click to view)`.

## Markdown rendering

Assistant messages are rendered as Markdown in the transcript: headings are styled by level without `#`
markers, unordered lists use bullets, ordered lists keep their numbers and start value, and fenced code is
highlighted by language.

Tables render as width-aware grids with box-drawing borders, a header separator, and per-column alignment from
the `:---:` row:

```text
┌──────────┬──────────────────────────────────┐
│ Item     │ Spec                             │
├──────────┼──────────────────────────────────┤
│ Model    │ Mac mini (Mac16,10), MCYT4HN/A   │
└──────────┴──────────────────────────────────┘
```

When the terminal is too narrow for the grid, each row falls back to stacked `Key: value` lines. Cells that
still overflow their column truncate with `…`. Wide characters are measured by display width, so CJK and emoji
do not shift column borders.

## Keys

| Key | Action |
|---|---|
| `Enter` | Submit; while streaming, steer the current run |
| `Alt+Enter` | Queue a follow-up message |
| `Shift+Enter`, `Ctrl+J` | Insert a newline |
| `Ctrl+C` | Cancel the run; a second press quits |
| `Esc` | Stop the active run; closes open reasoning first; when idle, return to the bottom |
| `Ctrl+O` | Expand or collapse the last tool block |
| `Ctrl+T` | Pick a tool block from the transcript |
| `Ctrl+Shift+O` | Expand or collapse every tool block |
| `Ctrl+Shift+T` | Expand or collapse every user turn |
| Mouse click on a tool or turn header | Toggle that block or turn |
| Mouse click on `Thinking` | Show the live reasoning stream |
| Mouse click on the footer | Show usage details |
| `Shift+Tab` | Toggle full-access mode (no permission prompts) |
| `Ctrl+Z` / `Ctrl+Shift+Z` | Undo / redo |
| `Ctrl+K` / `Ctrl+Y` | Kill to end of line / yank |
| `Ctrl+P` / `Ctrl+N` | Prompt history |
| `PgUp` / `PgDn`, `Shift+Up/Down`, `Alt+Up/Down`, mouse wheel | Scroll the transcript |
| `Tab` | Complete a slash command or an `@file` path |

Keybindings can be overridden in `~/.config/codie/keybindings.toml`:

```toml
[keys]
submit = ["enter"]
newline = ["shift+enter", "ctrl+j"]
quit = []
```

An empty list unbinds the action; unknown actions and keys are rejected at startup.

## Stopping work

Press `Esc` or run `/stop` to stop the active model request or tool call. A running `bash` command is killed
together with its process group. Open reasoning is closed by the first `Esc`; pressing it again stops the run.
When nothing is running, `/stop` reports that there is nothing to stop and `Esc` returns the transcript to the
bottom. Stopping also drops steering and follow-up messages that have not reached the model yet.

## Context and usage

The footer shows a context meter such as `ctx ████████░░ 81%`. It turns amber as compaction approaches and red
at the compaction threshold. Click the footer or run `/usage` for a breakdown of the context estimate, input,
output, cache reads, cache writes, total tokens, cost, throughput, and session id.

Starting a new prompt folds older turns into one summary line. Click a turn header to expand it, or use
`Ctrl+Shift+T` to expand or collapse every turn.

## Tool navigation

Click any tool header to expand or collapse that tool. `Ctrl+O` toggles the newest tool, `Ctrl+Shift+O` toggles
every tool, and `Ctrl+T` opens a keyboard-selectable list of tool calls.

## Slash commands

Pressing `/` in an empty editor opens the command palette. Type to filter, use `Up`/`Down` or `PageUp`/`PageDown`
to scroll, and `Enter` to select. `Esc` closes it.

`/help`, `/hotkeys`, `/new`, `/session`, `/tree`, `/fork [entry]`, `/clone`, `/name [text]`, `/export [path]`,
`/export html [path]`, `/copy`, `/debug`, `/update`, `/theme [name]`, `/provider [id]`, `/model [id]`,
`/thinking [level]`, `/compact [instructions]`, `/usage`, `/stop`, `/quit`.

Extensions can add commands. Invoke one as `/ext:<extension>:<command> [arguments]`; a bare
`/<command>` also works when it is unambiguous and does not collide with a built-in. Extension commands
appear in the palette and completion list. See [Extensions](extensions.md).

The session picker shows the list on the left and a preview on the right with the name, model, message count,
tokens, cost, working directory, age, last prompt, and last reply.

`/update` checks for a newer release, installs it when a prebuilt archive exists for the platform, and shows a
banner with OK. When a tag has no published artifact, the banner explains how to reinstall with the installer
instead. The app also runs one silent background check at startup and only reports an available release; it
never installs automatically. Disable the background check with `CODIE_NO_UPDATE_CHECK=1`. `/export html` writes a self-contained transcript; `/export` keeps the canonical JSONL format. `/copy` puts
the last assistant message on the system clipboard. `/debug` prints version, model, cwd, session, and token
diagnostics as a transcript line.

`/theme`, `/provider`, `/model`, and `/thinking` without an argument open live pickers. The model picker lists
the coding models for the active provider; OpenRouter entries carry their real context window and price from the
catalog cache. `/provider` switches between DeepSeek and OpenRouter in place, selects the new provider's default
model, and refreshes the model list. Model selection updates the context window and is persisted; reasoning
selection is persisted with the session. The footer shows context usage and tokens per second, with a live
estimate while streaming.

## Permissions

Read-only tools run without prompting. `write`, `edit`, and `bash` ask before executing. The prompt previews the
operation: edits show a diff, new files show numbered code, and bash shows the command.

```text
[y] allow once   [A] always allow   [n] deny   [Enter] allow   [Esc] deny   [Shift+Tab] full access
```

`y`/`Y` allows once, `n`/`N` or `Esc` denies, `A` allows the tool for the rest of the run, and `Enter` allows.
`Shift+Tab` toggles full-access mode; the footer shows `FULL ACCESS` while it is on, and turning it on allows a
visible prompt immediately.

`--allow edit,bash` (or `CODIE_ALLOW`) pre-approves tools for the run. A denial becomes an error tool result and
the model can recover.

## Footer

The footer shows the working directory and git branch, a context meter, model, thinking level, full-access
state, session token total, cost, live throughput, and session id. Lower-priority fields drop out as the
terminal narrows. Token totals abbreviate thousands and millions: `999`, `1k`, `1.2k`, `70.2k`, `1.2M`.

Click the footer or run `/usage` for the full token and cost breakdown.

## Themes

`--theme system`, `--theme dark`, `--theme light`, or a TOML file with semantic tokens. The built-in
palettes follow OpenCode's default theme: a near-black canvas, warm peach focus, purple headings, green code,
and distinct colors for emphasis, quotes, and lists.

```toml
background = "#0a0a0a"
surface = "#141414"
foreground = "#eeeeee"
muted = "#808080"
primary = "#fab283"
secondary = "#5c9cf5"
accent = "#9d7cd8"
heading = "#9d7cd8"
link = "#fab283"
code = "#7fd88f"
strong = "#f5a742"
emphasis = "#e5c07b"
quote = "#e5c07b"
list = "#fab283"
list_enum = "#56b6c2"
rule = "#808080"
tool = "#5c9cf5"
error = "#e06c75"
success = "#7fd88f"
warning = "#f5a742"
info = "#56b6c2"
diff_add = "#4fd6be"
diff_del = "#c53b53"
border = "#484848"
border_active = "#606060"
border_subtle = "#3c3c3c"
```

Newer tokens are optional. `primary` and `heading` fall back to `accent` when it is set; `link` and `list`
fall back to `primary`; `code` falls back to `success`; `strong`, `emphasis`, and `quote` fall back to
`warning`; `list_enum` falls back to `info`; and `rule` falls back to `muted`.

Custom themes must meet a 4.5:1 body contrast ratio. A theme file reloads within a second of being edited.
`system` follows `COLORFGBG`, with `CODIE_THEME` as an override.

## Performance

`cargo bench -p codie-tui --bench streaming` simulates 5,000 tokens over 60 frames. The accepted baseline is
under 40 ms total and about 0.5 ms per frame, well inside the 16.7 ms frame budget.
