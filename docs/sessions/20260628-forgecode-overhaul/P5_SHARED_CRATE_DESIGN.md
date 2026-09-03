# P5 — Cross-Repo Shared Crate Extraction (Design / Scoping)

**Status:** DESIGN ONLY — no production code in this phase.
**Date:** 2026-06-28
**Scope:** 3 KooshaPari repos that independently re-implement provider/model logic.

| Repo | Path | Language | Role |
|---|---|---|---|
| forgecode | `~/CodeProjects/Phenotype/repos/forgecode` | Rust | CLI coding agent |
| OmniRoute | `~/CodeProjects/Phenotype/repos` (the dir *is* the checkout) | TypeScript | Router/proxy |
| cliproxyapi-plusplus | `~/CodeProjects/Phenotype/repos/cliproxyapi-plusplus` | Go | CLI auth proxy |

The four duplicated concerns (W12/L35 audit): (1) provider/model registry + schema normalization, (2) OAuth2 + 5-min refresh buffer, (3) retry/backoff, (4) SSE stop-signal detection.

---

## 1. Dup Map (evidence-based, file paths + LOC)

All paths verified via `wc -l` / `rg` by per-repo探查 subagents. Repo prefixes omitted where obvious.

### Concern 1 — Provider/model registry + schema normalization

| Repo | File(s) | LOC | Implementation |
|---|---|---|---|
| forgecode | `crates/forge_repo/src/provider/provider_repo.rs` | 2019 | Core registry: merges custom+builtin configs, normalizes URL/env fallbacks, resolves model lists |
| | `crates/forge_domain/src/provider.rs` | 887 | Domain types `Provider`/`Model`/`ProviderConfig`, capability structs |
| | `crates/forge_repo/src/provider/{anthropic,openai}.rs` | 997 / 1183 | Per-provider model-list fetch + schema→domain normalization |
| OmniRoute | `src/lib/providers/catalog.ts` | 251 | Local catalog assembler from provider classes |
| | `src/lib/modelCapabilities.ts` | 444 | Capability/metadata normalization |
| | `src/lib/modelsDevSync.ts` | 933 | Syncs models.dev external lists into registry |
| | `src/lib/providers/{staticModels,validation}.ts` | 134 / 696 | Static lists + per-provider format normalizers |
| cliproxy | `pkg/llmproxy/registry/model_registry.go` | 1340 | Core registry: lists, configs, capability normalization |
| | `pkg/llmproxy/registry/{model_definitions,model_updater,kiro_model_converter}.go` | 349 / 369 / 303 | Static defs, dynamic refresh, per-provider schema converter |

**Overlap:** HIGH conceptually, LOW at code level. All three implement the *same domain model* (provider → models → capabilities) and the *same external source normalization* (models.dev / provider model endpoints). ~4–5k LOC each, but the genuinely-shared semantic surface is the **schema/data contract** (~200–400 LOC of types), not the imperative loading code. This is the strongest candidate for a *spec*, the weakest for a *shared binary*.

### Concern 2 — OAuth2 + ~5-min refresh buffer

| Repo | File(s) | LOC | Implementation |
|---|---|---|---|
| forgecode | `crates/forge_services/src/provider_auth.rs` | 214 | `let buffer = chrono::Duration::minutes(5); credential.needs_refresh(buffer)` — exact 5-min buffer |
| | `crates/forge_domain/src/auth/credentials.rs` | 150 | `needs_refresh(buffer)` on `Credential`/`OAuthTokens` |
| | `crates/forge_infra/src/auth/strategy.rs` | 1557 | Device flow, `exchange_oauth_for_api_key`, refresh |
| | `crates/forge_infra/src/auth/util.rs` | 356 | `calculate_token_expiry` |
| OmniRoute | `src/lib/tokenHealthCheck.ts` | 692 | `TOKEN_EXPIRY_BUFFER = 5*60*1000` drives `isAboutToExpire` |
| | `src/lib/oauth/**` (services + per-provider) | ~7,936 | Full OAuth2 stack, per-provider device flows |
| | `src/domain/providerExpiration.ts` | ~250 | Computes expiry status from token timestamps |
| cliproxy | `sdk/cliproxy/auth/{conductor,auto_refresh_loop,types}.go` | 5821 / 455 / 712 | Central refresh manager, ahead-of-expiry loop, refresh-lead registry |
| | `sdk/auth/{gitlab,kimi,xai,claude,codex,...}.go` | — | Per-provider `RefreshLead()` mostly `5 * time.Minute` |
| | `pkg/llmproxy/auth/oauth_token_manager.go` | 81 | `tokenRefreshLeadTime` refresh-if-expiring |

**Overlap:** The **5-minute refresh-buffer rule** is genuinely identical across all three (forgecode `minutes(5)`, OmniRoute `5*60*1000`, cliproxy `5 * time.Minute`). But the OAuth *flow* code is large, provider-specific, and deeply tied to each runtime's HTTP/storage stack. Shareable unit = the **buffer policy + expiry math contract**, not the flow machinery. Note cliproxy varies the lead per provider (codebuddy 24h, copilot/most 5m), so the contract must be *parameterized lead*, not a hardcoded 5m.

### Concern 3 — Retry / backoff

| Repo | File(s) | LOC | Implementation |
|---|---|---|---|
| forgecode | `crates/forge_app/src/retry.rs` | 39 | `backon::ExponentialBuilder` factor/max/jitter; `should_retry` gates on `Error::Retryable` |
| | `crates/forge_config/src/retry.rs` | 48 | `RetryConfig` (min_delay, factor, max_attempts) |
| | `crates/forge_eventsource/src/retry.rs` | 120 | *Separate* SSE-reconnect retry |
| OmniRoute | `src/sse/services/cooldownAwareRetry.ts` | 162 | `MAX_REQUEST_RETRY=10`, cooldown-aware |
| | `src/lib/resilience/settings.ts` | 840 | `minRetryCooldownMs * 2^(failures-1)` |
| cliproxy | `pkg/llmproxy/auth/kiro/jitter.go` | 174 | `backoffWithJitter(attempt, base, max)` |
| | `pkg/llmproxy/auth/kiro/rate_limiter.go` | 309 | Fail-count backoff schedule |
| | per-executor (gemini/cursor/antigravity) | — | Exponential backoff + jitter on 502/503/504 |

**Overlap:** MEDIUM. All implement exponential-backoff-with-jitter, but each wraps a *language-native library* (Rust `backon`, TS bespoke, Go bespoke). The algorithm is textbook; the *parameters* (factor, cap, max attempts, which errors are retryable) are the only thing worth aligning. Sharing code here is low-value — sharing a **config schema** (the parameter set + retryable-error taxonomy) is the realistic win.

### Concern 4 — SSE stop-signal detection

| Repo | File(s) | LOC | Implementation |
|---|---|---|---|
| forgecode | `crates/forge_repo/src/provider/event.rs` | 83 | Filters `[DONE]`/empty sentinel |
| | `crates/forge_repo/src/provider/openai_responses/repository.rs` | 1807 | Same `[DONE]` filter (dup #2 *within* forgecode) |
| | `crates/forge_repo/src/provider/anthropic.rs` | 997 | `[DONE]` check in Anthropic stream (dup #3 within forgecode) |
| OmniRoute | `src/lib/sseTextTransform.ts` | 346 | `checkIfStopSignal()`: `[DONE]` + OpenAI `finish_reason` + Anthropic `message_delta.stop_reason` |
| | `src/shared/utils/streamTracker.ts` | 187 | `[DONE]` + `choices[0].finish_reason` |
| cliproxy | `sdk/api/handlers/stream_forwarder.go` | 121 | Detects `[DONE]` terminal marker |
| | `pkg/llmproxy/runtime/executor/openai_compat_executor.go` | — | Injects synthetic `[DONE]` when upstream closes silently |
| | `pkg/llmproxy/translator/kiro/openai/kiro_openai_stream.go` | 212 | `finish_reason`/stop handling |

**Overlap:** MEDIUM-HIGH on *rules*, and notably there is **intra-repo duplication inside forgecode itself** (3 separate `[DONE]` filters). The shared semantic is a small, stable rule set: terminal markers (`[DONE]`), OpenAI `finish_reason`, Anthropic `stop_reason`/`message_delta`. This is ~50–80 LOC of pure logic per language. Best treated as a **shared spec + small per-language helper**, plus an immediate forgecode-internal consolidation.

### Dup summary (concern × repo × LOC)

| Concern | forgecode | OmniRoute | cliproxy | Real shared surface |
|---|---|---|---|---|
| 1 Registry/normalization | ~5.1k | ~2.5k | ~2.7k | Schema/data contract (~300 LOC types) |
| 2 OAuth + 5-min buffer | ~2.3k | ~8.9k | ~7.1k | Buffer/expiry *policy* (~80 LOC) |
| 3 Retry/backoff | ~0.2k | ~1.0k | ~0.7k | Param + retryable-error *schema* |
| 4 SSE stop-signal | ~0.2k (3× dup) | ~0.5k | ~0.3k | Terminal-marker *rule set* (~80 LOC) |

**Verdict on duplication:** All 4 concerns are genuinely re-implemented in all 3 repos. But the duplication is **semantic, not literal** — the shared part is the *contract/rules/policy*, while the bulk LOC is runtime-/HTTP-/storage-coupled glue that cannot be lifted as-is.

---

## 2. Cross-Language Reality Check

**A single Rust crate can only be a dependency of Rust.** It cannot be imported by TypeScript or Go without an FFI/WASM/codegen boundary. Honest assessment per option:

| Option | Feasibility | Notes |
|---|---|---|
| Rust crate consumed by all 3 | ❌ Not real | TS/Go cannot `cargo add` a crate. WASM-compiling the crate for TS is possible but heavyweight for registry/policy logic, and Go-via-cgo+WASM is impractical. Do **not** pretend this works. |
| Rust crate for forgecode only + shared **spec** for TS/Go | ✅ Realistic | forgecode gets a real crate; OmniRoute/cliproxy align *against the spec*, keeping their native impls. |
| Language-agnostic **contract** (JSON Schema for data, a small spec doc for policy/rules) + per-language impls | ✅ Realistic, primary | This is the genuinely shareable artifact across all 3. Schema can drive validation/codegen in each runtime. |
| Protobuf/gRPC | ⚠️ Overkill here | These are data/policy contracts, not an RPC surface. JSON Schema is the right weight; proto only if a runtime service emerges. |
| Extract a shared *service* (one repo proxies the others) | ❌ Out of scope | Changes the architecture; not what P5 asks. |

**Cross-language verdict:** The shareable unit is **a language-agnostic contract (JSON Schema for the model/provider data shape + a versioned spec for OAuth-buffer, retry, and SSE-stop rules)**, PLUS **a real Rust crate `phenotype-provider-models` that forgecode consumes directly and that is the *reference implementation* of that contract.** Full 3-repo *code* sharing is NOT feasible; 3-repo *spec/schema* alignment IS.

---

## 3. Proposed `phenotype-provider-models` (Rust crate) + contract

### 3a. Language-agnostic contract (the real cross-repo artifact)

Lives in a shared location (e.g. `repos/docs/contracts/provider-models/`):

- `provider-model.schema.json` — JSON Schema for `Provider`, `Model`, `ProviderConfig`, capabilities/metadata. Source of truth for concern 1.
- `auth-policy.md` + `auth-policy.schema.json` — refresh policy: parameterized `refresh_lead` (default `300s`), `needs_refresh(now, expires_at, lead)` semantics, expiry-math contract. Concern 2.
- `retry-policy.schema.json` — `{ min_delay_ms, backoff_factor, max_attempts, jitter, retryable_status[], retryable_error_kinds[] }`. Concern 3.
- `sse-stop.md` — terminal-marker rule set: `[DONE]` sentinel, OpenAI `finish_reason`, Anthropic `message_delta.stop_reason`; synthetic-`[DONE]`-on-silent-close rule. Concern 4.
- `VERSION` + changelog; each repo pins a contract version.

### 3b. Rust crate scope (forgecode consumer + reference impl)

```
phenotype-provider-models/
  src/
    model.rs       // Provider, Model, ProviderConfig, Capability  (from forge_domain/provider.rs)
    normalize.rs   // models.dev / provider-endpoint → domain normalization
    auth_policy.rs // Credential, OAuthTokens, needs_refresh(buffer); default lead = 5min
    retry_policy.rs// RetryConfig + is_retryable taxonomy (NOT the backon loop)
    sse_stop.rs    // StopSignal::detect(event) -> single shared [DONE]/finish_reason/stop_reason helper
    schema.rs      // serde types <-> provider-model.schema.json (round-trip tested)
```

Public API sketch (signatures only — no impl this phase):

```rust
pub struct Provider { /* id, base_url, auth_kind, models, ... */ }
pub struct Model { /* id, capabilities, context, pricing?, ... */ }
pub struct ProviderConfig { /* merged custom+builtin */ }

pub fn normalize_models_dev(raw: &serde_json::Value) -> Result<Vec<Model>>;
pub fn normalize_provider_endpoint(kind: ProviderKind, raw: &serde_json::Value) -> Result<Vec<Model>>;

pub struct OAuthTokens { pub expires_at: DateTime<Utc>, /* ... */ }
impl OAuthTokens { pub fn needs_refresh(&self, lead: Duration) -> bool; }
pub const DEFAULT_REFRESH_LEAD: Duration = Duration::minutes(5);

pub struct RetryPolicy { pub min_delay_ms: u64, pub backoff_factor: f64,
                         pub max_attempts: u32, pub jitter: bool }
impl RetryPolicy { pub fn is_retryable(err_kind: &ErrorKind, status: Option<u16>) -> bool; }

pub enum StopSignal { Done, FinishReason(String), AnthropicStop(String) }
impl StopSignal { pub fn detect(event_data: &str) -> Option<StopSignal>; }
```

**Crate does NOT contain:** the `backon` retry loop (stays in `forge_app`), HTTP transport, device-flow networking, or storage — those are runtime-coupled and stay in forgecode's infra crates. The crate is **types + pure functions + the schema round-trip**, which is exactly the part that maps 1:1 onto the language-agnostic contract.

### 3c. Who consumes what

| Repo | Consumes crate directly? | Consumes contract? |
|---|---|---|
| forgecode (Rust) | ✅ Yes — direct `cargo` dep; crate is reference impl | implicitly (crate == contract) |
| OmniRoute (TS) | ❌ No | ✅ Aligns `modelCapabilities`/`tokenHealthCheck`/`sseTextTransform` to schema; optional `json-schema-to-typescript` codegen for the data types |
| cliproxy (Go) | ❌ No | ✅ Aligns `model_registry`/refresh-lead/`stream_forwarder` to schema; optional Go struct codegen from JSON Schema |

---

## 4. Migration Plan (phased DAG)

Order by value/risk: data contract first (highest semantic overlap, lowest behavioral risk), then policy, then resilience/SSE.

```
P5.0 contract scaffolding ──► P5.1 provider-models ──► P5.2 OAuth policy ──► P5.3 resilience + SSE
       (schema dir,                (concern 1)             (concern 2)          (concerns 3 + 4)
        version pin)
```

### P5.0 — Contract scaffolding (predecessor of all)
- Create `repos/docs/contracts/provider-models/` with the 4 schema/spec files + VERSION.
- Risk: LOW. No code change. Establishes the SSOT every later phase pins to.

### P5.1 — provider-models (concern 1)  [depends: P5.0]
- **forgecode:** extract `forge_domain/provider.rs` types + normalization from `provider_repo.rs`/`anthropic.rs`/`openai.rs` into new crate `phenotype-provider-models`; add round-trip test against `provider-model.schema.json`. Forward-only: update `forge_repo` callers, delete moved code.
- **OmniRoute:** generate/align TS types from schema; refactor `modelCapabilities.ts` + `modelsDevSync.ts` to the schema shape. No code dep on crate.
- **cliproxy:** align `model_registry.go`/`model_definitions.go` field names + capability enum to schema; optional struct codegen.
- Shareable: **schema only** (3 repos). Crate code: forgecode only.
- Risk: MEDIUM (forgecode refactor touches 5k LOC of registry, but pure-type extraction is mechanical). TS/Go are field-alignment only — low risk.

### P5.2 — OAuth policy (concern 2)  [depends: P5.1]
- **forgecode:** move `needs_refresh`/`OAuthTokens`/expiry math into crate `auth_policy`; keep device-flow/HTTP in `forge_infra`. Default lead 5min, parameterized.
- **OmniRoute:** replace literal `5*60*1000` with a named contract constant + `needs_refresh(lead)` shape; keep oauth flow stack as-is.
- **cliproxy:** keep per-provider `RefreshLead()` but document them against the contract's *parameterized lead* (codebuddy 24h etc. are valid per-provider overrides, not violations).
- Shareable: **policy contract** (parameterized lead + expiry math). Flow code stays per-repo.
- Risk: MEDIUM-HIGH — auth bugs are high-impact. Keep flow untouched; only unify the buffer/expiry *decision*. cliproxy's variable leads mean the contract MUST be parameterized, not a hardcoded 5m.

### P5.3 — Resilience + SSE (concerns 3 & 4)  [depends: P5.2]
- **forgecode:** (a) consolidate the **3 internal `[DONE]` filters** (`event.rs`, `openai_responses/repository.rs`, `anthropic.rs`) into one `StopSignal::detect` in the crate — this is a real, immediate win independent of cross-repo. (b) Move `RetryConfig` + `is_retryable` taxonomy into crate `retry_policy`; leave the `backon` loop in `forge_app`.
- **OmniRoute:** align `sseTextTransform.checkIfStopSignal` + `streamTracker` to the `sse-stop.md` rule set; align `resilience/settings` params to `retry-policy.schema.json`.
- **cliproxy:** align `stream_forwarder`/translators to the SSE rule set (incl. synthetic-`[DONE]` rule); align jitter/backoff params to the retry schema.
- Shareable: **rule set + param schema**. Backoff loops stay language-native (backon / bespoke).
- Risk: LOW-MEDIUM. SSE rules are small and well-defined; forgecode internal consolidation is the clearest standalone benefit of the whole phase.

### Per-phase: can it actually be shared vs only spec-aligned?

| Phase | forgecode | OmniRoute | cliproxy |
|---|---|---|---|
| P5.1 | shared crate code | spec-aligned + optional codegen | spec-aligned + optional codegen |
| P5.2 | shared crate (policy) | spec-aligned (literal→constant) | spec-aligned (parameterized lead) |
| P5.3 | shared crate (rules) + internal de-dup | spec-aligned | spec-aligned |

---

## 5. Recommendation

1. **Do NOT build a single Rust crate as a 3-repo dependency.** It is technically false for TS/Go.
2. **Primary shareable unit = a versioned language-agnostic contract** (`provider-model.schema.json` + auth/retry/SSE policy specs) living in `repos/docs/contracts/provider-models/`. All 3 repos pin and align to it.
3. **Secondary = a real Rust crate `phenotype-provider-models`** that forgecode consumes directly and that serves as the **reference implementation** of the contract (types + pure functions + schema round-trip). OmniRoute and cliproxy consume the *contract/schema* (optionally via JSON-Schema codegen), not the crate.
4. **Highest-confidence concrete wins, independent of cross-repo politics:**
   - Consolidate forgecode's **3 internal `[DONE]` detectors** into one helper (P5.3a).
   - Single-source the parameterized refresh-lead policy.
   - Pin a shared model/provider JSON Schema so the three registries stop drifting.
5. **Order:** P5.0 contract → P5.1 registry (highest overlap, lowest risk) → P5.2 OAuth (parameterized lead — cliproxy varies it) → P5.3 resilience+SSE.
6. **Honest limit:** the bulk LOC (~15k+ across repos) is runtime/HTTP/storage glue and is NOT shareable. The genuinely shared surface is ~500–800 LOC of types + ~250 LOC of rules/policy. Set expectations accordingly: this is a **drift-elimination / contract-alignment** effort, not a 15k-LOC dedup.
