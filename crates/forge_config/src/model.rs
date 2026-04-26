use derive_setters::Setters;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A type alias for a provider identifier string.
pub type ProviderId = String;

/// A type alias for a model identifier string.
pub type ModelId = String;

/// Pairs a provider and model together for a specific operation.
#[derive(Debug, Setters, Clone, PartialEq, Serialize, Deserialize, JsonSchema, fake::Dummy)]
#[setters(into)]
pub struct ModelConfig {
    /// The provider to use for this operation.
    pub provider_id: String,
    /// The model to use for this operation.
    pub model_id: String,
}

impl ModelConfig {
    /// Creates a new ModelConfig with the given provider and model IDs.
    pub fn new(provider_id: impl Into<String>, model_id: impl Into<String>) -> Self {
        Self { provider_id: provider_id.into(), model_id: model_id.into() }
    }
}
