# Changelog

All notable changes to ForgeCode are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.0.0/) and
[Semantic Versioning](https://semver.org/spec/v2.0.0.html). This baseline was
reconstructed from the ten most recent GitHub releases and the latest 100 commits.

## [Unreleased]

### Added
- Weekly AgilePlus 31-pillar scorecard publishing to a GitHub issue.
- Nightly cleanup policy for stale and merged, unprotected remote branches.
- AgilePlus sprint, backlog, quality-gate, velocity, and ownership tracking.

### Changed
- Repository governance now includes Contributor Covenant v2.1.

## [v2.13.21-h.0.1.1] - 2026-08-19

### Added
- Multi-client named-pipe support with client workspace IDs.
- Workspace identity on daemon-routed repository upserts.

### Fixed
- Release version metadata and release-drafter pin alignment.
- Windows path, cancellation, and test-gate failures in repository and shell components.
- Zsh setup bounded so interactive configuration cannot hang.
- Cargo-deny failures and Windows-safe discovery tests.

### Changed
- Added guidance to preserve interactive Forge and HeliosLite sessions.

## [v2.10.8] - 2026-08-10

### Fixed
- Restored and repaired Rust 1.96 quality gates.
- Completed environment trait mocks and removed an unused base-path resolver.
- Retired obsolete merge dependency lineage.

### Added
- Deterministic SBOM generation policy and verified HeliosLite snapshot import.

## [v2.10.7] - 2026-08-08

### Added
- ForgeDB daemon with real SQLite operations, named-pipe transport, and spawn-on-first-write lifecycle.
- Split-database CLI integration coverage and doctor integrity checks.

### Fixed
- Restored the `indexmap` serde feature, hardened doctor and installer verification, and updated Infisical workflow permissions and pins.
- Hardened cross-platform path tests and removed stale CI workflow generators.

## [v2.10.6] - 2026-08-05

### Fixed
- Defaulted Zsh dispatch to ForgeCode.

### Changed
- Refreshed CycloneDX SBOM artifacts and removed a deprecated Handlebars type stub.

## [v2.10.5] - 2026-08-04

### Added
- Attestation for published release assets.

### Fixed
- Pinned updater matrix assets and hardened shell installation.

## [v2.10.4] - 2026-08-03

### Fixed
- Skipped scheduled bounty pull-request synchronization.

## [v2.10.3] - 2026-08-03

### Fixed
- Ran release checksums under Bash, disabled unsupported fork package channels, and escaped updater PowerShell braces.
- Corrected native Windows auto-update, PATH length handling, and Ctrl+C terminal restoration.
- Bounded automatic continuation after interrupts.

## [v2.10.2] - 2026-08-02

### Fixed
- Made doctor and shell setup portable, restored the release workflow, and compiled the Windows doctor skip guard.
- Removed an unsafe Infisical secret workflow and stale RustSec ignores.

### Changed
- Scoped Scorecard SARIF permission to its job and linked private vulnerability reporting.

## [v2.10.1] - 2026-08-01

### Fixed
- Used a modern AWS HTTPS client and promoted workflow-gate repairs.
- Hardened Scorecard, pinned the CodeQL upload action, and restored fork behavior.

## [v2.10.0] - 2026-07-29

### Added
- Fork release with deterministic compaction and workspace handling.
- Generated workflow/schema parity and nine-platform build artifacts.

[unreleased]: https://github.com/KooshaPari/forgecode/compare/v2.13.21-h.0.1.1...HEAD
[v2.13.21-h.0.1.1]: https://github.com/KooshaPari/forgecode/releases/tag/v2.13.21-h.0.1.1
[v2.10.8]: https://github.com/KooshaPari/forgecode/releases/tag/v2.10.8
[v2.10.7]: https://github.com/KooshaPari/forgecode/releases/tag/v2.10.7
[v2.10.6]: https://github.com/KooshaPari/forgecode/releases/tag/v2.10.6
[v2.10.5]: https://github.com/KooshaPari/forgecode/releases/tag/v2.10.5
[v2.10.4]: https://github.com/KooshaPari/forgecode/releases/tag/v2.10.4
[v2.10.3]: https://github.com/KooshaPari/forgecode/releases/tag/v2.10.3
[v2.10.2]: https://github.com/KooshaPari/forgecode/releases/tag/v2.10.2
[v2.10.1]: https://github.com/KooshaPari/forgecode/releases/tag/v2.10.1
[v2.10.0]: https://github.com/KooshaPari/forgecode/releases/tag/v2.10.0
