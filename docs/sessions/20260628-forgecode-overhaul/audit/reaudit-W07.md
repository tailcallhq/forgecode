# forgecode — W07 RE-AUDIT (L18 / L19 / L20 / L28)

> Re-scored against CURRENT clean code at `/tmp/fc-mainclean` @ `7c23c9cd5`
> (clean origin/main). Baseline cluster mean **1.38/3.0**. Pillars: L18 (secrets),
> L19 (supply-chain), L20 (threat model), L28 (dependency hygiene).
> **Verification-only — no source modified.** All citations are real, file-checked.

---

## L18 — Secrets Handling: 1.5 → **2.5** (LIFT +1.0)

**Landed (verified real):**

- **L18-1 — Debug leak FIXED.** `ApiKey`, `AccessToken`, `RefreshToken`,
  `AuthorizationCode`, `DeviceCode`, `PkceVerifier`, `State` all replace the raw
  `#[derive(Debug)]` with hand-written `impl std::fmt::Debug` emitting
  `<redacted>` — `crates/forge_domain/src/auth/new_types.rs:9-11, 53-55, 63-65,
  73-75, 167-169, 177-179, 196-198`. Composite types (`AuthCredential`,
  `AuthDetails`, `OAuthTokens` — `credentials.rs:9, 85, 119`) still
  `#[derive(Debug)]`, but every secret-bearing **field** they hold is now a
  redacting newtype, so derived `Debug` recursion prints `<redacted>` — the leak
  is structurally closed. Regression tests added and assert no plaintext +
  presence of `<redacted>`: `new_types.rs:216-260` (api_key/access_token/
  refresh_token/pkce_verifier debug-redaction tests).
- **L18-2 — credential file gitignored.** `.gitignore:76` lists
  `.credentials.json`; `git check-ignore .credentials.json` passes.

**Residual gap (why not 3.0):** No OS-keychain backend (`grep keyring|secret-service`
→ still NONE); no rotation/expiry enforcement for long-lived `ApiKey`
(`credentials.rs` `needs_refresh` → `false` for `ApiKey`). Plaintext-at-`0o600`
remains the only at-rest control. The most security-relevant gap (Debug leak) is
gone, so this clears "partial" into strong-but-incomplete.

---

## L19 — Supply-Chain Integrity: 2.0 → **2.0** (NO CHANGE)

**Verified unchanged.** No SBOM tooling added:
`grep -rilE "sbom|cyclonedx|spdx|syft" .github/` → NONE; `release-attestation.yml`
attests SLSA Build L2 provenance only (line 83-84), no `attest-sbom` step;
`docs/slsa.md` still does not mention SBOM. No continuous vuln scan:
`cargo-deny.yml:6-11` triggers are `push`/`pull_request`/`workflow_dispatch`
only — **no `schedule:`/cron**; no OSV/Trivy/Grype job. Matches task note
("if not added, no change"). SLSA L2 provenance + `cargo-deny` advisory gate keep
it at a solid 2.0.

---

## L20 — Threat Model / Trust Boundaries: 0.5 → **2.5** (LIFT +2.0, highest leverage)

**Landed (verified real):** `docs/security/threat-model.md` exists — **16.9 KB**,
real STRIDE threat model (not a stub). Confirmed contents:

- 4 named adversaries (A1 local malware/users, A2 malicious remote endpoint,
  A3 prompt-injection model, A4 passive network observer) with explicit
  out-of-scope section.
- **5 attack surfaces** with real crate/file citations: S1 credential store
  (`provider_repo.rs`, `env.rs`, `mcp_credentials.rs`), S2 prompt-injection→tool/
  subprocess exec (`tool_registry.rs`, `forge_pheno_shell`, `mcp_client.rs`),
  S3 MCP server trust (`mcp_client.rs`, `mcp.rs`), S4 telemetry egress
  (`forge_tracker/.../posthog.rs`, `dispatch.rs`, `can_track.rs`), S5 ZSH plugin
  (`shell-plugin/*.zsh`). Plus `forge_dbd` local-daemon surface noted inline.
- Full per-surface STRIDE breakdown (26 STRIDE/boundary keyword hits, 22 section
  headers). S1 shows all six STRIDE categories with the primary risk
  (info-disclosure of long-lived tokens) called out.

This addresses both L20-1 (threat model authored) and L20-2 (prompt-injection→exec
boundary documented as S2).

**Residual gap (why not 3.0):** L20-2's underlying *control* (enforced allowlist/
confirmation for model-driven destructive tool calls) is documented as a surface
but remains a tracked gap, not enforced code; propagated `docs/boundary/forgecode.md`
stub status not re-verified here. Documentation is comprehensive and cited, so this
is a major lift from the 0.5 anchor.

---

## L28 — Dependency Hygiene: 1.5 → **2.5** (LIFT +1.0)

**Landed (verified real):**

- **L28-1 — bots deduped.** `renovate.json` **removed** (absent at repo root and
  `.github/`); `grep -rniE automerge|renovate .github/ → NONE`. Only
  `.github/dependabot.yml` remains: weekly cargo + github-actions, grouped by
  major/minor/patch, **no automerge** anywhere. Blanket-automerge supply-chain
  risk eliminated.
- **L28-2 — advisory ignores reasoned.** All 9 `deny.toml` ignores
  (lines 31-43) now carry a `reason = "..."`, including the 5 previously
  unreasoned (`RUSTSEC-2026-0118/0119/0098/0099/0104`). Stale
  `RUSTSEC-2026-0049` documented as already removed (line 44).

**Residual gap (why not 3.0):** Reasons are present but not linked to tracking-issue
URLs; no dated/quarterly review cadence committed; `[advisories] unmaintained =
"workspace"` scoping not added. Single clear bot strategy + fully-reasoned ignores
clear the core findings.

---

## Score summary

| Pillar | Old | New | Δ | Status |
|--------|-----|-----|-----|--------|
| L18 | 1.5 | 2.5 | +1.0 | LANDED (Debug redaction + tests, .credentials.json gitignored) |
| L19 | 2.0 | 2.0 | 0.0 | UNCHANGED (no SBOM, no scheduled/OSV scan) |
| L20 | 0.5 | 2.5 | +2.0 | LANDED (16.9KB STRIDE threat model, 5 surfaces, cited) |
| L28 | 1.5 | 2.5 | +1.0 | LANDED (Renovate removed, no automerge, all ignores reasoned) |

**New mean = (2.5 + 2.0 + 2.5 + 2.5) / 4 = 2.375**

Cluster mean **1.38 → 2.375** (+0.995). Confirmed pillar LIFT.

### Missing (open follow-ups)
- L18: OS-keychain backend, ApiKey rotation/expiry.
- L19: CycloneDX SBOM emission + `actions/attest-sbom`; scheduled OSV/cargo-deny re-scan.
- L20: enforce (not just document) destructive-tool confirmation/allowlist (S2);
  re-fill propagated boundary stub.
- L28: tracking-issue URLs on ignores + dated review cadence.

CLUSTER_DONE W07 repo=forgecode pillars=L18,L19,L20,L28 mean=2.375
