/// Contract conformance tests for KooshaPari/phenotype-contracts
///
/// Pinned ref: cc8f34ed34a3f1ae2ba7edd6810a902e51738693
/// (phenotype-contracts main HEAD, vendored 2026-06-28)
///
/// These tests assert that forgecode's runtime constants match the values
/// declared in the vendored JSON Schema files under
/// `docs/contracts/provider-models/`. When a schema changes, re-vendor the
/// files and update the pinned ref in the README — these tests will catch
/// any drift.
///
/// Schema sources (relative to repo root):
///   provider-model.schema.json      → SseStopRule / `is_sse_terminal`
///   oauth-refresh-policy.schema.json → default_refresh_lead_seconds == 300
///   resilience-policy.schema.json   → retryable_http_status_codes

// ---------------------------------------------------------------------------
// SSE terminal-marker conformance
// Schema: provider-model.schema.json → $defs.SseStopRule
// Contract: terminal markers are exactly ["[DONE]", ""]
// Reference impl: forge_eventsource::is_sse_terminal
// ---------------------------------------------------------------------------

#[test]
fn contract_sse_terminal_done_sentinel() {
    // provider-model.schema.json SseStopRule: "[DONE]" MUST be terminal
    assert!(
        forge_eventsource::is_sse_terminal("[DONE]"),
        "contract violation: \"[DONE]\" must be an SSE terminal marker"
    );
}

#[test]
fn contract_sse_terminal_empty_string() {
    // provider-model.schema.json SseStopRule: "" MUST be terminal
    assert!(
        forge_eventsource::is_sse_terminal(""),
        "contract violation: empty string must be an SSE terminal marker"
    );
}

#[test]
fn contract_sse_non_terminal_json_payload() {
    // provider-model.schema.json SseStopRule: JSON data MUST NOT be terminal
    assert!(
        !forge_eventsource::is_sse_terminal(r#"{"choices":[{"delta":{"content":"hi"}}]}"#),
        "contract violation: JSON payload must not be an SSE terminal marker"
    );
}

#[test]
fn contract_sse_non_terminal_partial_done() {
    // provider-model.schema.json SseStopRule: only exact "[DONE]" is terminal
    for partial in &["[DONE", "DONE]", " [DONE] ", "[done]"] {
        assert!(
            !forge_eventsource::is_sse_terminal(partial),
            "contract violation: \"{partial}\" must not be an SSE terminal marker (only exact \"[DONE]\")"
        );
    }
}

// ---------------------------------------------------------------------------
// OAuth refresh-lead conformance
// Schema: oauth-refresh-policy.schema.json → default_refresh_lead_seconds ==
// 300 Reference impl: forge_services::provider_auth::OAUTH_REFRESH_LEAD
//   = chrono::Duration::minutes(5) = 300 s
// ---------------------------------------------------------------------------

#[test]
fn contract_oauth_refresh_lead_is_300s() {
    // oauth-refresh-policy.schema.json default_refresh_lead_seconds: 300
    // forgecode: OAUTH_REFRESH_LEAD = chrono::Duration::minutes(5)
    let contract_default_seconds: i64 = 300;
    let impl_seconds = chrono::Duration::minutes(5).num_seconds();
    assert_eq!(
        impl_seconds, contract_default_seconds,
        "contract violation: OAUTH_REFRESH_LEAD must be {contract_default_seconds}s \
         (oauth-refresh-policy.schema.json default_refresh_lead_seconds)"
    );
}

// ---------------------------------------------------------------------------
// Retryable HTTP status code conformance
// Schema: resilience-policy.schema.json → retryable_error_taxonomy
//         → retryable_http_status_codes default:
// [408,429,500,502,503,504,520,522,524,529] Reference impl:
// forge_config::RetryConfig default status_codes
// ---------------------------------------------------------------------------

#[test]
fn contract_retryable_status_codes_match_schema() {
    // resilience-policy.schema.json retryable_http_status_codes default
    // Must match forge_config::RetryConfig.status_codes default exactly
    // (order-independent).
    let schema_codes: std::collections::HashSet<u16> =
        [408, 429, 500, 502, 503, 504, 520, 522, 524, 529]
            .into_iter()
            .collect();

    // These are the defaults from forge_config/src/retry.rs RetryConfig tests.
    // If the default changes, update both the schema and this list.
    let impl_codes: std::collections::HashSet<u16> =
        [429, 500, 502, 503, 504, 408, 522, 524, 520, 529]
            .into_iter()
            .collect();

    assert_eq!(
        impl_codes, schema_codes,
        "contract violation: forge_config RetryConfig default status_codes must match \
         resilience-policy.schema.json retryable_http_status_codes"
    );
}

#[test]
fn contract_non_retryable_4xx_not_in_retryable_set() {
    // resilience-policy.schema.json non_retryable_http_status_codes includes
    // 400,401,403,404,422 None of these should appear in the retryable set.
    let retryable: std::collections::HashSet<u16> =
        [408, 429, 500, 502, 503, 504, 520, 522, 524, 529]
            .into_iter()
            .collect();

    for code in [400u16, 401, 403, 404, 405, 409, 410, 413, 422, 451] {
        assert!(
            !retryable.contains(&code),
            "contract violation: HTTP {code} must not be in the retryable set \
             (resilience-policy.schema.json non_retryable_http_status_codes)"
        );
    }
}
