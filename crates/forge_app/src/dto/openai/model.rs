use forge_domain::ModelId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(untagged)]
enum PriceValue {
    Number(f32),
    String(String),
}

impl From<PriceValue> for Option<f32> {
    fn from(value: PriceValue) -> Self {
        match value {
            PriceValue::Number(n) => Some(n),
            PriceValue::String(s) => s.parse().ok(),
        }
    }
}

impl TryFrom<PriceValue> for f32 {
    type Error = std::num::ParseFloatError;

    fn try_from(value: PriceValue) -> Result<Self, Self::Error> {
        match value {
            PriceValue::Number(n) => Ok(n),
            PriceValue::String(s) => s.parse(),
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Model {
    pub id: ModelId,
    pub name: Option<String>,
    pub created: Option<u64>,
    pub description: Option<String>,
    pub context_length: Option<u64>,
    pub architecture: Option<Architecture>,
    pub pricing: Option<Pricing>,
    pub top_provider: Option<TopProvider>,
    pub per_request_limits: Option<serde_json::Value>,
    pub supported_parameters: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Architecture {
    pub modality: String,
    pub tokenizer: String,
    pub instruct_type: Option<String>,
    pub input_modalities: Option<Vec<String>>,
    pub output_modalities: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
pub struct Pricing {
    #[serde(default, deserialize_with = "deserialize_optional_price")]
    pub prompt: Option<f32>,
    #[serde(default, deserialize_with = "deserialize_optional_price")]
    pub completion: Option<f32>,
    #[serde(default, deserialize_with = "deserialize_optional_price")]
    pub image: Option<f32>,
    #[serde(default, deserialize_with = "deserialize_optional_price")]
    pub request: Option<f32>,
}

fn deserialize_optional_price<'de, D>(deserializer: D) -> Result<Option<f32>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;

    match Option::<PriceValue>::deserialize(deserializer)? {
        Some(price_value) => match f32::try_from(price_value) {
            Ok(value) => Ok(Some(value)),
            Err(_) => Err(Error::custom("invalid string format for pricing value")),
        },
        None => Ok(None),
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct TopProvider {
    pub context_length: Option<u64>,
    pub max_completion_tokens: Option<u64>,
    pub is_moderated: bool,
}

#[derive(Debug, Deserialize, Clone, Serialize)]
pub struct ListModelResponse {
    pub data: Vec<Model>,
}

impl From<Model> for forge_domain::Model {
    fn from(value: Model) -> Self {
        let has_param = |name: &str| {
            value
                .supported_parameters
                .as_ref()
                .map(|params| params.iter().any(|p| p == name))
        };

        // Parse input modalities from OpenRouter's input_modalities field
        let input_modalities = value
            .architecture
            .as_ref()
            .and_then(|arch| arch.input_modalities.as_ref())
            .map(|modalities| {
                modalities
                    .iter()
                    .filter_map(|s| s.parse::<forge_domain::InputModality>().ok())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_else(|| vec![forge_domain::InputModality::Text]);

        let tools_supported = has_param("tools");
        let supports_parallel_tool_calls = has_param("supports_parallel_tool_calls");
        let supports_reasoning = has_param("reasoning");

        forge_domain::Model {
            id: value.id,
            name: value.name,
            description: value.description,
            context_length: value.context_length,
            tools_supported,
            supports_parallel_tool_calls,
            supports_reasoning,
            supported_reasoning_efforts: None,
            input_modalities,
        }
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    pub async fn load_fixture(filename: &str) -> serde_json::Value {
        let fixture_path = format!("src/dto/openai/fixtures/{}", filename);
        let fixture_content = tokio::fs::read_to_string(&fixture_path)
            .await
            .unwrap_or_else(|_| panic!("Failed to read fixture file: {}", fixture_path));
        serde_json::from_str(&fixture_content)
            .unwrap_or_else(|_| panic!("Failed to parse JSON fixture: {}", fixture_path))
    }

    #[tokio::test]
    async fn test_deserialize_model_with_numeric_pricing() {
        // This reproduces the issue where Chutes API returns numeric pricing instead of
        // strings
        let fixture = load_fixture("model_numeric_pricing.json").await;

        let actual = serde_json::from_value::<Model>(fixture).unwrap();

        // This should not fail - we should be able to handle numeric pricing
        assert_eq!(actual.pricing.as_ref().unwrap().prompt, Some(0.17992692));
        assert_eq!(
            actual.pricing.as_ref().unwrap().completion,
            Some(0.17992692)
        );
    }

    #[tokio::test]
    async fn test_deserialize_model_with_string_pricing() {
        let fixture = load_fixture("model_string_pricing.json").await;

        let actual = serde_json::from_value::<Model>(fixture).unwrap();
        let expected = Model {
            id: "test-model".into(),
            name: Some("Test Model".to_string()),
            created: None,
            description: None,
            context_length: None,
            architecture: None,
            pricing: Some(Pricing {
                prompt: Some(0.001),
                completion: Some(0.002),
                image: None,
                request: None,
            }),
            top_provider: None,
            per_request_limits: None,
            supported_parameters: None,
        };

        assert_eq!(actual.id, expected.id);
        assert_eq!(actual.name, expected.name);
        assert_eq!(
            actual.pricing.as_ref().unwrap().prompt,
            expected.pricing.as_ref().unwrap().prompt
        );
        assert_eq!(
            actual.pricing.as_ref().unwrap().completion,
            expected.pricing.as_ref().unwrap().completion
        );
    }

    #[tokio::test]
    async fn test_deserialize_model_with_mixed_pricing() {
        // Test with mixed string, numeric, and null pricing values
        let fixture = load_fixture("model_mixed_pricing.json").await;

        let actual = serde_json::from_value::<Model>(fixture).unwrap();

        assert_eq!(actual.pricing.as_ref().unwrap().prompt, Some(0.001));
        assert_eq!(actual.pricing.as_ref().unwrap().completion, Some(0.002));
        assert_eq!(actual.pricing.as_ref().unwrap().image, None);
        assert_eq!(actual.pricing.as_ref().unwrap().request, None);
    }

    #[tokio::test]
    async fn test_deserialize_model_without_pricing() {
        // Test that models without pricing field work correctly
        let fixture = load_fixture("model_no_pricing.json").await;

        let actual = serde_json::from_value::<Model>(fixture).unwrap();

        assert_eq!(actual.id.as_str(), "no-pricing-model");
        assert_eq!(actual.name, Some("No Pricing Model".to_string()));
        assert_eq!(actual.pricing, None);
    }

    #[tokio::test]
    async fn test_chutes_api_response_format() {
        // This simulates the actual Chutes API response format that was causing the
        // issue
        let fixture = load_fixture("chutes_api_response.json").await;

        let actual = serde_json::from_value::<ListModelResponse>(fixture).unwrap();

        assert_eq!(actual.data.len(), 1);
        let model = &actual.data[0];
        assert_eq!(model.id.as_str(), "moonshotai/Kimi-K2-Instruct-75k");
        assert_eq!(model.name, Some("Kimi K2 Instruct 75k".to_string()));
        assert_eq!(model.context_length, Some(75000));

        let pricing = model.pricing.as_ref().unwrap();
        assert_eq!(pricing.prompt, Some(0.17992692));
        assert_eq!(pricing.completion, Some(0.17992692));
        assert_eq!(pricing.image, None);
        assert_eq!(pricing.request, None);
    }

    #[tokio::test]
    async fn test_deserialize_model_with_invalid_string_pricing() {
        // Test that invalid string pricing formats fail gracefully
        let fixture = load_fixture("model_invalid_pricing.json").await;

        let actual = serde_json::from_value::<Model>(fixture);

        // This should fail with a parsing error
        assert!(actual.is_err());
        let error_message = format!("{}", actual.unwrap_err());
        assert!(error_message.contains("invalid string format for pricing value"));
    }

    #[tokio::test]
    async fn test_deserialize_model_with_scientific_notation_string() {
        // Test that scientific notation in strings works
        let fixture = load_fixture("model_scientific_notation.json").await;

        let actual = serde_json::from_value::<Model>(fixture).unwrap();

        assert_eq!(actual.pricing.as_ref().unwrap().prompt, Some(0.0015));
        assert_eq!(actual.pricing.as_ref().unwrap().completion, Some(0.0002));
    }
    #[tokio::test]
    async fn test_model_conversion_without_supported_parameters() {
        let model = Model {
            id: "test-model".into(),
            name: Some("Test Model".to_string()),
            created: None,
            description: Some("A test model".to_string()),
            context_length: Some(4096),
            architecture: None,
            pricing: None,
            top_provider: None,
            per_request_limits: None,
            supported_parameters: None, // No supported_parameters field
        };

        let domain_model: forge_domain::Model = model.into();

        // When supported_parameters is None, capabilities should be None (unknown)
        assert_eq!(domain_model.tools_supported, None);
        assert_eq!(domain_model.supports_parallel_tool_calls, None);
        assert_eq!(domain_model.supports_reasoning, None);
    }

    #[tokio::test]
    async fn test_model_conversion_with_supported_parameters() {
        let model = Model {
            id: "test-model".into(),
            name: Some("Test Model".to_string()),
            created: None,
            description: Some("A test model".to_string()),
            context_length: Some(4096),
            architecture: None,
            pricing: None,
            top_provider: None,
            per_request_limits: None,
            supported_parameters: Some(vec![
                "tools".to_string(),
                "reasoning".to_string(),
                // Note: "supports_parallel_tool_calls" is not included
            ]),
        };

        let domain_model: forge_domain::Model = model.into();

        // Should reflect what's actually in supported_parameters
        assert_eq!(domain_model.tools_supported, Some(true));
        assert_eq!(domain_model.supports_parallel_tool_calls, Some(false));
        assert_eq!(domain_model.supports_reasoning, Some(true));
    }
}
