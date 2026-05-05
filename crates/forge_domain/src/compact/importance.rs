//! Importance scoring for messages during compaction.
//!
//! Assigns importance scores to messages to determine which should be
//! preserved during eviction-based compaction.

use serde::{Deserialize, Serialize};

use crate::compact::strategy::CompactionStrategy;
use crate::context::ContextMessage;

use super::summary::{SummaryTool, SummaryToolCall};

/// Minimum importance score required to survive compaction
pub const MIN_SURVIVAL_SCORE: u8 = 60;

/// Base importance score for messages
const BASE_SCORE: u8 = 50;

/// Factors that contribute to message importance
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ImportanceFactor {
    /// Message contains tool calls
    HasToolCalls,
    /// Message contains tool results (success)
    HasToolResults,
    /// Message contains error results
    HasErrors,
    /// Message contains file operations (read/write/patch)
    HasFileChanges,
    /// Message contains shell execution
    HasShellExecution,
    /// Message contains search operations
    HasSearchOperations,
    /// Message contains reasoning/extended thinking
    HasReasoning,
    /// Message contains user intent
    HasUserIntent,
    /// Message contains key decisions
    HasDecision,
    /// Message is from system (lower priority)
    SystemMessage,
}

/// Calculated importance for a message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageImportance {
    /// Base importance score (0-100)
    pub score: u8,
    /// Factors contributing to score
    pub factors: Vec<ImportanceFactor>,
}

impl MessageImportance {
    /// Creates a new importance with the given score and factors
    pub fn new(score: u8, factors: Vec<ImportanceFactor>) -> Self {
        Self { score: score.min(100), factors }
    }

    /// Returns true if this message should survive compaction
    pub fn should_survive(&self) -> bool {
        self.score >= MIN_SURVIVAL_SCORE
    }
}

impl Default for MessageImportance {
    fn default() -> Self {
        Self {
            score: BASE_SCORE,
            factors: Vec::new(),
        }
    }
}

impl From<&ContextMessage> for MessageImportance {
    fn from(msg: &ContextMessage) -> Self {
        let mut score = BASE_SCORE;
        let mut factors = Vec::new();

        match msg {
            ContextMessage::Text(text_message) => {
                // Role-based scoring
                match text_message.role {
                    crate::context::Role::System => {
                        score = 30;
                        factors.push(ImportanceFactor::SystemMessage);
                    }
                    crate::context::Role::User => {
                        score = 60;
                        factors.push(ImportanceFactor::HasUserIntent);
                    }
                    crate::context::Role::Assistant => {
                        // Tool calls are high value
                        if text_message.tool_calls.is_some() {
                            score += 20;
                            factors.push(ImportanceFactor::HasToolCalls);

                            // Check for file changes
                            if let Some(calls) = &text_message.tool_calls {
                                if calls.iter().any(|c| {
                                    matches!(
                                        c.name.as_str(),
                                        "write" | "patch" | "remove" | "fs_write"
                                    )
                                }) {
                                    score += 10;
                                    factors.push(ImportanceFactor::HasFileChanges);
                                }
                                if calls.iter().any(|c| c.name.as_str() == "shell") {
                                    score += 5;
                                    factors.push(ImportanceFactor::HasShellExecution);
                                }
                                if calls.iter().any(|c| {
                                    matches!(c.name.as_str(), "fs_search" | "sem_search")
                                }) {
                                    score += 5;
                                    factors.push(ImportanceFactor::HasSearchOperations);
                                }
                            }
                        }

                        // Reasoning is valuable
                        if text_message.reasoning_details.is_some() {
                            score += 10;
                            factors.push(ImportanceFactor::HasReasoning);
                        }

                        // Content length can indicate importance
                        if text_message.content.len() > 500 {
                            score += 5;
                        }
                    }
                }
            }
            ContextMessage::Tool(tool_result) => {
                // Tool results are important, especially errors
                if tool_result.output.is_error {
                    score = 100; // Critical - always preserve errors
                    factors.push(ImportanceFactor::HasErrors);
                } else {
                    score = 55;
                    factors.push(ImportanceFactor::HasToolResults);
                }
            }
            ContextMessage::Image(_) => {
                // Images are generally low priority
                score = 30;
            }
        }

        Self { score: score.min(100), factors }
    }
}

impl From<&SummaryTool> for MessageImportance {
    fn from(tool: &SummaryTool) -> Self {
        let score;
        let mut factors = Vec::new();

        match tool {
            SummaryTool::FileRead { .. } => {
                score = 40;
            }
            SummaryTool::FileUpdate { .. } | SummaryTool::FileRemove { .. } => {
                score = 70;
                factors.push(ImportanceFactor::HasFileChanges);
            }
            SummaryTool::Shell { .. } => {
                score = 60;
                factors.push(ImportanceFactor::HasShellExecution);
            }
            SummaryTool::Search { .. } | SummaryTool::SemSearch { .. } => {
                score = 45;
                factors.push(ImportanceFactor::HasSearchOperations);
            }
            SummaryTool::Fetch { .. } | SummaryTool::Followup { .. } => {
                score = 35;
            }
            SummaryTool::Plan { .. } => {
                score = 65;
                factors.push(ImportanceFactor::HasDecision);
            }
            SummaryTool::Skill { .. } | SummaryTool::Task { .. } => {
                score = 50;
            }
            SummaryTool::TodoWrite { .. } => {
                score = 55;
            }
            SummaryTool::Mcp { .. } => {
                score = 50;
            }
            SummaryTool::Undo { .. } => {
                score = 60;
            }
            SummaryTool::TodoRead => {
                score = 30;
            }
        }

        Self { score, factors }
    }
}

impl From<&SummaryToolCall> for MessageImportance {
    fn from(call: &SummaryToolCall) -> Self {
        MessageImportance::from(&call.tool)
    }
}

/// Importance-based eviction strategy
#[derive(Debug, Clone, Default)]
pub struct ImportanceEvictionStrategy {
    /// Minimum score to protect from eviction
    pub protection_threshold: u8,
    /// Whether to use importance scoring
    pub enabled: bool,
}

impl ImportanceEvictionStrategy {
    /// Creates a new strategy with the given protection threshold
    pub fn new(protection_threshold: u8) -> Self {
        Self {
            protection_threshold,
            enabled: true,
        }
    }

    /// Returns true if the message should be protected from eviction
    pub fn is_protected(&self, importance: &MessageImportance) -> bool {
        if !self.enabled {
            return false;
        }
        importance.score >= self.protection_threshold
    }

    /// Calculate the effective eviction strategy considering importance
    pub fn adjust_strategy(
        &self,
        base_strategy: &CompactionStrategy,
        messages: &[ContextMessage],
    ) -> CompactionStrategy {
        if !self.enabled {
            return base_strategy.clone();
        }

        // Find protected message indices
        let protected_indices: Vec<usize> = messages
            .iter()
            .enumerate()
            .filter(|(_, msg)| {
                let importance = MessageImportance::from(*msg);
                importance.score >= self.protection_threshold
            })
            .map(|(i, _)| i)
            .collect();

        if protected_indices.is_empty() {
            return base_strategy.clone();
        }

        // Return the most conservative strategy that protects all important messages
        // For now, just return base strategy - more sophisticated logic can be added
        base_strategy.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::{ToolCallFull, ToolName, ToolOutput, ToolResult};

    #[test]
    fn test_message_importance_user() {
        let msg = ContextMessage::user("test content", None);
        let importance = MessageImportance::from(&msg);

        assert!(importance.should_survive());
        assert!(importance.factors.contains(&ImportanceFactor::HasUserIntent));
    }

    #[test]
    fn test_message_importance_assistant_with_tools() {
        let msg = ContextMessage::assistant(
            "I read the file",
            None,
            None,
            Some(vec![ToolCallFull::new(ToolName::new("write"))]),
        );
        let importance = MessageImportance::from(&msg);

        assert!(importance.should_survive());
        assert!(importance.factors.contains(&ImportanceFactor::HasToolCalls));
        assert!(importance.factors.contains(&ImportanceFactor::HasFileChanges));
        assert!(importance.score > BASE_SCORE);
    }

    #[test]
    fn test_message_importance_error_result() {
        let output = ToolOutput::default().is_error(true);
        let msg = ContextMessage::Tool(ToolResult::new("shell").output(Ok(output)));
        let importance = MessageImportance::from(&msg);

        assert_eq!(importance.score, 100);
        assert!(importance.factors.contains(&ImportanceFactor::HasErrors));
    }

    #[test]
    fn test_importance_eviction_strategy_protection() {
        let strategy = ImportanceEvictionStrategy::new(MIN_SURVIVAL_SCORE);

        let high_importance = MessageImportance::new(80, vec![]);
        let low_importance = MessageImportance::new(40, vec![]);

        assert!(strategy.is_protected(&high_importance));
        assert!(!strategy.is_protected(&low_importance));
    }

    #[test]
    fn test_importance_eviction_strategy_disabled() {
        let mut strategy = ImportanceEvictionStrategy::new(MIN_SURVIVAL_SCORE);
        strategy.enabled = false;

        let high_importance = MessageImportance::new(80, vec![]);
        assert!(!strategy.is_protected(&high_importance));
    }
}
