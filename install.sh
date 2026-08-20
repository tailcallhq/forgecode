#!/usr/bin/env bash
# install.sh — Install HeliosLite (formerly Forgecode) on POSIX systems
#
# Usage:
#   curl -fsSL https://helioslite.phenotype.space/install.sh | bash
#
#   # Pin a specific version:
#   curl -fsSL https://helioslite.phenotype.space/install.sh | bash -s -- 1.2.3
#
#   # Local install (no download): run from repo root
#   ./install.sh --local
#
#   # Override automatic Linux GNU/musl detection (useful in CI):
#   HELIOSLITE_TARGET=x86_64-unknown-linux-musl ./install.sh
#
# Installs the HeliosLite CLI as a single-binary `helioslite` on PATH.
# On Linux/macOS we download the matching raw `forge-*` release binary from
# GitHub Releases and install it as `helioslite`.

set -euo pipefail

VERSION=""
LOCAL=0
SKIP_FORGE=0
SKIP_UPDATE_CHECK=0
REPO="${HELIOSLITE_RELEASE_REPO:-KooshaPari/forgecode}"
TARGET_OVERRIDE="${HELIOSLITE_TARGET:-}"

validate_repo() { [[ "$1" =~ ^[A-Za-z0-9._-]+/[A-Za-z0-9._-]+$ ]] || { echo "Invalid release repo: $1" >&2; exit 1; }; }
validate_version() { printf '%s' "$1" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+([.-][0-9A-Za-z.-]+)?$' || { echo "Invalid release version: $1" >&2; exit 1; }; }
validate_reported_version() {
    local output="$1"
    printf '%s\n' "$output" | grep -Eq '(^|[[:space:]])v?[0-9]+\.[0-9]+\.[0-9]+([.-][0-9A-Za-z.-]+)?([[:space:]]|$)' \
        || { echo "Installed binary did not report a semantic version" >&2; exit 1; }
}
validate_repo "$REPO"

for arg in "$@"; do
    case "$arg" in
        --local)             LOCAL=1 ;;
        --skip-forge)        SKIP_FORGE=1 ;;
        --skip-update-check) SKIP_UPDATE_CHECK=1 ;;
        --help|-h)
            sed -n '2,12p' "$0"
            exit 0
            ;;
        -*) echo "Unknown flag: $arg" >&2; exit 1 ;;
        *)  VERSION="$arg" ;;
    esac
done

# Accept either `1.2.3` or the GitHub-style `v1.2.3` spelling.
VERSION="${VERSION#v}"

# 1) Resolve target version
if [ -z "$VERSION" ] && [ "$LOCAL" = "0" ]; then
    VERSION="$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" \
                | grep -oE '"tag_name":\s*"v?[0-9][^"]*"' \
                | head -1 \
                | sed -E 's/.*"v?([^"]+)".*/\1/' || true)"
    if [ -z "$VERSION" ]; then
        echo -e "  ✖ \033[31mCould not determine latest version; refusing an unpinned install\033[0m" >&2
        exit 1
    fi
fi
if [ "$LOCAL" = "0" ]; then
    validate_version "$VERSION"
    echo -e "  → \033[36mTarget version: $VERSION\033[0m"
else
    echo -e "  → \033[36mTarget version: local build\033[0m"
fi

# 2) Pick install location
INSTALL_DIR="${HELIOSLITE_INSTALL_DIR:-$HOME/.helioslite/bin}"
mkdir -p "$INSTALL_DIR"

# 3) Detect target triple
detect_target() {
    local os arch libc
    os="$(uname -s | tr '[:upper:]' '[:lower:]')"
    arch="$(uname -m)"
    case "$arch" in
        x86_64|amd64) arch="x86_64" ;;
        aarch64|arm64) arch="aarch64" ;;
        *) echo -e "  ✖ \033[31mUnsupported architecture: $arch\033[0m"; return 1 ;;
    esac
    case "$os" in
        linux)
            # Allow CI/packagers to pin an exact release target, but never
            # interpolate arbitrary input into an asset URL.
            if [ -n "$TARGET_OVERRIDE" ]; then
                case "$TARGET_OVERRIDE" in
                    x86_64-unknown-linux-gnu|x86_64-unknown-linux-musl|\
                    aarch64-unknown-linux-gnu|aarch64-unknown-linux-musl)
                        echo "$TARGET_OVERRIDE"; return 0 ;;
                    *)
                        echo -e "  ✖ \033[31mUnsupported HELIOSLITE_TARGET: $TARGET_OVERRIDE\033[0m" >&2
                        return 1 ;;
                esac
            fi
            libc="gnu"
            # musl's ldd identifies itself in its version output.  The
            # loader check covers minimal Alpine images where ldd is absent.
            if { command -v ldd >/dev/null 2>&1 && ldd --version 2>&1 \
                    | grep -qi musl; } \
                || compgen -G '/lib/ld-musl-*.so.1' >/dev/null 2>&1 \
                || compgen -G '/lib64/ld-musl-*.so.1' >/dev/null 2>&1; then
                libc="musl"
            fi
            echo "${arch}-unknown-linux-${libc}"
            ;;
        darwin)  echo "${arch}-apple-darwin" ;;
        *) echo -e "  ✖ \033[31mUnsupported OS: $os\033[0m"; return 1 ;;
    esac
}

if [ "$LOCAL" = "1" ]; then
    if ! command -v cargo >/dev/null 2>&1; then
        echo -e "  ✖ \033[31mcargo not on PATH — install rustup: https://rustup.rs/\033[0m"
        exit 1
    fi
    echo -e "  → \033[36mLocal install — building from source...\033[0m"
    pushd "$(cd "$(dirname "$0")" && pwd)" >/dev/null
    cargo build --release --bin helioslite
    cp "target/release/helioslite" "$INSTALL_DIR/helioslite"
    popd >/dev/null
else
    TARGET="$(detect_target)"
    ASSET="forge-${TARGET}"
    URL="https://github.com/$REPO/releases/download/v$VERSION/$ASSET"
    TMP="$(mktemp -d -t helioslite-install-XXXXXX)"
    trap 'rm -rf "$TMP"' EXIT INT TERM

    echo -e "  → \033[36mDownloading $URL\033[0m"
    if ! curl -fsSL "$URL" -o "$TMP/helioslite"; then
        echo -e "  ✖ \033[31mDownload failed\033[0m"
        exit 1
    fi
    CHECKSUM_URL="$URL.sha256"
    if ! curl -fsSL "$CHECKSUM_URL" -o "$TMP/helioslite.sha256"; then
        echo -e "  ✖ \033[31mRelease checksum is unavailable; refusing an unverified binary\033[0m" >&2
        exit 1
    fi
    EXPECTED_SHA="$(awk 'NF { print $1; exit }' "$TMP/helioslite.sha256")"
    case "$EXPECTED_SHA" in
        (''|*[!0123456789abcdefABCDEF]*)
            echo -e "  ✖ \033[31mInvalid SHA-256 checksum format\033[0m" >&2
            exit 1
            ;;
    esac
    if [ "${#EXPECTED_SHA}" -ne 64 ]; then
        echo -e "  ✖ \033[31mInvalid SHA-256 checksum length\033[0m" >&2
        exit 1
    fi
    if command -v sha256sum >/dev/null 2>&1; then
        ACTUAL_SHA="$(sha256sum "$TMP/helioslite" | awk '{print $1}')"
    elif command -v shasum >/dev/null 2>&1; then
        ACTUAL_SHA="$(shasum -a 256 "$TMP/helioslite" | awk '{print $1}')"
    else
        echo -e "  ✖ \033[31mNo SHA-256 utility found; refusing an unverified binary\033[0m" >&2
        exit 1
    fi
    EXPECTED_SHA_NORMALIZED="$(printf '%s' "$EXPECTED_SHA" | tr '[:upper:]' '[:lower:]')"
    ACTUAL_SHA_NORMALIZED="$(printf '%s' "$ACTUAL_SHA" | tr '[:upper:]' '[:lower:]')"
    if [ "$EXPECTED_SHA_NORMALIZED" != "$ACTUAL_SHA_NORMALIZED" ]; then
        echo -e "  ✖ \033[31mSHA-256 verification failed\033[0m" >&2
        exit 1
    fi
    echo -e "  ✓ \033[32mSHA-256 verified\033[0m"
    STAGED="$INSTALL_DIR/.helioslite.tmp.$$"
    cp "$TMP/helioslite" "$STAGED"
    chmod +x "$STAGED"
    mv -f "$STAGED" "$INSTALL_DIR/helioslite"
    trap - EXIT INT TERM
    rm -rf "$TMP"
fi
chmod +x "$INSTALL_DIR/helioslite"

# 4) PATH
add_to_path() {
    local dir="$1"
    case ":$PATH:" in
        *":$dir:"*) return 0 ;;
    esac
    for rc in "$HOME/.bashrc" "$HOME/.zshrc" "$HOME/.profile"; do
        if [ -f "$rc" ]; then
            if ! grep -q "$dir" "$rc"; then
                echo "" >> "$rc"
                echo "# Added by helioslite installer" >> "$rc"
                echo "export PATH=\"\$PATH:$dir\"" >> "$rc"
            fi
        fi
    done
    export PATH="$PATH:$dir"
}
add_to_path "$INSTALL_DIR"

# 5) Optional: legacy forge / forge-dev alias
if [ "$SKIP_FORGE" = "0" ]; then
    for old in forge forge-dev; do
        old_path="$INSTALL_DIR/$old"
        if [ ! -e "$old_path" ]; then
            cp "$INSTALL_DIR/helioslite" "$old_path"
            chmod +x "$old_path"
            echo -e "  ✓ \033[32mCreated legacy alias $old_path\033[0m"
        fi
    done
fi

# 6) Verify
VER_OUTPUT="$("$INSTALL_DIR/helioslite" --version 2>&1 | head -n 1 || true)"
if [ -z "$VER_OUTPUT" ]; then
    echo -e "  ✖ \033[31mhelioslite --version returned no output; refusing an unverified install\033[0m" >&2
    exit 1
fi
validate_reported_version "$VER_OUTPUT"
if [ "$LOCAL" = "0" ]; then
    EXPECTED_VERSION_PATTERN="${VERSION//./\\.}"
    if ! printf '%s\n' "$VER_OUTPUT" | grep -Eq "(^|[[:space:]])v?${EXPECTED_VERSION_PATTERN}([[:space:]]|$)"; then
        echo -e "  ✖ \033[31mInstalled binary version does not match requested version $VERSION\033[0m" >&2
        exit 1
    fi
fi
echo -e "  ✓ \033[32mhelioslite reports: $VER_OUTPUT\033[0m"

echo ""
echo -e "  🎉 \033[32mHeliosLite installed.\033[0m"
echo -e "     Try:  helioslite --help"
echo -e "     Docs: https://helioslite.phenotype.space"
echo -e "     Old:  forge / forge-dev   \033[90m(deprecated)\033[0m"
