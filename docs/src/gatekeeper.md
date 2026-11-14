# Gatekeeper, quarantine, and SmartScreen

This page describes what macOS and Windows do with a downloaded `codie` binary, and how to respond without
weakening system security.

## macOS

### Quarantine attribute

Files downloaded by a browser, Mail, or some CLI tools get the `com.apple.quarantine` extended attribute.
Gatekeeper checks it before the first launch. A binary from the Homebrew tap or the install script may or may
not carry it depending on how it was downloaded.

Inspect the attribute:

```bash
xattr -l "$(command -v codie)"
```

If a locally built or manually copied binary is quarantined, remove the attribute after verifying the source:

```bash
xattr -d com.apple.quarantine "$(command -v codie)"
```

Do not strip quarantine from a binary you have not verified.

### Signed and notarized releases

Developer ID signing and notarization are required for the `1.0` release. Until then, release candidates are
unsigned. Gatekeeper may refuse to launch a downloaded RC:

- **Right-click → Open** once, or
- install through the install script or Homebrew, or
- build from source with Rust 1.88.

If macOS reports that the binary is damaged, verify the checksum first. A checksum mismatch is a real problem;
a quarantine block on a checksum-valid binary is a signing problem.

### Verifying a download

```bash
shasum -a 256 codie-v0.9.0-rc.1-aarch64-apple-darwin.tar.gz
cat codie-v0.9.0-rc.1-aarch64-apple-darwin.tar.gz.sha256
```

The hashes must match. `install.sh` performs this check automatically and aborts on a mismatch.

### Homebrew

A Homebrew formula is planned for `1.0`. If you install through Homebrew, use `brew upgrade codie` rather than
mixing a manually downloaded binary into the same prefix.

## Windows

### SmartScreen

The Windows preview is unsigned, so SmartScreen may show “Windows protected your PC” for a downloaded archive
or executable. The PowerShell installer downloads through `Invoke-WebRequest` and verifies SHA-256; it does not
disable SmartScreen.

If you trust the release:

- verify the SHA-256 against the published `.sha256`, then
- choose **More info → Run anyway** for the archive or executable, or
- install from source with a Rust 1.88 toolchain.

Do not add broad antivirus exclusions for the whole disk.

### Verifying a download

```powershell
Get-FileHash -Algorithm SHA256 .\codie-v0.9.0-rc.1-x86_64-pc-windows-msvc.zip
Get-Content .\codie-v0.9.0-rc.1-x86_64-pc-windows-msvc.zip.sha256
```

The installer performs the same check and aborts on a mismatch.

## Reporting a blocked launch

Include the codie version, macOS or Windows version, whether the binary came from the install script, Homebrew,
or a manual download, and the exact Gatekeeper/SmartScreen message. Do not attach the binary.
