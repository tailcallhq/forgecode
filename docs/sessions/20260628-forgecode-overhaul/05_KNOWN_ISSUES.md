# forgecode Overhaul — Known Issues (live bugs found during audit)

These are concrete defects in current `main`, surfaced by the deep audit. Severity-ordered. Each maps to a WBS task in `03_DAG_WBS.md`.

| Sev | Issue | Location | Fix task |
|-----|-------|----------|----------|
| **P0** | Secrets printed in plaintext via `#[derive(Debug)]` on `ApiKey`/`AuthCredential`/OAuth tokens (only `Display` redacts) → leaks into logs + PostHog tracker | `forge_domain` auth types; `provider_repo.rs` | P1.1 |
| **P1** | `.credentials.json` (mode 0o600) not gitignored — risk of committing live creds | repo root / `.gitignore` | P0.5 |
| **P1** | Production `unimplemented!()` in a non-test `HttpClient` impl (`http_delete`) | `forge_repo/src/provider/openai_responses/repository.rs#L573` | P1.4 |
| **P1** | `NoopIntentExtractor` returns an error instead of a no-op | `forge_domain/src/intent.rs#L119,L129` | P1.4 |
| **P2** | User-facing `panic!` on bad `--directory` arg (should be a clean error) | `forge_main/src/main.rs#L135` | P1 (polish) |
| **P2** | Uncancellable FTS background loop; unbounded forge3d accept loop; fire-and-forget telemetry spawns | `forge_api.rs#L63-74`, `forge3d/src/server.rs#L225-240` | P2.4 |
| **P2** | `forge_dbd` loses queued writes on exit (no graceful drain) | `forge_dbd/src/server.rs#L52` | P2.3 |
| **P2** | Thread-unsafe runtime `set_var` in 3 files; lock held across `await`; executor `Mutex` across full child exec | `mcp_client.rs#L75`, `executor.rs#L101-141` | P3.3 |
| **P3** | Renovate `automerge:true` = unattended supply-chain merges on a fast-moving fork; 5/9 advisory ignores lack a `reason` (suppression-policy violation) | `renovate.json` | P1.6 |
| **P3** | Dead crate `ghostty-kit`; default system allocator (no jemalloc/mimalloc) | `Cargo.toml`; `forge_main` | P1.5 / P3.2 |

**Note:** the audit also corrected two prior-scorecard inaccuracies — L18 understated existing `0o600` hardening + env→file migration (credit due), and W01/W05/W12 had all been scored against the stale TS-evals scaffolding rather than the real Rust workspace (root cause → P0).
