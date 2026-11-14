# Troubleshooting

Start with the version and platform information:

```bash
codie --version
codie config path
```

Inside the interactive app, `/debug` prints the model, session, context usage, theme, and permission mode.

## Installation and PATH

### `codie: command not found`

`~/.local/bin` is not on your `PATH`. Add it to your shell startup file:

```bash
export PATH="$HOME/.local/bin:$PATH"
```

On Windows the installer uses `%LOCALAPPDATA%\Programs\codie`. Open a new terminal, or for the current window:

```powershell
$env:Path = "$env:LOCALAPPDATA\Programs\codie;$env:Path"
```

### Checksum verification failed

The downloaded archive did not match the published checksum. The installer aborts before writing a binary.
Retry; if it persists, report the URL and the expected/actual hash.

### The installer cannot find an archive for the version

Tagged releases publish archives for macOS arm64/x64, Linux arm64/x64, and Windows x64. A tag without artifacts
falls back to a source build on macOS and Linux. Windows requires a published `.zip`; pin a version that has one.

## Authentication and providers

### `auth status` exits 2

No provider is configured. Run:

```bash
codie auth login deepseek
# or
codie auth login openrouter
```

### `401` or `403` from a provider

Check that the key belongs to the selected provider. `codie auth status` shows which providers have stored
credentials. Environment variables (`DEEPSEEK_API_KEY`, `OPENROUTER_API_KEY`) take precedence over stored
credentials; an empty environment variable can shadow a valid stored key in some shells.

### `429` or repeated retries

codie honors `retry-after` and backs off. A server-requested delay above the configured cap fails fast instead
of waiting indefinitely. Reduce concurrent runs or switch models.

### TLS or proxy failures on Windows

The Windows preview uses bundled WebPKI roots. Corporate TLS interception may require a proxy configuration or
a custom build. Set the standard proxy environment variables for your network, and confirm that
`https://api.github.com` and your provider endpoint are reachable.

## Interactive terminal

### Garbled screen or missing cursor

Exit with `Ctrl+D`; codie restores the terminal on normal exit, panic, and POSIX signals. If a crashed terminal
is still broken:

```bash
reset
```

### Option-as-Meta does not work

macOS terminals do not send `Alt` for Option unless configured:

- **Terminal.app:** Settings → Profiles → Keyboard → *Use Option as Meta key*.
- **iTerm2:** Profiles → Keys → Left/Right Option key → *Esc+*.
- **Ghostty, kitty, WezTerm:** Option-as-Meta is usually enabled by default.
- **VS Code terminal:** enable the terminal's macOS Option-as-Meta setting.

`Ctrl+J` inserts a newline without Option, and `Alt+Enter` queues a follow-up only when Meta is available.

### Colors look wrong

codie emits truecolor or 256-color output depending on the terminal. If colors are clipped or unreadable, pick
an explicit theme:

```bash
codie --theme dark
codie --theme light
```

### Clipboard does not work

macOS uses `pbcopy`, Linux tries `wl-copy` then `xclip`, and Windows uses `clip`. Over SSH, the remote machine
may not have a clipboard command. Install one, or copy the relevant text from the terminal instead.

## Sessions

### `session is empty` when resuming

The session file exists but has no entries. Start a new session or choose another entry from the picker.

### A session is in use by another codie process

This preview does not lock session files across processes. Do not run two interactive sessions against the
same session file. Start a new session or close the other process.

### `codie sessions` shows nothing

The picker only lists sessions for the current working directory. Run it from the project directory, or pass
`--session-dir` where supported.

## Updates

### `codie update --check` does not offer a pre-release

Pre-releases are marked as such, so GitHub's “latest release” endpoint excludes them. Install a pre-release
explicitly with `CODIE_VERSION` or the platform installer.

### Update fails while codie is running

On Windows the running executable can block replacement. Close codie and re-run `install.ps1`. On macOS and
Linux, rerun `install.sh` if the in-place update fails.

## Crash reports

A panic writes `crash-<timestamp>.txt` under your data directory (`~/.local/share/codie/crashes` by default)
and prints the path to stderr. Reports contain the version, OS, architecture, panic message, location, and a
backtrace. They never contain environment variables, credentials, or transcript content. Review the file before
attaching it to an issue.

## Reporting a bug

Include:

- `codie --version`
- operating system and architecture
- the command or TUI action that failed
- relevant stderr, with secrets redacted
- whether the failure reproduces with `--no-session`

Use <https://github.com/amardeepio/codie/issues>.
