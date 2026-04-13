use std::borrow::Cow;

use derive_more::derive::Display;
use derive_setters::Setters;
use merge::Merge;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use strum_macros::{Display as StrumDisplay, EnumString};

use crate::{
    Compact, Error, EventContext, MaxTokens, ModelId, ProviderId, Result, SystemContext,
    Temperature, Template, ToolDefinition, ToolName, TopK, TopP,
};

// Unique identifier for an agent
#[derive(Debug, Display, Eq, PartialEq, Hash, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct AgentId(Cow<'static, str>);

impl From<&str> for AgentId {
    fn from(value: &str) -> Self {
        AgentId(Cow::Owned(value.to_string()))
    }
}

impl AgentId {
    // Creates a new agent ID from a string-like value
    pub fn new(id: impl ToString) -> Self {
        Self(Cow::Owned(id.to_string()))
    }

    // Returns the agent ID as a string reference
    pub fn as_str(&self) -> &str {
        self.0.as_ref()
    }

    pub const FORGE: AgentId = AgentId(Cow::Borrowed("forge"));
    pub const MUSE: AgentId = AgentId(Cow::Borrowed("muse"));
    pub const SAGE: AgentId = AgentId(Cow::Borrowed("sage"));
}

impl Default for AgentId {
    fn default() -> Self {
        AgentId::FORGE
    }
}

#[derive(Default, Debug, Clone, Serialize, Deserialize, Merge, Setters, JsonSchema, PartialEq)]
#[setters(strip_option)]
#[merge(strategy = merge::option::overwrite_none)]
pub struct ReasoningConfig {
    /// Controls the effort level of the agent's reasoning
    /// supported by openrouter and forge provider
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<Effort>,

    /// Controls how many tokens the model can spend thinking.
    /// supported by openrouter, anthropic and forge provider
    /// should be greater then 1024 but less than overall max_tokens
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<usize>,

    /// Model thinks deeply, but the reasoning is hidden from you.
    /// supported by openrouter and forge provider
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclude: Option<bool>,

    /// Enables reasoning at the "medium" effort level with no exclusions.
    /// supported by openrouter, anthropic and forge provider
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, StrumDisplay, EnumString)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase", ascii_case_insensitive)]
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

/// Estimates the token count from a string representation
/// This is a simple estimation that should be replaced with a more accurate
/// tokenizer
/// Estimates token count from a string representation
/// Re-exported for compaction reporting
pub fn estimate_token_count(count: usize) -> usize {
    // A very rough estimation that assumes ~4 characters per token on average
    // In a real implementation, this should use a proper LLM-specific tokenizer
    count / 4
}

/// Runtime agent representation with required model and provider
#[derive(Debug, Clone, PartialEq, Setters, Serialize, Deserialize, JsonSchema)]
#[setters(strip_option, into)]
pub struct Agent {
    /// Unique identifier for the agent
    pub id: AgentId,

    /// Human-readable title for the agent
    pub title: Option<String>,

    /// Human-readable description of the agent's purpose
    pub description: Option<String>,

    /// Flag to enable/disable tool support for this agent.
    pub tool_supported: Option<bool>,

    /// Path to the agent definition file, if loaded from a file
    pub path: Option<String>,

    /// Required provider for the agent
    pub provider: ProviderId,

    /// Required language model ID to be used by this agent
    pub model: ModelId,

    /// Template for the system prompt provided to the agent
    pub system_prompt: Option<Template<SystemContext>>,

    /// Template for the user prompt provided to the agent
    pub user_prompt: Option<Template<EventContext>>,

    /// Tools that the agent can use
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ToolName>>,

    /// Maximum number of turns the agent can take
    pub max_turns: Option<u64>,

    /// Configuration for automatic context compaction
    pub compact: Compact,

    /// A set of custom rules that the agent should follow
    pub custom_rules: Option<String>,

    /// Temperature used for agent
    pub temperature: Option<Temperature>,

    /// Top-p (nucleus sampling) used for agent
    pub top_p: Option<TopP>,

    /// Top-k used for agent
    pub top_k: Option<TopK>,

    /// Maximum number of tokens the model can generate
    pub max_tokens: Option<MaxTokens>,

    /// Reasoning configuration for the agent.
    pub reasoning: Option<ReasoningConfig>,

    /// Maximum number of times a tool can fail before sending the response back
    pub max_tool_failure_per_turn: Option<usize>,

    /// Maximum number of requests that can be made in a single turn
    pub max_requests_per_turn: Option<usize>,
}

/// Lightweight metadata about an agent, used for listing without requiring a
/// configured provider or model.
#[derive(Debug, Default, Clone, PartialEq, Setters, Serialize, Deserialize, JsonSchema)]
#[setters(strip_option, into)]
pub struct AgentInfo {
    /// Unique identifier for the agent
    pub id: AgentId,

    /// Human-readable title for the agent
    pub title: Option<String>,

    /// Human-readable description of the agent's purpose
    pub description: Option<String>,
}

impl Agent {
    /// Create a new Agent with required provider and model
    pub fn new(id: impl Into<AgentId>, provider: ProviderId, model: ModelId) -> Self {
        Self {
            id: id.into(),
            title: Default::default(),
            description: Default::default(),
            provider,
            model,
            tool_supported: Default::default(),
            system_prompt: Default::default(),
            user_prompt: Default::default(),
            tools: Default::default(),
            max_turns: Default::default(),
            compact: Compact::default(),
            custom_rules: Default::default(),
            temperature: Default::default(),
            top_p: Default::default(),
            top_k: Default::default(),
            max_tokens: Default::default(),
            reasoning: Default::default(),
            max_tool_failure_per_turn: Default::default(),
            max_requests_per_turn: Default::default(),
            path: Default::default(),
        }
    }

    /// Creates a ToolDefinition from this agent
    ///
    /// # Errors
    ///
    /// Returns an error if the agent has no description
    pub fn tool_definition(&self) -> Result<ToolDefinition> {
        if self.description.is_none() || self.description.as_ref().is_none_or(|d| d.is_empty()) {
            return Err(Error::MissingAgentDescription(self.id.clone()));
        }
        Ok(ToolDefinition::new(self.id.as_str().to_string())
            .description(self.description.clone().unwrap()))
    }

    /// Sets the model in compaction config if not already set
    pub fn set_compact_model_if_none(mut self) -> Self {
        if self.compact.model.is_none() {
            self.compact.model = Some(self.model.clone());
        }
        self
    }

    /// Gets the tool ordering for this agent, derived from the tools list
    pub fn tool_order(&self) -> crate::ToolOrder {
        self.tools
            .as_ref()
            .map(|tools| crate::ToolOrder::from_tool_list(tools))
            .unwrap_or_default()
    }
}

impl From<Agent> for ToolDefinition {
    fn from(value: Agent) -> Self {
        let description = value.description.unwrap_or_default();
        let name = ToolName::new(value.id);
        ToolDefinition {
            name,
            description,
            input_schema: schemars::schema_for!(crate::AgentInput),
        }
    }
}
