use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{ModelId, ProviderId};

/// Domain-level configuration that pairs a provider with a model.
///
/// Used as the unified payload for [`super::ConfigOperation`] variants that
/// configure a provider/model pair (session, commit, suggest). Both fields are
/// required; use `Option<ModelConfig>` at the call-site when the configuration
/// is optional.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct ModelConfig {
    /// The provider ID (e.g. `"anthropic"`).
    pub provider: ProviderId,

    /// The model ID to use with this provider.
    pub model: ModelId,
}

impl ModelConfig {
    /// Creates a new [`ModelConfig`] with the given provider and model.
    ///
    /// # Arguments
    /// * `provider` - The provider identifier
    /// * `model` - The model identifier
    pub fn new(provider: impl Into<ProviderId>, model: impl Into<ModelId>) -> Self {
        Self { provider: provider.into(), model: model.into() }
    }
}
