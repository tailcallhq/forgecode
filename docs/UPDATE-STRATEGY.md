# HeliosLite update strategy (Gate 5b)

This document is the source-of-truth for how the HeliosLite CLI stays
current after install. It cross-references `docs/FORK.md` and
`docs/RENAMES-STRATEGY.md` so the additive-rename policy is honored.

## Channels

HeliosLite is currently released from `KooshaPari/forgecode`. The release
workflow publishes platform binaries with adjacent `.sha256` sidecars; an
installer must verify the sidecar before executing or replacing a binary.
Package-manager channels remain deferred until fork-owned repositories and
signed release provenance are available.

| Channel  | Source                                  | Use when                                 |
|----------|-----------------------------------------|------------------------------------------|
| stable   | `KooshaPari/forgecode` `release/v*`     | production users                        |
| rc       | `KooshaPari/forgecode` `rc-v*`          | QA / willing early adopters             |
| nightly  | `helios-lite-nightly` workflow artifact | short-lived; pinned by SHA              |
| legacy   | `forgecode.dev/cli`                     | bootstrap only; must expose `cli.sha256` |

Stable and rc go through `cargo-dist`-style release pipelines; nightly
runs via `helios-lite-nightly.yml`.

## Install entrypoints

| Platform  | Command                                             | Source                                    |
|-----------|-----------------------------------------------------|-------------------------------------------|
| curl \\|sh (Linux/macOS) | `curl -fsSL https://helioslite.dev/cli \| sh`  | `install.sh` + mandatory `cli.sha256`     |
| irm (Windows PowerShell) | `irm https://helioslite.dev/install.ps1 \| iex` | `install.ps1`                             |
| Homebrew (macOS/Linux)   | deferred                                | fork-owned tap + signed release required  |
| Chocolatey (Windows)    | deferred                                | fork-owned feed + signed release required  |
| winget (Windows)        | deferred                                | fork-owned manifest + signed release required |
| crates.io (Rust users)  | deferred                                | workspace crates must be publishable first |

## In-app update behaviour

1. On every CLI invocation we consult `update_informer` against the
   `KooshaPari/forgecode` repo (`HELIOSLITE_REPO` env var overrides).
2. If `frequency = Always` and the process is in a TTY, we ask whether
   to upgrade.
3. If `--apply` was passed or `--yes` was paired with the prompt, the updater
   downloads `$HELIOSLITE_UPDATE_URL` and its adjacent `.sha256` sidecar,
   verifies the exact 64-hex SHA-256 digest, and only then executes the
   installer from a unique temporary directory. Missing or mismatched
   sidecars fail closed. The default endpoint is `helioslite.dev/cli`, with
   `forgecode.dev/cli` retained only as a legacy fallback.
4. If the CLI is non-interactive (CI, agent fleet, scripted install),
   the check is skipped.

Legacy `forge-dev` installs still work because `forge_main`'s `[[bin]]`
list keeps `forge-dev` as an alias of the same compiled binary.

## Nightly ratchet

A nightly workflow runs at 06:30 UTC (after the ArgisMonitor nightly so
the cross-fork pair stays consistent). It:

- Reformats and clippy-runs the entire workspace with `-D warnings`.
- Tests the entire workspace.
- Builds the renamed binary `helioslite` plus the legacy alias
  `forge-dev`.
- Uploads both binaries as workflow artifacts under
  `helioslite-nightly-<run-number>`.
- Emits a `phenomonitor://nightly?project=helioslite&date=<date>` event
  into the workspace tracker.

The nightly build does *not* publish; release publishing is gated on a
human tag-pushing a `v*` release.

## Deprecation timeline

- **T+0**: New name `helioslite` is published; legacy names continue
  publishing unchanged.
- **T+3 months**: First deprecation warnings on legacy installs.
- **T+6 months**: Final wrap-up; legacy aliases remain (keg-only for
  brew, deprecation-message-only for npm).
- **T+12 months** *(forward plan)*: Legacy aliases removed in a major.
