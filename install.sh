#!/bin/sh
# codie installer — https://github.com/amardeepio/codie
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/amardeepio/codie/main/install.sh | sh
#
# Prebuilt binaries are downloaded and checksum-verified when a release exists.
# Otherwise the script builds from source and installs Rust 1.88 through rustup
# when it is missing (disable with --no-rust-install).

set -eu

REPO="amardeepio/codie"
BIN_NAME="codie"
REPO_URL="https://github.com/${REPO}.git"
RAW_BASE="https://github.com/${REPO}/releases/download"

VERSION="${CODIE_VERSION:-}"
BIN_DIR="${CODIE_INSTALL_DIR:-${HOME}/.local/bin}"
NO_RUST_INSTALL="${CODIE_NO_RUST_INSTALL:-0}"

TMP_DIR=""
TMP_BIN=""
INSTALLED_PATH=""

say() {
    printf '%s\n' "$*"
}

warn() {
    printf 'warning: %s\n' "$*" >&2
}

die() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}

usage() {
    cat <<'EOF'
Install codie.

Usage:
  install.sh [--version VERSION] [--bin-dir DIR] [--no-rust-install]

Options:
  --version VERSION  Install a specific release (for example 0.4.0 or v0.4.0).
                     Defaults to the latest tag. Use "main" to build from source.
  --bin-dir DIR      Directory for the codie binary. Default: ~/.local/bin.
  --no-rust-install  Do not install Rust automatically; fail if it is missing.
  -h, --help         Show this help.

Environment:
  CODIE_VERSION          Same as --version.
  CODIE_INSTALL_DIR      Same as --bin-dir.
  CODIE_NO_RUST_INSTALL  Set to 1 for the same behavior as --no-rust-install.

Examples:
  curl -fsSL https://raw.githubusercontent.com/amardeepio/codie/main/install.sh | sh
  curl -fsSL https://raw.githubusercontent.com/amardeepio/codie/main/install.sh \
    | sh -s -- --version 0.4.0 --bin-dir "$HOME/bin"
EOF
}

cleanup() {
    if [ -n "$TMP_BIN" ] && [ -e "$TMP_BIN" ]; then
        rm -f "$TMP_BIN" 2>/dev/null || true
    fi
    if [ -n "$TMP_DIR" ] && [ -d "$TMP_DIR" ]; then
        rm -rf "$TMP_DIR" 2>/dev/null || true
    fi
}

trap cleanup 0 1 2 15

while [ $# -gt 0 ]; do
    case "$1" in
        --version)
            [ $# -ge 2 ] || die "--version requires a value"
            VERSION=$2
            shift 2
            ;;
        --version=*)
            VERSION=${1#*=}
            shift
            ;;
        --bin-dir)
            [ $# -ge 2 ] || die "--bin-dir requires a value"
            BIN_DIR=$2
            shift 2
            ;;
        --bin-dir=*)
            BIN_DIR=${1#*=}
            shift
            ;;
        --no-rust-install)
            NO_RUST_INSTALL=1
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            die "unknown option: $1 (try --help)"
            ;;
    esac
done

[ -n "$BIN_DIR" ] || die "install directory is empty"

command -v curl >/dev/null 2>&1 || die "curl is required but was not found"

OS=$(uname -s)
ARCH=$(uname -m)

case "$OS" in
    Darwin) TARGET_OS="apple-darwin" ;;
    Linux) TARGET_OS="unknown-linux-gnu" ;;
    *) die "unsupported operating system: $OS. codie supports macOS 13+ and Linux at 0.4.0." ;;
esac

case "$ARCH" in
    arm64|aarch64) TARGET_ARCH="aarch64" ;;
    x86_64|amd64) TARGET_ARCH="x86_64" ;;
    *) die "unsupported architecture: $ARCH. codie supports arm64 and x64." ;;
esac

TARGET="${TARGET_ARCH}-${TARGET_OS}"

normalize_version() {
    case "$1" in
        main) printf 'main' ;;
        v*) printf '%s' "$1" ;;
        *) printf 'v%s' "$1" ;;
    esac
}

resolve_latest_tag() {
    # The tags API is public and needs no token. Rate limits are fine for an
    # installer; if it fails, fall back to building from main.
    # `grep -o` extracts one tag per match, so compact and pretty JSON both work;
    # a greedy `sed` over compact JSON would capture the last tag instead.
    curl -fsSL --retry 2 --retry-delay 1 \
        -H "Accept: application/vnd.github+json" \
        "https://api.github.com/repos/${REPO}/tags" 2>/dev/null \
        | grep -o '"name"[[:space:]]*:[[:space:]]*"v[0-9][^"]*"' \
        | sed 's/.*"\(v[0-9][^"]*\)"$/\1/' \
        | sed 's/^v//' \
        | sort -t. -k1,1n -k2,2n -k3,3n \
        | tail -n 1 \
        | sed 's/^/v/' || true
}

if [ -z "$VERSION" ] || [ "$VERSION" = "latest" ]; then
    VERSION=$(resolve_latest_tag)
    if [ -z "$VERSION" ]; then
        warn "could not determine the latest release; building from main"
        VERSION="main"
    fi
fi

TAG=$(normalize_version "$VERSION")

TMP_DIR=$(mktemp -d "${TMPDIR:-/tmp}/codie-install.XXXXXX") || die "could not create a temporary directory"

version_at_least() {
    major=${1%%.*}
    rest=${1#*.}
    minor=${rest%%.*}
    case "$major" in ''|*[!0-9]*) return 1 ;; esac
    case "$minor" in ''|*[!0-9]*) return 1 ;; esac
    if [ "$major" -gt 1 ]; then
        return 0
    fi
    [ "$major" -eq 1 ] && [ "$minor" -ge 88 ]
}

# Prefer an already-installed 1.88 toolchain through rustup, otherwise use the
# default cargo and verify its version below.
run_cargo() {
    if [ "${USE_PINNED_TOOLCHAIN:-0}" = "1" ]; then
        cargo +1.88.0 "$@"
    else
        cargo "$@"
    fi
}

verify_checksum() {
    archive=$1
    vdir=$(dirname "$archive")
    vbase=$(basename "$archive")

    vline=""
    if [ -f "${archive}.sha256" ]; then
        vline=$(cat "${archive}.sha256")
    elif [ -f "${vdir}/sha256.sum" ]; then
        vline=$(grep " ${vbase}\$" "${vdir}/sha256.sum" 2>/dev/null || true)
    fi

    if [ -z "$vline" ]; then
        warn "no checksum published for ${vbase}; skipping verification"
        return 0
    fi

    vhash=$(printf '%s\n' "$vline" | awk 'NR == 1 { print $1 }')
    case "$vhash" in
        ''|*[!0-9a-fA-F]*) warn "malformed checksum for ${vbase}; skipping verification"; return 0 ;;
    esac

    printf '%s  %s\n' "$vhash" "$vbase" > "${vdir}/.checksum"

    if command -v shasum >/dev/null 2>&1; then
        ( cd "$vdir" && shasum -a 256 -c .checksum >/dev/null 2>&1 )
    elif command -v sha256sum >/dev/null 2>&1; then
        ( cd "$vdir" && sha256sum -c .checksum >/dev/null 2>&1 )
    else
        warn "neither shasum nor sha256sum is available; skipping verification"
        return 0
    fi
}

try_prebuilt() {
    [ "$TAG" = "main" ] && return 1

    asset_base="${BIN_NAME}-${TAG}-${TARGET}"

    for ext in tar.gz tar.xz; do
        url="${RAW_BASE}/${TAG}/${asset_base}.${ext}"
        archive="${TMP_DIR}/${asset_base}.${ext}"
        if curl -fsSL --retry 2 --retry-delay 1 -o "$archive" "$url" 2>/dev/null; then
            curl -fsSL --retry 2 --retry-delay 1 -o "${archive}.sha256" \
                "${url}.sha256" 2>/dev/null || true

            if ! verify_checksum "$archive"; then
                die "checksum verification failed for ${asset_base}.${ext}"
            fi

            mkdir -p "${TMP_DIR}/extract"
            tar -xf "$archive" -C "${TMP_DIR}/extract" 2>/dev/null \
                || die "could not extract ${asset_base}.${ext}"

            found=$(find "${TMP_DIR}/extract" -type f -name "$BIN_NAME" 2>/dev/null | head -n 1)
            [ -n "$found" ] || die "no ${BIN_NAME} binary found in ${asset_base}.${ext}"

            PREBUILT_BIN=$found
            return 0
        fi
    done

    return 1
}

ensure_rust() {
    command -v git >/dev/null 2>&1 || die "git is required to build codie from source."

    # rustup is the preferred path: it can fetch the exact pinned toolchain.
    if command -v rustup >/dev/null 2>&1; then
        : "${CARGO_HOME:=${HOME}/.cargo}"
        PATH="${CARGO_HOME}/bin:${PATH}"
        export PATH

        if ! rustup toolchain list 2>/dev/null | grep -q '^1\.88'; then
            say "Rust 1.88 is not installed; installing it with rustup (minimal profile)."
            rustup toolchain install 1.88.0 --profile minimal \
                || die "could not install Rust 1.88 with rustup"
        fi

        USE_PINNED_TOOLCHAIN=1
        return 0
    fi

    # A non-rustup cargo is fine when it is new enough.
    if command -v cargo >/dev/null 2>&1; then
        cargo_version=$(cargo --version 2>/dev/null | awk '{ print $2 }')
        if version_at_least "${cargo_version:-0.0}"; then
            return 0
        fi
    fi

    if [ "$NO_RUST_INSTALL" = "1" ]; then
        die "codie needs Rust 1.88 or newer, but it is not installed.
Automatic installation is disabled. Install it with:
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain 1.88.0
Then run this installer again."
    fi

    say "Rust 1.88 was not found; downloading rustup and a minimal Rust 1.88 toolchain."
    rustup_init="${TMP_DIR}/rustup-init.sh"
    if ! curl --proto '=https' --tlsv1.2 -sSf -o "$rustup_init" https://sh.rustup.rs; then
        die "could not download rustup from https://sh.rustup.rs"
    fi
    if ! RUSTUP_INIT_SKIP_PATH_CHECK=yes sh "$rustup_init" \
        -y --default-toolchain 1.88.0 --profile minimal --no-modify-path; then
        die "could not install Rust automatically. Install it manually with:
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain 1.88.0
Then run this installer again."
    fi

    : "${CARGO_HOME:=${HOME}/.cargo}"
    PATH="${CARGO_HOME}/bin:${PATH}"
    export PATH
    say "Rust 1.88 installed in ${CARGO_HOME}/bin (not added to PATH; the installer only needed it for this build)."

    command -v cargo >/dev/null 2>&1 || die "Rust was installed but cargo is not on PATH at ${CARGO_HOME}/bin"
    if ! rustup toolchain list 2>/dev/null | grep -q '^1\.88'; then
        rustup toolchain install 1.88.0 --profile minimal \
            || die "could not install Rust 1.88 with rustup"
    fi

    USE_PINNED_TOOLCHAIN=1
}

build_from_source() {
    ensure_rust

    say "Building codie ${TAG} from source. This takes a few minutes."
    mkdir -p "${TMP_DIR}/root"
    if [ "$TAG" = "main" ]; then
        run_cargo install codie-cli --git "$REPO_URL" --branch main --locked --bin "$BIN_NAME" \
            --root "${TMP_DIR}/root"
    else
        run_cargo install codie-cli --git "$REPO_URL" --tag "$TAG" --locked --bin "$BIN_NAME" \
            --root "${TMP_DIR}/root"
    fi

    PREBUILT_BIN="${TMP_DIR}/root/bin/${BIN_NAME}"
    [ -x "$PREBUILT_BIN" ] || die "build finished but ${BIN_NAME} was not produced"
}

install_binary() {
    src=$1
    mkdir -p "$BIN_DIR" || die "could not create ${BIN_DIR}"

    dest="${BIN_DIR}/${BIN_NAME}"
    staged="${BIN_DIR}/.${BIN_NAME}.new.$$"
    TMP_BIN=$staged

    cp "$src" "$staged" || die "could not copy ${BIN_NAME} to ${BIN_DIR}"
    chmod 755 "$staged"
    mv -f "$staged" "$dest"
    TMP_BIN=""
    INSTALLED_PATH=$dest
}

if try_prebuilt; then
    say "Downloaded codie ${TAG} for ${TARGET}."
else
    build_from_source
fi

install_binary "$PREBUILT_BIN"

say ""
say "Installed ${BIN_NAME} to ${INSTALLED_PATH}"
if version_output=$("$INSTALLED_PATH" --version 2>/dev/null); then
    say "$version_output"
fi

# Warn when another codie earlier in PATH will run instead of this install.
resolved=$(command -v "$BIN_NAME" 2>/dev/null || true)
if [ -n "$resolved" ] && [ "$resolved" != "$INSTALLED_PATH" ]; then
    say ""
    say "warning: \`${BIN_NAME}\` resolves to ${resolved}, not ${INSTALLED_PATH}."
    shadow_version=$("$resolved" --version 2>/dev/null || true)
    if [ -n "$shadow_version" ]; then
        say "         That binary reports: ${shadow_version}"
    fi
    say "         Put ${BIN_DIR} first in PATH or remove the older binary."
fi

case ":${PATH:-}:" in
    *":${BIN_DIR}:"*) ;;
    *)
        case "$(basename "${SHELL:-sh}")" in
            zsh) rc_file="${HOME}/.zshrc" ;;
            bash) rc_file="${HOME}/.bashrc" ;;
            *) rc_file="${HOME}/.profile" ;;
        esac
        say ""
        say "${BIN_DIR} is not on your PATH. Add this line to ${rc_file}:"
        say ""
        say "  export PATH=\"${BIN_DIR}:\$PATH\""
        say ""
        say "Then open a new terminal or run: export PATH=\"${BIN_DIR}:\$PATH\""
        ;;
esac

say ""
say "Next steps:"
say "  ${BIN_NAME} auth login deepseek    # or: ${BIN_NAME} auth login openrouter"
say "  ${BIN_NAME}                        # start the interactive app"
