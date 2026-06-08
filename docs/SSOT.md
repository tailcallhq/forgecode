# SSOT — ForgeCode Evals

## State
- Default branch: main
- Last verified: 2026-06-08
- CI status: green
- Open PRs: 0 (upstream PRs tracked separately)
- Open branches: 1 (main)
- Stashes: 0

## Dependencies
- Rust: N/A
- Node: 20
- Python: N/A

## Architecture
- Hexagonal: yes (in progress)
- Ports: ProviderPort, StoragePort, NotifierPort
- Adapters: GithubApiAdapter, CsvAdapter, ConsoleNotifier
- Domain: ScoringEngine, EvaluationModel, BountyRule

## Next Steps (DAG)
1. [x] P0: State unification (stash dropped, devcontainer branch merged)
2. [x] P1: Tooling + governance (README, LICENSE, Justfile, CI)
3. [x] P2: Hexagonal refactor (src/domain, src/ports, src/adapters, src/app)
4. [x] P3: Tests (domain tests)
5. [ ] P4: Migrate benchmarks/cli.ts to adapters
6. [ ] P5: Add schema validation (zod) for eval inputs

## Fleet Links
- Parent: Phenotype
- Related: ForgeCode (upstream fork)
- Consumes: N/A
- Merged into: N/A
