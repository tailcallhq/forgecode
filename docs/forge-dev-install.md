# forge-dev install guide

`forge-dev` is the Phenotype build of `KooshaPari/forgecode`, the
`tailcallhq/forge` fork maintained by `@KooshaPari`. It is shipped as a
side-by-side binary so operators can keep the upstream `forge` release on
`$PATH` while exercising fork-only features (`bifrost-routing`, `forge_tui`,
the `~/.forge-dev/` config directory, and the `https://forge-dev.sh/cli`
auto-update endpoint). The `forge-dev` `[[bin]]` target in
`crates/forge_main/Cargo.toml` is gated behind the `dev-binary` Cargo
feature, so a plain `cargo install` from this repo produces the unchanged
`forge` artifact -- opting into the Phenotype build requires the
`--features dev-binary` flag. After install, `forge-dev --version` reports
the fork version, configuration lives under `~/.forge-dev/` (override via
the `FORGE_DEV_CONFIG` environment variable), and the auto-update endpoint
resolves to `https://forge-dev.sh/cli`, so the install collides with
neither the upstream `forge` binary on `$PATH` nor with an existing
`~/.forge/` upstream config directory.

Install the Phenotype binary directly from the git fork by enabling the
`dev-binary` feature and selecting the `forge-dev` target. This `cargo
install` invocation places the binary at `~/.cargo/bin/forge-dev`, isolated
from any upstream `forge` install on the same machine:

```bash
cargo install --git https://github.com/KooshaPari/forgecode \
  --features dev-binary --bin forge-dev
```
