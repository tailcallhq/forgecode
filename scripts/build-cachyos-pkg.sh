#!/usr/bin/env bash
#
# Build a CachyOS-optimized forge .pkg.tar.zst from the current source tree.
# Used locally on CachyOS/Arch hosts and by .github/workflows/cachyos-release.yml.
#
# The PKGBUILD in cachyos/ is copied to the repo root with a dynamic pkgrel
# (date + git sha) so overlay installs always upgrade cleanly.
#
# Usage:
#   ./scripts/build-cachyos-pkg.sh
#   TARGET_CPU=x86-64-v4 ./scripts/build-cachyos-pkg.sh
#
# Co-Authored-By: ForgeCode <noreply@forgecode.dev>

set -euo pipefail

REPO_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
TARGET_CPU="${TARGET_CPU:-x86-64-v3}"
MIN_RUST_VERSION="${MIN_RUST_VERSION:-1.94}"

log() { printf '==> %s\n' "$*"; }
die() { printf 'ERROR: %s\n' "$*" >&2; exit 1; }

require_linux_x86_64() {
  [[ "$(uname -s)" == Linux ]] || die "CachyOS packages must be built on Linux"
  [[ "$(uname -m)" == x86_64 ]] || die "Only x86_64 is supported"
}

read_pkgver() {
  awk -F'"' '/^version = / { print $2; exit }' "$REPO_ROOT/Cargo.toml"
}

install_cachyos_makepkg_conf() {
  local dest="/etc/makepkg.conf.d/99-forge-cachyos-optimizations.conf"
  if [[ "$(id -u)" -eq 0 ]]; then
    mkdir -p /etc/makepkg.conf.d
    sed "s/@TARGET_CPU@/${TARGET_CPU}/g" \
      "$REPO_ROOT/cachyos/makepkg.conf.d/99-cachyos-optimizations.conf" \
      >"$dest"
    log "Installed ${dest} (target-cpu=${TARGET_CPU})"
  elif command -v sudo >/dev/null 2>&1; then
    sudo mkdir -p /etc/makepkg.conf.d
    sed "s/@TARGET_CPU@/${TARGET_CPU}/g" \
      "$REPO_ROOT/cachyos/makepkg.conf.d/99-cachyos-optimizations.conf" \
      | sudo tee "$dest" >/dev/null
    log "Installed ${dest} via sudo (target-cpu=${TARGET_CPU})"
  else
    log "Skipping system makepkg.conf.d install (not root, no sudo); using RUSTFLAGS env"
  fi
}

rustc_version_meets_minimum() {
  local current
  current="$(rustc --version 2>/dev/null | awk '{print $2}' | cut -d- -f1 || true)"
  [[ -n "$current" ]] || return 1
  rustc --version >/dev/null 2>&1 || return 1
  printf '%s\n%s\n' "$MIN_RUST_VERSION" "$current" | sort -C -V
}

ensure_rust_toolchain() {
  if rustc_version_meets_minimum; then
    log "Rust $(rustc --version | awk '{print $2}') satisfies >= ${MIN_RUST_VERSION}"
    return
  fi

  log "Installing rustup stable (need rustc >= ${MIN_RUST_VERSION})"
  if ! command -v curl >/dev/null 2>&1; then
    die "curl is required to install rustup"
  fi
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
  # shellcheck disable=SC1091
  source "${HOME}/.cargo/env"
  rustup default stable
  rustc_version_meets_minimum || die "rustup stable is still below ${MIN_RUST_VERSION}"
  export PATH="${HOME}/.cargo/bin:${PATH}"
}

prepare_pkgbuild() {
  local pkgver="$1"
  local pkgrel="$2"

  cp "$REPO_ROOT/cachyos/PKGBUILD" "$REPO_ROOT/PKGBUILD"
  sed -i "s/^pkgver=.*/pkgver=${pkgver}/" "$REPO_ROOT/PKGBUILD"
  sed -i "s/^pkgrel=.*/pkgrel=${pkgrel}/" "$REPO_ROOT/PKGBUILD"

  # When rustup supplies cargo, drop pacman's cargo makedep to avoid version skew.
  if [[ -x "${HOME}/.cargo/bin/cargo" ]]; then
    sed -i "/'cargo'/d" "$REPO_ROOT/PKGBUILD"
  fi
}

run_makepkg() {
  cd "$REPO_ROOT"
  export APP_VERSION="${pkgver}-${pkgrel}"
  export RUSTFLAGS="${RUSTFLAGS:--C target-cpu=${TARGET_CPU} -C opt-level=3 -C codegen-units=1 -C lto=fat}"
  export PATH="${HOME}/.cargo/bin:${PATH}"

  log "Building forge ${pkgver}-${pkgrel}"
  log "  TARGET_CPU=${TARGET_CPU}"
  log "  APP_VERSION=${APP_VERSION}"
  log "  RUSTFLAGS=${RUSTFLAGS}"

  makepkg -f -s --nocheck --noconfirm
}

write_outputs() {
  local pkg_file
  pkg_file="$(ls -1 "$REPO_ROOT"/forge-"${pkgver}"-*.pkg.tar.zst | tail -1)"
  [[ -f "$pkg_file" ]] || die "Expected package artifact not found"

  sha256sum "$pkg_file" >"${pkg_file}.sha256"

  log "Built package: ${pkg_file}"
  cat "${pkg_file}.sha256"

  if [[ -n "${GITHUB_OUTPUT:-}" ]]; then
    {
      echo "package_file=${pkg_file}"
      echo "package_name=$(basename "$pkg_file")"
      echo "app_version=${APP_VERSION}"
      echo "pkgver=${pkgver}"
      echo "pkgrel=${pkgrel}"
    } >>"$GITHUB_OUTPUT"
  fi
}

cleanup() {
  rm -f "$REPO_ROOT/PKGBUILD"
}

require_linux_x86_64
install_cachyos_makepkg_conf
ensure_rust_toolchain

pkgver="$(read_pkgver)"
git_sha="$(git -C "$REPO_ROOT" rev-parse --short HEAD)"
date_tag="$(date -u +%Y%m%d)"
pkgrel="1.cachy.${date_tag}.${git_sha}"

trap cleanup EXIT
prepare_pkgbuild "$pkgver" "$pkgrel"
run_makepkg
write_outputs