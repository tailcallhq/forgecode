use derive_setters::Setters;
use serde::{Deserialize, Serialize};

use crate::ToolCallId;

/// Represents the output of a tool search operation, containing discovered
/// tools that can be used in subsequent turns.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Setters)]
#[setters(into)]
pub struct ToolSearchOutput {
    /// The ID of the tool search call that produced this output.
    /// For server-executed tool search, this is `None` (the API returns
    /// `null`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_id: Option<ToolCallId>,
    /// The status of the tool search operation
    pub status: ToolSearchStatus,
    /// The execution type (server or client)
    pub execution: ToolSearchExecution,
    /// The discovered tools as JSON values
    #[setters(skip)]
    pub tools: Vec<serde_json::Value>,
    /// The raw tool_search_call item from the API response.
    /// Stored as JSON so it can be replayed verbatim in subsequent requests.
    /// The API expects this to appear before the tool_search_output in input.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_search_call: Option<serde_json::Value>,
}

impl ToolSearchOutput {
    /// Creates a new ToolSearchOutput with server execution defaults
    pub fn new(call_id: Option<impl Into<ToolCallId>>) -> Self {
        Self {
            call_id: call_id.map(|id| id.into()),
            status: ToolSearchStatus::Completed,
            execution: ToolSearchExecution::Server,
            tools: Vec::new(),
            tool_search_call: None,
        }
    }

    /// Adds a discovered tool to the output
    pub fn add_tool(mut self, tool: impl Serialize) -> anyhow::Result<Self> {
        let tool_json = serde_json::to_value(tool)?;
        self.tools.push(tool_json);
        Ok(self)
    }
}

/// Status of a tool search operation
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ToolSearchStatus {
    /// The tool search is still in progress
    InProgress,
    /// The tool search completed successfully
    #[default]
    Completed,
    /// The tool search was incomplete
    Incomplete,
}

/// Execution type for tool search
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ToolSearchExecution {
    /// Server-side execution
    #[default]
    Server,
    /// Client-side execution
    Client,
}
