use forge_domain::Transformer;

use crate::dto::openai::Request;

/// Moves session_id to prompt_cache_isolation_key for Fireworks provider.
/// Clears session_id so it's not serialized in the request body.
pub struct MakeFireworksCompat;

impl Transformer for MakeFireworksCompat {
    type Value = Request;

    fn transform(&mut self, mut request: Self::Value) -> Self::Value {
        // Transfer session_id to prompt_cache_isolation_key for Fireworks
        if let Some(session_id) = request.session_id.take() {
            request.prompt_cache_isolation_key = Some(session_id);
        }
        request
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn test_transform_clears_session_id() {
        let mut transformer = MakeFireworksCompat;

        let request = Request {
            session_id: Some("test-session-123".to_string()),
            prompt_cache_isolation_key: Some("test-session-123".to_string()),
            ..Default::default()
        };

        let result = transformer.transform(request);

        assert_eq!(result.session_id, None);
        assert_eq!(
            result.prompt_cache_isolation_key,
            Some("test-session-123".to_string())
        );
    }

    #[test]
    fn test_transform_without_session_id() {
        let mut transformer = MakeFireworksCompat;

        let request = Request { session_id: None, ..Default::default() };

        let result = transformer.transform(request);

        assert_eq!(result.session_id, None);
    }
}
