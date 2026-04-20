use crate::Effort;
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
    /// Reasoning effort levels supported by the model (if applicable)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supported_reasoning_efforts: Option<Vec<Effort>>,
    /// Input modalities supported by the model (defaults to text-only)
    #[serde(default = "default_input_modalities")]
    pub input_modalities: Vec<InputModality>,
}

impl Model {
    /// Returns the supported reasoning effort levels for this model.
    ///
    /// This method prioritizes the explicitly defined `supported_reasoning_efforts` 
    /// field. If that field is missing or empty, it uses the model's identity 
    /// to determine a sensible set of levels if the model supports reasoning.
    pub fn reasoning_efforts(&self) -> Vec<Effort> {
        // 1. Prioritize explicit definition from the provider/config
        if let Some(ref efforts) = self.supported_reasoning_efforts
            && !efforts.is_empty()
        {
            return efforts.clone();
        }

        // 2. If the model doesn't support reasoning at all, return empty
        if !self.supports_reasoning.unwrap_or(false) {
            return vec![];
        }

        // 3. Fallback to robust family-based logic for models that support reasoning
        // but haven't explicitly listed their levels (e.g. from dynamic APIs).
        let id = self.id.as_str().to_lowercase().replace('.', "-");

        // Anthropic Claude 4 adaptive reasoning family
        if id.contains("opus-4-7") || id.contains("opus-4-6") {
            return vec![Effort::Low, Effort::Medium, Effort::High, Effort::Max];
        }

        // Anthropic Claude 4 sonnet family
        if id.contains("sonnet-4-6") {
            return vec![Effort::Low, Effort::Medium, Effort::High];
        }

        // Anthropic Claude 3.7 / 4.5 family
        if id.contains("claude-3-7") || id.contains("opus-4-5") || id.contains("sonnet-4-5") || id.contains("haiku-4-5") {
            return vec![Effort::Low, Effort::Medium, Effort::High];
        }

        // GLM family (ZhipuAI)
        if id.contains("glm") {
            return vec![Effort::Low, Effort::Medium, Effort::High];
        }

        // OpenAI o1/o3 family
        if id.contains("o1") || id.contains("o3") {
            return vec![Effort::Low, Effort::Medium, Effort::High];
        }

        // DeepSeek R1 / V3 family
        if id.contains("deepseek-r1") || id.contains("deepseek-v3") {
            return vec![Effort::Low, Effort::Medium, Effort::High];
        }

        // GPT-5.4 series (OpenAI frontier)
        if id.contains("gpt-5-4") {
            return vec![Effort::Low, Effort::Medium, Effort::High, Effort::XHigh];
        }

        // Generic fallback for any other reasoning-capable model.
        // Most OpenAI-compatible APIs support at least these three.
        vec![Effort::Low, Effort::Medium, Effort::High]
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
