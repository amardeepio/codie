# Quickstart

This page is the shortest path from a clean machine to a working session.

## Install

### macOS and Linux

```bash
curl -fsSL https://raw.githubusercontent.com/amardeepio/codie/main/install.sh | sh
```

### Windows x64 (preview)

Open PowerShell and run:

```powershell
irm https://raw.githubusercontent.com/amardeepio/codie/main/install.ps1 | iex
```

Pin the release candidate explicitly:

```powershell
$env:CODIE_VERSION = "0.9.0-rc.1"
irm https://raw.githubusercontent.com/amardeepio/codie/main/install.ps1 | iex
```

The installer writes to `%LOCALAPPDATA%\Programs\codie` and adds that directory to your user `PATH`.
Open a new terminal afterwards.

See [Installation](install.md) for checksums, custom directories, source builds, and uninstall steps.

## Authenticate

```bash
codie auth login deepseek
# or
codie auth login openrouter
codie auth status
```

Credentials are stored under your data directory and never written to session files or child processes.

## Run one prompt

```bash
codie -p --provider deepseek --model deepseek-flash \
  "Reply with exactly: codie-ok"
```

Headless mode denies mutating tools by default. Allow them explicitly:

```bash
codie -p --allow-all --cwd /path/to/project \
  "Make the failing test pass, then run it"
```

## Start the interactive app

```bash
cd /path/to/project
codie
```

Press `/` for the command palette, `Ctrl+C` to stop a run, and `Ctrl+D` to quit. See
[Interactive terminal](tui.md) for keys, themes, and permissions.

## Automation

```bash
codie --mode json -p "Summarize this repository"
codie --mode rpc          # line-delimited JSON on stdin/stdout
codie sessions list
codie update --check
```

## Next steps

- [Providers](providers.md) — supported providers, models, and authentication.
- [Sessions](sessions.md) — resume, fork, compact, export.
- [Configuration and environment](configuration.md) — config files and environment variables.
- [Troubleshooting](troubleshooting.md) — when something does not work.
