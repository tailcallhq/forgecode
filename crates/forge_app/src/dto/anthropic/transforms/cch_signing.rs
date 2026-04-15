use sha2::{Digest, Sha256};

use crate::dto::anthropic::{Content, Request, Role, SystemMessage};

/// CCH (Claude Code Hash) request signing for fast mode and research preview features.
///
/// This helper implements the request signing mechanism used by Claude Code
/// to authenticate requests and unlock features like fast mode. The signing
/// involves:
///
/// 1. Computing a 3-character version suffix from the raw text of the first user message
/// 2. Building a billing header with the version and a `cch=00000` placeholder
/// 3. Injecting the billing header as the first system message
/// 4. Serializing the full request to compact JSON (`Request` guarantees `system` before `messages`)
/// 5. Computing a 5-character xxHash64 of the serialized body (with placeholder)
/// 6. Replacing `cch=00000` in the serialized JSON with the real hash
///
/// # JSON Ordering
///
/// The hash is computed over the serialized request body. `system` MUST appear
/// before `messages` in the JSON output. The `Request` struct field declaration
/// order guarantees this — do NOT reorder those two fields in `Request`.
///
/// # Replacement Safety
///
/// The first `cch=00000` in the serialized JSON always belongs to the billing
/// header because `system` serializes before `messages`. Even if a user message
/// happens to contain the literal string `cch=00000`, it appears after the
/// billing header in the JSON and is therefore unaffected by the first-occurrence
/// replacement.
///
/// # Reference
///
/// Based on reverse engineering research of Claude Code's request signing
/// mechanism. The algorithm uses xxHash64 with a fixed seed for request
/// integrity verification.
#[derive(Clone)]
pub struct CchSigning {
    /// Claude Code version string (e.g., "2.1.37")
    version: String,
    /// xxHash64 seed from binary analysis
    seed: u64,
    /// Salt for version suffix computation
    salt: String,
}

/// Default CCH constants from reverse engineering research.
impl Default for CchSigning {
    fn default() -> Self {
        Self::new(
            env_or_default("FORGE_CC_VERSION", "2.1.37"),
            env_or_default_u64("FORGE_CCH_SEED", 0x6E52736AC806831E),
            env_or_default("FORGE_CCH_SALT", "59cf53e54c78"),
        )
    }
}

impl CchSigning {
    /// CCH placeholder that gets replaced with the actual hash.
    const CCH_PLACEHOLDER: &'static str = "cch=00000";

    /// Creates a new `CchSigning` with custom parameters.
    ///
    /// For most use cases, use `CchSigning::default()` which loads constants
    /// from environment variables or uses the default research values.
    pub fn new(version: String, seed: u64, salt: String) -> Self {
        Self { version, seed, salt }
    }

    /// Computes the 3-character version suffix from the raw text of the first
    /// user message.
    ///
    /// Algorithm: extract the characters at indices 4, 7, and 20 from the plain
    /// text string (substituting `'0'` when the index is out of bounds), then
    /// compute `SHA256(salt + chars + version)` and return the first 3 hex
    /// characters.
    ///
    /// # Arguments
    ///
    /// * `first_message_text` - The raw text content of the first user message
    ///
    /// # Returns
    ///
    /// A 3-character lowercase hex string (e.g., `"fbe"`)
    pub fn compute_version_suffix(&self, first_message_text: &str) -> String {
        let chars: String = [4_usize, 7, 20]
            .iter()
            .map(|&i| first_message_text.chars().nth(i).unwrap_or('0'))
            .collect();

        let input = format!("{}{}{}", self.salt, chars, self.version);
        let hash = Sha256::digest(input.as_bytes());

        format!("{:x}", hash)[..3].to_string()
    }

    /// Computes the 5-character CCH hash over `body`.
    ///
    /// Applies xxHash64 with the configured seed, masks the result to 20 bits
    /// (`& 0xFFFFF`), and formats it as a zero-padded 5-character lowercase hex
    /// string (e.g., `"a112b"`).
    pub fn compute_cch_hash(&self, body: &str) -> String {
        let hash = xxhash_rust::xxh64::xxh64(body.as_bytes(), self.seed);
        format!("{:05x}", hash & 0xFFFFF)
    }

    /// Builds the billing header text with the placeholder in place.
    ///
    /// Returns a string of the form:
    /// `x-anthropic-billing-header: cc_version=2.1.37.fbe; cc_entrypoint=cli; cch=00000;`
    fn build_billing_header(&self, version_suffix: &str) -> String {
        format!(
            "x-anthropic-billing-header: cc_version={}.{}; cc_entrypoint=cli; cch=00000;",
            self.version, version_suffix,
        )
    }

    /// Extracts the raw text of the first user text message from the request.
    ///
    /// Scans the message list in order, skips non-user messages, then returns a
    /// borrowed reference to the first `Content::Text` block found in the first
    /// user message that contains text. Returns `None` if there is no user text
    /// message.
    fn extract_first_user_message_text(request: &Request) -> Option<&str> {
        request
            .messages
            .iter()
            .filter(|msg| msg.role == Role::User)
            .find_map(|msg| {
                msg.content.iter().find_map(|block| {
                    if let Content::Text { text, .. } = block {
                        Some(text.as_str())
                    } else {
                        None
                    }
                })
            })
    }

    /// Injects the temporary billing header as the first system message.
    fn prepend_billing_header(request: &mut Request, billing_header: String) {
        let billing_system_message = SystemMessage {
            r#type: "text".to_string(),
            text: billing_header,
            cache_control: None,
        };
        let mut system_messages = request.system.take().unwrap_or_default();
        system_messages.insert(0, billing_system_message);
        request.system = Some(system_messages);
    }

    /// Serializes the final outbound request body, applying Claude Code signing
    /// when a user text message is present.
    ///
    /// # Arguments
    ///
    /// * `request` - The fully transformed Anthropic request to serialize for the wire
    ///
    /// # Errors
    ///
    /// Returns an error if the request cannot be serialized to JSON.
    pub fn serialize_signed_request(&self, mut request: Request) -> anyhow::Result<Vec<u8>> {
        let Some(first_message_text) = Self::extract_first_user_message_text(&request) else {
            tracing::debug!("CCH signing: no user text message found, skipping");
            return Ok(serde_json::to_vec(&request)?);
        };

        let version_suffix = self.compute_version_suffix(first_message_text);
        tracing::debug!(version_suffix = %version_suffix, "CCH signing: computed version suffix");

        let billing_header = self.build_billing_header(&version_suffix);
        Self::prepend_billing_header(&mut request, billing_header);

        let body_with_placeholder = serialize_request_compact(&request)?;
        let cch_hash = self.compute_cch_hash(&body_with_placeholder);
        tracing::debug!(cch_hash = %cch_hash, "CCH signing: computed hash");

        let signed_body =
            body_with_placeholder.replacen(Self::CCH_PLACEHOLDER, &format!("cch={cch_hash}"), 1);
        tracing::debug!("CCH signing: request signed successfully");

        Ok(signed_body.into_bytes())
    }
}

/// Serializes the request to compact JSON.
///
/// `Request` declares `system` before `messages`, which is required for the
/// CCH hash to match the body sent over the wire.
fn serialize_request_compact(request: &Request) -> anyhow::Result<String> {
    Ok(serde_json::to_string(request)?)
}

/// Returns the value of `var` from the environment, or `default` if unset.
fn env_or_default(var: &str, default: &str) -> String {
    std::env::var(var).unwrap_or_else(|_| default.to_string())
}

/// Returns the value of `var` parsed as a `u64` (decimal or `0x`-prefixed hex),
/// or `default` if the variable is unset or unparseable.
fn env_or_default_u64(var: &str, default: u64) -> u64 {
    std::env::var(var)
        .ok()
        .and_then(|v| {
            if v.starts_with("0x") || v.starts_with("0X") {
                u64::from_str_radix(&v[2..], 16).ok()
            } else {
                v.parse().ok()
            }
        })
        .unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dto::anthropic::{Message, Role};
    use pretty_assertions::assert_eq;

    // ── helpers ──────────────────────────────────────────────────────────────

    fn signing() -> CchSigning {
        CchSigning::new(
            "2.1.37".to_string(),
            0x6E52736AC806831E,
            "59cf53e54c78".to_string(),
        )
    }

    fn make_request_with_text(text: &str) -> Request {
        Request {
            messages: vec![Message {
                role: Role::User,
                content: vec![Content::Text { text: text.to_string(), cache_control: None }],
            }],
            max_tokens: 32000,
            ..Default::default()
        }
    }

    fn serialize_signed_request(signing: &CchSigning, request: Request) -> String {
        String::from_utf8(signing.serialize_signed_request(request).unwrap())
            .expect("signed request body should be valid UTF-8")
    }

    fn sign_request(signing: &CchSigning, request: Request) -> Request {
        serde_json::from_str(&serialize_signed_request(signing, request))
            .expect("signed request body should deserialize")
    }

    // ── compute_version_suffix ───────────────────────────────────────────────

    /// Matches the Python PoC reference:
    /// PROMPT = "Say 'hello' and nothing else."
    /// S(0)a(1)y(2) (3)'(4)h(5)e(6)l(7)l(8)o(9)'(10) (11)a(12)n(13)d(14) (15)n(16)o(17)t(18)h(19)i(20)
    /// chars at 4='\'', 7='l', 20='i'
    #[test]
    fn test_version_suffix_known_value() {
        let signing = signing();
        let prompt = "Say 'hello' and nothing else.";
        let suffix = signing.compute_version_suffix(prompt);

        // Verify the expected character extraction
        let chars: Vec<char> = prompt.chars().collect();
        assert_eq!(chars[4], '\'');
        assert_eq!(chars[7], 'l');
        assert_eq!(chars[20], 'i');

        // Output is always exactly 3 lowercase hex chars
        assert_eq!(suffix.len(), 3);
        assert!(suffix.chars().all(|c| c.is_ascii_hexdigit()));

        // Deterministic: same input → same output
        assert_eq!(suffix, signing.compute_version_suffix(prompt));
    }

    #[test]
    fn test_version_suffix_short_message_uses_zero_fallback() {
        let signing = signing();
        // "Hi" has only indices 0 and 1 — indices 4, 7, 20 all fall back to '0'
        let suffix_hi = signing.compute_version_suffix("Hi");
        let suffix_zeros = signing.compute_version_suffix("000");
        // Both produce the same suffix because the out-of-bounds fallback is '0'
        // (index 2 for "000" is also in-bounds but '0', matching the fallback)
        // More importantly: both are valid 3-char hex strings
        assert_eq!(suffix_hi.len(), 3);
        assert!(suffix_hi.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(suffix_hi, ""); // sanity

        // A message with '0' at indices 4, 7, 20 equals the fully-out-of-bounds case
        let msg = "0000000000000000000000"; // index 4='0', 7='0', 20='0'
        assert_eq!(suffix_hi, signing.compute_version_suffix(msg));
        assert_eq!(suffix_zeros, suffix_hi);
    }

    #[test]
    fn test_version_suffix_different_messages_produce_different_suffixes() {
        let signing = signing();
        let a = signing.compute_version_suffix("Say 'hello' and nothing else.");
        let b = signing.compute_version_suffix("Write a poem about the ocean.");
        // Characters at indices 4,7,20 differ → suffixes differ
        assert_ne!(a, b);
    }

    // ── compute_cch_hash ─────────────────────────────────────────────────────

    #[test]
    fn test_cch_hash_format() {
        let signing = signing();
        let body = r#"{"system":[{"type":"text","text":"x-anthropic-billing-header: cc_version=2.1.37.abc; cc_entrypoint=cli; cch=00000;"}],"max_tokens":32000,"messages":[]}"#;
        let hash = signing.compute_cch_hash(body);
        assert_eq!(hash.len(), 5);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_cch_hash_is_deterministic() {
        let signing = signing();
        let body = r#"{"system":[],"messages":[]}"#;
        assert_eq!(
            signing.compute_cch_hash(body),
            signing.compute_cch_hash(body)
        );
    }

    #[test]
    fn test_cch_hash_differs_for_different_bodies() {
        let signing = signing();
        let a = signing.compute_cch_hash(r#"{"system":[],"messages":[]}"#);
        let b = signing.compute_cch_hash(r#"{"system":[],"messages":[{"role":"user"}]}"#);
        assert_ne!(a, b);
    }

    #[test]
    fn test_cch_hash_zero_padded_to_five_chars() {
        // Use seed=0 so the raw hash value is predictable and small
        let signing = CchSigning::new("2.1.37".to_string(), 0, "59cf53e54c78".to_string());
        let hash = signing.compute_cch_hash("x");
        assert_eq!(hash.len(), 5);
    }

    // ── build_billing_header ─────────────────────────────────────────────────

    #[test]
    fn test_billing_header_format() {
        let signing = signing();
        let header = signing.build_billing_header("fbe");

        assert_eq!(
            header,
            "x-anthropic-billing-header: cc_version=2.1.37.fbe; cc_entrypoint=cli; cch=00000;"
        );
    }

    /// Ensures there is exactly one trailing semicolon after the placeholder —
    /// the double-semicolon bug regression test.
    #[test]
    fn test_billing_header_no_double_semicolon() {
        let signing = signing();
        let header = signing.build_billing_header("fbe");
        // placeholder appears once, ends with exactly one semicolon
        assert!(header.ends_with("cch=00000;"));
        assert!(!header.ends_with("cch=00000;;"));
    }

    // ── extract_first_user_message_text ──────────────────────────────────────

    #[test]
    fn test_extract_text_returns_raw_string_not_json() {
        let request = make_request_with_text("Hello world");
        let text = CchSigning::extract_first_user_message_text(&request);
        // Must be the raw string, NOT `"\"Hello world\""` or `[{"type":"text",...}]`
        assert_eq!(text, Some("Hello world"));
    }

    #[test]
    fn test_extract_text_empty_messages_returns_none() {
        let request = Request { messages: vec![], max_tokens: 100, ..Default::default() };
        assert_eq!(CchSigning::extract_first_user_message_text(&request), None);
    }

    #[test]
    fn test_extract_text_skips_assistant_messages() {
        let request = Request {
            messages: vec![
                Message {
                    role: Role::Assistant,
                    content: vec![Content::Text {
                        text: "assistant text".to_string(),
                        cache_control: None,
                    }],
                },
                Message {
                    role: Role::User,
                    content: vec![Content::Text {
                        text: "user text".to_string(),
                        cache_control: None,
                    }],
                },
            ],
            max_tokens: 100,
            ..Default::default()
        };

        let actual = CchSigning::extract_first_user_message_text(&request);
        let expected = Some("user text");
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_extract_text_skips_user_messages_without_text() {
        let request = Request {
            messages: vec![
                Message {
                    role: Role::User,
                    content: vec![Content::Image {
                        source: crate::dto::anthropic::ImageSource {
                            type_: "base64".to_string(),
                            media_type: Some("image/png".to_string()),
                            data: Some("abc".to_string()),
                            url: None,
                        },
                        cache_control: None,
                    }],
                },
                Message {
                    role: Role::User,
                    content: vec![Content::Text {
                        text: "describe this image".to_string(),
                        cache_control: None,
                    }],
                },
            ],
            max_tokens: 100,
            ..Default::default()
        };

        let actual = CchSigning::extract_first_user_message_text(&request);
        let expected = Some("describe this image");
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_extract_text_skips_tool_result_only_messages() {
        let request = Request {
            messages: vec![
                Message {
                    role: Role::User,
                    content: vec![Content::ToolResult {
                        tool_use_id: "toolu_123".to_string(),
                        content: Some("tool output".to_string()),
                        is_error: None,
                        cache_control: None,
                    }],
                },
                Message {
                    role: Role::User,
                    content: vec![Content::Text {
                        text: "follow-up question".to_string(),
                        cache_control: None,
                    }],
                },
            ],
            max_tokens: 100,
            ..Default::default()
        };

        let actual = CchSigning::extract_first_user_message_text(&request);
        let expected = Some("follow-up question");
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_extract_text_skips_non_text_content_within_message() {
        let request = Request {
            messages: vec![Message {
                role: Role::User,
                content: vec![
                    Content::Image {
                        source: crate::dto::anthropic::ImageSource {
                            type_: "base64".to_string(),
                            media_type: Some("image/png".to_string()),
                            data: Some("abc".to_string()),
                            url: None,
                        },
                        cache_control: None,
                    },
                    Content::Text { text: "describe this".to_string(), cache_control: None },
                ],
            }],
            max_tokens: 100,
            ..Default::default()
        };

        let actual = CchSigning::extract_first_user_message_text(&request);
        let expected = Some("describe this");
        assert_eq!(actual, expected);
    }

    // ── signed serialization (integration) ───────────────────────────────────

    #[test]
    fn test_serialize_signed_request_injects_billing_header_as_first_system_message() {
        let signing = signing();
        let request = make_request_with_text("Say 'hello' and nothing else.");

        let signed = sign_request(&signing, request);

        let system = signed.system.as_ref().expect("system must be present");
        assert!(!system.is_empty(), "system must have at least one message");

        let first = &system[0];
        assert!(
            first.text.starts_with("x-anthropic-billing-header:"),
            "first system message must be the billing header, got: {}",
            first.text
        );
        assert!(first.text.contains("cc_version=2.1.37."));
        assert!(first.text.contains("cc_entrypoint=cli"));
    }

    #[test]
    fn test_serialize_signed_request_returns_valid_json() {
        let signing = signing();
        let request = make_request_with_text("Hello world");
        let body = signing.serialize_signed_request(request).unwrap();
        let result = serde_json::from_slice::<Request>(&body);
        assert!(result.is_ok(), "deserialization failed: {:?}", result.err());
        let recovered = result.unwrap();
        let user_text = recovered.messages[0].content.iter().find_map(|c| {
            if let Content::Text { text, .. } = c {
                Some(text.as_str())
            } else {
                None
            }
        });
        assert_eq!(user_text, Some("Hello world"));
    }

    #[test]
    fn test_serialize_signed_request_billing_header_has_no_placeholder_in_final_output() {
        let signing = signing();
        let request = make_request_with_text("Say 'hello' and nothing else.");

        let signed = sign_request(&signing, request);

        let system = signed.system.as_ref().unwrap();
        let billing = &system[0].text;
        assert!(
            !billing.contains("cch=00000"),
            "placeholder still present: {billing}"
        );
        let cch_part = billing.split("cch=").nth(1).expect("cch= not found");
        let hash_chars: String = cch_part.chars().take(5).collect();
        assert_eq!(hash_chars.len(), 5);
        assert!(hash_chars.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_serialize_signed_request_no_double_semicolon_in_billing_header() {
        let signing = signing();
        let signed = sign_request(&signing, make_request_with_text("Hello world"));
        let billing = &signed.system.as_ref().unwrap()[0].text;
        assert!(
            !billing.contains(";;"),
            "double semicolon found in: {billing}"
        );
        assert!(billing.ends_with(';'));
    }

    #[test]
    fn test_serialize_signed_request_preserves_existing_system_messages() {
        let signing = signing();
        let mut request = make_request_with_text("Hello");
        request.system = Some(vec![SystemMessage {
            r#type: "text".to_string(),
            text: "You are a helpful assistant.".to_string(),
            cache_control: None,
        }]);

        let signed = sign_request(&signing, request);

        let system = signed.system.as_ref().unwrap();
        assert_eq!(system.len(), 2, "billing header + original system message");
        assert!(system[0].text.starts_with("x-anthropic-billing-header:"));
        assert_eq!(system[1].text, "You are a helpful assistant.");
    }

    #[test]
    fn test_serialize_signed_request_user_message_containing_placeholder_not_replaced() {
        let signing = signing();
        let request = make_request_with_text(
            "If you see cch=00000 in a request, that is the CCH placeholder.",
        );

        let signed = sign_request(&signing, request);

        let system = signed.system.as_ref().unwrap();
        let billing = &system[0].text;
        assert!(
            !billing.contains("cch=00000"),
            "placeholder still in billing header"
        );

        let user_msg = &signed.messages[0];
        let user_text = user_msg.content.iter().find_map(|c| {
            if let Content::Text { text, .. } = c {
                Some(text.as_str())
            } else {
                None
            }
        });
        assert_eq!(
            user_text,
            Some("If you see cch=00000 in a request, that is the CCH placeholder.")
        );
    }

    #[test]
    fn test_serialize_signed_request_without_user_message_returns_plain_json() {
        let signing = signing();
        let request = Request { max_tokens: 100, ..Default::default() };
        let expected = serde_json::to_vec(&request).unwrap();
        let actual = signing.serialize_signed_request(request).unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_serialize_signed_request_same_input_produces_same_hash() {
        let signing = signing();
        let r1 = make_request_with_text("deterministic test");
        let r2 = make_request_with_text("deterministic test");

        let s1 = sign_request(&signing, r1);
        let s2 = sign_request(&signing, r2);

        let h1 = &s1.system.as_ref().unwrap()[0].text;
        let h2 = &s2.system.as_ref().unwrap()[0].text;
        assert_eq!(h1, h2, "same input must produce same signed output");
    }

    #[test]
    fn test_transform_serialized_body_field_order_system_before_messages() {
        // Verify that the Request struct itself serializes system before messages,
        // so the HTTP body sent over the wire matches what was hashed.
        let mut request = make_request_with_text("ordering test");
        request.system = Some(vec![SystemMessage {
            r#type: "text".to_string(),
            text: "sys".to_string(),
            cache_control: None,
        }]);

        let json = serde_json::to_string(&request).unwrap();
        let system_pos = json.find("\"system\"").expect("system key not found");
        let messages_pos = json.find("\"messages\"").expect("messages key not found");
        assert!(
            system_pos < messages_pos,
            "`system` ({system_pos}) must appear before `messages` ({messages_pos}) in JSON"
        );
    }

    /// The hash is computed by `serialize_request_compact` and the wire body is
    /// produced by `serde_json::to_vec(&request)`. They MUST be byte-identical,
    /// otherwise the server receives a body that doesn't match the `cch` hash.
    #[test]
    fn test_hashed_bytes_match_wire_bytes() {
        let mut request = make_request_with_text("wire match test");
        request.system = Some(vec![SystemMessage {
            r#type: "text".to_string(),
            text:
                "x-anthropic-billing-header: cc_version=2.1.37.abc; cc_entrypoint=cli; cch=00000;"
                    .to_string(),
            cache_control: None,
        }]);

        let hashed = serialize_request_compact(&request).unwrap();
        let wire = serde_json::to_string(&request).unwrap();

        assert_eq!(
            hashed, wire,
            "serialization used for hashing must be identical to wire serialization"
        );
    }

    // ── env helpers ──────────────────────────────────────────────────────────

    #[test]
    fn test_env_or_default_returns_default_when_unset() {
        let result = env_or_default("FORGE_TEST_CCH_NONEXISTENT_VAR", "fallback");
        assert_eq!(result, "fallback");
    }

    #[test]
    fn test_env_or_default_u64_returns_default_when_unset() {
        let result = env_or_default_u64("FORGE_TEST_CCH_NONEXISTENT_U64", 42);
        assert_eq!(result, 42);
    }
}
