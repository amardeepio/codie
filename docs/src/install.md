# Installation

`codie` ships as a single binary. The install script supports macOS 13+ on arm64 and x64 (tier 1) and Linux
glibc 2.31+ on arm64 and x64 (best effort).

## Install with curl

```bash
curl -fsSL https://raw.githubusercontent.com/amardeepio/codie/main/install.sh | sh
```

The script installs to `~/.local/bin` and prints the `PATH` line to add if that directory is not already on
your path.

Pin a release or choose another directory:

```bash
curl -fsSL https://raw.githubusercontent.com/amardeepio/codie/main/install.sh \
  | sh -s -- --version 0.7.1

curl -fsSL https://raw.githubusercontent.com/amardeepio/codie/main/install.sh \
  | sh -s -- --bin-dir "$HOME/bin"
```

Equivalent environment variables are `CODIE_VERSION` and `CODIE_INSTALL_DIR`.

If you would rather inspect the script first:

```bash
curl -fsSL https://raw.githubusercontent.com/amardeepio/codie/main/install.sh -o install.sh
less install.sh
sh install.sh
```

## What the installer does

1. Detects the operating system and CPU architecture and maps them to a Rust target triple.
2. Resolves the requested version, or the latest git tag when none is given.
3. Tries to download `codie-<version>-<target>.tar.gz` (or `.tar.xz`) from GitHub Releases and verifies its
   SHA-256 checksum. A missing checksum downgrades to a warning; a mismatch aborts the install.
4. If no prebuilt archive exists for that version, builds from source with Cargo. Rust 1.88 is required; the
   script installs it through rustup when it is missing. Pass `--no-rust-install` (or set
   `CODIE_NO_RUST_INSTALL=1`) to opt out and fail instead.
5. Copies the binary into the install directory atomically and runs `codie --version` to confirm it works.

The script never uses `sudo`, never edits shell startup files, and only writes to the install directory and a
temporary directory that is removed afterwards.

## Requirements

| Platform | Status | Notes |
|---|---|---|
| macOS 13+ arm64 | Tier 1 | Release-blocking. |
| macOS 13+ x64 | Tier 1 | Release-blocking. |
| Linux glibc 2.31+ | Tier 2 | Best effort; WSL works through the same target. |
| Windows | Post-1.0 | Not supported by the installer. |

`curl` is required. Building from source additionally needs `git` and Rust 1.88 or newer. See
[Platforms](platforms.md) for the full support statement.

## Verify the install

```bash
command -v codie
codie --version
codie auth login deepseek    # or: codie auth login openrouter
codie auth status
```

## Upgrade

Run the installer again. It replaces the existing binary in place.

```bash
curl -fsSL https://raw.githubusercontent.com/amardeepio/codie/main/install.sh | sh
```

Install a newer release from the published artifacts:

```bash
codie update
```

Check without installing anything:

```bash
codie update --check
```

## Uninstall

```bash
rm ~/.local/bin/codie
```

Remove `~/.local/share/codie` and `~/.config/codie` only if you also want to delete sessions, credentials, and
configuration. The installer does not touch those directories.

## Troubleshooting

### `codie: command not found`

`~/.local/bin` is not on your `PATH`. Add it to the startup file for your shell:

```bash
export PATH="$HOME/.local/bin:$PATH"
```

Then open a new terminal.

### Checksum verification failed

The downloaded archive did not match the published checksum. The install aborts without writing a binary.
Retry, and if the failure persists report it at <https://github.com/amardeepio/codie/issues>.

### Source build is slow

Tagged releases publish prebuilt archives for macOS (arm64/x64) and Linux (arm64/x64). If the requested tag has
no artifact, the script compiles from source with Rust 1.88, which takes a few minutes. Older tags (including
`0.5.0` and `0.6.0`) predate the release workflow and always build from source.

### Building from a checkout

For development, build with the pinned toolchain instead of the installer:

```bash
git clone https://github.com/amardeepio/codie
cd codie
cargo build
cargo run -- --version
```

See [Development](development.md) for the spec-first workflow and test commands.
