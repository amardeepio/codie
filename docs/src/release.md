# Release checklist

This is the release-captain checklist for a tagged codie release. It is written for the `0.9.0` release
and the later `1.0.0` gate.

## Before tagging

1. **Freeze scope.** Confirm the release notes and `CHANGELOG.md` describe only merged work.
2. **Version.** `Cargo.toml` workspace version, every internal `codie-*` dependency version, and the tag must
   agree. `release.yml` fails the build when they do not.
3. **Changelog.** Add a `## [<version>] - YYYY-MM-DD` section. Use Added/Changed/Fixed/Removed/Security.
4. **Roadmap.** Update `ROADMAP.md` and `docs/src/roadmap.md` status.
5. **Docs.** Build the book and check the pages touched by the release.
6. **Specs.** `cargo xtask spec-check --strict` is green.
7. **Local gate.** Run:
   ```bash
   cargo fmt --all --check
   cargo clippy --workspace --all-targets -- -D warnings
   cargo test --workspace
   cargo xtask spec-check --strict
   cargo xtask schemas --check
   cargo xtask coverage
   cargo xtask perf
   mdbook build docs
   ```
8. **Windows smoke.** On a Windows x64 machine:
   ```powershell
   cargo build --workspace --all-targets
   cargo test -p codie-tools --test powershell
   ```
9. **No open P0/P1 issues.** Triage the tracker and close or downgrade with a written reason.

## Release candidate

1. Tag:
   ```bash
   git tag -a v0.9.0 -m "codie 0.9.0"
   git push origin v0.9.0
   ```
2. The release workflow builds macOS arm64/x64, Linux arm64/x64, and Windows x64 archives, verifies the tag,
   and publishes checksums.
3. Release candidates are created as GitHub pre-releases. They do not appear in `/releases/latest`, so
   `codie update` will not offer them without an explicit version.
4. Verify the release page has:
   - `codie-v<version>-aarch64-apple-darwin.tar.gz` and `.sha256`
   - `codie-v<version>-x86_64-apple-darwin.tar.gz` and `.sha256`
   - `codie-v<version>-aarch64-unknown-linux-gnu.tar.gz` and `.sha256`
   - `codie-v<version>-x86_64-unknown-linux-gnu.tar.gz` and `.sha256`
   - `codie-v<version>-x86_64-pc-windows-msvc.zip` and `.sha256`
   - `sha256.sum`

## Post-release verification

### macOS and Linux

```bash
curl -fsSL https://raw.githubusercontent.com/amardeepio/codie/main/install.sh \
  | sh -s -- --version 0.9.0
codie --version
codie -p --provider deepseek --model deepseek-flash "Reply with exactly: codie-ok"
```

### Windows

```powershell
$env:CODIE_VERSION = "0.9.0"
irm https://raw.githubusercontent.com/amardeepio/codie/main/install.ps1 | iex
codie --version
codie -p --provider deepseek --model deepseek-flash "Reply with exactly: codie-ok"
```

Confirm the `powershell` tool, `/copy`, and session persistence work.

### Fresh machine

Install on a clean macOS 13+ machine and a clean Windows 11 x64 machine. Confirm Gatekeeper/SmartScreen behavior
matches [Gatekeeper](gatekeeper.md).

## `1.0.0` gate

The following remain open before `1.0.0`:

- Developer ID signing, hardened runtime, notarization, and stapling.
- Homebrew formula and tap.
- SBOM and signed/attested artifacts.
- Reproducible-build verification.
- macOS terminal matrix run on real hardware for every listed terminal.
- Zero open P0/P1 issues.

## Rollback

If a release is broken:

1. Mark the GitHub release as a pre-release or delete it if it was published by mistake.
2. Publish a fixed patch release; do not move an existing tag.
3. Post a short note with the affected versions and the workaround.
4. If `codie update` is affected, point users at the installer with a pinned known-good version.
