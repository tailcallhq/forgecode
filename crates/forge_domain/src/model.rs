use derive_more::derive::Display;
use derive_setters::Setters;
use fake::Dummy;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use strum_macros::EnumString;

/// Represents input modalities that a model can accept
#[derive(
    Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, EnumString, JsonSchema, Dummy,
)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase", ascii_case_insensitive)]
pub enum InputModality {
    /// Text input (all models support this)
    Text,
    /// Image input (vision-capable models)
    Image,
}

/// Default input modalities when not specified (text-only)
fn default_input_modalities() -> Vec<InputModality> {
    vec![InputModality::Text]
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, Setters, JsonSchema, Dummy)]
#[setters(strip_option)]
pub struct Model {
    pub id: ModelId,
    pub name: Option<String>,
    pub description: Option<String>,
    pub context_length: Option<u64>,
    // TODO: add provider information to the model
    pub tools_supported: Option<bool>,
    /// Whether the model supports parallel tool calls
    pub supports_parallel_tool_calls: Option<bool>,
    /// Whether the model supports reasoning
    pub supports_reasoning: Option<bool>,
    /// Input modalities supported by the model (defaults to text-only)
    #[serde(default = "default_input_modalities")]
    pub input_modalities: Vec<InputModality>,
}

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct Parameters {
    pub tool_supported: bool,
}

impl Parameters {
    pub fn new(tool_supported: bool) -> Self {
        Self { tool_supported }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, Hash, Eq, Display, JsonSchema, Dummy)]
#[serde(transparent)]
pub struct ModelId(String);

impl ModelId {
    pub fn new<T: Into<String>>(id: T) -> Self {
        Self(id.into())
    }
}

impl Model {
    /// Creates a new `Model` with the given id and default values for all other
    /// fields.
    pub fn new(id: impl Into<ModelId>) -> Self {
        Self {
            id: id.into(),
            name: None,
            description: None,
            context_length: None,
            tools_supported: None,
            supports_parallel_tool_calls: None,
            supports_reasoning: None,
            input_modalities: default_input_modalities(),
        }
    }
}

impl From<String> for ModelId {
    fn from(value: String) -> Self {
        ModelId(value)
    }
}

impl From<&str> for ModelId {
    fn from(value: &str) -> Self {
        ModelId(value.to_string())
    }
}

impl ModelId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::str::FromStr for ModelId {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(ModelId(s.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use pretty_assertions::assert_eq;

    use super::*;

    /// Reusable fixture producing a bare `Model` with default fields.
    fn model_fixture() -> Model {
        Model::new("gpt-4o")
    }

    #[test]
    fn test_model_new_defaults_to_text_only() {
        let actual = model_fixture();

        let expected = Model {
            id: ModelId::new("gpt-4o"),
            name: None,
            description: None,
            context_length: None,
            tools_supported: None,
            supports_parallel_tool_calls: None,
            supports_reasoning: None,
            input_modalities: vec![InputModality::Text],
        };
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_setters_strip_option_wraps_values() {
        let actual = model_fixture()
            .name("GPT-4o".to_string())
            .context_length(128_000u64)
            .tools_supported(true)
            .supports_reasoning(false);

        let expected = Model {
            id: ModelId::new("gpt-4o"),
            name: Some("GPT-4o".to_string()),
            description: None,
            context_length: Some(128_000),
            tools_supported: Some(true),
            supports_parallel_tool_calls: None,
            supports_reasoning: Some(false),
            input_modalities: vec![InputModality::Text],
        };
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_model_id_new_and_as_str_roundtrip() {
        let fixture = ModelId::new("claude-3");

        let actual = fixture.as_str();

        let expected = "claude-3";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_model_id_from_str_matches_from_string() {
        let actual = ModelId::from_str("anthropic/claude").unwrap();

        let expected = ModelId::from("anthropic/claude".to_string());
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_model_id_displays_inner_string() {
        let fixture = ModelId::new("o1-mini");

        let actual = fixture.to_string();

        let expected = "o1-mini".to_string();
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_model_id_serializes_transparently() {
        let fixture = ModelId::new("gemini-pro");

        let actual = serde_json::to_string(&fixture).unwrap();

        let expected = "\"gemini-pro\"".to_string();
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_input_modality_serializes_lowercase() {
        let fixture = vec![InputModality::Text, InputModality::Image];

        let actual = serde_json::to_string(&fixture).unwrap();

        let expected = "[\"text\",\"image\"]".to_string();
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_input_modality_from_str_is_case_insensitive() {
        let actual = InputModality::from_str("IMAGE").unwrap();

        let expected = InputModality::Image;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_deserialize_model_applies_default_input_modalities() {
        let setup = r#"{"id":"gpt-4o","name":null,"description":null,"context_length":null,
            "tools_supported":null,"supports_parallel_tool_calls":null,"supports_reasoning":null}"#;

        let actual: Model = serde_json::from_str(setup).unwrap();

        let expected = model_fixture();
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_parameters_new_sets_tool_supported() {
        let actual = Parameters::new(true).tool_supported;

        let expected = true;
        assert_eq!(actual, expected);
    }
}
