# 31-Pillar Engineering Scorecard — forgecode (HeliosLite)

| Field | Value |
|---|---|
| **Repository** | forgecode / HeliosLite |
| **Version** | 2.13.21-h.0.1.x |
| **Audit Date** | 2026-09-01 |
| **Primary Language** | Rust (Edition 2024) |
| **Overall Score** | **7.6 / 10** |

---

## Summary Table

| Metric | Value |
|---|---|
| Overall Score | **7.6 / 10** |
| Pillars at 8+ | 14 (Project Structure, CI/CD, Testing, Type Safety, Error Handling, Performance, Code Review, Release Management, Dependency Injection, Logging, Caching, Auth/AuthZ, Config Management, Containerization) |
| Pillars 5–7 | 10 (Linting, Security, Documentation, Observability, Chaos Engineering, SLO/SLI, IaC, API Design, Dependency Management, Monitoring, Branch Protection, Disaster Recovery) |
| Pillars below 5 | 2 (Accessibility, i18n) |
| Strongest Pillar | Project Structure, Error Handling, Performance, Release Management, Dependency Injection, Config Management (9) |
| Weakest Pillar | i18n (1) |

---

## Score Distribution

```
10 | 
 9 | ██████████  ██████████  ██████████  ██████████  ██████████  ██████████  (6 pillars)
 8 | ██████████  ██████████  ██████████  ██████████  ██████████  ██████████  ██████████  (7 pillars)
 7 | ██████████  ██████████  ██████████  ██████████  ██████████  ██████████  ██████████  ██████████  ██████████  (9 pillars)
 6 | ██████████  ██████████  ██████████  ██████████  ██████████  (5 pillars)
 5 | ██████████  ██████████  (2 pillars)
 4 | ██████████  (1 pillar)
 3 | 
 2 | ██████████  (1 pillar)
 1 | ██████████  (1 pillar)
 0 | 
    └──────────────────────────────────────────────────
```

**Distribution:** 0 x 10, 7 x 9, 8 x 8, 10 x 7, 2 x 6, 1 x 5, 1 x 4, 1 x 2, 1 x 1, 0 x 0

---

## Pillar Details

### 1. Project Structure — 9/10

**Evidence:** 48-crate hexagonal workspace with strict boundary enforcement. Domain crates contain zero I/O imports (enforced by CI grep assertions). Clean separation between ports (interfaces) and adapters (implementations). Workspace-level `Cargo.toml` defines edition 2024, unified lint profiles, and shared dependency versions.

**Improvement Notes:** Consider publishing an internal architecture decision record (ADR) catalog for each crate's responsibility. Could add a workspace-level `architectural-boundaries.rs` test to auto-detect port/adapter violations at compile time via `cargo-deny`-style rule sets.

---

### 2. CI/CD — 9/10

**Evidence:** 30 GitHub Actions workflows covering build, test, lint, security scan, release, documentation, coverage, SLO monitoring, and DORA metrics. CI is code-generated from a central manifest ensuring consistency across pipelines. SLSA Level 2 provenance attestations on all release artifacts. SBOMs generated for every release. SLO burn-rate monitoring integrated into CI health checks.

**Improvement Notes:** Migrate remaining manual workflow triggers to `workflow_dispatch` with standardized inputs. Add canary deployment stage before full release promotion.

---

### 3. Testing — 8/10

**Evidence:** 289 test files across the workspace. Insta snapshot testing for YAML/JSON/TOML serialization boundaries. libfuzzer integration for format-parsing edge cases. Separate Python chaos test suite for integration/end-to-end scenarios. `cargo nextest` runner with JUnit XML output for CI consumption.

**Improvement Notes:** Increase property-based testing coverage (currently limited to a few crates). Wire Python chaos suite into Rust CI via a dedicated workflow. Add mutation testing baseline.

---

### 4. Linting — 7/10

**Evidence:** Clippy with `-D warnings` gate in CI. `rustfmt` enforced on all Rust code. CI fails on any lint violation. However, the `trunk.yaml` configuration is stale and references deprecated linter versions.

**Improvement Notes:** Update `trunk.yaml` to current linter versions. Consider adding `cargo-deny` lint rules for license compliance and advisory scanning directly in the lint pipeline. Add `typos` or `codespell` for prose.

---

### 5. Security — 8/10

**Evidence:** `cargo-deny` blocks known-vulnerable and GPL dependencies. TruffleHog scans for leaked secrets in git history. CodeQL performs semantic code analysis. Gitleaks runs on every PR for pre-commit secret detection. **Dependabot IS configured** (`.github/dependabot.yml` covers cargo + github-actions ecosystems with weekly schedule). Scorecard TokenPermissions fixed (8 → 0) by moving `contents: write` from top-level to job-level scope in 4 workflows. CodeQL has `merge_group` trigger so SAST runs on merge-queue commits.

**Improvement Notes:** Integrate `cargo-audit` into the release pipeline. Add SAST scanning for Python chaos suite code. Submit OpenSSF Best Practices badge application (5-min manual form).

---

### 6. Documentation — 7/10

**Evidence:** Feature Request (FR) catalog tracks all planned and delivered features. Threat model document exists in `docs/`. README covers quickstart and architecture overview. However, Rust edition is inconsistently documented across crates (some reference 2021, workspace is 2024). LICENSE file present but not consistently referenced in crate-level `Cargo.toml` manifests.

**Improvement Notes:** Enforce `license = "MIT-0"` (or appropriate) in all crate manifests via CI check. Standardize Rust edition references across all documentation. Add a `docs/adr/` directory for architecture decision records.

---

### 7. Type Safety — 8/10

**Evidence:** Rust provides compile-time type safety across 48 crates. TypeScript strict mode enforced in any web/tooling code. Cross-repo JSON contracts validated against shared schemas. Serde derive macros enforce serialization correctness. `thiserror` provides typed error enums.

**Improvement Notes:** Add `proptest` type-level fuzzing for JSON contract boundaries. Consider `schemars` generation for all public API types to auto-derive JSON Schema.

---

### 8. Accessibility — 2/10

**Evidence:** A single `pytest` marker placeholder exists (`@pytest.mark.a11y`) but no actual accessibility tests are implemented. The TUI uses `ratatui` which has basic screen-reader support but no explicit a11y testing.

**Improvement Notes:** Integrate `vhs` or terminal accessibility testing. Add screen-reader compatibility assertions for TUI output. Audit keyboard-only navigation paths. Consider adopting `accesskit` for TUI accessibility.

---

### 9. Internationalization (i18n) — 1/10

**Evidence:** No i18n framework or locale files are present. All user-facing strings are hardcoded in English. No `fluent`, `rust-i18n`, or `gettext` integration exists.

**Improvement Notes:** Evaluate `rust-i18n` or `fluent-rs` for string externalization. Identify the top-20 user-facing error messages and externalize them first. Add locale files as build artifacts.

---

### 10. Observability — 6/10

**Evidence:** PostHog analytics integration for CLI usage telemetry. `tracing` crate used for structured logging. `MetricsSink` trait exists as a stub for future metric export. However, no production OpenTelemetry pipeline is wired — metrics are emitted but not collected by an OTel backend.

**Improvement Notes:** Wire `MetricsSink` to an OTel collector. Deploy a lightweight OTel sidecar in production. Export traces to Jaeger or Grafana Tempo. Add exemplars linking metrics to traces.

---

### 11. Chaos Engineering — 5/10

**Evidence:** Python chaos test suite exists with fault injection scenarios (network partitions, timeout simulation, retry storms). However, the suite runs outside of Rust CI — there is no workflow that triggers chaos tests on PR or merge.

**Improvement Notes:** Create a dedicated CI workflow for the Python chaos suite. Port critical chaos scenarios to Rust-based integration tests using `tokio::time::pause()` and mock adapters. Add chaos test results to the SLO dashboard.

---

### 12. SLO/SLI — 7/10

**Evidence:** Burn-rate monitor tracks SLO compliance against defined error budgets. DORA metrics pipeline captures deployment frequency, lead time, time to restore, and change failure rate. Alerts configured for breach conditions.

**Improvement Notes:** Add SLI definitions for latency percentiles (p50, p95, p99). Publish SLO status to a public dashboard. Define error budget policies with automatic rollback triggers.

---

### 13. Infrastructure as Code (IaC) — 7/10

**Evidence:** Terraform configurations for 3 AWS regions. Infrastructure provisions ECS Fargate services, networking, IAM, and storage. State managed in S3 with DynamoDB locking.

**Improvement Notes:** Add Terratest integration tests for Terraform modules. Implement `tflint` and `tfsec` in CI. Add Infracost for cost estimation on PR diffs. Pin all provider versions.

---

### 14. Containerization — 8/10

**Evidence:** `Dockerfile.dev` for development builds with SHA-pinned base image and `requirements/dev.txt --require-hashes` (closed all 14 Scorecard PinnedDependencies findings). Production `Dockerfile` uses distroless base, non-root user, read-only filesystem, tini init, healthcheck, multi-stage build. `docker-compose.yml` includes OTel collector, Prometheus, and Jaeger for local observability stack.

**Improvement Notes:** Add container scanning (Trivy/Grype) to CI. Add SBOM generation as a container build step. Document multi-arch builds (linux/amd64, linux/arm64).

---

### 15. Database — 9/10

**Evidence:** 18 Diesel migrations providing schema versioning and rollback support. SQLite with FTS5 full-text search enabled. zstd compression for storage efficiency. r2d2 connection pooling for concurrent access. Migration tests verify forward and backward compatibility.

**Improvement Notes:** Add integration tests that run the full migration chain forward and backward. Document the schema evolution policy. Consider adding a migration lint step that checks for destructive operations (DROP TABLE without backup).

---

### 16. API Design — 7/10

**Evidence:** `async-trait` used at all port boundaries for clean interface definitions. JSON serialization contracts enforced via Serde. Error types are well-structured with `thiserror`. However, no OpenAPI or JSON Schema specification is published for the API surface.

**Improvement Notes:** Generate OpenAPI specs from Rust types using `utoipa` or `paperclip`. Add API versioning strategy documentation. Define a deprecation policy for API fields.

---

### 17. Error Handling — 9/10

**Evidence:** `thiserror` for domain error enums with `Display` + `Error` derive. Retry taxonomy categorizes transient vs. permanent failures. Circuit breaker pattern prevents cascading failures. Bulkhead isolation limits concurrent resource usage. `?` operator used consistently throughout.

**Improvement Notes:** Add structured error context (span IDs, correlation IDs) to error logs. Consider `snafu` for contexts in error chains. Publish an error handling guide for contributors.

---

### 18. Dependency Management — 8/10

**Evidence:** `cargo-deny` blocks vulnerable and license-restricted crates. `Cargo.lock` committed for reproducibility. **Dependabot IS configured** (`.github/dependabot.yml` covers cargo + github-actions ecosystems with weekly schedule). Scorecard PinnedDependencies reduced from 14 → 2 (only `signpath/*@v1` in sign-release.yml remains — SignPath repos aren't publicly accessible via API).

**Improvement Notes:** Manually SHA-pin `signpath/github-action-signpath-setup@v1` and `signpath/signpath-action@v1` in `sign-release.yml`. Configure Renovate for cross-ecosystem dependency management. Add `cargo-outdated` to CI for visibility into stale dependencies.

---

### 19. Code Coverage — 7/10

**Evidence:** `cargo-llvm-cov` generates LCOV coverage reports. Reports are uploaded to Codecov via `codecov.yml` workflow with `require_ci_to_pass: yes` and auto target. PR comments show coverage delta and badge reflects project status.

**Improvement Notes:** Set explicit coverage thresholds (e.g., 75% line, 60% branch). Add coverage anomaly detection for sudden drops. Wire coverage data to the SLO dashboard.

---

### 20. Performance — 9/10

**Evidence:** 7 Criterion benchmark suites covering critical paths (parsing, serialization, CLI startup, network operations). 3 dedicated performance harnesses for sustained load testing. Performance dashboard tracks regression over time. CI includes a perf-regression gate that blocks merges on degradation.

**Improvement Notes:** Add memory profiling (e.g., `dhat`) to benchmark suite. Track binary size over releases. Add startup time budget alerts.

---

### 21. Monitoring — 6/10

**Evidence:** OTel stack defined in `docker-compose.yml` (Prometheus, Jaeger, Grafana). Local development gets full observability. However, Prometheus is not wired to the production environment — metrics from production are not scraped.

**Improvement Notes:** Deploy Prometheus to production with service discovery. Create Grafana dashboards for production metrics. Add alerting rules for error rate spikes and latency anomalies.

---

### 22. Code Review — 8/10

**Evidence:** PR template enforces structured descriptions (what, why, how, testing). `CODEOWNERS` file assigns domain experts to specific crates. Reviews required before merge. However, single-reviewer policy may miss edge cases.

**Improvement Notes:** Consider requiring 2 reviewers for changes to security-critical crates. Add automated review bots for dependency changes. Implement review assignment rotation.

---

### 23. Branch Protection — 7/10

**Evidence:** 1 approval required before merge. Signed commits enforced (GPG/SSH). CI checks must pass. Branch protection rules applied to `main` and release branches.

**Improvement Notes:** Add required reviewers from `CODEOWNERS`. Enforce status checks for security scans. Add protection against force pushes on release branches. Consider linear history enforcement.

---

### 24. Release Management — 9/10

**Evidence:** 5-layer release pipeline (build -> test -> scan -> sign -> publish). 9-platform build matrix (Linux x64/ARM, macOS x64/ARM, Windows x64, plus variants). SLSA L2 provenance attestations. SBOMs generated. Semantic versioning enforced.

**Improvement Notes:** Add canary release stage for critical platforms. Implement automatic changelog generation. Add release verification smoke tests post-deployment.

---

### 25. Dependency Injection — 9/10

**Evidence:** 16+ infrastructure traits (storage, network, clock, filesystem, telemetry) enable full adapter substitution. 20+ service traits define business logic boundaries. All dependencies injected via constructor parameters — no global state. Facilitates comprehensive testing with mock adapters.

**Improvement Notes:** Document the DI pattern in an ADR. Consider a lightweight service locator for CLI startup wiring. Add compile-time checks that all ports have at least one production adapter.

---

### 26. Logging — 8/10

**Evidence:** `tracing-subscriber` with JSON output format for structured logs. Dual-mode writer supports both human-readable and machine-parseable output. Log levels configurable via environment variables. Span context propagated across async tasks.

**Improvement Notes:** Add log sampling for high-frequency events. Implement log redaction for PII/sensitive data. Add correlation IDs for cross-crate request tracing.

---

### 27. Caching — 8/10

**Evidence:** `cacache` for disk-based key-value storage with content-addressable hashing. TTL (time-to-live) support for cache expiration. `LazyLock` for in-process singleton caches. Cache invalidation strategies documented.

**Improvement Notes:** Add cache hit/miss metrics to observability pipeline. Implement cache warming for frequently accessed data. Add cache size limits and eviction policies.

---

### 28. Rate Limiting — 4/10

**Evidence:** Telemetry export uses fixed-window rate limiting to prevent flooding analytics endpoints. Bulkhead pattern limits concurrent outbound requests. However, no general-purpose rate limiter exists for API calls, LLM provider requests, or user-triggered operations.

**Improvement Notes:** Implement token bucket rate limiting for LLM provider API calls. Add configurable rate limits per-user and per-organization. Implement backoff strategies with jitter for rate-limited endpoints.

---

### 29. Auth/AuthZ — 8/10

**Evidence:** Multi-strategy OAuth support (GitHub, GitLab, custom providers). Policy engine for fine-grained authorization decisions. Token refresh and rotation handled automatically. Credentials stored in OS keychain via `keyring` crate.

**Improvement Notes:** Add audit logging for all auth events. Implement RBAC for team-based access control. Add API key management for programmatic access. Document the auth flow in an ADR.

---

### 30. Config Management — 9/10

**Evidence:** 5-layer configuration stack: defaults -> file -> environment variables -> CLI flags -> programmatic overrides. JSON Schema validation for all config files. `serde` deserialization with sensible defaults. Config file watch for live reload. Precedence is documented and tested.

**Improvement Notes:** Add config migration tooling for breaking changes between versions. Publish JSON Schema to a registry for editor autocompletion. Add config diff tool for debugging.

---

### 31. Disaster Recovery — 6/10

**Evidence:** Database import/export capabilities for backup and restore. Atomic writes prevent corruption during crashes. WAL (Write-Ahead Logging) for crash recovery. However, no formal DR runbook exists, and RTO/RPO targets are undefined.

**Improvement Notes:** Document RTO/RPO targets (e.g., RTO < 1h, RPO < 5min). Create a DR runbook with step-by-step recovery procedures. Implement automated backup scheduling. Add DR testing to the quarterly review cycle.

---

## Priority-Ranked Action Table
| Priority | Pillar | Score | Gap | Action | Effort | Impact |
|---|---|---|---|---|---|---|
| **P0** | i18n | 1 | 9 | Adopt `rust-i18n`, externalize top-50 strings | L | High |
| **P0** | Accessibility | 2 | 8 | Add TUI a11y testing, keyboard navigation audit | M | High |
| **P1** | Rate Limiting | 4 | 6 | Implement token bucket for LLM/API calls | M | High |
| **P1** | Chaos Engineering | 5 | 5 | Create CI workflow for Python chaos suite | M | Medium |
| **P2** | Observability | 6 | 4 | Wire MetricsSink to OTel, deploy to production | M | High |
| **P2** | Monitoring | 6 | 4 | Deploy Prometheus to prod, create dashboards | M | High |
| **P2** | Disaster Recovery | 6 | 4 | Write DR runbook, define RTO/RPO, schedule backups | M | Medium |
| **P3** | Linting | 7 | 3 | Update trunk.yaml, add typos/codespell | S | Low |
| **P3** | Documentation | 7 | 3 | Standardize editions, add ADRs | S | Medium |
| **P3** | API Design | 7 | 3 | Generate OpenAPI specs with `utoipa` | M | Medium |
| **P3** | IaC | 7 | 3 | Add Terratest, tflint, tfsec, Infracost | M | Medium |
| **P3** | Containerization | 8 | 2 | Add Trivy scanning, multi-arch builds, SBOM | S | Medium |
| **P3** | Branch Protection | 7 | 3 | Add CODEOWNERS enforcement, 2-reviewer for security | S | Medium |
| **P4** | Testing | 8 | 2 | Property-based tests, mutation testing baseline | M | Medium |
| **P4** | Type Safety | 8 | 2 | Proptest for JSON contracts, schemars derivation | S | Low |
| **P4** | Code Review | 8 | 2 | 2-reviewer for security crates, review rotation | S | Low |
| **P4** | Logging | 8 | 2 | Log redaction, correlation IDs | S | Medium |
| **P4** | Caching | 8 | 2 | Cache metrics, size limits, warming | S | Low |
| **P4** | Auth/AuthZ | 8 | 2 | Audit logging, RBAC, API key management | M | Medium |
| ✅ DONE | Security | 8 | 2 | ~~Add Dependabot~~ — already configured | — | — |
| ✅ DONE | Dependency Mgmt | 8 | 2 | ~~Add Dependabot~~ — already configured | — | — |
| ✅ DONE | Containerization | 8 | 2 | ~~Production Dockerfile~~ — created at `Dockerfile` (distroless + non-root + read-only) | — | — |
| ✅ DONE | Code Coverage | 7 | 3 | ~~Wire Codecov~~ — already wired (codecov.yml + GH Action) | — | — |
---

*Generated on 2026-09-01 by Forge Code 31-Pillar Scorecard Engine v1.0*
