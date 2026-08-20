# Contributing

Contributions are welcome. This is a Rust Cargo workspace of 39 crates (see
`Cargo.toml` `[workspace.members]`), published as `helioslite`. Read
`CLAUDE.md` and `AGENTS.md` first — they are the canonical contributor
contract (stack map, crate map, agent guidelines). If your change affects a
documented capability, reference the matching FR-ID from
[`FUNCTIONAL_REQUIREMENTS.md`](FUNCTIONAL_REQUIREMENTS.md) in your PR.

## Development setup

1. Fork and clone: `git clone https://github.com/<you>/forgecode.git`
2. Install the pinned toolchain (no manual install needed):

   ```bash
   rustup toolchain install 1.96.0   # channel pinned in rust-toolchain.toml
   ```

3. Install the dev tools used by CI:

   ```bash
   cargo install cargo-insta cargo-nextest cargo-deny
   ```

4. Alternative hermetic setup: `.devcontainer/devcontainer.json`
   (VS Code devcontainer installs fzf, fd, cargo-insta, cargo-nextest,
   ast-grep, zsh) or `nix develop` (flake.nix devShell pins the full
   toolchain including llvm-cov and protobuf).

5. Install system deps used by the workspace: `libsqlite3-dev` and
   `protobuf-compiler` (or `brew install protobuf sqlite` on macOS).

## Verify commands (all verified working)

The workspace is large — **only build/check the crates you touch**:

| Task | Command |
|---|---|
| Type-check a crate | `cargo check -p <crate>` |
| Test a crate | `cargo nextest run -p <crate>` (fallback: `cargo test -p <crate>`) |
| Snapshot tests | `cargo insta test --accept` (only when snapshot changes are intended) |
| Lint a crate | `cargo clippy -p <crate> --all-targets -- -D warnings` |
| Format | `cargo fmt --all -- --check` (check) / `cargo fmt --all` (apply) |
| Full gate (CI parity) | `cargo nextest run --all-features --workspace` with `RUSTFLAGS="-D warnings"` |
| Eval harness (TS) | `npm run eval benchmarks/evals/<name>/task.yml` (see `benchmarks/README.md`) |

Shortcuts: `just build`, `just test`, `just lint`, `just fmt`, `just ci`
(see `Justfile`); the same recipes exist in `Taskfile.yml`.

> Never run `cargo build --release` unless you need a distribution binary —
> debug builds are enough for development feedback (AGENTS.md).

## Code style

- Follow `AGENTS.md`: `anyhow::Result` + `thiserror` errors, no service-to-
  service dependencies, `derive_setters` for domain types, Rust docs (`///`)
  on all public items with `# Arguments`/`# Errors` sections and no code
  examples.
- Tests live in the same file as the source, use `pretty_assertions`,
  `fixture`/`actual`/`expected` naming, and fixtures via `new`/`Default`/
  setters. Snapshot tests use `insta`.

## Submitting changes

1. Create a feature branch from `main`.
2. Make your changes; add or update tests (including insta snapshots) and
   Rust docs.
3. Verify with the commands above on the crates you touched.
4. Open a PR using the template (`.github/PULL_REQUEST_TEMPLATE.md`): summary,
   linked FR-ID, tests run, checklist (docs updated, no secrets, CI gates).
5. CI gates that must pass: `ci`, `lint`, `test`, `cargo-deny`
   (advisories + licenses + sources), `trufflehog`. Branch protection
   requires one approval and signed commits on `main`.

## Docs

- Capability changes update `FUNCTIONAL_REQUIREMENTS.md` (status and
  acceptance criteria) and the journey manifest affected
  (`docs/journeys/manifests/`).
- Visual changes update `docs/VISUAL_SPEC.md` and the token source
  `assets/tokens.css`.
- Secrets: credentials are stored locally in `~/.forge/.credentials.json`
  (`0o600`, gitignored). Never commit credentials; use env vars or the local
  store.
- Storage (split-DB default): conversation writes go to
  `~/.forge/.forge.writes.db`; reads union the legacy `~/.forge/.forge.db`
  via the `conversations_all` TEMP VIEW (ATTACHed read-only by
  `SqliteCustomizer`). Override the write target with `FORGE_WRITE_DB_PATH`
  and the legacy read source with `FORGE_LEGACY_DB_PATH`.

## Questions

Open an issue for questions or discussions. Use the issue templates
(`.github/ISSUE_TEMPLATE/`) — feature requests, bug reports, and performance
reports have dedicated forms.
