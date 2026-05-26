# Term Reverse Index + Function Similarity Lattice

## Objective

Build a system that discovers, fingerprints, and indexes all Rust function declarations across AI-agent codebases, merges structurally similar functions into a lattice via equivalence classes, and produces a term reverse index (term → list of declaring files) for semantic search and code reuse analysis.

## Existing Infrastructure (Reusable)

The following tools and data already exist and will be leveraged directly:

| Asset | Location | What It Provides |
|---|---|---|
| `syn`-based `decl-splitter` | `forgecode-decl-splitter/tools/decl_splitter/src/main.rs:1-402` | Parses `.rs` files into individual declaration files (fn, struct, enum, trait, impl, const, static, type, union) |
| `DeclFingerprint` | `forgecode-decl-splitter/tools/decl_splitter/src/fingerprint.rs:9-89` | Canonical structural fingerprint: kind, crates_used, structural_shape (hash of AST shape), trait_bounds, visibility_class, qualifiers |
| `PatternFingerprint` with `UsageKind` | `forgecode-decl-splitter/tools/decl_splitter/src/patterns.rs:17-118` | Crate usage patterns: Derive, ImportPath, TraitImpl, FunctionReturn, FieldType, MethodCall |
| `IdentCollector` + `UseCrateCollector` | `forgecode-decl-splitter/tools/decl_splitter/src/lattice.rs:81-154` | AST visitors extracting identifiers and external crate names from declarations |
| `decl-lattice` builder | `forgecode-decl-splitter/tools/decl_splitter/src/lattice.rs:308-430` | Topological sort, SCC detection, layer computation over declaration dependency graphs |
| `build_lattice.sh` | `cargo-vendormod/.forge/skills/decl-lattice/scripts/build_lattice.sh` | One-click: split + graph build + lattice generation for any Rust project |
| Existing `decl_lattice/` | `cargo-vendormod/decl_lattice/` | 508 micro-crates with 2,970 dependency edges (cargo-vendormod only) |
| Existing `global_graph/` | `cargo-vendormod/global_graph/` | 4,653 nodes, 22,304 edges across 38 projects |

## Implementation Plan

### Discovery & Background

The system will be built as a new binary in the `decl_splitter` tool workspace (alongside `decl-splitter`, `decl-lattice`, `decl-patterns`). It will reuse all existing syn-based visitors and fingerprint structures.

The pipeline has three stages that feed into each other: split and fingerprint source files into declarative fingerprints, group those fingerprints into a similarity lattice based on equivalence classes, and build a term reverse index from all identifiers across all declarations. Stage 1 reuses the existing `decl-splitter` split logic and adds fingerprint computation. Stage 2 groups fingerprints into equivalence classes (isomorphic functions) and builds a lattice layer from those groups. Stage 3 walks all declarations and builds an inverted index mapping every identifier/term to the list of declarations that reference it.

### Files to Create

- [ ] Create `src/term_index.rs` — New module in the `decl_splitter` workspace containing the term reverse index logic. Defines a `TermIndex` struct that maps normalized terms (identifiers, type names, crate names, method names, qualifier keywords) to `Vec<DeclLocation>` entries. Each `DeclLocation` stores the file path, declaration name, kind, project name, and fingerprint hash. The module exposes the `build_term_index` function which takes a directory of split decl files and returns the populated `TermIndex`. Uses the existing `IdentCollector` and `UseCrateCollector` AST visitors from `src/lattice.rs` to extract terms from each declaration. Normalizes terms by lowercasing and stripping common suffixes/prefixes. Deduplicates within each document so a term appears once per decl.

- [ ] Create `src/merge_index.rs` — New module containing the similarity-based lattice merge logic. Defines a `SimilarityLattice` struct where each node is a `FunctionEquivalenceClass`: a group of declarations sharing the same `DeclFingerprint` (from `src/fingerprint.rs`). The `structural_shape` hash serves as the primary equivalence key; declarations with identical shape + same `crates_used` set are grouped together. The module exposes the `build_similarity_lattice` function which: (a) loads all fingerprint JSON files, (b) groups by structural_shape and crates_used tuple, (c) discards singletons (unique functions), (d) sorts equivalence classes by size descending, (e) assigns each class to a lattice layer based on topological depth of its declaration dependencies, (f) serializes the lattice to `similarity_lattice.json`. Uses the existing `decl-lattice` layering logic from `src/lattice.rs:381-415`.

- [ ] Create `src/bin/term-index.rs` — New CLI binary entry point. Accepts arguments: `--decls-dir` (directory of split decl files), `--fingerprints-dir` (directory with fingerprint JSON files), `--output` (output directory). Orchestrates the three-stage pipeline: runs `build_term_index`, writes `term_index.json`; runs `build_similarity_lattice`, writes `similarity_lattice.json`. Reports summary statistics: total declarations, unique terms, equivalence classes, largest equivalence class, terms per document average.

- [ ] Update `Cargo.toml` — Register the new `[[bin]]` entry for `term-index` at `src/bin/term-index.rs`. No new dependencies needed — all required crates (`syn`, `serde`, `serde_json`, `petgraph`, `walkdir`, `anyhow`, `clap`) are already listed in the workspace `Cargo.toml` at `forgecode-decl-splitter/tools/decl_splitter/Cargo.toml:1-34`.

## Data Model

### TermIndex Output (`term_index.json`)

The term index is a JSON object with an `index` map and `stats` section. Each key in the index map is a normalized term string (lowercased identifier). The value for each term is an array of declaration location objects, each containing the file path, declaration name, declaration kind, and project name. The stats section records total declarations indexed, unique terms discovered, and distribution metrics like max and median documents per term.

### SimilarityLattice Output (`similarity_lattice.json`)

The similarity lattice is a JSON object with three sections: an `equivalence_classes` array, a `layers` array, and a `stats` section. Each equivalence class has a unique identifier, the shape hash that defines the equivalence, an array of member declarations (file path, declaration name, project name), a lattice layer assignment, and an isomorphism flag. The layers section maps lattice depth to the list of equivalence class identifiers at that depth. The stats section reports total declarations processed, number of equivalence classes found, number of singletons discarded, and the size of the largest class.

## Verification Criteria

- [ ] `cargo build --bin term-index` compiles cleanly with no errors
- [ ] Running `term-index --decls-dir /tmp/decls_forge_domain --fingerprints-dir /tmp/fingerprints --output /tmp/term-index-output` produces valid `term_index.json` with at least 100 unique terms
- [ ] verified: the index has string keys, each mapping to an array of objects with file, decl, kind, and project fields
- [ ] `similarity_lattice.json` contains at least 1 equivalence class with 2+ members (isomorphic functions across projects)
- [ ] Running the tool on all available decl archives (forge_domain, pi_agent, cargo-vendormod's 508 decls) produces index entries for every declaration
- [ ] No regressions in existing binaries (`decl-splitter`, `decl-lattice`, `decl-patterns`) — all still compile and pass existing tests

## Potential Risks and Mitigations

1. **Fingerprint hash collisions**: Two structurally different declarations could produce the same `structural_shape` hash. Mitigation: use the full DeclFingerprint tuple of kind, crates_used, shape_hash, trait_bounds, visibility, and qualifiers as the equivalence key, not just the hash alone. The hash is a 64-bit value from `std::hash::DefaultHasher` which has a 2^-64 collision probability per pair — negligible at this scale.

2. **Index size blowup**: A large codebase (arti-tor-rs has 7,335 decls) could produce a term index with millions of entries. Mitigation: deduplicate terms per document (one entry per term per decl), use compact JSON encoding, and consider streaming serialization for very large outputs. The existing decl archives from cargo-vendormod (947,362 files) already demonstrate the system handles this scale.

3. **Equivalence class sparsity**: Most functions might be unique (singletons), producing few meaningful equivalence classes. Mitigation: the pipeline discards singletons by design and only reports groups of 2+. If the ratio is too low, the fingerprint granularity can be reduced (e.g., ignore `visibility_class` or `qualifiers` for a looser grouping). The existing pattern matcher at `patterns.rs:340-346` already implements overlap scoring that can be used for fuzzy matching if exact matches are insufficient.

## Alternative Approaches

1. **Fuzzy fingerprint matching**: Instead of exact `(shape_hash, crates_used)` equivalence, use Jaccard similarity on crate usage sets to find near-isomorphic functions. Trade-off: catches more matches but introduces a threshold parameter. The existing `overlap_score` function at `patterns.rs:452-461` provides a foundation for this.

2. **Full-text search index**: Instead of an AST-level term index, use a simple `rg`/grep-based inverted index built from raw source text. Trade-off: faster to build but misses type-aware information (e.g., cannot distinguish `use serde::Serialize` from a variable named `serde`). The AST-based approach is more precise.

3. **Database-backed index**: Instead of JSON files, use SQLite for the term index and equivalence lattice. Trade-off: enables efficient queries (WHERE term = 'serde' AND project = 'forge_domain') but adds a build dependency. JSON is simpler for initial implementation and sufficient for the target scale (< 100K decls).
