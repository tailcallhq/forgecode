//! Pre-compaction filtering to remove noise from context before summarization.
//!
//! This module provides filters that clean up context by removing:
//! - Short/empty tool results
//! - Debug output (print statements, logs)
//! - Duplicate consecutive operations
//! - Noise artifacts from failed commands

use std::collections::HashSet;

use crate::{Context, ContextMessage, MessageEntry, ToolOutput};

/// Get the text length of a ToolOutput
fn tool_output_text_len(output: &ToolOutput) -> usize {
    output.as_str().map(|s| s.len()).unwrap_or(0)
}

/// Configuration for pre-compaction filtering
#[derive(Debug, Clone)]
pub struct PreCompactionFilterConfig {
    /// Minimum length for tool result content (bytes)
    pub min_tool_result_length: usize,
    /// Remove debug output (print statements, logs)
    pub remove_debug_output: bool,
    /// Collapse duplicate consecutive operations
    pub collapse_duplicates: bool,
    /// Remove empty messages
    pub remove_empty: bool,
}

impl PreCompactionFilterConfig {
    /// Creates a default configuration with sensible defaults
    pub fn default_config() -> Self {
        Self {
            min_tool_result_length: 10, // Keep tool results > 10 chars
            remove_debug_output: true,
            collapse_duplicates: true,
            remove_empty: true,
        }
    }
}

/// Pre-compaction filter that cleans up context before summarization
#[derive(Debug, Clone, Default)]
pub struct PreCompactionFilter {
    config: PreCompactionFilterConfig,
}

impl PreCompactionFilter {
    /// Create a new filter with the given configuration
    pub fn new(config: PreCompactionFilterConfig) -> Self {
        Self { config }
    }

    /// Create a filter with default configuration
    pub fn default_filter() -> Self {
        Self::new(PreCompactionFilterConfig::default_config())
    }

    /// Apply all filters to the context
    pub fn filter(&self, context: &mut Context) {
        self.remove_short_tool_results(context);
        if self.config.remove_debug_output {
            self.remove_debug_output(context);
        }
        if self.config.remove_empty {
            self.remove_empty_messages(context);
        }
        if self.config.collapse_duplicates {
            self.collapse_duplicate_operations(context);
        }
    }

    /// Remove tool results that are too short (likely empty or error messages)
    fn remove_short_tool_results(&self, context: &mut Context) {
        context.messages.retain(|msg| {
            if let ContextMessage::Tool(result) = &msg.message {
                // Keep tool results that are substantive or errors
                tool_output_text_len(&result.output) > self.config.min_tool_result_length
                    || result.is_error()
            } else {
                true
            }
        });
    }

    /// Remove debug output (print statements, console.log, etc.)
    fn remove_debug_output(&self, context: &mut Context) {
        let debug_patterns = [
            "console.log",
            "console.warn",
            "console.error",
            "print!(",
            "println!(",
            "printf(",
            "System.out.println",
            "console.debug",
            "logging.debug",
            "logger.debug",
            "// DEBUG",
            "/* DEBUG",
            "# DEBUG",
        ];

        context.messages.retain(|msg| {
            if let ContextMessage::Tool(result) = &msg.message {
                let output = result.output.as_str().unwrap_or("");
                !debug_patterns.iter().any(|pattern| output.contains(pattern))
            } else {
                true
            }
        });
    }
    /// Remove empty or whitespace-only messages
    fn remove_empty_messages(&self, context: &mut Context) {
        context.messages.retain(|msg| {
            match &msg.message {
                ContextMessage::Text(text) => {
                    !text.content.trim().is_empty()
                }
                ContextMessage::Tool(_) => {
                    // Keep tool results even if empty (for atomicity)
                    true
                }
                ContextMessage::Image(_) => {
                    // Always keep image messages
                    true
                }
            }
        });
    }

    /// Collapse duplicate consecutive operations (e.g., multiple reads of same file)
    fn collapse_duplicate_operations(&self, context: &mut Context) {
        let mut result: Vec<MessageEntry> = Vec::new();
        let mut seen_tools: HashSet<String> = HashSet::new();

        for msg in &context.messages {
            let should_add = match &msg.message {
                ContextMessage::Tool(tool) => {
                    let key = format!("{}:{}", tool.name, tool.output.as_str().unwrap_or(""));
                    if seen_tools.contains(&key) {
                        // Already seen this exact tool call - skip unless it's an error
                        if tool.is_error() {
                            true
                        } else {
                            false
                        }
                    } else {
                        seen_tools.insert(key);
                        true
                    }
                }
                _ => true,
            };

            if should_add {
                result.push(msg.clone());
            }
        }

        context.messages = result;
    }

    /// Get configuration reference
    pub fn config(&self) -> &PreCompactionFilterConfig {
        &self.config
    }

    /// Update configuration
    pub fn set_config(&mut self, config: PreCompactionFilterConfig) {
        self.config = config;
    }
}

impl Default for PreCompactionFilterConfig {
    fn default() -> Self {
        Self::default_config()
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::{Context, ContextMessage, ToolResult};

    fn make_context(msgs: Vec<ContextMessage>) -> Context {
        let mut ctx = Context::default();
        for msg in msgs {
            ctx = ctx.add_message(msg);
        }
        ctx
    }

    fn short_tool_result() -> ContextMessage {
        ContextMessage::Tool(
            ToolResult::new("shell")
                .success("err")
        )
    }

    fn long_tool_result() -> ContextMessage {
        ContextMessage::Tool(
            ToolResult::new("shell")
                .success("This is a longer output with actual content")
        )
    }

    fn debug_tool_result() -> ContextMessage {
        ContextMessage::Tool(
            ToolResult::new("shell")
                .success("console.log('debug message')")
        )
    }

    #[test]
    fn test_removes_short_tool_results() {
        let filter = PreCompactionFilter::new(PreCompactionFilterConfig {
            min_tool_result_length: 10,
            ..Default::default()
        });

        let mut ctx = make_context(vec![
            short_tool_result(),  // Will be removed (3 chars < 10)
            long_tool_result(),    // Will be kept (43 chars > 10)
        ]);

        filter.remove_short_tool_results(&mut ctx);

        assert_eq!(ctx.messages.len(), 1);
        assert!(matches!(
            &ctx.messages[0].message,
            ContextMessage::Tool(t) if tool_output_text_len(&t.output) > 10
        ));
    }

    #[test]
    fn test_keeps_error_tool_results() {
        let filter = PreCompactionFilter::new(PreCompactionFilterConfig {
            min_tool_result_length: 100,
            ..Default::default()
        });

        let error_result = ContextMessage::Tool(
            ToolResult::new("shell").failure(anyhow::anyhow!("error"))
        );

        let mut ctx = make_context(vec![
            error_result,  // Will be kept even though short (it's an error)
            short_tool_result(),  // Will be removed
        ]);

        filter.remove_short_tool_results(&mut ctx);

        assert_eq!(ctx.messages.len(), 1);
    }

    #[test]
    fn test_removes_debug_output() {
        let filter = PreCompactionFilter::new(PreCompactionFilterConfig {
            remove_debug_output: true,
            ..Default::default()
        });

        let mut ctx = make_context(vec![
            debug_tool_result(),  // Will be removed
            long_tool_result(),   // Will be kept
        ]);

        filter.remove_debug_output(&mut ctx);

        assert_eq!(ctx.messages.len(), 1);
    }

    #[test]
    fn test_removes_empty_text_messages() {
        let filter = PreCompactionFilter::new(PreCompactionFilterConfig {
            remove_empty: true,
            ..Default::default()
        });

        let mut ctx = make_context(vec![
            ContextMessage::user("   ", None),  // Will be removed
            ContextMessage::user("Hello", None), // Will be kept
        ]);

        filter.remove_empty_messages(&mut ctx);

        assert_eq!(ctx.messages.len(), 1);
    }

    #[test]
    fn test_collapse_duplicate_consecutive_operations() {
        let filter = PreCompactionFilter::new(PreCompactionFilterConfig {
            collapse_duplicates: true,
            ..Default::default()
        });

        let tool1 = ContextMessage::Tool(
            ToolResult::new("read")
                .success("file content")
        );
        let tool2 = ContextMessage::Tool(
            ToolResult::new("read")
                .success("same content")
        );

        let mut ctx = make_context(vec![
            tool1.clone(),
            tool2,   // Duplicate - will be removed
            tool1,   // Different position, will be kept
        ]);

        filter.collapse_duplicate_operations(&mut ctx);

        assert_eq!(ctx.messages.len(), 2);
    }

    #[test]
    fn test_full_filter_pipeline() {
        let filter = PreCompactionFilter::default_filter();

        let mut ctx = make_context(vec![
            short_tool_result(),  // Will be removed (short)
            debug_tool_result(),   // Will be removed (debug)
            ContextMessage::user("   ", None),  // Will be removed (empty)
            long_tool_result(),    // Will be kept
        ]);

        filter.filter(&mut ctx);

        // Should keep only the long tool result
        assert_eq!(ctx.messages.len(), 1);
    }
}
