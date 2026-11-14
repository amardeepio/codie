---
id: SPEC-CLI-002
title: Windows x64 preview packaging and install
status: implemented
version: 1.0.0
owners: ["@codie-maintainers"]
requirements: ["CLI-009", "CLI-010", "CLI-011"]
---

# Context

`0.9.0-rc.1` ships a Windows x64 preview. Windows is not a tier-1 platform in `1.0`; the preview makes the
binary installable with one PowerShell command and keeps the updater aware that Windows archives are `.zip`
files. The platform shell is specified separately as `TOOL-030`.

# Normative behavior

**CLI-009 (MUST)** A Windows release publishes `codie-v<version>-x86_64-pc-windows-msvc.zip` containing `codie.exe` at the archive root, a per-asset `.sha256`, and an entry in `sha256.sum`.
Verification: `crates/codie-ai/src/update.rs::release_assets_follow_the_installer_naming`

**CLI-010 (MUST)** `install.ps1` installs the newest published release, or the version in `CODIE_VERSION`, by downloading the Windows archive, verifying its SHA-256, extracting `codie.exe` into `%LOCALAPPDATA%\Programs\codie`, adding that directory to the user PATH unless `CODIE_NO_PATH=1`, and running `codie --version`. The installer never requires elevation or writes machine-wide settings.
Verification: `crates/codie-cli/tests/cli.rs::spec_cli_010_windows_installer_is_one_command`

**CLI-011 (MUST)** On Windows, `codie update` looks for a `.zip` release asset and reports the PowerShell installer command when an in-place replacement fails. On macOS and Linux it continues to look for `.tar.gz` and `.tar.xz` and reports the `install.sh` command.
Verification: `crates/codie-ai/src/update.rs::the_installer_command_matches_the_platform`

# Data shapes

| Input | Value |
|---|---|
| Release target | `x86_64-pc-windows-msvc` |
| Archive | `codie-v<version>-x86_64-pc-windows-msvc.zip` |
| Install directory | `%LOCALAPPDATA%\Programs\codie` |
| Pinned version | `$env:CODIE_VERSION` |
| Skip PATH update | `$env:CODIE_NO_PATH=1` |

# Edge cases

- A missing Windows archive fails the install with a message naming the version.
- A checksum mismatch aborts before extracting anything.
- A running `codie.exe` makes the installer stop with a close-and-retry message.
- PowerShell 5.1 is the minimum; the script also runs on PowerShell 7.
- The preview supports Windows x64 only; other architectures exit with a clear error.

# Non-goals

Windows tier-1 support (Job Objects, ConPTY tests, long/UNC paths, CRLF preservation, ARM64) remains post-`1.0`
work. Self-update through a background helper process is out of scope; a failed in-place replace points at the
installer instead.
