#!/usr/bin/env bash
#
# Setup helper / documented procedure for a CachyOS-optimized build environment
# on a Proxmox VM (LXC or full VM) that will host clean chroots for custom
# overlay packages (Forge, and similar projects).
#
# This script does NOT create .md documentation files (per project AGENTS.md).
# Instead it *is* the script + living documentation: run it with --help (or
# without args) to see the exact steps, commands, and rationale. It can also
# perform local sanity checks (--check) when executed on a CachyOS-like host.
#
# Why a dedicated build host / chroot:
# - Reproducible packages with CachyOS performance flags (RUSTFLAGS target-cpu
#   x86-64-v3 / v4 + makepkg.conf CFLAGS etc.)
# - Clean chroot prevents host contamination (devtools + extra-x86_64-build)
# - Ability to serve a small local pacman repo (nginx/caddy + repo-add) to
#   Proxmox nodes / workstations / the overlay users.
# - Snapshottable storage (ZFS / Btrfs subvols) for /var/cache/pacman and chroot roots.
#
# Relationship to the other deliverables on this branch:
# - scripts/install.sh : use *outside* the chroot on the build host or on
#   developer workstations for fast "from source + same RUSTFLAGS" iteration:
#       RUSTFLAGS="-C target-cpu=x86-64-v3" ./scripts/install.sh --reinstall
# - cachyos/PKGBUILD : the packaging recipe consumed by makepkg inside the
#   clean chroot. It respects the RUSTFLAGS coming from the chroot's
#   /etc/makepkg.conf (or the one injected by devtools) and bakes APP_VERSION.
#
# Co-Authored-By: ForgeCode <noreply@forgecode.dev>

set -euo pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[0;33m'
NC='\033[0m'

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
REPO_ROOT="$(cd -- "$SCRIPT_DIR/.." && pwd -P)"

usage() {
  cat <<'EOM'
CachyOS optimized build environment setup for custom packages (Proxmox VM)

This script prints (and can partially validate) the procedure to turn a
CachyOS installation (LXC or VM under Proxmox) into a build host that
produces x86-64-v3 / v4 optimized .pkg.tar.zst packages using clean chroots.

Run on the target build machine (or just read the output anywhere):
  ./scripts/setup-cachyos-build-env.sh            # show full guide
  ./scripts/setup-cachyos-build-env.sh --help
  ./scripts/setup-cachyos-build-env.sh --check    # run local sanity (if on CachyOS)

Key produced artifacts that live in the forge repo:
  - scripts/install.sh          (local from-source + Cachy RUSTFLAGS + zsh)
  - cachyos/PKGBUILD            (the recipe for the overlay)

The two are deliberately compatible: use the install script for quick dev
builds on the host; use the PKGBUILD + this chroot for "official" overlay pkgs.

EOM
  print_steps
}

print_steps() {
  cat <<'STEPS'

================================================================================
1. Base OS on the Proxmox VM / LXC
================================================================================
- Create a CachyOS LXC (or VM) from the official CachyOS template / ISO.
  Recommended: privileged LXC for easier bind-mounts of ZFS datasets, or
  unprivileged with proper idmap if you prefer.

- Update:
    sudo pacman -Syu

- Install core build tooling:
    sudo pacman -S --needed base-devel devtools git pacman-contrib

  (devtools brings extra-x86_64-build, makechrootpkg, etc.)

================================================================================
2. Storage layout (highly recommended on Proxmox)
================================================================================
Use ZFS or Btrfs on the build host so you can snapshot before/after big builds
and roll back the chroot or the pacman cache.

Example ZFS (from the Proxmox host or inside the guest if it owns a dataset):
    zfs create -o mountpoint=/var/lib/archbuild rpool/ARCHBUILD
    zfs create -o mountpoint=/var/cache/pacman/pkg rpool/PACCACHE
    zfs create -o mountpoint=/var/cache/pacman/cachyos rpool/OUR_REPO

    # Optional: separate dataset for sources
    zfs create -o mountpoint=/build/sources rpool/BUILDSRC

Inside the guest, make sure the mounts are there and have sane permissions:
    sudo mkdir -p /var/lib/archbuild /var/cache/pacman/pkg
    sudo chown -R builduser:builduser /var/lib/archbuild /var/cache/pacman 2>/dev/null || true

Btrfs equivalent: subvolumes + snapshots.

================================================================================
3. CachyOS makepkg.conf & RUSTFLAGS (the optimization source of truth)
================================================================================
CachyOS ships a tuned /etc/makepkg.conf (or in /usr/share/makepkg.conf.d/cachyos.conf
included by the cachyos-keyring / cachyos-mirrorlist packages).

Typical interesting bits that the PKGBUILD relies on:
    CFLAGS+=" -march=x86-64-v3 -mtune=znver3 ..."   # or v4
    RUSTFLAGS+=" -C target-cpu=x86-64-v3 -C opt-level=3 ..."
    LDFLAGS+=" -Wl,-O2,..."

If you are on a stock Arch in the VM and want CachyOS-like flags, you can
drop in the relevant snippets from a real CachyOS /etc/makepkg.conf or set
them in /etc/makepkg.conf.d/99-cachyos-optimizations.conf :

    # /etc/makepkg.conf.d/99-cachyos-optimizations.conf
    CARCH="x86_64"
    CHOST="x86_64-pc-linux-gnu"
    CFLAGS="-march=x86-64-v3 -mtune=generic -O3 -pipe ..."
    CXXFLAGS="${CFLAGS}"
    RUSTFLAGS="-C target-cpu=x86-64-v3 -C opt-level=3 -C codegen-units=1 -C lto ..."
    MAKEFLAGS="-j$(nproc)"

The cachyos/PKGBUILD in this repo deliberately does *not* set RUSTFLAGS
itself; it lets whatever the chroot's makepkg.conf exports win. This way
the same PKGBUILD produces v3 on a v3 host and v4 on a v4 host when the
overlay's makepkg.conf is the source of the flags.

================================================================================
4. Create the clean chroot (once)
================================================================================
As your normal user (or a dedicated "build" user):

    # The first time this will download a full base chroot (~1-2 GiB)
    extra-x86_64-build -- --syncdeps -- --noconfirm

This creates /var/lib/archbuild/extra-x86_64/root

You can also create a cachyos-specific root if your overlay has its own
mirrorlist / pacman.conf :

    # Advanced: custom pacman.conf for the chroot that includes your overlay
    # and the upstream CachyOS mirrors + pacoloco cache.

To enter a shell in the chroot for debugging:
    arch-nspawn /var/lib/archbuild/extra-x86_64/root

================================================================================
5. Building packages (using the PKGBUILD from this repo)
================================================================================
Typical flow inside your overlay checkout (the dir that contains the copied
or symlinked cachyos/PKGBUILD for the "forge" package):

    # From the dir containing the PKGBUILD
    extra-x86_64-build

This will:
  - rsync the PKGBUILD + any needed sources into the clean chroot
  - run prepare/build/package under the CachyOS-tuned makepkg.conf
  - produce forge-0.1.0-1.cachy-x86_64.pkg.tar.zst in the current dir

For even faster local iteration (outside any chroot) on the build host itself
or on a dev workstation that is already CachyOS:
    cd /path/to/forge-source-checkout
    RUSTFLAGS="-C target-cpu=x86-64-v3" APP_VERSION="0.1.0-cachy-test" \
        ./scripts/install.sh --reinstall --prefix=/tmp/forge-test-install

(The install.sh and the PKGBUILD share the same build command + env contract.)

After a successful chroot build, sign if your overlay uses signed packages:
    gpg --detach-sign --default-key $KEYID *.pkg.tar.zst

================================================================================
6. Local repository serving (so other machines can pacman -Syu your builds)
================================================================================
After builds:

    mkdir -p /var/cache/pacman/cachyos/forge/x86_64
    cp *.pkg.tar.zst *.pkg.tar.zst.sig /var/cache/pacman/cachyos/forge/x86_64/
    repo-add /var/cache/pacman/cachyos/forge/x86_64/forge.db.tar.gz \
             /var/cache/pacman/cachyos/forge/x86_64/*.pkg.tar.zst

Serve it (simple static + directory index is enough):

    # Option A: caddy (one-liner)
    caddy file-server --root /var/cache/pacman/cachyos --listen :8080

    # Option B: nginx snippet
    # location /cachyos/ {
    #     alias /var/cache/pacman/cachyos/;
    #     autoindex on;
    # }

On client machines (or other Proxmox containers) add to /etc/pacman.conf :

    [cachyos-forge]
    SigLevel = Optional TrustAll   # or proper key setup
    Server = http://buildhost:8080/forge/x86_64

    # (and also keep the real CachyOS mirrors + pacoloco if you run it)

Then: pacman -Sy forge

================================================================================
7. Caching & mirrors (Pacoloco recommended)
================================================================================
Run pacoloco on the build host (or a dedicated proxy LXC). It acts as a
transparent cache for all upstream Arch / CachyOS mirrors and dramatically
speeds up repeated chroot bootstraps and dep downloads.

Example pacoloco config + systemd socket activation is in the CachyOS wiki /
pacoloco README. Point makepkg / chroot pacman.conf at
http://localhost:9129/repo/...

Also consider:
    paccache -rk 2   # keep only last 2 versions of cached pkgs

================================================================================
8. Automation / CI on the build host (optional but nice)
================================================================================
- A small cron / systemd timer that pulls the latest feat/xai... (or main),
  runs extra-x86_64-build for each package that has a PKGBUILD in the overlay,
  signs, repo-add, and rsyncs to a "latest" dir.
- Or a tiny webhook receiver that reacts to GitHub "release" or push events
  on the feature branch.
- Store the overlay git repo on the same ZFS dataset so you can `git pull`
  inside a snapshot.

================================================================================
9. Pairing with the deliverables in this branch (feat/xai-supergrok-oauth)
================================================================================
- Use scripts/install.sh for "YOLO but optimized" builds on any CachyOS box
  (including the build VM itself) during development of the XAI / supergrok
  OAuth changes.
- Use cachyos/PKGBUILD inside the clean chroot when you are ready to cut a
  package for the overlay mirror that other machines will consume.
- The Proxmox VM becomes the single source of truth for "our" builds of Forge
  with the exact CachyOS CPU targeting + the latest from the feature branch.

================================================================================
10. One-time first-run checklist on the new build VM
================================================================================
[ ] CachyOS guest installed + fully updated
[ ] base-devel + devtools installed
[ ] ZFS/Btrfs datasets mounted for archbuild + paccache + our_repo
[ ] /etc/makepkg.conf.d/* shows the v3/v4 RUSTFLAGS (or you added the snippet)
[ ] extra-x86_64-build ran successfully at least once (created the root)
[ ] A test build of the forge PKGBUILD succeeded and produced a .pkg.tar.zst
[ ] repo-add + a trivial http server works from another container
[ ] (optional) pacoloco running and pacman.conf points at it
[ ] (optional) the scripts/install.sh from a fresh clone of the branch also
    works with the same RUSTFLAGS and produces a runnable forge

Happy building!

STEPS
}

do_check() {
  echo "Running local sanity checks (best effort - works best on a real CachyOS host)..."
  local ok=0

  for cmd in pacman makepkg extra-x86_64-build arch-nspawn repo-add caddy nginx pacoloco; do
    if command -v "$cmd" >/dev/null 2>&1; then
      printf "  ${GREEN}✓${NC} %s present\n" "$cmd"
    else
      printf "  ${YELLOW}?${NC} %s not found in PATH (may be ok depending on role)\n" "$cmd"
    fi
  done

  if [ -f /etc/makepkg.conf ]; then
    if grep -q 'x86-64-v3\|target-cpu' /etc/makepkg.conf /etc/makepkg.conf.d/* 2>/dev/null; then
      printf "  ${GREEN}✓${NC} RUSTFLAGS / target-cpu hints found in makepkg.conf\n"
    else
      printf "  ${YELLOW}!${NC} No obvious x86-64-v3 RUSTFLAGS in makepkg.conf (you may need to add the CachyOS tuning snippet)\n"
    fi
  fi

  if [ -d /var/lib/archbuild ]; then
    printf "  ${GREEN}✓${NC} /var/lib/archbuild exists (chroots live here)\n"
  else
    printf "  ${YELLOW}!${NC} /var/lib/archbuild missing - run extra-x86_64-build first\n"
  fi

  if [ -d "$REPO_ROOT/cachyos" ] && [ -f "$REPO_ROOT/cachyos/PKGBUILD" ]; then
    printf "  ${GREEN}✓${NC} cachyos/PKGBUILD found relative to this script\n"
  fi
  if [ -f "$REPO_ROOT/scripts/install.sh" ]; then
    printf "  ${GREEN}✓${NC} scripts/install.sh present (the companion from-source tool)\n"
  fi

  echo "Check finished."
}

main() {
  case "${1:-}" in
    --help|-h)
      usage
      exit 0
      ;;
    --check)
      do_check
      exit 0
      ;;
    *)
      usage
      exit 0
      ;;
  esac
}

main "$@"
