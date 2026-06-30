# forge-dev Packaging & Installer Spec

**Status**: Draft  
**Owner**: @KooshaPari  
**Repo**: `KooshaPari/forgecode` (fork of `tailcallhq/forge`, no upstream rename)  
**Binary**: `forge-dev` (not `forge`)  

---

## 1. Motivation

`KooshaPari/forgecode` is a fork of `tailcallhq/forge`. The upstream publishes its
binary as `forge` with config at `~/.forge`. To allow side-by-side installation
(dev fork + upstream release on the same machine) we must rename:

| Artifact | Upstream (`forge`) | This fork (`forge-dev`) |
|---|---|---|
| Binary on `$PATH` | `forge` | `forge-dev` |
| Config directory | `~/.forge` | `~/.forge-dev` |
| Legacy directory | `~/forge` | `~/forge-dev` |
| Config env override | `FORGE_CONFIG` | `FORGE_DEV_CONFIG` |
| Shell plugin binary ref | `forge` | `forge-dev` |
| Update URL | `https://forgecode.dev/cli` | `https://forge-dev.sh/cli` |

---

## 2. Binary Rename: `forge` → `forge-dev`

### 2.1 Cargo (`[[bin]]` entry)

**File**: `crates/forge_main/Cargo.toml:8-10`

| Field | Current | Target |
|---|---|---|
| `[[bin]] name` | `forge` | `forge-dev` |
| `[[bin]] path` | `src/main.rs` | unchanged |

```toml
[[bin]]
name = "forge-dev"
path = "src/main.rs"
```

### 2.2 Cargo workspace / flake

**File**: `flake.nix:38,50-57,93`

| Ref | Current | Target |
|---|---|---|
| `pname` | `forge` | `forge-dev` |
| `cargoBuildFlags --bin` | `forge` | `forge-dev` |
| `cargoInstallFlags --bin` | `forge` | `forge-dev` |
| `mainProgram` | `forge` | `forge-dev` |

### 2.3 Justfile

**File**: `Justfile:18`

| Ref | Current | Target |
|---|---|---|
| `cargo run --bin forge` | `forge` | `forge-dev` |

### 2.4 NPM packages

Two npm distribution repos (`antinomyhq/npm-code-forge`, `antinomyhq/npm-forgecode`)
manage publishing. The `update-package.sh` script called during CI references the
binary name. Affected paths:

- `.github/workflows/release.yml:126-128` — `matrix.repository` entries
- `update-package.sh` (external repo) — binary name in install script

### 2.5 Homebrew

**External repo**: `antinomyhq/homebrew-code-forge`  
The `update-formula.sh` script generates a formula referencing `forge`. A new tap
`KooshaPari/homebrew-forge-dev` should be created. The formula:

- `binary` → `forge-dev`
- `test` block calls `forge-dev --version`
- Installs to `$(brew --prefix)/bin/forge-dev`

### 2.6 Update informer

**File**: `crates/forge_main/src/update.rs:16`

The auto-update shell command currently points at the upstream install URL. For
forge-dev this must change to a fork-owned endpoint:

```rust
// Current (upstream):
"curl -fsSL https://forgecode.dev/cli | sh"
// Target (fork):
"curl -fsSL https://forge-dev.sh/cli | sh"
```

---

## 3. Config & Data Directory: `~/.forge` → `~/.forge-dev`

### 3.1 `ConfigReader::resolve_base_path()`

**File**: `crates/forge_config/src/reader.rs:67-84`

| Aspect | Current | Target |
|---|---|---|
| Env override | `FORGE_CONFIG` | `FORGE_DEV_CONFIG` |
| Legacy dir | `~/forge` | `~/forge-dev` |
| Default dir | `~/.forge` | `~/.forge-dev` |

```rust
fn resolve_base_path() -> PathBuf {
    if let Ok(path) = std::env::var("FORGE_DEV_CONFIG") {
        return PathBuf::from(path);
    }
    let base = dirs::home_dir().unwrap_or(PathBuf::from("."));
    let path = base.join("forge-dev");
    if path.exists() {
        tracing::info!("Using legacy path");
        return path;
    }
    tracing::info!("Using new path");
    base.join(".forge-dev")
}
```

### 3.2 Derived paths

File                 | Code reference                      | Current                        | Target
---------------------|-------------------------------------|--------------------------------|--------------------------
TOML config          | `reader.rs:51`                     | `~/.forge/.forge.toml`         | `~/.forge-dev/.forge.toml`
Legacy JSON config   | `reader.rs:45`                     | `~/.forge/.config.json`        | `~/.forge-dev/.config.json`
Conversation DB      | (diesel, in `~/.forge/conversations.db`) | `~/.forge/*`            | `~/.forge-dev/*`
Workspace index      | (diesel, in `~/.forge/workspace/`) | `~/.forge/workspace/`          | `~/.forge-dev/workspace/`
Provider credentials | (diesel, in `~/.forge/`)           | `~/.forge/`                    | `~/.forge-dev/`
Log files            | `info.rs` (log tail)               | `~/.forge/logs/`               | `~/.forge-dev/logs/`
MCP server configs   | (TOML per-provider)                | `~/.forge/*.toml`              | `~/.forge-dev/*.toml`

> **Note**: The codebase uses `ConfigReader::base_path()` to compute all of these.
> Changing `resolve_base_path()` is sufficient — no individual path string needs
> updating unless they hardcode a `.forge` literal.

### 3.3 Migration from legacy

The existing `ConfigCommand::Migrate` (`cli.rs:654`) transitions `~/forge` →
`~/.forge`. For forge-dev this should transition `~/forge-dev` → `~/.forge-dev`.

---

## 4. Shell Plugin

### 4.1 `config.zsh` — binary default

**File**: `shell-plugin/lib/config.zsh:6`

```zsh
typeset -h _FORGE_BIN="${FORGE_BIN:-forge-dev}"
```

### 4.2 `forge.setup.zsh` — eval command

**File**: `shell-plugin/forge.setup.zsh:14`

```zsh
eval "$(forge-dev zsh plugin)"
```
```zsh
eval "$(forge-dev zsh theme)"
```

### 4.3 `forge.plugin.zsh` — internal references

The core plugin file invokes `$FORGE_BIN` internally via `_FORGE_BIN` —
no literal `forge` string change needed once `_FORGE_BIN` defaults to
`forge-dev`. Verify with:

```bash
grep -rn '"forge"' shell-plugin/lib/   # should be 0 hits after update
```

---

## 5. CI / Release Artifacts

### 5.1 Release workflow

**File**: `.github/workflows/release.yml:36-80`

All `binary_name` and `binary_path` patterns use the string `forge`. These need
to produce `forge-dev-{target}` names:

```yaml
# Current:
binary_name: forge-x86_64-unknown-linux-musl
binary_path: target/x86_64-unknown-linux-musl/release/forge

# Target:
binary_name: forge-dev-x86_64-unknown-linux-musl
binary_path: target/x86_64-unknown-linux-musl/release/forge-dev
```

### 5.2 Download URL pattern

| Channel | Current pattern | Target pattern |
|---|---|---|
| GitHub Release tarball | `forge-{target}.tar.gz` | `forge-dev-{target}.tar.gz` |
| Homebrew bottle | `forge--*.bottle.tar.gz` | `forge-dev--*.bottle.tar.gz` |
| NPM package | `npm-code-forge` / `npm-forgecode` | `npm-forge-dev` (single repo) |

---

## 6. Per-OS Installer Spec

### 6.1 macOS — `.app` Bundle

**Distribution**: `.tar.gz` with `forge-dev.app` inside.

**Bundle structure**:

```
forge-dev.app/
└── Contents/
    ├── Info.plist
    ├── MacOS/
    │   └── forge-dev          # the statically-linked binary
    └── Resources/
        └── forge-dev.icns     # app icon
```

**`Info.plist`**:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key>
    <string>forge-dev</string>
    <key>CFBundleIdentifier</key>
    <string>dev.forge.cli</string>
    <key>CFBundleName</key>
    <string>forge-dev</string>
    <key>CFBundleVersion</key>
    <string>$(version)</string>
</dict>
</plist>
```

**Build step** (CI `macos-latest` runner):

```bash
# Binary already at target/release/forge-dev
mkdir -p forge-dev.app/Contents/{MacOS,Resources}
cp target/release/forge-dev forge-dev.app/Contents/MacOS/
# Generate icon (see §7 below)
cp assets/forge-dev.icns forge-dev.app/Contents/Resources/
cp scripts/macos/Info.plist forge-dev.app/Contents/
tar czf forge-dev-x86_64-apple-darwin.tar.gz forge-dev.app/
```

**Signing** (optional for dev builds, recommended for release):

```bash
codesign --force --options runtime \
  --sign "Developer ID Application: Koosha Pari" \
  forge-dev.app/Contents/MacOS/forge-dev
```

**Install method**: User extracts `.tar.gz`, moves `forge-dev.app` to
`/Applications/`, and the install script symlinks the binary:

```bash
sudo ln -sf /Applications/forge-dev.app/Contents/MacOS/forge-dev \
  /usr/local/bin/forge-dev
```

**Alternative (preferred)**: Standalone CLI installer that downloads the binary
directly (see §6.4), plus the `.app` bundle as a convenience for users who
want the GUI icon / Launchpad integration.

### 6.2 Windows — MSI Installer

**Tool**: WiX Toolset v4 (heat + candle + light) or the modern WiX v5.

**Source**:

- Binary: `target/release/forge-dev.exe`
- MSI embeds the binary and adds `forge-dev.exe` to `%PATH%`.

**`forge-dev.wxs`** (WiX):

```xml
<Wix xmlns="http://wixtoolset.org/schemas/v4/wxs">
    <Package Name="forge-dev"
             Manufacturer="KooshaPari"
             Version="$(version)"
             UpgradeCode="GUID-GOES-HERE">
        <Directory Id="TARGETDIR" Name="SourceDir">
            <Directory Id="ProgramFiles64Folder">
                <Directory Id="INSTALLFOLDER" Name="forge-dev" />
            </Directory>
        </Directory>
        <DirectoryRef Id="INSTALLFOLDER">
            <Component Id="forge-dev.exe">
                <File Id="forge-dev.exe"
                      Source="target/release/forge-dev.exe"
                      KeyPath="yes" />
                <Environment Id="PATH" Name="PATH" Part="last"
                             Permanent="no"
                             Value="[INSTALLFOLDER]"
                             System="yes" />
            </Component>
        </DirectoryRef>
        <Feature Id="MainFeature">
            <ComponentRef Id="forge-dev.exe" />
        </Feature>
    </Package>
</Wix>
```

**CI integration** (Windows runner):

```yaml
# In release.yml
- name: Build MSI
  run: |
    dotnet tool install --global wix --version 5.*
    wix build forge-dev.wxs -o forge-dev-x86_64-pc-windows-msvc.msi
```

**User experience**: Double-click MSI → Next → Next → Install → `forge-dev`
available in new terminals.

### 6.3 Linux — AppImage

**Tool**: `appimagetool` from AppImageKit.

**Approach**: Wrap the static binary in an AppDir and produce a single
executable AppImage.

**AppDir structure**:

```
ForgeDev.AppDir/
├── AppRun              # shell script: exec $APPDIR/usr/bin/forge-dev "$@"
├── forge-dev.desktop   # desktop entry
├── forge-dev.png       # icon (256x256)
└── usr/
    └── bin/
        └── forge-dev   # the compiled binary
```

**`forge-dev.desktop`**:

```desktop
[Desktop Entry]
Name=forge-dev
Exec=forge-dev
Icon=forge-dev
Type=Application
Terminal=true
Categories=Development;
```

**CI integration**:

```yaml
- name: Build AppImage
  run: |
    mkdir -p ForgeDev.AppDir/usr/bin
    cp target/release/forge-dev ForgeDev.AppDir/usr/bin/
    # Download appimagetool
    wget -q https://github.com/AppImage/AppImageKit/releases/download/continuous/appimagetool-x86_64.AppImage
    chmod +x appimagetool-x86_64.AppImage
    ./appimagetool-x86_64.AppImage ForgeDev.AppDir
  # Produces: forge-dev-x86_64.AppImage
```

**Alternatives considered**:
- **`.deb` / `.rpm`**: More complex, distro-specific, and require maintainer
  commitment. AppImage is chosen because it works on any Linux distribution
  without package-manager-specific tooling.
- **Snap / Flatpak**: Require store accounts. AppImage is zero-infrastructure.

### 6.4 Universal CLI Install Script

**URL**: `https://forge-dev.sh/cli`

**Behaviour**:

```bash
curl -fsSL https://forge-dev.sh/cli | sh
```

The script:

1. Detects OS + arch (`uname -s`, `uname -m`)
2. Maps to the correct asset name (`forge-dev-x86_64-unknown-linux-musl`,
   `forge-dev-aarch64-apple-darwin`, `forge-dev-x86_64-pc-windows-msvc.exe`)
3. Downloads from `https://github.com/KooshaPari/forgecode/releases/latest/download/{asset}`
4. Installs to `/usr/local/bin/forge-dev` (Unix) or `%LOCALAPPDATA%\forge-dev\bin\forge-dev.exe` (Windows)
5. Adds to `$PATH` (appends to `~/.zshrc` / `~/.bashrc` / `$PROFILE` if not already present)
6. Runs `forge-dev --version` to verify

**Idempotency**: Overwrites existing `forge-dev` binary. Does not touch
`~/.forge-dev/` config directory.

### 6.5 Cargo Install (developer path)

```bash
cargo install --git https://github.com/KooshaPari/forgecode --bin forge-dev
```

This produces `~/.cargo/bin/forge-dev`. No collisions with upstream `forge`
installed via the same method because the binary name is different.

---

## 7. Icon Assets

| Format | Size | Path | Used by |
|---|---|---|---|
| PNG | 256×256 | `assets/forge-dev.png` | AppImage, docs |
| ICNS | multi-resolution | `assets/forge-dev.icns` | macOS `.app` |
| ICO | 256×256 | `assets/forge-dev.ico` | Windows `.msi` |

---

## 8. Validation: Side-by-Side Install Test

A CI workflow (`.github/workflows/test-collision.yml`) that verifies zero
collisions with upstream `forge`:

```yaml
name: Collision Test
on: [push, pull_request]
jobs:
  collision:
    runs-on: ${{ matrix.os }}
    strategy:
      matrix:
        os: [ubuntu-latest, macos-latest, windows-latest]
    steps:
      - uses: actions/checkout@v4
      - run: rustup update && cargo build --release --bin forge-dev

      # --- Install upstream forge (if available) ---
      - run: |
          if command -v brew &>/dev/null; then
            brew install tailcallhq/tap/forge || true
          elif command -v cargo &>/dev/null; then
            cargo install forge || true
          fi

      # --- Both binaries on PATH? ---
      - run: which forge      # may be absent; that's OK
      - run: which forge-dev  # must succeed

      # --- Config dirs are distinct ---
      - run: |
          test "$(forge-dev config path 2>/dev/null || echo "~/.forge-dev")" \
               != "$(forge config path 2>/dev/null || echo "~/.forge")"

      # --- Both run without crashing ---
      - run: forge-dev --version
      - run: |
          if command -v forge &>/dev/null; then
            forge --version
          fi

      # --- Shell plugin paths are distinct ---
      - run: |
          forge-dev zsh plugin 2>/dev/null | grep -q "forge-dev" || \
            echo "WARN: plugin eval string does not contain forge-dev"
```

### Manual validation checklist

| Check | Command |
|---|---|
| Binaries on PATH | `which forge-dev && which forge` (both, or same dir) |
| Config isolation | `forge-dev config path` → `~/.forge-dev/` |
| Config isolation | `forge config path` → `~/.forge/` |
| No cross-contamination | `forge-dev info \| grep ".forge-dev"` |
| Version distinct | `forge-dev --version` reports our version |
| Shell plugin | `eval "$(forge-dev zsh plugin)"` loads without errors |
| Update URL | `forge-dev update --dry-run` hits `forge-dev.sh` |
| Concurrent run | `forge-dev -p "hello"` + `forge -p "world"` in separate terminals |

---

## 9. Implementation Plan (PR Sequence)

| PR# | Title | Files Changed | Scope |
|---|---|---|---|
| 1 | `docs(packaging): forge-dev installer + CLI spec` | `docs/packaging/FORGE_DEV_PACKAGING.md` | **This PR** — spec only, no code changes |
| 2 | `refactor(bin): rename crate binary to forge-dev` | `crates/forge_main/Cargo.toml`, `Justfile` | Cargo rename |
| 3 | `refactor(config): rename config dir to ~/.forge-dev` | `crates/forge_config/src/reader.rs` | Config path rename |
| 4 | `refactor(shell): update plugin default binary to forge-dev` | `shell-plugin/lib/config.zsh`, `shell-plugin/forge.setup.zsh` | Shell plugin |
| 5 | `ci(release): rename release artifacts to forge-dev-*` | `.github/workflows/release.yml` | CI artifacts |
| 6 | `ci(release): add macOS .app bundle` | `.github/workflows/release.yml`, `scripts/macos/Info.plist`, `assets/forge-dev.icns` | macOS bundle |
| 7 | `ci(release): add Windows MSI installer` | `.github/workflows/release.yml`, `forge-dev.wxs`, `assets/forge-dev.ico` | Windows MSI |
| 8 | `ci(release): add Linux AppImage` | `.github/workflows/release.yml`, `ForgeDev.AppDir/*` | Linux AppImage |
| 9 | `feat(install): add universal install script` | `scripts/install.sh` | CLI installer |
| 10 | `ci(collision): add side-by-side collision test` | `.github/workflows/test-collision.yml` | Validation |
| 11 | `chore(nix): rename flake package to forge-dev` | `flake.nix` | Nix |
| 12 | `chore(update): point auto-update to forge-dev.sh` | `crates/forge_main/src/update.rs` | Update URL |

---

## 10. Risks & Mitigations

| Risk | Impact | Mitigation |
|---|---|---|
| Users have both `forge` and `forge-dev` on PATH, but `~/.forge/` is shared | Config corruption | Rename config dir (PR#3) before any binary runs |
| Shell plugin sources `forge` even after installing `forge-dev` | Wrong binary runs | `_FORGE_BIN` env var + clear error if binary not found |
| Homebrew formula for `forge-dev` conflicts with upstream tap | Tap conflict | Separate tap name `KooshaPari/homebrew-forge-dev` |
| Windows MSI install dir conflicts | Silent failure | Use `ProgramFiles64Folder\forge-dev\` (not `forge`) |
| AppImage filename collision with upstream | User confusion | `forge-dev-{arch}.AppImage` (includes dev suffix) |
