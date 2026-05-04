use derive_more::derive::Display;
use derive_setters::Setters;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use strum_macros::EnumString;

/// Represents input modalities that a model can accept
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, EnumString)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase", ascii_case_insensitive)]
pub enum InputModality {
    /// Text input (all models support this)
    Text,
    /// Image input (vision-capable models)
    Image,
}

/// Anthropic model family metadata used by provider-specific request
/// normalization.
///
/// Families are only assigned where Anthropic exposes special reasoning
/// semantics. Claude 4, 4.1, Sonnet 4.5, and Haiku 4.5 intentionally remain
/// untagged so they continue through legacy reasoning handling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AnthropicModelFamily {
    /// Claude Opus 4.7 adaptive-thinking family.
    #[serde(rename = "opus-4-7")]
    Opus47,
    /// Claude Opus 4.6 adaptive-thinking family.
    #[serde(rename = "opus-4-6")]
    Opus46,
    /// Claude Sonnet 4.6 adaptive-thinking family.
    #[serde(rename = "sonnet-4-6")]
    Sonnet46,
    /// Claude Opus 4.5 legacy-budget family with effort support.
    #[serde(rename = "opus-4-5")]
    Opus45,
    /// Unrecognized family metadata.
    ///
    /// Unknown strings are treated like missing catalog metadata so request
    /// normalization preserves legacy behavior. The original unrecognized
    /// string is not retained.
    #[serde(other)]
    Unknown,
}

impl AnthropicModelFamily {
    /// Infers an Anthropic model family from known canonical id fragments.
    ///
    /// # Arguments
    ///
    /// * `model_id` - Provider model id to inspect.
    pub fn infer_from_model_id(model_id: &str) -> Option<Self> {
        let id = model_id.to_lowercase();
        if id.contains("opus-4-7") || id.contains("47-opus") {
            Some(Self::Opus47)
        } else if id.contains("opus-4-6") || id.contains("46-opus") {
            Some(Self::Opus46)
        } else if id.contains("sonnet-4-6") || id.contains("46-sonnet") {
            Some(Self::Sonnet46)
        } else if id.contains("opus-4-5") || id.contains("45-opus") {
            Some(Self::Opus45)
        } else {
            None
        }
    }

    /// Returns whether this family should be used for reasoning normalization.
    pub fn is_known(self) -> bool {
        !matches!(self, Self::Unknown)
    }

    /// Returns whether chat requests need the interleaved-thinking beta header.
    pub fn requires_interleaved_thinking_header(self) -> bool {
        !matches!(self, Self::Opus47 | Self::Opus46 | Self::Sonnet46)
    }
}

/// Default input modalities when not specified (text-only)
fn default_input_modalities() -> Vec<InputModality> {
    vec![InputModality::Text]
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, Setters)]
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
    /// Optional Anthropic family metadata for provider-specific normalization.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub family: Option<AnthropicModelFamily>,
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

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, Hash, Eq, Display, JsonSchema)]
#[serde(transparent)]
pub struct ModelId(String);

impl ModelId {
    pub fn new<T: Into<String>>(id: T) -> Self {
        Self(id.into())
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
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn test_unknown_anthropic_family_deserializes_as_unknown() {
        let fixture = r#""sonnet-4-7""#;

        let actual: AnthropicModelFamily = serde_json::from_str(fixture).unwrap();

        let expected = AnthropicModelFamily::Unknown;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_model_with_unknown_family_deserializes() {
        let fixture = r#"{"id":"future-claude","family":"sonnet-4-7"}"#;

        let actual: Model = serde_json::from_str(fixture).unwrap();

        let expected = Some(AnthropicModelFamily::Unknown);
        assert_eq!(actual.family, expected);
    }
}
