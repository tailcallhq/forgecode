use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::Local;
use tokio::sync::Notify;

use crate::{ToolCallFull, ToolName, ToolResult};

#[derive(Debug, Clone, PartialEq)]
pub enum ChatResponseContent {
    // Should be only used to send tool input events.
    ToolInput(TitleFormat),
    // Should be only used to send tool outputs.
    ToolOutput(String),
    Markdown { text: String, partial: bool },
}

impl From<ChatResponseContent> for ChatResponse {
    fn from(content: ChatResponseContent) -> Self {
        ChatResponse::TaskMessage { content }
    }
}

impl From<TitleFormat> for ChatResponse {
    fn from(title: TitleFormat) -> Self {
        ChatResponse::TaskMessage { content: ChatResponseContent::ToolInput(title) }
    }
}

impl From<TitleFormat> for ChatResponseContent {
    fn from(title: TitleFormat) -> Self {
        ChatResponseContent::ToolInput(title)
    }
}

impl ChatResponseContent {
    pub fn contains(&self, needle: &str) -> bool {
        self.as_str().contains(needle)
    }

    pub fn as_str(&self) -> &str {
        match self {
            ChatResponseContent::ToolOutput(text) | ChatResponseContent::Markdown { text, .. } => {
                text
            }
            ChatResponseContent::ToolInput(_) => "",
        }
    }
}

/// Events that are emitted by the agent for external consumption. This includes
/// events for all internal state changes.
#[derive(Debug, Clone)]
pub enum ChatResponse {
    TaskMessage {
        content: ChatResponseContent,
    },
    TaskReasoning {
        content: String,
    },
    TaskComplete,
    ToolCallStart {
        tool_call: ToolCallFull,
        notifier: Arc<Notify>,
    },
    ToolCallEnd(ToolResult),
    /// A user-configured hook blocked execution of a tool call.
    HookError {
        /// Name of the tool that was blocked.
        tool_name: ToolName,
        /// Human-readable reason provided by the hook (from stderr or JSON
        /// output).
        reason: String,
    },
    /// A user-configured hook encountered an error or produced a warning.
    /// Displayed in the UI as a warning regardless of whether the hook
    /// blocked execution or not.
    HookWarning {
        /// Human-readable warning message.
        message: String,
    },
    RetryAttempt {
        cause: Cause,
        duration: Duration,
    },
    Interrupt {
        reason: InterruptionReason,
    },
}

impl ChatResponse {
    /// Returns `true` if the response contains no meaningful content.
    ///
    /// A response is considered empty if it's a `TaskMessage` or
    /// `TaskReasoning` with empty string content. All other variants are
    /// considered non-empty.
    pub fn is_empty(&self) -> bool {
        match self {
            ChatResponse::TaskMessage { content, .. } => match content {
                ChatResponseContent::ToolInput(_) => false,
                ChatResponseContent::ToolOutput(content) => content.is_empty(),
                ChatResponseContent::Markdown { text, .. } => text.is_empty(),
            },
            ChatResponse::TaskReasoning { content } => content.is_empty(),
            _ => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InterruptionReason {
    MaxToolFailurePerTurnLimitReached {
        limit: u64,
        errors: HashMap<ToolName, usize>,
    },
    MaxRequestPerTurnLimitReached {
        limit: u64,
    },
}

#[derive(Clone)]
pub struct Cause(String);

impl Cause {
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl std::fmt::Debug for Cause {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0.as_str())
    }
}

impl From<&anyhow::Error> for Cause {
    fn from(value: &anyhow::Error) -> Self {
        Self(format!("{value:?}"))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Category {
    Action,
    Info,
    Debug,
    Error,
    Completion,
    Warning,
}

#[derive(Clone, derive_setters::Setters, Debug, PartialEq)]
#[setters(into, strip_option)]
pub struct TitleFormat {
    pub title: String,
    pub sub_title: Option<String>,
    pub category: Category,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

pub trait TitleExt {
    fn title_fmt(&self) -> TitleFormat;
}

impl<T> TitleExt for T
where
    T: Into<TitleFormat> + Clone,
{
    fn title_fmt(&self) -> TitleFormat {
        self.clone().into()
    }
}

impl TitleFormat {
    /// Create a status for executing a tool
    pub fn info(message: impl Into<String>) -> Self {
        Self {
            title: message.into(),
            sub_title: None,
            category: Category::Info,
            timestamp: Local::now().into(),
        }
    }

    /// Create a status for executing a tool
    pub fn action(message: impl Into<String>) -> Self {
        Self {
            title: message.into(),
            sub_title: None,
            category: Category::Action,
            timestamp: Local::now().into(),
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self {
            title: message.into(),
            sub_title: None,
            category: Category::Error,
            timestamp: Local::now().into(),
        }
    }

    pub fn debug(message: impl Into<String>) -> Self {
        Self {
            title: message.into(),
            sub_title: None,
            category: Category::Debug,
            timestamp: Local::now().into(),
        }
    }

    pub fn warning(message: impl Into<String>) -> Self {
        Self {
            title: message.into(),
            sub_title: None,
            category: Category::Warning,
            timestamp: Local::now().into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Utc};
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn test_title_format_with_timestamp() {
        let timestamp = DateTime::parse_from_rfc3339("2023-10-26T10:30:00Z")
            .unwrap()
            .with_timezone(&Utc);

        let title = TitleFormat {
            title: "Test Action".to_string(),
            sub_title: Some("Subtitle".to_string()),
            category: Category::Action,
            timestamp,
        };

        assert_eq!(title.title, "Test Action");
        assert_eq!(title.sub_title, Some("Subtitle".to_string()));
        assert_eq!(title.category, Category::Action);
        assert_eq!(title.timestamp, timestamp);
    }
}
