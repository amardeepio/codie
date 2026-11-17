---
id: SPEC-TUI-008
title: Clipboard image paste and path-safe palette
status: accepted
version: 1.0.0
owners: ["@codie-maintainers"]
requirements: ["TUI-031", "TUI-032"]
---

# Context

Terminals only forward text. Copying a screenshot puts an image on the system clipboard and sends no bytes to the
application, so an agent has to read the clipboard itself. The same flow breaks in the command palette: the first
`/` of an absolute path opens the palette and the rest of the path is consumed as a filter.

# Normative behavior

**TUI-031 (MUST)** Ctrl+V is bound to the `paste_image` action (overridable through `[keys]` in `keybindings.toml`).
The action reads an image from the system clipboard, saves it as a PNG under the codie paste directory in the
temporary directory, and inserts an attachment token for it at the cursor. When the clipboard holds no image, the
action inserts clipboard text instead; when it holds neither, it reports that the clipboard is empty. Pasted images
older than one day are removed. On macOS, PNG and TIFF clipboard flavours are supported (`TIFF` is converted with
`sips`). Dragging a screenshot into the terminal, which inserts an escaped path one character at a time, converts to
the same attachment as soon as the path resolves, before the ephemeral file disappears. Attachments appear in the
editor as compact labels (`[Image 1]`, `[File 1]`) rather than full paths, and each label is replaced by the
canonical `@path` token when the prompt is submitted. Repeating the same path reuses its label; pressing Ctrl+C
clears the labels; submitting clears them after expansion.
Verification: `crates/codie-tui/src/app.rs::spec_tui_031_clipboard_paste_inserts_an_attachment_token`

**TUI-032 (MUST)** A command-palette query that contains `/` or backslash, and no `:`, is treated as a filesystem
path rather than a filter: the palette closes and the text returns to the editor with the leading `/` restored. A
bracketed paste of an existing file path becomes an attachment token instead. Command arguments that contain `:`
(`skill:review`, `template:fix`) stay in the palette. Submitting a whole prompt that is an existing file path
attaches the file even when every character arrived as a separate key event. A path-like submission whose file is
gone never reports `Unknown command`: codie attaches the clipboard image when one is available, and otherwise
prints an attachment error that explains how to retry.
Verification: `crates/codie-tui/src/app.rs::spec_tui_032_paths_close_the_command_palette`

# Data shapes

```text
Ctrl+V with an image  ->  @"\/tmp\/codie-paste/paste-1763000000000-4242.png"
Ctrl+V with text      ->  inserted like a bracketed paste
Ctrl+V with nothing   ->  notice: Clipboard has no image or text.
```

`/` at an empty editor opens the palette; typing `var/folders` closes it and restores `/var/folders` to the editor.

# Edge cases

- The paste directory name contains no spaces, so the inserted token is unquoted in practice; quoting still applies
  when a backend returns a path with spaces.
- `pngpaste` is preferred on macOS when installed; otherwise AppleScript writes the clipboard flavour and `sips`
  converts TIFF.
- Linux uses `wl-paste` on Wayland and `xclip` on X11; Windows uses PowerShell and `System.Drawing`.
- Clipboard failures are silent apart from the empty-clipboard notice; the editor is never left in a partial state.
- Plain mode has no image paste; it remains line-oriented.
