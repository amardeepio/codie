# Terminal compatibility

codie targets macOS terminals as its tier-1 surface and treats Windows Terminal as a preview. Universal
behavior comes from `ratatui` and `crossterm`: raw mode, alternate screen, bracketed paste, mouse capture,
resize handling, and terminal restore.

## Matrix

| Terminal | Print/JSON/RPC | Interactive TUI | Truecolor | 256-color | Option-as-Meta | Clipboard |
|---|---|---|---|---|---|---|
| Terminal.app | Yes | Yes | No | Yes | Terminal setting | `pbcopy` |
| iTerm2 | Yes | Yes | Yes | Yes | Profile setting | `pbcopy` |
| Ghostty | Yes | Yes | Yes | Yes | Usually on | `pbcopy` |
| kitty | Yes | Yes | Yes | Yes | Usually on | `pbcopy` |
| WezTerm | Yes | Yes | Yes | Yes | Usually on | `pbcopy` |
| VS Code terminal | Yes | Yes | Yes | Yes | Setting | `pbcopy` |
| Windows Terminal | Yes | Yes | Yes | Yes | Not applicable | `clip` |
| Linux terminal | Yes | Yes | Depends | Yes | Usually on | `wl-copy` / `xclip` |

“Preview” means the Windows path builds and the core tools work, but the full terminal matrix is not yet part of
the release gate.

## Option-as-Meta

codie binds `Alt` combinations such as `Alt+Enter` for follow-up messages. macOS terminals do not send an
`Alt` modifier for Option by default; they either insert a special character or send nothing.

Enable Option-as-Meta in your terminal:

- **Terminal.app:** Settings → Profiles → Keyboard → *Use Option as Meta key*.
- **iTerm2:** Profiles → Keys → Left/Right Option key → *Esc+*.
- **VS Code:** Terminal → Integrated → *Mac Option Is Meta*.
- **Ghostty / kitty / WezTerm:** usually enabled by default; check the terminal's option/meta setting.

If you cannot enable it, use `Ctrl+J` to insert a newline instead of `Shift+Enter`.

## Colors

The themes emit RGB colors. Terminals without truecolor support approximate them; pick `--theme dark` or
`--theme light` if the system theme detects the wrong background. `NO_COLOR` and `TERM=dumb` are not yet
special-cased in this release.

## Bracketed paste

Large pastes collapse into a marker such as `[paste #1: 12000 chars]` and expand when the prompt is submitted.
The marker is intentional: it keeps the editor responsive and makes accidental giant pastes visible.

## Resize

The TUI redraws on a 33 ms tick while streaming, and the layout adapts to width. If a resize leaves stale
content, force a redraw by sending a prompt or pressing `Ctrl+L` (clear editor).

## Manual checklist

Run this before a release on each terminal:

1. `codie --version` prints the version.
2. `codie -p "Reply with exactly: ok"` streams and exits 0.
3. `codie --mode json -p "Reply with exactly: ok"` emits JSONL.
4. `codie` opens the TUI, the banner renders, and the editor accepts input.
5. `/help` and `/` (command palette) render.
6. A long reply scrolls, and `Esc` stops an active run.
7. Paste a multi-line block and submit it.
8. Resize the window while streaming; the layout reflows.
9. `Alt+Enter` queues a follow-up when Option-as-Meta is enabled.
10. `/copy` reports that the message was copied.
11. Exit with `Ctrl+D`; the cursor and alternate screen are restored.
12. Send `Ctrl+C` during a run; codie stops and restores the terminal.

## Reporting a terminal bug

Include the terminal name and version, `TERM`, `TERM_PROGRAM`, `COLORTERM`, the codie version, and a screenshot
or recording if rendering is involved.
