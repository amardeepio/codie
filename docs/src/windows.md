# Windows (preview)

`0.9.0` ships a Windows x64 preview. It is not a tier-1 platform yet, but the core agent works and the
binary installs with one PowerShell command.

## Requirements

- Windows 10 21H2 or newer, or Windows 11, on x64.
- PowerShell 5.1 or newer (PowerShell 7 is also supported).
- Outbound HTTPS access to GitHub Releases.
- ARM64 Windows is not supported in this preview.

## Install

```powershell
irm https://raw.githubusercontent.com/amardeepio/codie/main/install.ps1 | iex
```

The installer:

1. Resolves the newest published release, including release candidates.
2. Downloads `codie-v<version>-x86_64-pc-windows-msvc.zip`.
3. Verifies the SHA-256 checksum.
4. Extracts `codie.exe` to `%LOCALAPPDATA%\Programs\codie`.
5. Adds that directory to your user `PATH`.
6. Runs `codie --version`.

It never asks for administrator rights and never writes machine-wide settings.

### Pin a version

```powershell
$env:CODIE_VERSION = "0.9.0"
irm https://raw.githubusercontent.com/amardeepio/codie/main/install.ps1 | iex
```

Release candidates are published as pre-releases. If you do not pin a version, the installer follows the newest
published release, which may be a candidate during the `0.9.0` cycle.

### Choose another directory

```powershell
$env:CODIE_INSTALL_DIR = "C:\Tools\codie"
irm https://raw.githubusercontent.com/amardeepio/codie/main/install.ps1 | iex
```

### Skip the PATH update

```powershell
$env:CODIE_NO_PATH = "1"
irm https://raw.githubusercontent.com/amardeepio/codie/main/install.ps1 | iex
```

### Inspect before running

```powershell
irm https://raw.githubusercontent.com/amardeepio/codie/main/install.ps1 -OutFile install.ps1
notepad install.ps1
.\install.ps1
```

## What works

| Area | Status |
|---|---|
| `read`, `write`, `edit` | Works |
| `grep`, `find`, `ls` | Works |
| `web_fetch` | Works |
| `powershell` tool | Works; prefers `pwsh.exe`, falls back to `powershell.exe` |
| Interactive TUI | Works in Windows Terminal and modern conhost |
| Sessions, resume, fork, compaction | Works |
| RPC and JSON modes | Works |
| Extensions | Works when Python or Node is installed |
| Clipboard copy | Works through `clip` |

## Known gaps

- The POSIX `bash` tool is not available. Use the `powershell` tool instead.
- Process-tree termination uses `taskkill /T /F`; Windows Job Objects are post-1.0 work.
- CRLF preservation in `read`/`edit`/`write` is not guaranteed by this preview.
- There is no ConPTY test matrix yet.
- Windows ARM64 is not built.
- Paths use the usual Windows rules; UNC and long-path handling are post-1.0 work.
- `codie update` may report that an in-place replacement failed while the binary is running. Re-run the
  installer instead.

## Uninstall

```powershell
Remove-Item -Recurse -Force "$env:LOCALAPPDATA\Programs\codie"
```

Remove `%USERPROFILE%\.config\codie` and `%USERPROFILE%\.local\share\codie` only if you also want to delete
configuration, credentials, sessions, and cache.

## Troubleshooting

### `irm` is not recognized

You are not in PowerShell. Open **Windows Terminal** or **PowerShell**, not `cmd.exe`.

### `codie` is not recognized after installing

The installer added the directory to your user `PATH`. Open a new terminal. To use the current window:

```powershell
$env:Path = "$env:LOCALAPPDATA\Programs\codie;$env:Path"
```

### SmartScreen or antivirus blocks the download

The `0.9.0` release is not code-signed yet. If your organization blocks unsigned binaries, install from
source with a Rust 1.88 toolchain, or wait for the signed `1.0` release. Do not disable security controls
globally.

### The installer says codie is running

Close every `codie` window, then run the installer again. Windows locks the running executable.

### `codie update` cannot replace the binary

Close codie and run the installer. In-place replacement is best-effort on Windows in this preview.
