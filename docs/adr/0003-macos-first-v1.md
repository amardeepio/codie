# ADR 0003: macOS-first v1.0 platform support

- Status: Accepted
- Date: 2026-11-11
- Deciders: codie maintainers
- Related: `PLAN.MD` §1.2, §12.5, §12.6, §13.2 (D-11)

## Context

Supporting three desktop platforms at 1.0 multiplies terminal compatibility, process control, path handling,
packaging, and signing work. The project has one full-time engineer for the initial schedule. A platform claim
is only credible when CI runs the full suite on real hardware and release artifacts are signed and installable.

The confirmed D-11 decision selects macOS as the only tier-1 platform for v1.0, with Linux kept as a
portability signal and Windows deferred to a post-1.0 epic. The decision is recorded here because it shapes
packaging, `codie-platform`, the CI matrix, and the release checklist.

## Decision

v1.0 ships macOS first.

- **Tier 1:** macOS 13+ on arm64 and x64. Native binaries (or one universal2 binary) are Developer ID signed,
  notarized, and tested on a terminal matrix: Terminal.app, iTerm2, Ghostty, kitty, WezTerm, and the VS Code
  integrated terminal.
- **Tier 2:** Linux glibc 2.31+ (x64 and arm64) and WSL. Builds and core paths work; fixes are best-effort;
  gaps are documented.
- **Tier 3 / post-1.0:** Windows 10 21H2+ and other Unixes. No guarantees at 1.0.
- macOS-specific behavior lives behind `cfg(target_os = "macos")` in a dedicated `codie-platform` module and
  must not leak into shared modules.
- POSIX process groups terminate abort and timeout trees; APFS case-insensitivity and Unicode normalization
  (NFD/NFC) are covered by tests; clipboard uses `pbcopy`/`pbpaste` with an OSC 52 fallback.
- A red macOS arm64 or x64 CI job blocks a release tag.

## Consequences

Positive:

- The Phase 7 matrix is finite and testable on hardware the maintainer can own.
- Signing, notarization, Homebrew, and quarantine behavior are solved once, well.
- The codebase keeps `std::path` and portable process abstractions from day one, so Linux promotion and the
  Windows epic remain packaging and testing work rather than a rewrite.
- Users get an honest support statement instead of untested cross-platform claims.

Negative:

- Linux and Windows users are explicitly second-class at 1.0, which limits initial adoption.
- macOS-only process-group semantics must be abstracted before Windows can use Job Objects.
- Two macOS architectures and a terminal matrix increase release time.
- Deferred Windows work (ConPTY, long paths, PowerShell tool) stays on the roadmap and must not be forgotten.

## Alternatives considered

- **Windows tier 1 at launch.** Rejected: ConPTY, Job Objects, path semantics, and signing would dominate the
  schedule and delay a usable macOS release.
- **Linux tier 1 at launch.** Rejected for 1.0: requires a maintainer-owned runner and a terminal matrix;
  promoted after 1.0 when those exist.
- **All three platforms best-effort.** Rejected: "best-effort everywhere" is indistinguishable from "supported
  nowhere" and undermines the release gate.

## References

- `PLAN.MD` §12.5 (platform tiers), §12.6 (post-1.0 platform roadmap), §13.2 (D-11).
- `docs/src/platforms.md`.

## Addendum: 0.9.0 release candidate

`0.9.0-rc.1` publishes a Windows x64 preview: an MSVC build, a PowerShell installer, and a `powershell` tool.
This does not promote Windows to tier 1. Job Objects, ConPTY tests, long/UNC paths, CRLF preservation, signing,
and Windows ARM64 remain post-1.0 work. macOS remains the only release-blocking platform.
