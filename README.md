# ForgeCode Evals

TypeScript evaluation and bounty-cli tooling for the ForgeCode ecosystem.

## Status

| Check | State |
|-------|-------|
| Default branch | `main` |
| CI | ![CI](https://github.com/KooshaPari/forgecode/actions/workflows/ci.yml/badge.svg) |
| License | MIT / Apache-2.0 |

## Architecture

Hexagonal (ports-and-adapters) layout:

```
src/
  domain/      — Evaluation models, scoring logic, bounty rules
  ports/       — Trait definitions (provider, storage, notifier)
  adapters/    — GitHub API adapter, CSV adapter, CLI adapter
  app/         — Composition root (wires adapters to domain)
```

## Quick Start

```sh
# Install dependencies
npm install

# Run evaluation
just eval

# Run bounty tests
just test

# Lint & format
just lint
```

## Stack

- TypeScript / Node 20
- Testing: Node built-in test runner
- CI: GitHub Actions

## License

Dual-licensed under MIT or Apache-2.0 at your option.
