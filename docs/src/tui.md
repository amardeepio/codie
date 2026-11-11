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

## Keys

| Key | Action |
|---|---|
| `Enter` | Submit; while streaming, steer the current run |
| `Alt+Enter` | Queue a follow-up message |
| `Shift+Enter`, `Ctrl+J` | Insert a newline |
| `Ctrl+C` | Cancel the run; a second press quits |
| `Ctrl+O` | Expand or collapse the last tool block |
| `Ctrl+Z` / `Ctrl+Shift+Z` | Undo / redo |
| `Ctrl+K` / `Ctrl+Y` | Kill to end of line / yank |
| `Ctrl+P` / `Ctrl+N` | Prompt history |
| `PgUp` / `PgDn`, `Esc` | Scroll transcript, return to the bottom |
| `Tab` | Complete a slash command |

## Slash commands

`/help`, `/hotkeys`, `/new`, `/session`, `/model [id]`, `/thinking [level]`, `/compact`, `/quit`.

`/model` and `/thinking` apply to the next run. `/compact` is a placeholder until Phase 3.

## Permissions

Read-only tools run without prompting. `write`, `edit`, and `bash` ask before executing:

```text
[a] allow once   [A] always   [d] deny
```

`--allow edit,bash` (or `CODIE_ALLOW`) pre-approves tools for the run. A denial becomes an error tool result and
the model can recover.

## Themes

`--theme system`, `--theme dark`, `--theme light`, or a TOML file with semantic tokens:

```toml
background = "#121216"
foreground = "#e2e2e8"
accent = "#7aa2f7"
tool = "#bb9af7"
error = "#f7768e"
```

Custom themes must meet a 4.5:1 body contrast ratio. `system` follows `COLORFGBG`, with `CODIE_THEME` as an
override.
