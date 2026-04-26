use derive_setters::Setters;
use forge_domain::{
    Agent, AgentId, Compact, EventContext, MaxTokens, ModelId, ProviderId, ReasoningConfig,
    SystemContext, Temperature, Template, ToolName, TopK, TopP,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Agent definition - used for deserialization from configuration files.
/// Fields like model and provider are optional to support defaults.
/// This type is a repo concern: it models how agents are stored on disk and
/// is converted to the domain [`Agent`] type before use.
#[derive(Debug, Clone, Serialize, Deserialize, Setters, JsonSchema)]
#[setters(strip_option, into)]
#[serde(rename = "Agent")]
pub(crate) struct AgentDefinition {
    /// Flag to enable/disable tool support for this agent.
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_supported: Option<bool>,

    // Unique identifier for the agent
    pub id: AgentId,

    /// Path to the agent definition file, if loaded from a file
    #[serde(skip)]
    pub path: Option<String>,

    /// Human-readable title for the agent
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<ProviderId>,

    // The language model ID to be used by this agent
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<ModelId>,

    // Human-readable description of the agent's purpose
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    // Template for the system prompt provided to the agent
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<Template<SystemContext>>,

    // Template for the user prompt provided to the agent
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_prompt: Option<Template<EventContext>>,

    /// Tools that the agent can use
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ToolName>>,

    /// Maximum number of turns the agent can take
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_turns: Option<u64>,

    /// Configuration for automatic context compaction
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compact: Option<Compact>,

    /// A set of custom rules that the agent should follow
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_rules: Option<String>,

    /// Temperature used for agent
    ///
    /// Temperature controls the randomness in the model's output.
    /// - Lower values (e.g., 0.1) make responses more focused, deterministic,
    ///   and coherent
    /// - Higher values (e.g., 0.8) make responses more creative, diverse, and
    ///   exploratory
    /// - Valid range is 0.0 to 2.0
    /// - If not specified, the model provider's default temperature will be
    ///   used
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<Temperature>,

    /// Top-p (nucleus sampling) used for agent
    ///
    /// Controls the diversity of the model's output by considering only the
    /// most probable tokens up to a cumulative probability threshold.
    /// - Lower values (e.g., 0.1) make responses more focused
    /// - Higher values (e.g., 0.9) make responses more diverse
    /// - Valid range is 0.0 to 1.0
    /// - If not specified, the model provider's default will be used
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<TopP>,

    /// Top-k used for agent
    ///
    /// Controls the number of highest probability vocabulary tokens to keep.
    /// - Lower values (e.g., 10) make responses more focused
    /// - Higher values (e.g., 100) make responses more diverse
    /// - Valid range is 1 to 1000
    /// - If not specified, the model provider's default will be used
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_k: Option<TopK>,

    /// Maximum number of tokens the model can generate
    ///
    /// Controls the maximum length of the model's response.
    /// - Lower values (e.g., 100) limit response length for concise outputs
    /// - Higher values (e.g., 4000) allow for longer, more detailed responses
    /// - Valid range is 1 to 100,000
    /// - If not specified, the model provider's default will be used
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<MaxTokens>,

    /// Reasoning configuration for the agent.
    /// Controls the reasoning capabilities of the agent
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<ReasoningConfig>,

    /// Maximum number of times a tool can fail before sending the response back
    /// to the LLM forces the completion.
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tool_failure_per_turn: Option<usize>,

    /// Maximum number of requests that can be made in a single turn
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_requests_per_turn: Option<usize>,
}

impl AgentDefinition {
    /// Converts this definition into a domain [`Agent`] by applying the
    /// provided default provider and model when the definition does not
    /// specify its own.
    ///
    /// # Arguments
    ///
    /// * `provider_id` - Default provider to use when the definition has none
    /// * `model_id` - Default model to use when the definition has none
    pub fn into_agent(self, provider_id: ProviderId, model_id: ModelId) -> Agent {
        Agent {
            tool_supported: self.tool_supported,
            id: self.id,
            title: self.title,
            description: self.description,
            provider: self.provider.unwrap_or(provider_id),
            model: self.model.unwrap_or(model_id),
            system_prompt: self.system_prompt,
            user_prompt: self.user_prompt,
            temperature: self.temperature,
            max_tokens: self.max_tokens,
            top_p: self.top_p,
            top_k: self.top_k,
            tools: self.tools,
            reasoning: self.reasoning,
            compact: self.compact.unwrap_or_default(),
            max_turns: self.max_turns,
            custom_rules: self.custom_rules,
            max_tool_failure_per_turn: self.max_tool_failure_per_turn,
            max_requests_per_turn: self.max_requests_per_turn,
            path: self.path,
        }
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use serde_json::json;

    use super::*;

    #[test]
    fn test_temperature_validation() {
        // Valid temperature values should deserialize correctly
        let valid_temps = [0.0, 0.5, 1.0, 1.5, 2.0];
        for temp in valid_temps {
            let json = json!({
                "id": "test-agent",
                "temperature": temp
            });

            let agent: std::result::Result<AgentDefinition, serde_json::Error> =
                serde_json::from_value(json);
            assert!(agent.is_ok(), "Valid temperature {temp} should deserialize");
            assert_eq!(agent.unwrap().temperature.unwrap().value(), temp);
        }

        // Invalid temperature values should fail deserialization
        let invalid_temps = [-0.1, 2.1, 3.0, -1.0, 10.0];
        for temp in invalid_temps {
            let json = json!({
                "id": "test-agent",
                "temperature": temp
            });

            let agent: std::result::Result<AgentDefinition, serde_json::Error> =
                serde_json::from_value(json);
            assert!(
                agent.is_err(),
                "Invalid temperature {temp} should fail deserialization"
            );
            let err = agent.unwrap_err().to_string();
            assert!(
                err.contains("temperature must be between 0.0 and 2.0"),
                "Error should mention valid range: {err}"
            );
        }

        // No temperature should deserialize to None
        let json = json!({
            "id": "test-agent"
        });

        let agent: AgentDefinition = serde_json::from_value(json).unwrap();
        assert_eq!(agent.temperature, None);
    }

    #[test]
    fn test_top_p_validation() {
        // Valid top_p values should deserialize correctly
        let valid_values = [0.0, 0.1, 0.5, 0.9, 1.0];
        for value in valid_values {
            let json = json!({
                "id": "test-agent",
                "top_p": value
            });

            let agent: std::result::Result<AgentDefinition, serde_json::Error> =
                serde_json::from_value(json);
            assert!(agent.is_ok(), "Valid top_p {value} should deserialize");
            assert_eq!(agent.unwrap().top_p.unwrap().value(), value);
        }

        // Invalid top_p values should fail deserialization
        let invalid_values = [-0.1, 1.1, 2.0, -1.0, 10.0];
        for value in invalid_values {
            let json = json!({
                "id": "test-agent",
                "top_p": value
            });

            let agent: std::result::Result<AgentDefinition, serde_json::Error> =
                serde_json::from_value(json);
            assert!(
                agent.is_err(),
                "Invalid top_p {value} should fail deserialization"
            );
            let err = agent.unwrap_err().to_string();
            assert!(
                err.contains("top_p must be between 0.0 and 1.0"),
                "Error should mention valid range: {err}"
            );
        }

        // No top_p should deserialize to None
        let json = json!({
            "id": "test-agent"
        });

        let agent: AgentDefinition = serde_json::from_value(json).unwrap();
        assert_eq!(agent.top_p, None);
    }

    #[test]
    fn test_top_k_validation() {
        // Valid top_k values should deserialize correctly
        let valid_values = [1, 10, 50, 100, 500, 1000];
        for value in valid_values {
            let json = json!({
                "id": "test-agent",
                "top_k": value
            });

            let agent: std::result::Result<AgentDefinition, serde_json::Error> =
                serde_json::from_value(json);
            assert!(agent.is_ok(), "Valid top_k {value} should deserialize");
            assert_eq!(agent.unwrap().top_k.unwrap().value(), value);
        }

        // Invalid top_k values should fail deserialization
        let invalid_values = [0, 1001, 2000, 5000];
        for value in invalid_values {
            let json = json!({
                "id": "test-agent",
                "top_k": value
            });

            let agent: std::result::Result<AgentDefinition, serde_json::Error> =
                serde_json::from_value(json);
            assert!(
                agent.is_err(),
                "Invalid top_k {value} should fail deserialization"
            );
            let err = agent.unwrap_err().to_string();
            assert!(
                err.contains("top_k must be between 1 and 1000"),
                "Error should mention valid range: {err}"
            );
        }

        // No top_k should deserialize to None
        let json = json!({
            "id": "test-agent"
        });

        let agent: AgentDefinition = serde_json::from_value(json).unwrap();
        assert_eq!(agent.top_k, None);
    }

    #[test]
    fn test_max_tokens_validation() {
        // Valid max_tokens values should deserialize correctly
        let valid_values = [1, 100, 1000, 4000, 8000, 100_000];
        for value in valid_values {
            let json = json!({
                "id": "test-agent",
                "max_tokens": value
            });

            let agent: std::result::Result<AgentDefinition, serde_json::Error> =
                serde_json::from_value(json);
            assert!(agent.is_ok(), "Valid max_tokens {value} should deserialize");
            assert_eq!(agent.unwrap().max_tokens.unwrap().value(), value);
        }

        // Invalid max_tokens values should fail deserialization
        let invalid_values = [0, 100_001, 200_000, 1_000_000];
        for value in invalid_values {
            let json = json!({
                "id": "test-agent",
                "max_tokens": value
            });

            let agent: std::result::Result<AgentDefinition, serde_json::Error> =
                serde_json::from_value(json);
            assert!(
                agent.is_err(),
                "Invalid max_tokens {value} should fail deserialization"
            );
            let err = agent.unwrap_err().to_string();
            assert!(
                err.contains("max_tokens must be between 1 and 100000"),
                "Error should mention valid range: {err}"
            );
        }

        // No max_tokens should deserialize to None
        let json = json!({
            "id": "test-agent"
        });

        let agent: AgentDefinition = serde_json::from_value(json).unwrap();
        assert_eq!(agent.max_tokens, None);
    }
}
