# W12 Re-Audit — forgecode (L34 / L35 / L36 / L37)

**Repo audited:** `/tmp/fc-mainclean` (clean `origin/main` @ `7c23c9cd5`)
**Date:** 2026-06-28
**Baseline:** `.audit-run-v37/out/forgecode/W12.md` — mean **1.15** (L34 1.0 / L35 2.2 / L36 2.1 / L37 0.3)
**Method:** Verify each claimed landing against current code, then re-score. All citations verified live; no fabrication.

---

## Verification of claimed landings

| Claim | Verified? | Evidence (real paths, clean checkout) |
|-------|-----------|----------------------------------------|
| L34: README de-forked to Rust-first | ✅ | `README.md:1-3` titles it an "agentic coding CLI/TUI … built in Rust"; `:18` "Cargo workspace of 33 crates … hexagonal"; crate-map table real. No TS/npm-as-primary. |
| L34: SSOT de-forked | ✅ | `docs/SSOT.md:1-21` "Rust: 2021 edition (33 crates)", "Node: N/A (no JS/TS product code)", `Last verified: 2026-06-28`, hexagonal layout with real crates. |
| L34: P4 docs (threat-model, operations) | ✅ | `docs/security/threat-model.md` (319 L), `docs/operations/runbook.md` (207 L), `docs/operations/slo.md` (82 L), `postmortem-template.md`, `iconography/`. Substantive, not stubs. |
| L36: cargo Justfile | ✅ | `Justfile:1` "Rust Cargo workspace"; `build/release/run/test (nextest)/lint (clippy -D warnings + fmt)/fmt` all `cargo`. No npm. |
| L36: blocking CI gates | ✅ | `.github/workflows/lint.yml` — `fmt` job (`cargo fmt --all -- --check`) + `clippy` job (`cargo clippy --all-targets --all-features -- -D warnings`, `RUSTFLAGS=-D warnings`). `.github/workflows/test.yml` — `cargo nextest run --all-features --workspace`. Both trigger on PR + push to main. |
| L37: production `unimplemented!()` implemented | ✅ | `crates/forge_repo/src/provider/openai_responses/repository.rs` — `http_delete()` now `Ok(self.client.delete(url.clone()).send().await?)`. Zero `unimplemented!()` remain in that file. |
| L37: NoopIntentExtractor true no-op | ✅ | `crates/forge_domain/src/intent.rs:110-132` — `extract_intent` returns `Ok(ExtractedIntent{episodic:Null,…})`, `verify_extraction` returns `Ok(false)`. No longer errors. Doc-comment states it's a placeholder that succeeds with identity results. |
| L37: dead `ghostty-kit` removed | ✅ | `ghostty-kit/` directory gone; no `ghostty` reference in `Cargo.toml`. |
| L37: `forge_dbd` WIP-marked | ✅ | `crates/forge_dbd/README.md:1-3` "# forge_dbd — WIP … not yet wired into the main application"; `server.rs:285,290` TODOs labelled stub. Still a workspace member (`Cargo.toml:7`). |
| L35: NOT addressed (P5, sponsor-gated) | ✅ (confirmed unchanged) | No `phenotype-provider-models`/`phenotype-oauth`/`phenotype-resilience` crate; cross-repo dup persists as before. Correct — deferred to P5. |

**Residual stub signals (still open):**
- L34/L37: `docs/journeys/manifests/` still effectively empty (only a 20-byte `README.md`); `docs/operations/journey-traceability.md:9-14` still 0/4 checklist.
- L37: `docs/intent/forgecode.md:17` and `docs/boundary/forgecode.md:15,19,24` still contain `TODO` markers (these are registry-propagated stubs, `do-not-edit-locally`).
- L37: all other `unimplemented!()` hits across crates verified under `#[cfg(test)]`/Mock impls (e.g. `orch_runner.rs:208` is a test `Runner` that also uses `panic!("No mock…")`; `openai.rs`, `discovery.rs`, etc. all inside `mod tests`) — acceptable, not production stubs.
- L34: zero media richness — no `mermaid` blocks in `docs/`, no committed image/video assets (`*.png/svg/gif/mp4/webm`).

---

## Re-scored pillars

### L34 — Docs / Journeys / Media Richness: **1.0 → 2.0** (LIFT +1.0)
README + SSOT now correctly Rust-first (the single largest deficit — wrong-project docs — is gone); index/precedence and a real P4 doc surface (threat-model 319L, runbook, slo, postmortem) added. Held back from 2.5+ by: zero media (no diagrams/screenshots/casts) and empty journey manifests (0/4 traceability). Docs are now *correct and reasonably deep* but not *rich*.

### L35 — Meta-Ecosystem / Shared-Code: **2.2 → 2.2** (NO CHANGE — deferred to P5)
Intra-repo layering remains clean (`forge_domain`/`forge_infra`/`forge_config` reused foundation). Cross-repo extraction (provider-models, OAuth, resilience/SSE) is sponsor-gated P5 and intentionally untouched. Score correctly unchanged.

### L36 — Quality-Polish / QOL: **2.1 → 2.6** (LIFT +0.5)
Justfile now drives the actual Rust product; blocking `fmt` + `clippy -D warnings` + `nextest` CI gates landed on standard Linux runners. This converts previously-unenforced `clippy.toml`/`.rustfmt.toml` into hard gates. Held below 3.0 by remaining L36-4 debt not in this scope (unwrap density; `--color`/`NO_COLOR` honoring) and the historical `panic!` on bad `--directory` not re-verified as fixed in this pass.

### L37 — Stub / Empty / In-Progress Detection: **0.3 → 2.3** (LIFT +2.0)
The four production-grade stub signals are cleared: the only production `unimplemented!()` is implemented; `NoopIntentExtractor` is a true no-op (no longer errors at runtime); dead `ghostty-kit` removed; `forge_dbd` explicitly WIP-marked (honest in-progress, not silent). All other `unimplemented!()` confirmed test-only. Held below 3.0 by remaining `TODO` governance stubs (`docs/intent`, `docs/boundary` — registry-propagated, do-not-edit-locally) and empty journey manifests.

---

## Mean

| Pillar | Old | New | Δ |
|--------|-----|-----|---|
| L34 | 1.0 | 2.0 | +1.0 |
| L35 | 2.2 | 2.2 | 0 |
| L36 | 2.1 | 2.6 | +0.5 |
| L37 | 0.3 | 2.3 | +2.0 |
| **Mean** | **1.15** | **2.275** | **+1.125** |

Cluster mean nearly doubled (1.15 → 2.28). PILLAR LIFT confirmed on L34, L36, L37; L35 correctly flat (P5 sponsor-gated).

CLUSTER_DONE W12 repo=forgecode pillars=L34,L35,L36,L37 baseline_mean=1.15 mean=2.28
