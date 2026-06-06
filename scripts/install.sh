#!/usr/bin/env bash
#
# Local from-source build + install/uninstall/reinstall script for Forge.
#
# This is the equivalent of the official installer (curl -fsSL https://forgecode.dev/cli | sh)
# but for building and installing directly from a source checkout. It is intended for:
# - Local development on the feat/xai-supergrok-oauth (and similar) branches
# - CachyOS custom packaging / optimized builds before feeding into PKGBUILD + devtools chroot
#
# Features:
# - Builds with: cargo build --release -p forge_main --bin forge
# - Honors RUSTFLAGS (for CachyOS: -C target-cpu=x86-64-v3 , x86-64-v4, LTO etc.)
# - Honors APP_VERSION (baked via build.rs into CARGO_PKG_VERSION for the binary)
# - Installs the resulting binary (default: $HOME/.local/bin/forge or /usr/local/bin with sudo)
# - Replicates 'forge zsh setup' NON-INTERACTIVELY: inserts the exact marker block
#   (# >>> forge initialize >>> ... # <<< forge initialize <<<) using content from
#   shell-plugin/forge.setup.zsh (same as the one baked into the binary).
# - Supports uninstall: removes binary + cleans the forge initialize markers from .zshrc
#   (and creates timestamped .bak like the Rust implementation). Also cleans any PATH
#   markers we may have added.
# - --reinstall = uninstall + install
# - --build-only to just produce target/release/forge
# - --prefix=..., --force, --no-zsh, --help
#
# Usage examples (CachyOS optimized dev build):
#   RUSTFLAGS="-C target-cpu=x86-64-v3" APP_VERSION="0.1.0-cachy1" ./scripts/install.sh
#   ./scripts/install.sh --prefix=/usr/local --force
#   ./scripts/install.sh --reinstall
#   ./scripts/install.sh --uninstall
#   ./scripts/install.sh --build-only
#
# The script is self-contained, uses the local tree for the embedded shell-plugin content,
# and can be used inside a PKGBUILD %build / %package or for manual custom package testing.
#
# After install (for ZSH users):
#   exec zsh   # or open a new terminal
#   # then test:
#   forge --version
#   :doctor
#
# IMPORTANT: This script does NOT run the interactive `forge zsh setup` (which asks about
# Nerd Fonts + editor). It performs a non-interactive default setup (no NERD_FONT=0, no
# FORGE_EDITOR override). Run `forge zsh setup` manually afterwards if you want the prompts.
#
# Follows project conventions where applicable (e.g. no new *.md docs created here).
# Errors use clear messages; the script is executable after `chmod +x`.
#
# Co-Authored-By: ForgeCode <noreply@forgecode.dev>

set -euo pipefail

# Colors (mimic official installer style, using printf)
RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[0;33m'
NC='\033[0m' # No Color

# Resolve repo root relative to this script (works when invoked from anywhere)
SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
REPO_ROOT="$(cd -- "$SCRIPT_DIR/.." && pwd -P)"

# Defaults (can be overridden by env or flags)
PREFIX="${PREFIX:-${HOME}/.local}"
APP_VERSION="${APP_VERSION:-}"
RUSTFLAGS="${RUSTFLAGS:-}"
UNINSTALL=false
REINSTALL=false
BUILD_ONLY=false
FORCE=false
NO_ZSH=false
VERBOSE=false

usage() {
  cat <<'EOF'
Forge local from-source installer (like official curl | sh, but builds here)

Usage:
  ./scripts/install.sh [options]

Options:
  --help, -h           Show this help
  --prefix DIR         Installation prefix (binary goes to DIR/bin/forge)
                       Default: $HOME/.local   (so ~/.local/bin/forge)
                       Use --prefix=/usr/local for system-wide (may need sudo)
  --uninstall          Remove installed binary and clean ZSH markers from .zshrc
  --reinstall          Uninstall then install (useful for upgrades from source)
  --build-only         Only run the cargo release build; do not install or touch shell
  --force              Overwrite existing binary without warning
  --no-zsh             Skip non-interactive ZSH marker block setup/update
  --verbose, -v        More output during build

Environment variables respected (for CachyOS etc.):
  RUSTFLAGS            Passed through (e.g. "-C target-cpu=x86-64-v3")
  APP_VERSION          Baked into binary via build.rs (e.g. "0.1.0-mybuild")
  PREFIX               Same as --prefix

Typical CachyOS optimized flow (before or instead of full PKGBUILD):
  RUSTFLAGS="-C target-cpu=x86-64-v3" APP_VERSION="$(date +%Y%m%d)-cachy" \
    ./scripts/install.sh --reinstall

The script will:
  1. Build target/release/forge (honoring RUSTFLAGS + APP_VERSION)
  2. Install the binary
  3. Ensure the install bin dir is mentioned in PATH (via marker in rc files)
  4. Insert/update the exact "forge initialize" marker block in .zshrc / $ZDOTDIR/.zshrc
     (content taken from shell-plugin/forge.setup.zsh at build time of *this* script)

Uninstall will:
  - Delete the forge binary from common locations under the chosen/current prefix
  - Remove the >>> / <<< forge initialize block (with .bak timestamp like Rust code)
  - Remove any PATH marker lines we added

See also:
  - Official installer:  curl -fsSL https://forgecode.dev/cli | sh
  - After install:       forge zsh setup   (for the interactive Nerd Font / editor flow)
  - ZSH doctor:          :doctor   or   forge zsh doctor
EOF
}

log_info()  { printf "${BLUE}%s${NC}\n" "$*"; }
log_ok()    { printf "${GREEN}✓ %s${NC}\n" "$*"; }
log_warn()  { printf "${YELLOW}%s${NC}\n" "$*"; }
log_error() { printf "${RED}Error: %s${NC}\n" "$*" >&2; }

# Parse command line (support --foo=bar and --foo bar)
parse_args() {
  while [ $# -gt 0 ]; do
    case "$1" in
      --help|-h)
        usage
        exit 0
        ;;
      --prefix=*)
        PREFIX="${1#*=}"
        ;;
      --prefix)
        shift
        PREFIX="${1:-}"
        ;;
      --uninstall)
        UNINSTALL=true
        ;;
      --reinstall)
        REINSTALL=true
        ;;
      --build-only)
        BUILD_ONLY=true
        ;;
      --force)
        FORCE=true
        ;;
      --no-zsh)
        NO_ZSH=true
        ;;
      --verbose|-v)
        VERBOSE=true
        ;;
      --)
        shift
        break
        ;;
      -*)
        log_error "Unknown option: $1"
        usage
        exit 1
        ;;
      *)
        log_error "Unexpected argument: $1"
        usage
        exit 1
        ;;
    esac
    shift
  done
}

require_command() {
  if ! command -v "$1" >/dev/null 2>&1; then
    log_error "Required command not found: $1"
    exit 1
  fi
}

# Build step (always uses release profile from Cargo.toml which has lto + opt-level=3 + strip)
do_build() {
  log_info "Building Forge from source in $REPO_ROOT"

  if [ -n "$APP_VERSION" ]; then
    log_info "  APP_VERSION=$APP_VERSION (will be baked via build.rs)"
    export APP_VERSION
  fi

  if [ -n "$RUSTFLAGS" ]; then
    log_info "  RUSTFLAGS=$RUSTFLAGS (CachyOS / custom target-cpu etc.)"
    export RUSTFLAGS
  else
    log_warn "  RUSTFLAGS is empty. For CachyOS x86-64-v3+ optimizations run with:"
    log_warn "    RUSTFLAGS=\"-C target-cpu=x86-64-v3\" $0 ..."
    log_warn "  (or -C target-cpu=x86-64-v4 if your CPU and CachyOS makepkg.conf support it)"
  fi

  require_command cargo

  # We intentionally build --release here; this script exists precisely to produce
  # optimized binaries (the AGENTS.md guidance against `cargo build --release` in
  # day-to-day dev does not apply to this packaging/install helper).
  BUILD_CMD=(cargo build --release -p forge_main --bin forge)
  if $VERBOSE; then
    BUILD_CMD+=(--verbose)
  fi

  log_info "  ${BUILD_CMD[*]}"
  (cd "$REPO_ROOT" && "${BUILD_CMD[@]}")

  local src_bin="$REPO_ROOT/target/release/forge"
  if [ ! -x "$src_bin" ]; then
    log_error "Build succeeded but binary not found or not executable: $src_bin"
    exit 1
  fi

  log_ok "Build complete: $src_bin"
  "$src_bin" --version || true
}

# Locate the just-built binary (or fail)
get_src_bin() {
  local src_bin="$REPO_ROOT/target/release/forge"
  if [ ! -x "$src_bin" ]; then
    log_error "No built binary at $src_bin. Run without --build-only first, or use --reinstall."
    exit 1
  fi
  printf '%s' "$src_bin"
}

# Install the binary to $PREFIX/bin/forge (with sudo if the dir is not writable)
do_install_binary() {
  local src_bin
  src_bin="$(get_src_bin)"

  local bin_dir="$PREFIX/bin"
  local dest="$bin_dir/forge"

  log_info "Installing binary to $dest"

  mkdir -p "$bin_dir" 2>/dev/null || true

  local use_sudo=""
  if [ ! -w "$bin_dir" ] && [ "$(id -u)" -ne 0 ]; then
    if command -v sudo >/dev/null 2>&1; then
      use_sudo="sudo"
      log_warn "Directory $bin_dir not writable by $(id -un); using sudo"
    else
      log_error "Cannot write to $bin_dir and no sudo available"
      exit 1
    fi
  fi

  if [ -e "$dest" ] && ! $FORCE && ! $REINSTALL; then
    log_warn "Forge already exists at $dest"
    log_warn "Use --force or --reinstall to overwrite"
    # Still continue so that zsh setup etc. can be (re)done
  fi

  $use_sudo install -Dm755 "$src_bin" "$dest" 2>/dev/null || {
    # Fallback for systems without GNU install -D
    $use_sudo mkdir -p "$bin_dir"
    $use_sudo cp "$src_bin" "$dest"
    $use_sudo chmod 755 "$dest"
  }

  log_ok "forge installed: $dest"
  "$dest" --version || true

  # Also ensure PATH contains the bin dir (mimics official installer behavior)
  ensure_path_entry "$bin_dir"
}

# Ensure a bin dir is on PATH in the usual shell rc files (using a marker so uninstall can clean it)
# We only touch .zshrc and .bashrc to stay close to official.
ensure_path_entry() {
  local bin_dir="$1"
  local path_marker="# Added by Forge (local from-source installer)"
  local export_line="export PATH=\"$bin_dir:\$PATH\""

  for rc in "$HOME/.zshrc" "$HOME/.bashrc"; do
    # Ensure file exists
    if [ ! -f "$rc" ]; then
      # Create with the marker + export (harmless if user never uses that shell)
      {
        printf '%s\n' "$path_marker"
        printf '%s\n' "$export_line"
      } >> "$rc"
      log_info "Created $rc with PATH entry for $bin_dir"
      continue
    fi

    # If the exact export is already there (under our marker or otherwise), do nothing
    if grep -Fq "$export_line" "$rc" 2>/dev/null; then
      continue
    fi

    # Remove any previous lines we added (by marker or the exact export) then prepend fresh
    local tmp
    tmp="$(mktemp)"
    grep -vF "$path_marker" "$rc" | grep -vF "$export_line" > "$tmp" || true

    # Prepend our block at the very top (after possible shebang or first lines is ok for PATH)
    {
      printf '%s\n' "$path_marker"
      printf '%s\n' "$export_line"
      printf '\n'
      cat "$tmp"
    } > "$rc"
    rm -f "$tmp"

    log_info "Updated $rc to ensure $bin_dir is on PATH (marker: $path_marker)"
  done
}

# Remove PATH marker lines we may have added (best effort)
clean_path_markers() {
  local path_marker="# Added by Forge (local from-source installer)"
  for rc in "$HOME/.zshrc" "$HOME/.bashrc"; do
    if [ -f "$rc" ] && grep -Fq "$path_marker" "$rc" 2>/dev/null; then
      local tmp
      tmp="$(mktemp)"
      grep -vF "$path_marker" "$rc" > "$tmp" || true
      # Also drop any orphan export lines that look like ours for this installer
      # (we keep other PATH manipulation)
      mv "$tmp" "$rc"
      log_ok "Removed Forge PATH marker from $rc"
    fi
  done
}

# Replicate the non-interactive equivalent of setup_zsh_integration() from
# crates/forge_main/src/zsh/plugin.rs using the same markers and the exact
# content of shell-plugin/forge.setup.zsh (no nerd-font or editor overrides).
#
# This inserts:
#   # >>> forge initialize >>>
#   <contents of forge.setup.zsh>
#   # <<< forge initialize <<<
#
# And creates a timestamped backup on change, exactly like the Rust code.
do_zsh_setup() {
  if $NO_ZSH; then
    log_info "Skipping ZSH integration (--no-zsh)"
    return 0
  fi

  log_info "Setting up ZSH integration (non-interactive 'forge zsh setup' equivalent)"

  local zdotdir="${ZDOTDIR:-$HOME}"
  local zshrc="$zdotdir/.zshrc"

  local start_marker="# >>> forge initialize >>>"
  local end_marker="# <<< forge initialize <<<"

  local setup_src="$REPO_ROOT/shell-plugin/forge.setup.zsh"
  if [ ! -f "$setup_src" ]; then
    log_error "Cannot find embedded setup block source: $setup_src"
    log_error "This script must be run from inside a full forge source tree."
    exit 1
  fi

  # Normalize (strip stray CRs like the Rust normalize_script does)
  local forge_init_config
  forge_init_config="$(sed 's/\r$//' "$setup_src")"

  # Build the full block exactly as Rust does (no extra nerd/editor lines)
  local forge_config
  forge_config="${start_marker}
${forge_init_config}
${end_marker}"

  # Create backup if file exists (timestamp format matches Rust: %Y-%m-%d_%H-%M-%S)
  local backup_path=""
  if [ -f "$zshrc" ]; then
    local ts
    ts="$(date +%Y-%m-%d_%H-%M-%S)"
    backup_path="$zshrc.bak.$ts"
    cp "$zshrc" "$backup_path"
    log_info "Backup created: $backup_path"
  fi

  # Use bash arrays + simple scan to replicate parse + splice logic.
  # This keeps behavior very close to the Rust implementation (replace in place
  # or append at end, preserving other content).
  local -a lines=()
  if [ -f "$zshrc" ]; then
    mapfile -t lines < "$zshrc"
  fi

  local start_idx=-1
  local end_idx=-1
  local i
  for i in "${!lines[@]}"; do
    if [ "${lines[$i]}" = "$start_marker" ]; then
      start_idx=$i
    fi
    if [ "${lines[$i]}" = "$end_marker" ]; then
      end_idx=$i
    fi
  done

  # Split the block into lines (preserve exact content including internal blanks)
  local -a block_lines=()
  while IFS= read -r line || [ -n "$line" ]; do
    block_lines+=("$line")
  done <<< "$forge_config"

  local -a new_lines=()
  if [ $start_idx -ge 0 ] && [ $end_idx -gt $start_idx ]; then
    # Valid existing block: replace it (splice)
    new_lines=( "${lines[@]:0:$start_idx}" )
    new_lines+=( "${block_lines[@]}" )
    local after_start=$(( end_idx + 1 ))
    if [ $after_start -le ${#lines[@]} ]; then
      new_lines+=( "${lines[@]:$after_start}" )
    fi
  else
    # No (valid) block: append at end, with a separating blank line if needed
    new_lines=( "${lines[@]}" )
    if [ ${#new_lines[@]} -gt 0 ]; then
      local last="${new_lines[-1]}"
      if [ -n "${last//[[:space:]]/}" ]; then
        new_lines+=( "" )
      fi
    fi
    new_lines+=( "${block_lines[@]}" )
  fi

  # Write atomically via temp + mv
  local tmp
  tmp="$(mktemp)"
  if [ ${#new_lines[@]} -gt 0 ]; then
    printf '%s\n' "${new_lines[@]}" > "$tmp"
  else
    : > "$tmp"
  fi
  mv "$tmp" "$zshrc"

  log_ok "forge plugins added/updated in $zshrc"
  if [ -n "$backup_path" ]; then
    log_info "(previous version backed up)"
  fi

  # Friendly next steps (mimic what the interactive on_zsh_setup prints)
  log_info "Run: exec zsh   (or open a new terminal) to load the updated config"
  log_info "Then try: :doctor   or   forge zsh doctor"
}

# Remove the forge initialize marker block (and the markers themselves) from .zshrc
# Best-effort; also handles the case where only one marker exists.
clean_zsh_markers() {
  local zdotdir="${ZDOTDIR:-$HOME}"
  local zshrc="$zdotdir/.zshrc"

  if [ ! -f "$zshrc" ]; then
    return 0
  fi

  local start_marker="# >>> forge initialize >>>"
  local end_marker="# <<< forge initialize <<<"

  if ! grep -Fq "$start_marker" "$zshrc" && ! grep -Fq "$end_marker" "$zshrc"; then
    return 0
  fi

  # Backup before mutating (always, like the Rust update path)
  local ts
  ts="$(date +%Y-%m-%d_%H-%M-%S)"
  local backup_path="$zshrc.bak.$ts"
  cp "$zshrc" "$backup_path"

  # Remove the entire block (from start to end inclusive). If markers are
  # mismatched we still do a best-effort removal of any lines containing them.
  local tmp
  tmp="$(mktemp)"
  awk -v s="$start_marker" -v e="$end_marker" '
    BEGIN { skipping=0 }
    $0 == s { skipping=1; next }
    $0 == e { skipping=0; next }
    !skipping { print }
  ' "$zshrc" > "$tmp" || {
    # Fallback: at least strip the literal marker lines
    grep -vF "$start_marker" "$zshrc" | grep -vF "$end_marker" > "$tmp" || true
  }

  mv "$tmp" "$zshrc"
  log_ok "Cleaned forge initialize markers from $zshrc (backup: $backup_path)"
}

# Full uninstall (binary + markers + path hints)
do_uninstall() {
  log_info "Uninstalling Forge (local from-source)"

  local bin_dir="$PREFIX/bin"
  local candidates=(
    "$bin_dir/forge"
    "$HOME/.local/bin/forge"
    "/usr/local/bin/forge"
    "$HOME/.forge/bin/forge"   # in case someone used an older layout
  )

  local removed=0
  local cand
  for cand in "${candidates[@]}"; do
    if [ -f "$cand" ] || [ -L "$cand" ]; then
      local dir
      dir="$(dirname "$cand")"
      if [ -w "$dir" ]; then
        rm -f "$cand"
      else
        if command -v sudo >/dev/null 2>&1; then
          sudo rm -f "$cand"
        else
          log_warn "Cannot remove $cand (no write permission and no sudo)"
          continue
        fi
      fi
      log_ok "Removed binary: $cand"
      removed=1
    fi
  done

  if [ $removed -eq 0 ]; then
    log_warn "No forge binary found in common locations for prefix $PREFIX"
  fi

  clean_zsh_markers
  clean_path_markers

  log_ok "Uninstall complete"
  log_info "Note: user config in ~/.config/forge (if any) and cloned workspaces are left untouched."
  log_info "Use rm -rf ~/.config/forge if you also want a full purge (not done by default)."
}

main() {
  parse_args "$@"

  if [ "$PREFIX" = "" ]; then
    log_error "--prefix cannot be empty"
    exit 1
  fi

  # Always start from the repo root for cargo and for locating shell-plugin/
  cd "$REPO_ROOT"

  if $UNINSTALL && $REINSTALL; then
    log_error "Cannot combine --uninstall and --reinstall"
    exit 1
  fi

  if $UNINSTALL || $REINSTALL; then
    do_uninstall
  fi

  if ! $UNINSTALL; then
    if ! $REINSTALL && [ -x "$REPO_ROOT/target/release/forge" ] && ! $FORCE && ! $BUILD_ONLY; then
      log_warn "A release binary already exists at target/release/forge"
      log_warn "Use --force to rebuild, or --reinstall"
    fi

    do_build

    if ! $BUILD_ONLY; then
      do_install_binary
      do_zsh_setup
    fi
  fi

  if ! $UNINSTALL && ! $BUILD_ONLY; then
    log_ok "Done."
    local installed_bin="$PREFIX/bin/forge"
    if [ -x "$installed_bin" ]; then
      log_info "Binary: $installed_bin"
      log_info "Run 'forge --help' (after ensuring $PREFIX/bin is on PATH)"
    fi
    if ! $NO_ZSH; then
      log_info "ZSH users: exec zsh  (or new terminal), then try ':doctor'"
    fi
  fi
}

main "$@"
