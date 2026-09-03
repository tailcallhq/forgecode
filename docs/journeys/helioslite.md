# Journey — helioslite side-by-side (Windows power user)

> **Stage 2 journey** that was missing from requirements. Covers the `helioslite`
> isolated-home + sync contract defined in
> [`docs/requirements.md`](../requirements.md) (sections 1-2) and
> [`docs/architecture.md`](../architecture.md) (Homes & sync).
> Traceability: `FR-004` (config/home isolation), `FR-011` (update/versioning
> is install/verify shape); implementation guard is `FUNCTIONAL_REQUIREMENTS.md`
> acceptance via `heliosdoctor`.

## 1. Persona

**Power user on Windows** running *stable* upstream `forge` and *experimental*
`helioslite` on the **same box**:

- Keeps `forge` (upstream `2.13.21`, or prior fork) on `PATH` for daily work;
  conversations in `~/.forge/.forge.db`.
- Wants to trial `helioslite` (`KooshaPari/forgecode` fork build
  `2.13.21-h.0.1.x`) without risking `~/.forge` — needs a second home,
  side-by-side binaries, and a union view of history.
- Comfortable with `~/.cargo/bin`, Windows named pipes
  (`\\.\pipe\forge-dbd-*`), and `heliosdoctor` as the health check.
- Constraint: Windows file locks — a running `forge.exe`/`helioslite.exe`
  holds the binary open; upgrades must rename-aside, not `taskkill`.

## 2. Trigger

Two failure modes the journey eliminates:

1. **Cross-home hijack.** `forge` opened with `helioslite` on `PATH` (or vice
   versa) writes to the wrong home — `~/.forge/.forge.db` mutated by
   `helioslite` or `~/.helioslite/.forge.writes.db` picked up by `forge`.
   Prior iterations lacked `is_helioslite_binary()` gating and
   `forge_base_path()` vs `base_path()` separation.
2. **PATH clash.** `cargo install --bin forge` from the fork overwrites the
   stable `forge` on `PATH`; user loses the ability to run both. Fix:
   fork installs as `helioslite` (distinct binary name + distinct home) or
   version-qualified `forge 2.13.21-h.0.1.x` is explicitly asserted via
   `heliosdoctor`/`--version`.

Entry condition: `~/.forge/.forge.db` exists and is non-empty (user has
history in stable forge). `~/.helioslite/` may not exist yet.

## 3. Flow (happy path)

All commands are Windows `PowerShell 7+`. Steps are idempotent where noted.

### Step 1 — Install the fork without clobbering stable `forge`

```powershell
# Option A: release asset (preferred, pinned version)
# 36 assets, prerelease:false, latest — version comes from
# crates/forge_ci/src/jobs/release_draft.rs::FORK_RELEASE_VERSION
gh api repos/KooshaPari/forgecode/releases/latest --jq .tag_name
# expect: 2.13.21-h.0.1.x

# Download + place as helioslite (rename-aside if live binary is locked — never taskkill)
# If helioslite.exe is running, Windows allows rename of the in-use file:
#   Rename-Item ~/.cargo/bin/helioslite.exe helioslite.exe.old -ErrorAction SilentlyContinue
#   Copy-Item ./helioslite.exe ~/.cargo/bin/helioslite.exe
```

Fork versioning: `vUPSTREAM-h.FORK` where `FORK = 0.1.N` (`h` = helioslite).
Single source `FORK_RELEASE_VERSION` is wired to `release-drafter`
`version:` input (action input honoured, config `version:` ignored); bump `h`
on each fork release.

```powershell
# Option B: from source (dev-machine)
cargo install --git https://github.com/KooshaPari/forgecode --bin helioslite
```

Verify the binary reports the fork version and does not shadow `forge`:

```powershell
helioslite --version
# expect: 2.13.21-h.0.1.x
forge --version
# expect: still 2.13.21 (or prior stable) — unchanged
```

### Step 2 — Run `helioslite` (isolated home is created)

```powershell
helioslite -p "Reply with exactly: helioslite-ready"
# expect: agent response contains "helioslite-ready", exit 0
```

On first launch the `helioslite` binary resolves:

- `ConfigReader::is_helioslite_binary()` == true
- `base_path()` -> `~/.helioslite/`
- `forge_base_path()` -> `~/.forge/` (upstream home, read-only source for sync)
- Writes go to `~/.helioslite/.forge.writes.db`; `~/.forge/.forge.db` is never
  mutated by `helioslite`.

Verify home separation:

```powershell
Test-Path ~/.helioslite/.forge.writes.db   # True — helioslite's own DB
Test-Path ~/.forge/.forge.db               # True — upstream DB, still owned by forge
helioslite config --help 2>&1 | Select-String "helioslite"
```

### Step 3 — Auto-sync from `~/.forge` (5 s poll)

`helioslite` spawns `ForgeAPI::spawn_upstream_sync_task` **only** when
`is_helioslite_binary() && homes distinct`. The task:

- Polls `~/.forge/.forge.db` every `5s` (`FORGE_SYNC_INTERVAL_SECS`; disable
  with `FORGE_SYNC_DISABLED=1`).
- Uses an mtime gate to skip unchanged polls.
- Calls `import_forge_db` idempotently — new rows only, row-level, via the
  `conversations_all` union view. Local `~/.helioslite/*` writes are also kept.

Observe the sync (wait one interval):

```powershell
# Create a conversation in stable forge (seeds ~/.forge/.forge.db)
forge -p "Seed sync probe: hello from forge"

# Within ~5s helioslite's union view includes it:
Start-Sleep 6
helioslite --help 2>&1 | Out-Null  # ensures sync task has ticked if helioslite is running as daemon
# Conversations from ~/.forge are visible in helioslite's history:
# (via helioslite's conversation list / FTS — union view)
```

Disable sync when needed (e.g., isolated bench):

```powershell
$env:FORGE_SYNC_DISABLED = "1"; helioslite -p "no sync this session"; Remove-Item Env:FORGE_SYNC_DISABLED
```

Tune interval (power-user, not required):

```powershell
$env:FORGE_SYNC_INTERVAL_SECS = "10"; helioslite -p "slower poll"; Remove-Item Env:FORGE_SYNC_INTERVAL_SECS
```

### Step 4 — Verify with `heliosdoctor`

`heliosdoctor` is the acceptance gate (see `docs/requirements.md` section 6):

```powershell
heliosdoctor
# expect:
#   forge 2.13.21-h.0.1.x
#   base ~/.helioslite  (when invoked as helioslite)
#   forge base ~/.forge
#   FTS ok
```

Upstream `forge` still reports its own home:

```powershell
heliosdoctor  # when invoked as forge
# expect: base ~/.forge, FTS ok, version 2.13.21 (no -h suffix)
```

End state: both binaries co-exist on `PATH`, `~/.forge` owns upstream writes,
`~/.helioslite/.forge.writes.db` owns experimental writes, union view is
complete, sync is idempotent and polled.

## 4. Acceptance criteria

- [ ] `helioslite --version` prints `2.13.21-h.0.1.x` matching
      `crates/forge_ci/src/jobs/release_draft.rs::FORK_RELEASE_VERSION`.
- [ ] `helioslite` writes only to `~/.helioslite/.forge.writes.db`; `~/.forge/.forge.db`
      mtime/content unchanged after a `helioslite` session (when `FORGE_SYNC_DISABLED=1`
      to isolate).
- [ ] With sync enabled (default), a conversation created in `forge` appears in
      `helioslite`'s history within `5s + FTS refresh` (union `conversations_all`).
- [ ] `heliosdoctor` (as `helioslite`) shows `forge 2.13.21-h.0.1.x`, base
      `~/.helioslite`, forge base `~/.forge`, FTS ok.
- [ ] `forge` and `helioslite` co-exist on `PATH` — `Get-Command forge,helioslite`
      resolves to two distinct binaries.
- [ ] `cargo fmt --all -- --check` passes (stable toolchain; nightly
      `unstable_features` in `.rustfmt.toml` not enforced).
- [ ] `gh api repos/KooshaPari/forgecode/releases/latest` returns fork assets
      (36 assets, `prerelease:false`), matching the installed `helioslite.exe`
      (`49.1MB` reference in requirements).

## 5. Failure modes & mitigations

| Failure | Symptom | Mitigation / detection |
|---|---|---|
| **PATH order hijack** — `forge` resolves to `helioslite` shim | `forge --version` shows `-h.` suffix unexpectedly | Assert `forge --version` has no `-h.` in CI/journey verify; fix `PATH` order or use fully-qualified `~/.cargo/bin/forge.exe` |
| **Home collision** — `helioslite` ran as `forge` (binary renamed) | `is_helioslite_binary()` false, writes land in `~/.forge` | Never rename `helioslite` to `forge`; `forge_base_path()` vs `base_path()` gate + `heliosdoctor` base check |
| **Sync not ticking** — `FORGE_SYNC_DISABLED=1` leaked from prior session | New `forge` conversations never appear in `helioslite` | `Remove-Item Env:FORGE_SYNC_DISABLED`; verify `~/.helioslite` mtime advances ~5s after a `forge` write |
| **File lock on upgrade** — `helioslite.exe` in use, `Copy-Item` fails | Installer error `process cannot access the file` | Rename-aside pattern (see Step 1): `Rename-Item helioslite.exe helioslite.exe.old` then copy; tell user to relaunch — never `Stop-Process`/`taskkill` (see `AGENTS.md` Process Management) |
| **FTS stale** — union view missing recent rows | Search returns 0 results for known conversation | `ForgeAPI::spawn_upstream_sync_task` also refreshes FTS via `BackgroundTasks`; wait one more poll or restart `helioslite` |
| **Version drift** — `FORK_RELEASE_VERSION` vs `release-drafter.yml` `version:` mismatch | Draft release publishes wrong tag, assets mis-versioned | `FORK_RELEASE_VERSION` is the single source; both `ci.yml` Draft Release and `release-drafter.yml` use `version: FORK_RELEASE_VERSION` (config `version:` ignored) |
| **Daemon pipe contention** (Windows `\\.\pipe\forge-dbd-*`) | `helioslite` write stalls, `Unavailable` fallback to direct pool | Expected — `DaemonWriteOutcome::Unavailable` falls back; `Indeterminate` (after send) is surfaced, never replayed — check `heliosdoctor` and retry |

## 6. References

- `docs/requirements.md` — Fork identity, Home isolation, Sync invariant, Acceptance
- `docs/architecture.md` — Homes & sync diagram, `spawn_upstream_sync_task`, `BackgroundTasks`
- `crates/forge_ci/src/jobs/release_draft.rs::FORK_RELEASE_VERSION` — version source
- `crates/forge_config/src/reader.rs` — `is_helioslite_binary()`, `forge_base_path()` vs `base_path()`
- `crates/forge_api/src/forge_api.rs` — `spawn_upstream_sync_task`, `FORGE_SYNC_INTERVAL_SECS`
- `AGENTS.md` — Process Management (never kill `forge.exe`/`helioslite.exe`)
