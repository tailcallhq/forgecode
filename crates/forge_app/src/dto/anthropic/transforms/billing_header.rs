use forge_domain::Transformer;
use sha2::{Digest, Sha256};

use crate::dto::anthropic::{Content, Message, Request, Role, SystemMessage};

/// Claude Code version used for billing header computation.
const CLAUDE_CODE_VERSION: &str = "1.0.43";

/// Salt used in version suffix computation.
const CCH_SALT: &str = "v3";

/// Character positions sampled from the first user message for version suffix.
const CCH_POSITIONS: &[usize] = &[3, 11, 19, 27, 35, 43, 51, 59, 67, 75];

/// Entrypoint name reported in the billing header.
const ENTRYPOINT: &str = "forge";

/// Adds the Anthropic billing header as the first system message block.
///
/// This mimics the official Claude Code client's billing telemetry so
/// Anthropic routes usage to the user's Claude Code subscription rather
/// than API credits.
pub struct BillingHeader;

impl BillingHeader {
    /// Extract plain text from the first user message's first text block.
    fn extract_first_user_text(messages: &[Message]) -> String {
        let user_msg = messages.iter().find(|m| matches!(m.role, Role::User));
        let Some(user_msg) = user_msg else {
            return String::new();
        };

        user_msg
            .content
            .iter()
            .find_map(|block| match block {
                Content::Text { text, .. } => Some(text.clone()),
                _ => None,
            })
            .unwrap_or_default()
    }

    /// Compute `cch`: first 5 hex characters of SHA-256(text).
    fn compute_cch(text: &str) -> String {
        let hash = Sha256::digest(text.as_bytes());
        hex::encode(hash)[..5].to_string()
    }

    /// Compute the 3-character version suffix from sampled message characters.
    fn compute_version_suffix(text: &str) -> String {
        let chars: String = CCH_POSITIONS
            .iter()
            .map(|&pos| text.chars().nth(pos).unwrap_or('0'))
            .collect();

        let input = format!("{CCH_SALT}{chars}{CLAUDE_CODE_VERSION}");
        let hash = Sha256::digest(input.as_bytes());
        hex::encode(hash)[..3].to_string()
    }

    /// Build the complete billing header value.
    fn build_header_value(messages: &[Message]) -> String {
        let text = Self::extract_first_user_text(messages);
        let suffix = Self::compute_version_suffix(&text);
        let cch = Self::compute_cch(&text);

        format!(
            "x-anthropic-billing-header: cc_version={CLAUDE_CODE_VERSION}.{suffix}; cc_entrypoint={ENTRYPOINT}; cch={cch};"
        )
    }
}

impl Transformer for BillingHeader {
    type Value = Request;

    fn transform(&mut self, mut request: Self::Value) -> Self::Value {
        if request.messages.is_empty() {
            return request;
        }

        let header_text = Self::build_header_value(&request.messages);
        let billing_message = SystemMessage {
            r#type: "text".to_string(),
            text: header_text,
            cache_control: None,
        };

        let mut system_messages = request.system.take().unwrap_or_default();
        system_messages.insert(0, billing_message);
        request.system = Some(system_messages);
        request
    }
}

#[cfg(test)]
mod tests {
    use forge_domain::{Context, ContextMessage, ModelId};

    use super::*;

    #[test]
    fn test_build_header_value_format() {
        let messages = vec![Message {
            role: Role::User,
            content: vec![Content::Text {
                text: "Hello world this is a test message for billing".to_string(),
                cache_control: None,
            }],
        }];

        let header = BillingHeader::build_header_value(&messages);

        assert!(
            header.starts_with("x-anthropic-billing-header: cc_version=1.0.43."),
            "Header should start with correct prefix, got: {header}"
        );
        assert!(
            header.contains("cc_entrypoint=forge"),
            "Header should contain forge entrypoint, got: {header}"
        );
        assert!(
            header.contains("cch="),
            "Header should contain cch, got: {header}"
        );
    }

    #[test]
    fn test_transform_prepends_billing_header() {
        let context = Context::default()
            .add_message(ContextMessage::user("test message", Some(ModelId::new("claude-3-5-sonnet-20241022"))));

        let request = Request::try_from(context).unwrap();
        let transformed = BillingHeader.transform(request);

        let system = transformed.system.unwrap();
        assert_eq!(system.len(), 1);
        assert!(
            system[0].text.starts_with("x-anthropic-billing-header:"),
            "First system block should be billing header, got: {}",
            system[0].text
        );
    }

    #[test]
    fn test_transform_with_existing_system_messages() {
        let context = Context::default()
            .add_message(ContextMessage::system("You are helpful"))
            .add_message(ContextMessage::user("hello", Some(ModelId::new("claude-3-5-sonnet-20241022"))));

        let request = Request::try_from(context).unwrap();
        let transformed = BillingHeader.transform(request);

        let system = transformed.system.unwrap();
        assert_eq!(system.len(), 2);
        assert!(system[0].text.starts_with("x-anthropic-billing-header:"));
        assert_eq!(system[1].text, "You are helpful");
    }

    #[test]
    fn test_empty_messages_no_panic() {
        let request = Request::default();
        let transformed = BillingHeader.transform(request);
        assert!(transformed.system.is_none() || transformed.system.as_ref().unwrap().is_empty());
    }
}
