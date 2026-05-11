use serde::{Deserialize, Serialize};

use crate::reasoning::ReasoningFull;
use crate::tool_search::ToolSearchOutput;
use crate::{ToolCallArguments, ToolCallId, ToolName};

/// An ordered output item from the Responses API.
///
/// Items are recorded in the exact order they arrive from the API stream,
/// preserving the interleaving of reasoning, tool_search, and function_call
/// items. When present on a `TextMessage`, the serializer emits these items
/// directly instead of the bundled reasoning_details/tool_calls fields.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResponseOutputItem {
    /// Model reasoning item (may appear multiple times per turn)
    Reasoning(ReasoningFull),
    /// Server-side tool search call (captured as raw JSON for exact replay)
    ToolSearchCall(serde_json::Value),
    /// Server-side tool search output with discovered tools
    ToolSearchOutput(ToolSearchOutput),
    /// A function call made by the model
    FunctionCall {
        id: String,
        call_id: ToolCallId,
        name: ToolName,
        arguments: ToolCallArguments,
        #[serde(skip_serializing_if = "Option::is_none")]
        namespace: Option<String>,
    },
    /// Text content emitted by the model
    Text(String),
}
