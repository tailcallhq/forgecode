use forge_domain::Transformer;

use crate::dto::openai::Request;

/// Transformer that applies minimax-specific parameter adjustments
///
/// Minimax models require specific temperature, top_p, and top_k values
/// for optimal performance:
/// - Temperature: 1.0
/// - Top P: 0.95
/// - Top K: 40 (for m2.1), 20 (for all other models including M2, M2.5, M2.7,
///   M3)
///
/// These parameters are based on official MiniMax evaluation methodology
/// (see VideoMMMU, Video-MME benchmarks in M3 blog post).
pub struct SetMinimaxParams;

impl Transformer for SetMinimaxParams {
    type Value = Request;

    fn transform(&mut self, mut request: Self::Value) -> Self::Value {
        let model_id = request
            .model
            .as_ref()
            .map(|m| m.as_str().to_lowercase())
            .unwrap_or_default();

        // Match MiniMax model patterns (minimax-m2, minimax-m3, etc.)
        let is_minimax = model_id.contains("minimax-m2") || model_id.contains("minimax-m3");

        if !is_minimax {
            return request;
        }

        // Set temperature to 1.0 for minimax models
        request.temperature = Some(1.0);

        // Set top_p to 0.95 for minimax models
        request.top_p = Some(0.95);

        // Set top_k based on model variant
        if model_id.contains("minimax-m2.1") {
            request.top_k = Some(40);
        } else {
            // M2, M2.5, M2.7, M3 all use top_k = 20
            request.top_k = Some(20);
        }

        request
    }
}

#[cfg(test)]
mod tests {
    use forge_domain::ModelId;
    use pretty_assertions::assert_eq;

    use super::*;

    fn create_request_fixture(model: &str) -> Request {
        Request::default()
            .model(ModelId::new(model))
            .temperature(0.7)
            .top_p(0.8)
            .top_k(50)
    }

    #[test]
    fn test_minimax_m2_sets_parameters() {
        let fixture = create_request_fixture("minimax-m2");
        let mut transformer = SetMinimaxParams;
        let actual = transformer.transform(fixture);

        assert_eq!(actual.temperature, Some(1.0));
        assert_eq!(actual.top_p, Some(0.95));
        assert_eq!(actual.top_k, Some(20));
    }

    #[test]
    fn test_minimax_m2_1_sets_higher_top_k() {
        let fixture = create_request_fixture("minimax-m2.1-large");
        let mut transformer = SetMinimaxParams;
        let actual = transformer.transform(fixture);

        assert_eq!(actual.temperature, Some(1.0));
        assert_eq!(actual.top_p, Some(0.95));
        assert_eq!(actual.top_k, Some(40));
    }

    #[test]
    fn test_non_minimax_model_unchanged() {
        let fixture = create_request_fixture("gpt-4");
        let mut transformer = SetMinimaxParams;
        let actual = transformer.transform(fixture.clone());

        assert_eq!(actual.temperature, fixture.temperature);
        assert_eq!(actual.top_p, fixture.top_p);
        assert_eq!(actual.top_k, fixture.top_k);
    }

    #[test]
    fn test_minimax_case_insensitive() {
        let fixture = create_request_fixture("MiniMax-M2-Pro");
        let mut transformer = SetMinimaxParams;
        let actual = transformer.transform(fixture);

        assert_eq!(actual.temperature, Some(1.0));
        assert_eq!(actual.top_p, Some(0.95));
        assert_eq!(actual.top_k, Some(20));
    }

    #[test]
    fn test_minimax_m2_1_case_insensitive() {
        let fixture = create_request_fixture("MINIMAX-M2.1-XL");
        let mut transformer = SetMinimaxParams;
        let actual = transformer.transform(fixture);

        assert_eq!(actual.temperature, Some(1.0));
        assert_eq!(actual.top_p, Some(0.95));
        assert_eq!(actual.top_k, Some(40));
    }

    #[test]
    fn test_minimax_m2_with_no_existing_parameters() {
        let fixture = Request::default().model(ModelId::new("minimax-m2"));
        let mut transformer = SetMinimaxParams;
        let actual = transformer.transform(fixture);

        assert_eq!(actual.temperature, Some(1.0));
        assert_eq!(actual.top_p, Some(0.95));
        assert_eq!(actual.top_k, Some(20));
    }

    #[test]
    fn test_minimax_partial_match_ignored() {
        let fixture = create_request_fixture("not-minimax");
        let mut transformer = SetMinimaxParams;
        let actual = transformer.transform(fixture.clone());

        assert_eq!(actual.temperature, fixture.temperature);
        assert_eq!(actual.top_p, fixture.top_p);
        assert_eq!(actual.top_k, fixture.top_k);
    }

    #[test]
    fn test_no_model_unchanged() {
        let fixture = Request::default().temperature(0.7).top_p(0.8).top_k(50);
        let mut transformer = SetMinimaxParams;
        let actual = transformer.transform(fixture.clone());

        assert_eq!(actual.temperature, fixture.temperature);
        assert_eq!(actual.top_p, fixture.top_p);
        assert_eq!(actual.top_k, fixture.top_k);
    }

    #[test]
    fn test_minimax_m3_sets_parameters() {
        let fixture = create_request_fixture("minimax-m3");
        let mut transformer = SetMinimaxParams;
        let actual = transformer.transform(fixture);

        assert_eq!(actual.temperature, Some(1.0));
        assert_eq!(actual.top_p, Some(0.95));
        assert_eq!(actual.top_k, Some(20)); // M3 uses same config as M2.7
    }

    #[test]
    fn test_minimax_m3_case_insensitive() {
        let fixture = create_request_fixture("MiniMax-M3");
        let mut transformer = SetMinimaxParams;
        let actual = transformer.transform(fixture);

        assert_eq!(actual.temperature, Some(1.0));
        assert_eq!(actual.top_p, Some(0.95));
        assert_eq!(actual.top_k, Some(20)); // M3 uses same config as M2.7
    }

    #[test]
    fn test_minimax_m3_bedrock_provider_id() {
        let fixture = create_request_fixture("minimax.minimax-m3");
        let mut transformer = SetMinimaxParams;
        let actual = transformer.transform(fixture);

        assert_eq!(actual.temperature, Some(1.0));
        assert_eq!(actual.top_p, Some(0.95));
        assert_eq!(actual.top_k, Some(20)); // M3 uses same config as M2.7
    }
}
