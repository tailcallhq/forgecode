use forge_domain::{ConversationId, InterruptionReason, ToolCallArgumentError, ToolName};

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Invalid tool call arguments: {0}")]
    CallArgument(ToolCallArgumentError),

    #[error("Tool {0} not found")]
    NotFound(ToolName),

    #[error("Tool '{tool_name}' timed out after {timeout} minutes")]
    CallTimeout { tool_name: ToolName, timeout: u64 },

    #[error(
        "Tool '{name}' is not available. Please try again with one of these tools: [{supported_tools}]"
    )]
    NotAllowed {
        name: ToolName,
        supported_tools: String,
    },

    #[error(
        "Tool '{tool_name}' requires {required_modality} modality, but model only supports: {supported_modalities}"
    )]
    UnsupportedModality {
        tool_name: ToolName,
        required_modality: String,
        supported_modalities: String,
    },

    #[error("Empty tool response")]
    EmptyToolResponse,

    #[error("Agent execution was interrupted: {0:?}")]
    AgentToolInterrupted(InterruptionReason),

    #[error("Authentication still in progress")]
    AuthInProgress,

    #[error("Agent '{0}' not found")]
    AgentNotFound(forge_domain::AgentId),

    #[error("Conversation '{id}' not found")]
    ConversationNotFound { id: ConversationId },

    #[error("No active provider configured")]
    NoActiveProvider,

    #[error("No active model configured")]
    NoActiveModel,
}
