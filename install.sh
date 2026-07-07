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
# Installs the HeliosLite CLI as a single-binary `helioslite` on PATH.
# On Linux/macOS we use the matching `helioslite-x86_64-unknown-linux-gnu.tar.xz`
# from GitHub Releases (cargo-dist artifact).

set -euo pipefail

VERSION=""
LOCAL=0
SKIP_FORGE=0
SKIP_UPDATE_CHECK=0
REPO="KooshaPari/heliosLite"

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

# 1) Resolve target version
if [ -z "$VERSION" ] && [ "$LOCAL" = "0" ]; then
    VERSION="$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" \
                | grep -oE '"tag_name":\s*"v?[0-9][^"]*"' \
                | head -1 \
                | sed -E 's/.*"v?([^"]+)".*/\1/' || true)"
    if [ -z "$VERSION" ]; then
        echo -e "  ⚠ \033[33mCould not determine latest version — falling back to v0.1.0\033[0m"
        VERSION="0.1.0"
    fi
fi
echo -e "  → \033[36mTarget version: $VERSION\033[0m"

# 2) Pick install location
INSTALL_DIR="${HELIOSLITE_INSTALL_DIR:-$HOME/.helioslite/bin}"
mkdir -p "$INSTALL_DIR"

# 3) Detect target triple
detect_target() {
    local os arch
    os="$(uname -s | tr '[:upper:]' '[:lower:]')"
    arch="$(uname -m)"
    case "$arch" in
        x86_64|amd64) arch="x86_64" ;;
        aarch64|arm64) arch="aarch64" ;;
        *) echo -e "  ✖ \033[31mUnsupported architecture: $arch\033[0m"; return 1 ;;
    esac
    case "$os" in
        linux)   echo "${arch}-unknown-linux-gnu" ;;
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
    pushd "$(cd "$(dirname "$0")" && pwd)/.." >/dev/null
    cargo build --release --bin helioslite
    popd >/dev/null
    cp "target/release/helioslite" "$INSTALL_DIR/helioslite"
else
    TARGET="$(detect_target)"
    ARCHIVE_EXT="tar.xz"
    if [ "${TARGET##*-}" = "apple-darwin" ]; then ARCHIVE_EXT="tar.xz"; fi
    ASSET="helioslite-${TARGET}.${ARCHIVE_EXT}"
    URL="https://github.com/$REPO/releases/download/v$VERSION/$ASSET"
    TMP="$(mktemp -d -t helioslite-install-XXXXXX)"

    echo -e "  → \033[36mDownloading $URL\033[0m"
    if ! curl -fsSL "$URL" -o "$TMP/$ASSET"; then
        echo -e "  ✖ \033[31mDownload failed\033[0m"
        exit 1
    fi
    echo -e "  → \033[36mExtracting...\033[0m"
    tar -xJf "$TMP/$ASSET" -C "$TMP"
    cp "$TMP/helioslite" "$INSTALL_DIR/helioslite"
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
echo -e "  ✓ \033[32mhelioslite reports: $VER_OUTPUT\033[0m"

echo ""
echo -e "  🎉 \033[32mHeliosLite installed.\033[0m"
echo -e "     Try:  helioslite --help"
echo -e "     Docs: https://helioslite.phenotype.space"
echo -e "     Old:  forge / forge-dev   \033[90m(deprecated)\033[0m"