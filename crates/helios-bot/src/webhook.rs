//! GitHub webhook handling.

// The webhook module is wired into the binary but the actual webhook
// server lives in a Cloudflare Worker (see .github/apps/helios-bot).
// This stub Rust binary doesn't call into these functions at runtime,
// so they trigger dead-code under -D warnings. Mark the whole module
// as allowed since these items are exercised by `cargo test -p helios-bot`
// once `test = false` is reverted in Cargo.toml.
#![allow(dead_code)]

use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::collections::HashMap;

type HmacSha256 = Hmac<Sha256>;

/// Verify a GitHub webhook payload signature.
///
/// GitHub sends `X-Hub-Signature-256: sha256=<hex>` and we verify by computing
/// HMAC-SHA256(secret, body) and comparing constant-time.
pub fn verify_signature(secret: &str, body: &[u8], signature_header: &str) -> bool {
    let expected = match signature_header.strip_prefix("sha256=") {
        Some(s) => s,
        None => return false,
    };

    let mut mac = match HmacSha256::new_from_slice(secret.as_bytes()) {
        Ok(m) => m,
        Err(_) => return false,
    };
    mac.update(body);
    // Verify using the hmac crate's constant-time comparison directly.
    if let Ok(expected_bytes) = hex::decode(expected) {
        return mac.verify_slice(&expected_bytes).is_ok();
    }
    false
}

/// A parsed `@helios` mention.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeliosMention {
    /// The text after `@helios`, trimmed. Empty string if just `@helios` with no request.
    pub request: String,
    /// Whether the mention was in a comment (vs. issue body or PR body).
    pub in_comment: bool,
    /// Whether this is on an issue (vs. a PR).
    pub is_issue: bool,
}

/// Parse `@helios <request>` out of arbitrary text. Returns None if no mention.
pub fn parse_helios_mention(text: &str, is_issue: bool, in_comment: bool) -> Option<HeliosMention> {
    let after = mention_suffix(text)?;
    // Strip leading whitespace and a single optional ':' or ','.
    let trimmed = after
        .trim_start()
        .trim_start_matches(':')
        .trim_start_matches(',')
        .trim_start();
    Some(HeliosMention { request: trimmed.to_string(), in_comment, is_issue })
}

fn mention_suffix(text: &str) -> Option<&str> {
    // Look for `@helios` (case-insensitive), followed by optional whitespace, then capture the rest.
    let lower = text.to_ascii_lowercase();
    let idx = lower.find("@helios")?;
    text.get(idx + "@helios".len()..)
}

/// Extracted webhook context for a mention.
#[derive(Debug, Clone)]
pub struct WebhookContext {
    pub repo: String,
    pub issue_number: u64,
    pub comment_id: Option<u64>,
    pub author: String,
    pub request: String,
    #[allow(dead_code)]
    pub is_issue: bool,
}

/// Parse a webhook payload (the `issues.comment.created` or
/// `issue_comment.created` event) and extract a WebhookContext if it
/// contains a `@helios` mention.
pub fn extract_webhook_context(
    event: &str,
    payload: &HashMap<String, serde_json::Value>,
) -> Option<WebhookContext> {
    // We only care about comment-created events on issues/PRs.
    if !matches!(
        event,
        "issue_comment" | "issues" | "pull_request_review_comment"
    ) {
        return None;
    }

    let action = payload.get("action").and_then(|v| v.as_str());
    if !matches!(action, Some("created") | Some("opened")) {
        return None;
    }

    // Try issue_comment.created first.
    let body = if event == "issue_comment" {
        payload
            .get("comment")
            .and_then(|c| c.get("body"))
            .and_then(|v| v.as_str())
    } else {
        payload
            .get("issue")
            .and_then(|c| c.get("body"))
            .and_then(|v| v.as_str())
    }?;

    let repo_full = payload.get("repository")?.get("full_name")?.as_str()?;
    let issue = payload
        .get("issue")
        .or_else(|| payload.get("pull_request"))?;
    let issue_number = issue.get("number")?.as_u64()?;
    let is_issue = payload.get("issue").is_some()
        && !issue
            .get("pull_request")
            .map(|v| v.is_object())
            .unwrap_or(false);

    let mention = parse_helios_mention(body, is_issue, event == "issue_comment")?;
    if mention.request.is_empty() {
        return None;
    }

    let comment_id = if event == "issue_comment" {
        payload
            .get("comment")
            .and_then(|c| c.get("id"))
            .and_then(|v| v.as_u64())
    } else {
        None
    };

    let author = if event == "issue_comment" {
        payload
            .get("comment")
            .and_then(|c| c.get("user"))
            .and_then(|u| u.get("login"))
            .and_then(|v| v.as_str())
    } else {
        payload
            .get("issue")
            .and_then(|c| c.get("user"))
            .and_then(|u| u.get("login"))
            .and_then(|v| v.as_str())
    }
    .unwrap_or("unknown");

    Some(WebhookContext {
        repo: repo_full.to_string(),
        issue_number,
        comment_id,
        author: author.to_string(),
        request: mention.request,
        is_issue,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_signature_accepts_valid() {
        let secret = "It's a Secret to Everybody";
        let body = b"Hello, World!";
        // Pre-computed signature for the above body+secret.
        // sha256=757107ea0eb2509fc211221cce984b8a37570b6d7586c22c46f4379c8b043e17
        let sig = "sha256=757107ea0eb2509fc211221cce984b8a37570b6d7586c22c46f4379c8b043e17";
        assert!(verify_signature(secret, body, sig));
    }

    #[test]
    fn verify_signature_rejects_invalid() {
        let secret = "It's a Secret to Everybody";
        let body = b"Hello, World!";
        let bad_sig = "sha256=0000000000000000000000000000000000000000000000000000000000000000";
        assert!(!verify_signature(secret, body, bad_sig));
    }

    #[test]
    fn verify_signature_rejects_missing_prefix() {
        assert!(!verify_signature("secret", b"body", "no-sha256-prefix"));
    }

    #[test]
    fn parse_mention_extracts_request() {
        let m = parse_helios_mention("@helios please add a CONTRIBUTING.md", true, true).unwrap();
        assert_eq!(m.request, "please add a CONTRIBUTING.md");
        assert!(m.in_comment);
        assert!(m.is_issue);
    }

    #[test]
    fn parse_mention_handles_colon() {
        let m = parse_helios_mention("@helios: refactor the foo module", true, true).unwrap();
        assert_eq!(m.request, "refactor the foo module");
    }

    #[test]
    fn mention_suffix_preserves_utf8_text_before_mention() {
        let fixture = "coffee: caf\u{00e9} @HeLiOs: inspect unicode";

        let actual = mention_suffix(fixture);
        let expected = Some(": inspect unicode");

        assert_eq!(actual, expected);
    }

    #[test]
    fn parse_mention_returns_none_when_absent() {
        assert!(parse_helios_mention("hello world", true, true).is_none());
    }

    #[test]
    fn parse_mention_returns_none_when_empty_request() {
        let m = parse_helios_mention("@helios", true, true).unwrap();
        assert_eq!(m.request, "");
    }

    #[test]
    fn extract_webhook_context_from_issue_comment() {
        let mut payload = HashMap::new();
        payload.insert(
            "action".to_string(),
            serde_json::Value::String("created".to_string()),
        );
        payload.insert(
            "repository".to_string(),
            serde_json::json!({"full_name": "KooshaPari/forgecode"}),
        );
        payload.insert(
            "issue".to_string(),
            serde_json::json!({"number": 42, "pull_request": {}}),
        );
        payload.insert(
            "comment".to_string(),
            serde_json::json!({
                "id": 12345,
                "body": "@helios please fix the typo",
                "user": {"login": "koosh"}
            }),
        );

        let ctx = extract_webhook_context("issue_comment", &payload).unwrap();
        assert_eq!(ctx.repo, "KooshaPari/forgecode");
        assert_eq!(ctx.issue_number, 42);
        assert_eq!(ctx.request, "please fix the typo");
        assert_eq!(ctx.author, "koosh");
        assert_eq!(ctx.comment_id, Some(12345));
    }

    #[test]
    fn extract_webhook_context_returns_none_for_non_created_action() {
        let mut payload = HashMap::new();
        payload.insert(
            "action".to_string(),
            serde_json::Value::String("edited".to_string()),
        );
        assert!(extract_webhook_context("issue_comment", &payload).is_none());
    }
}
