# Platforms

Platform support is explicit. A tier is a promise about testing and release blocking, not a claim that the code
happens to compile somewhere.

| Tier | Meaning | Release-blocking | Platform |
|---|---|---|---|
| 1 | Fully supported. Full CI suite on real hardware; P0/P1 bugs block release; documented behavior is guaranteed. | Yes | macOS 13+ (arm64 and x64) |
| 2 | Builds and core paths work; fixes are best-effort; known gaps are documented. | No | Linux glibc 2.31+ (x64/arm64), WSL |
| 3 | Community-supported; no guarantees; post-1.0 roadmap. | No | Windows 10 21H2+, other Unixes |

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

Every release is built on x64 in CI and smoke-tested on arm64. A red `check` job blocks the tag.

## Linux (tier 2)

Linux builds and core paths are kept green as a portability signal. The full suite runs locally and before
releases; CI is macOS x64 only. Fixes are best-effort and there is no terminal compatibility guarantee at 1.0.
A static musl build and a maintainer-owned runner are post-1.0 work. Linux is promoted to tier 1 when that
runner and a terminal matrix exist.

## Windows (post-1.0)

Windows is tier 3 at 1.0 and is not release-blocking. The post-1.0 epic is scoped so it does not surprise the
codebase later:

- MSVC build, signed `.exe`, no Git Bash or MSYS2 required to run.
- Windows Terminal and modern conhost support.
- Process-tree termination through Job Objects, including grandchildren.
- Drive letters, UNC paths, long paths, junctions, case-insensitive comparison, and `\\?\` prefixes.
- CRLF and BOM preservation in `read`, `edit`, and `write`.
- A `powershell` tool; `bash` uses Git Bash only when present.
- Console control events and ConPTY-based integration tests.

macOS-only code lives behind `cfg(target_os = "macos")` in `codie-platform` and must not leak into shared
modules. Windows work must not require rewriting the session, provider, or agent layers.

## Distribution

Release artifacts are produced by `cargo-dist`: macOS arm64 and x64 archives (or universal2), a Homebrew
formula, checksums, and an SBOM. Artifacts are signed, and the release checklist is documented in the
repository. Linux archives are best-effort; Windows archives are post-1.0.

See [`PLAN.MD`](../../PLAN.MD) §12.5 and §12.6 for the normative platform scope and
[`docs/adr/0003-macos-first-v1.md`](../adr/0003-macos-first-v1.md) for the decision record.
