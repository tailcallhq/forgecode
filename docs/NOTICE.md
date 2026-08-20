# NOTICE

This product includes software developed by third parties.

## Upstream

```
Forgecode / Forge — AI agentic coding CLI
Copyright (c) Tarek Ziadé and contributors

This product is a fork of Forgecode (https://github.com/tailcallhq/forgecode),
distributed under the MIT License. The full MIT license text is reproduced
in the LICENSE file at the root of this repository.

The fork (HeliosLite) is maintained by KooshaPari / Phenotype.
Copyright (c) 2026 KooshaPari / Phenotype.
```

## Redirect chain

HeliosLite is the renamed continuation of this fork. The chain is:

```
tailcallhq/forgecode   (upstream, MIT)
        ↓  merge-up + fork (2025-2026)
KooshaPari/forgecode   (preserved identifiers, additive rename policy)
        ↓  in-place rename (2026)
KooshaPari/heliosLite  (this repo, all publish surface flipped)
```

Internal source identifiers (`forge-dev`, `forge`, `FORGE_API_KEY`,
`FORGE_LOG`) are preserved as legacy aliases to keep upstream merges
tractable. The new canonical surface is `helioslite`, `HeliosLite`,
`HELIOSLITE_API_KEY`, `HELIOSLITE_LOG`. See [`docs/FORK.md`](./FORK.md)
§ 3 for the additive-rename policy and
[`docs/RENAMES-STRATEGY.md`](./RENAMES-STRATEGY.md) for the migration
guide.

## Trademarks

- **HeliosLite** is a trademark of KooshaPari / Phenotype.
- **Phenotype.** is a trademark of KooshaPari / Phenotype. The period is
  used for stylistic consistency; the legal entity name omits it.
- **Forgecode** / **Forge** are trademarks of their respective owners;
  this fork uses the marks only to identify upstream provenance.
- **KooshaPari** is the personal moniker of the maintainer.

## Third-party dependencies

All third-party dependencies are listed in `Cargo.toml` and locked in
`Cargo.lock`. SBOMs are generated at build time via `cargo-cyclonedx`.
Each dependency retains its own license — see `THIRD-PARTY-NOTICES.md`
(generated) for the full attribution text.

## AI-DD / HITL-less disclosure

HeliosLite is developed with AI-Driven Development (AI-DD) and runs
without a Human-in-the-Loop (HITL) review gate on a routine basis. See
[`docs/FORK.md`](./FORK.md) § 2 for the full disclosure.

## License compatibility

This fork is distributed under the MIT License, compatible with upstream
and with the dependency set as recorded in [`docs/security/`](./security).