# Platforms

Platform support is explicit. A tier is a promise about testing and release blocking, not a claim that the code
happens to compile somewhere.

| Tier | Meaning | Release-blocking | Platform |
|---|---|---|---|
| 1 | Fully supported. Full CI suite on real hardware; P0/P1 bugs block release; documented behavior is guaranteed. | Yes | macOS 13+ (arm64 and x64) |
| 2 | Builds and core paths work; fixes are best-effort; known gaps are documented. | No | Linux glibc 2.31+ (x64/arm64), WSL; Windows 10 21H2+ / 11 x64 (preview) |
| 3 | Community-supported; no guarantees; post-1.0 roadmap. | No | Windows ARM64, other Unixes |

## macOS (tier 1)

v1.0 is macOS-first. The tier-1 scope includes:

- Native arm64 and x64 binaries, or one universal2 binary, signed with a Developer ID and notarized so
  Gatekeeper opens it without workarounds.
- Print, JSON, RPC, and interactive TUI modes in Terminal.app, iTerm2, Ghostty, kitty, WezTerm, and the VS Code
  integrated terminal.
- Raw mode, bracketed paste, truecolor and 256-color fallback, resize handling, and Option-as-Meta. Cmd is left
  to the terminal; codie binds Ctrl and Alt only.
- POSIX process groups, so abort and timeout kill the whole process tree with no orphaned children.
- Path handling for `~`, APFS case-insensitivity, Unicode normalization (NFD/NFC), `/Volumes`, symlinks, and
  filenames with spaces or emoji.
- Clipboard through `pbcopy`/`pbpaste`, with an OSC 52 fallback over SSH.
- PTY-based integration tests, terminal restore on panic and signals, and documented quarantine behavior.

The `0.9.0` release does not satisfy all of these yet: signing, notarization, the full terminal
matrix, APFS/NFC coverage, and OSC 52 remain open. They are tracked in [Releases](release.md).

Every release builds both macOS targets, Linux, and Windows in CI. A red `check`, `linux`, or `windows` job
blocks merges, and the release workflow revalidates the tag and version before publishing.
Terminal coverage is tracked in [Terminal compatibility](terminals.md).

## Linux (tier 2)

Linux builds and core paths are kept green as a portability signal. The full suite runs locally and before
releases; CI builds the workspace on Ubuntu. Fixes are best-effort and there is no terminal compatibility
guarantee at 1.0. A static musl build and a maintainer-owned runner are post-1.0 work. Linux is promoted to
tier 1 when that runner and a terminal matrix exist.

## Windows (x64 preview)

`0.9.0` publishes a Windows x64 preview. It builds and the core tools work, but it is not release-blocking
and it is not tier 1.

What the preview provides:

- An MSVC `codie.exe` built by `.github/workflows/release.yml` and packaged as a `.zip`.
- A one-command PowerShell installer, `install.ps1`.
- `read`, `write`, `edit`, `grep`, `find`, `ls`, and `web_fetch`.
- A `powershell` tool that prefers `pwsh.exe` and falls back to Windows PowerShell 5.1.
- Process-tree termination through `taskkill /T /F /PID` for shell and extension timeouts.
- Clipboard copy through `clip`.
- Interactive TUI in Windows Terminal and modern conhost.

Known gaps:

- No POSIX `bash` tool; use `powershell`.
- Windows Job Objects, UNC/long-path handling, CRLF/BOM preservation, and a ConPTY test matrix are post-1.0.
- Windows ARM64 is not built.
- The preview is unsigned.

The post-1.0 Windows epic remains:

- Signed `.exe`, no Git Bash or MSYS2 required to run.
- Job Objects for process-tree termination.
- Drive letters, UNC paths, long paths, junctions, case-insensitive comparison, and `\\?\` prefixes.
- CRLF and BOM preservation in `read`, `edit`, and `write`.
- Console control events and ConPTY-based integration tests.

See [Windows (preview)](windows.md) for install and troubleshooting.

macOS-only code lives behind `cfg(target_os = "macos")` and must not leak into shared modules. Windows work must
not require rewriting the session, provider, or agent layers.

## Distribution

Tagged releases are built by `.github/workflows/release.yml`, which publishes:

- `codie-v<version>-<target>.tar.gz` archives for macOS arm64/x64 and Linux arm64/x64.
- `codie-v<version>-x86_64-pc-windows-msvc.zip`.
- A per-asset `.sha256` and a combined `sha256.sum`.

Linux archives are built in an Ubuntu 20.04 container so they run on glibc 2.31 or newer. `install.sh`,
`install.ps1`, and `codie update` / `/update` consume those assets. Developer ID signing, notarization, a
Homebrew formula, and an SBOM remain required for the `1.0` gate; Windows ARM64 archives are post-1.0.

See [`PLAN.MD`](../../PLAN.MD) §12.5 and §12.6 for the normative platform scope and
[`docs/adr/0003-macos-first-v1.md`](../adr/0003-macos-first-v1.md) for the decision record.
