use derive_setters::Setters;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use strum_macros::Display as StrumDisplay;

/// Controls the reasoning behaviour of a model, including effort level, token
/// budget, and visibility of the thinking process.
#[derive(
    Default, Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, fake::Dummy, Setters,
)]
#[serde(rename_all = "snake_case")]
#[setters(strip_option)]
pub struct ReasoningConfig {
    /// Controls the effort level of the model's reasoning.
    /// Supported by openrouter and forge provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<Effort>,

    /// Controls how many tokens the model can spend thinking.
    /// Should be greater than 1024 but less than the overall max_tokens.
    /// Supported by openrouter, anthropic, and forge provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<usize>,

    /// When true, the model thinks deeply but the reasoning is hidden from the
    /// caller. Supported by openrouter and forge provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclude: Option<bool>,

    /// Enables reasoning at the "medium" effort level with no exclusions.
    /// Supported by openrouter, anthropic, and forge provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
}

/// Effort level for model reasoning.
#[derive(
    Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, fake::Dummy, StrumDisplay,
)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase")]
pub enum Effort {
    /// No reasoning; skips the thinking step entirely.
    None,
    /// Minimal reasoning; fastest and cheapest.
    Minimal,
    /// Low reasoning effort.
    Low,
    /// Medium reasoning effort; the default for most providers.
    Medium,
    /// High reasoning effort.
    High,
    /// Extra-high reasoning effort (OpenAI / OpenRouter).
    XHigh,
    /// Maximum reasoning effort; only available on select Anthropic models.
    Max,
}
