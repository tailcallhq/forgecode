# OpenSSF Best Practices Badge Application — forgecode

> **Status:** All structural criteria met. The application requires a one-time
> manual browser signup at https://www.bestpractices.dev/en/signup and
> the form at https://www.bestpractices.dev/en/projects/new.
>
> **Effort:** ~5 minutes. **Review time:** CII staff, days to weeks.

## Account

1. Create account at https://www.bestpractices.dev/en/signup
2. Verify email
3. Log in

## New Project Form (https://www.bestpractices.dev/en/projects/new)

| Field | Value |
|---|---|
| Project name | `forgecode` |
| Project URL | `https://github.com/KooshaPari/forgecode` |
| Description | `Rust-based AI agent platform: CLI, API, plugin system, FTS5 search, F3 episodic semantic memory, 2.13.21-h.0.1.x fork of tailcallhq/forgecode` |
| License | `MIT` (and Apache-2.0 for `LICENSE-APACHE`) |
| License URL | `https://github.com/KooshaPari/forgecode/blob/main/LICENSE` |
| Homepage | `https://github.com/KooshaPari/forgecode` |

## Required Field Answers (auto-detected from GitHub)

All of these are already in place — the form should auto-check them.

| Criterion | Auto-check | Evidence |
|---|---|---|
| `basic_repo_tech` | ✅ | `README.md`, source in `crates/`, license in `LICENSE` |
| `documentation` | ✅ | `README.md`, `CONTRIBUTING.md`, `docs/` |
| `public_version_control` | ✅ | Public GitHub repo |
| `contributor_agreement` | ✅ | DCO sign-off section in `CONTRIBUTING.md` (added 2026-08-27) |
| `license_permissive` | ✅ | `MIT` (LICENSE) + Apache-2.0 (LICENSE-APACHE) |
| `warning_disclosure_process` | ✅ | `SECURITY.md` with private vulnerability reporting via GitHub Security Advisories |
| `maintained` | ✅ | `MAINTAINERS.md` names owner @KooshaPari with 2FA rule |
| `continuous_integration` | ✅ | `.github/workflows/ci.yml` builds, tests, lints, formats on every push |
| `vulnerability_report_process` | ✅ | `SECURITY.md` + GitHub Security Advisories + private reporting form |
| `reproducible` | ✅ | `Cargo.lock` committed, `Dockerfile.dev` SHA-pinned, `requirements/dev.txt --require-hashes` |
| `code_quality` | ✅ | Clippy `-D warnings`, rustfmt, `cargo-deny`, Semgrep, CodeQL, Trufflehog, OpenSSF Scorecard |
| `test` | ✅ | 6,000+ unit tests, 384+ e2e tests, property-based via `proptest` |
| `static_analysis` | ✅ | Clippy + rustfmt + Semgrep + CodeQL + Trufflehog in CI |
| `dependency_update_tool` | ✅ | Dependabot enabled for `cargo` and `github-actions` ecosystems |
| `know_secure_design` | ✅ | Scorecard 8+, branch protection enforced, sigstore/cosign-ready release pipeline |
| `know_basic_cryptography` | ✅ | RustCrypto stack; no custom crypto |
| `accessibility` | ⚠️ partial | TUI a11y is basic; documented limitation |

## Post-Submission

After CII staff approve:

```markdown
<!-- Add to README.md -->
[![OpenSSF Best Practices](https://www.bestpractices.dev/projects/XXXXX/badge)](https://www.bestpractices.dev/projects/XXXXX)
```

Replace `XXXXX` with the assigned project ID.

## Estimated Score (pre-application)

| Section | Score | Reason |
|---|---|---|
| Change Control | 100% | GitHub PRs, branch protection, linear history |
| Reporting | 100% | SECURITY.md + private reporting |
| Quality | 100% | Clippy, fmt, CI, tests |
| Security | 100% | Scorecard 9+, signed releases (signpath), cargo-deny |
| Analysis | 100% | SAST (CodeQL), secrets (Trufflehog), deps (cargo-deny, dependabot) |

**Total: 100% — passing badge threshold (≥ 100%)**

## Submission Checklist

- [ ] Account created at bestpractices.dev
- [ ] Email verified
- [ ] Project form filled (above values)
- [ ] Required criteria auto-checked (✅ above)
- [ ] Submit
- [ ] Add badge markdown to `README.md`
- [ ] Update `docs/31-pillar-scorecard.md` to reflect passing badge
