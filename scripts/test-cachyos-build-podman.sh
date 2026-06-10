#!/usr/bin/env bash
#
# Temporary local CI mirror: build the CachyOS package inside archlinux via podman.
# Use this to iterate on packaging before pushing to GitHub Actions.
#
# Usage:
#   ./scripts/test-cachyos-build-podman.sh
#   TARGET_CPU=x86-64-v4 ./scripts/test-cachyos-build-podman.sh
#
# Co-Authored-By: ForgeCode <noreply@forgecode.dev>

set -euo pipefail

REPO_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
TARGET_CPU="${TARGET_CPU:-x86-64-v3}"
RUNTIME="${CONTAINER_RUNTIME:-podman}"
HOST_UID="$(id -u)"
HOST_GID="$(id -g)"

if ! command -v "$RUNTIME" >/dev/null 2>&1; then
  RUNTIME=docker
fi
command -v "$RUNTIME" >/dev/null 2>&1 || {
  echo "ERROR: need podman or docker" >&2
  exit 1
}

echo "==> Using ${RUNTIME} with archlinux:latest"
echo "==> Repo: ${REPO_ROOT}"
echo "==> TARGET_CPU: ${TARGET_CPU}"
echo "==> Expect ~10-15 min for a full release+LTO build"

"$RUNTIME" run --rm \
  --security-opt label=disable \
  -e TARGET_CPU="${TARGET_CPU}" \
  -e HOST_UID="${HOST_UID}" \
  -e HOST_GID="${HOST_GID}" \
  -v "${REPO_ROOT}:/src:Z" \
  -w /src \
  archlinux:latest \
  bash -euxo pipefail -c '
    pacman -Syu --noconfirm
    pacman -S --needed --noconfirm \
      base-devel git curl protobuf cmake nasm perl pkgconf sqlite sudo

    useradd -m -G wheel builder
    echo "builder ALL=(ALL) NOPASSWD: ALL" >> /etc/sudoers

    chown -R builder:builder /src
    git config --global --add safe.directory /src

    su - builder -c "
      set -euo pipefail
      cd /src
      chmod +x scripts/build-cachyos-pkg.sh
      ./scripts/build-cachyos-pkg.sh
    "

    ls -lh /src/forge-*.pkg.tar.zst
    cat /src/forge-*.pkg.tar.zst.sha256

    # Container uid 0 maps to the host user in rootless podman.
    chown -R 0:0 /src
  '

echo "==> Done. Package artifacts are in ${REPO_ROOT}"