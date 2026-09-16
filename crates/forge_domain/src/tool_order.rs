use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::Arc;

use glob::Pattern;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{ToolDefinition, ToolName};

/// Defines the ordering for tools in an agent's context.
/// Tools are ordered based on weights - higher weight tools appear first.
/// When the list is empty, tools are sorted alphabetically.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ToolOrder {
    /// The ordered list of tool names and patterns
    tools: Vec<ToolName>,
    /// Weight map: tool name -> weight (position index in the tools vector)
    /// Pre-compiled for O(1) lookups during sorting
    #[serde(skip)]
    #[schemars(skip)]
    weights: Arc<HashMap<ToolName, usize>>,
    /// Pre-compiled glob patterns for matching with their positions
    #[serde(skip)]
    #[schemars(skip)]
    patterns: Arc<Vec<(Pattern, usize)>>,
}

impl Default for ToolOrder {
    fn default() -> Self {
        Self {
            tools: Vec::new(),
            weights: Arc::new(HashMap::new()),
            patterns: Arc::new(Vec::new()),
        }
    }
}

impl ToolOrder {
    /// Creates a new ToolOrder with the specified tool names
    ///
    /// # Arguments
    ///
    /// * `tools` - List of tool names (and patterns) to use as the basis for
    ///   ordering
    pub fn new(tools: Vec<ToolName>) -> Self {
        let mut weights = HashMap::new();
        let mut patterns = Vec::new();

        for (index, tool_name) in tools.iter().enumerate() {
            // Try to compile as a glob pattern
            if let Ok(pattern) = Pattern::new(tool_name.as_str()) {
                // Check if it's actually a pattern (contains wildcards)
                if tool_name.as_str().contains('*') || tool_name.as_str().contains('?') {
                    patterns.push((pattern, index));
                    continue;
                }
            }

            // Not a pattern, store as exact match
            weights.insert(tool_name.clone(), index);
        }

        Self {
            tools,
            weights: Arc::new(weights),
            patterns: Arc::new(patterns),
        }
    }

    /// Creates a ToolOrder from a list of tool names, using the exact order
    /// as specified in the list, including glob patterns.
    ///
    /// # Arguments
    ///
    /// * `tools` - List of tool names (and patterns) to use as the basis for
    ///   ordering
    pub fn from_tool_list(tools: &[ToolName]) -> Self {
        if tools.is_empty() {
            return Self::default();
        }

        Self::new(tools.to_vec())
    }

    /// Sorts tool definitions according to the ordering strategy
    ///
    /// # Arguments
    ///
    /// * `tools` - Mutable slice of tool definitions to sort
    pub fn sort(&self, tools: &mut [ToolDefinition]) {
        if self.tools.is_empty() {
            // Empty order means alphabetical
            tools.sort_by(|a, b| a.name.as_str().cmp(b.name.as_str()));
        } else {
            tools.sort_by(|a, b| self.compare_by_weight(&a.name, &b.name));
        }
    }

    /// Sorts tool definition references according to the ordering strategy
    ///
    /// # Arguments
    ///
    /// * `tools` - Mutable slice of tool definition references to sort
    pub fn sort_refs(&self, tools: &mut [&ToolDefinition]) {
        if self.tools.is_empty() {
            // Empty order means alphabetical
            tools.sort_by(|a, b| a.name.as_str().cmp(b.name.as_str()));
        } else {
            tools.sort_by(|a, b| self.compare_by_weight(&a.name, &b.name));
        }
    }

    /// Gets the weight (position) of a tool by checking exact matches first,
    /// then patterns
    ///
    /// Returns None if the tool doesn't match any entry in the order list.
    fn get_weight(&self, tool: &ToolName) -> Option<usize> {
        // First check exact match in weights map - O(1)
        if let Some(&weight) = self.weights.get(tool) {
            return Some(weight);
        }

        // Then check glob patterns - O(p) where p is number of patterns
        for (pattern, weight) in self.patterns.iter() {
            if pattern.matches(tool.as_str()) {
                return Some(*weight);
            }
        }

        None
    }

    /// Compares two tool names by their weights
    ///
    /// Tools with lower position indices come first. Tools with no weight are
    /// sorted alphabetically and appear after weighted tools.
    fn compare_by_weight(&self, a: &ToolName, b: &ToolName) -> Ordering {
        let a_weight = self.get_weight(a);
        let b_weight = self.get_weight(b);

        match (a_weight, b_weight) {
            // Both have weights - lower weight (earlier position) comes first
            (Some(w_a), Some(w_b)) => match w_a.cmp(&w_b) {
                // If weights are equal (e.g., both match same pattern), sort
                // alphabetically
                Ordering::Equal => a.as_str().cmp(b.as_str()),
                other => other,
            },
            // Only 'a' has weight, so it comes first
            (Some(_), None) => Ordering::Less,
            // Only 'b' has weight, so it comes first
            (None, Some(_)) => Ordering::Greater,
            // Neither has weight, sort alphabetically
            (None, None) => a.as_str().cmp(b.as_str()),
        }
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn test_alphabetical_sort() {
        let fixture_order = ToolOrder::new(vec![]); // Empty list = alphabetical
        let mut fixture = vec![
            ToolDefinition::new("zebra").description("Z tool"),
            ToolDefinition::new("alpha").description("A tool"),
            ToolDefinition::new("beta").description("B tool"),
        ];

        fixture_order.sort(&mut fixture);

        let actual: Vec<String> = fixture.iter().map(|t| t.name.to_string()).collect();
        let expected = vec!["alpha", "beta", "zebra"];

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_custom_order_all_specified() {
        let fixture_order = ToolOrder::new(vec![
            ToolName::new("beta"),
            ToolName::new("alpha"),
            ToolName::new("zebra"),
        ]);
        let mut fixture = vec![
            ToolDefinition::new("zebra").description("Z tool"),
            ToolDefinition::new("alpha").description("A tool"),
            ToolDefinition::new("beta").description("B tool"),
        ];

        fixture_order.sort(&mut fixture);

        let actual: Vec<String> = fixture.iter().map(|t| t.name.to_string()).collect();
        let expected = vec!["beta", "alpha", "zebra"];

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_custom_order_partial_specification() {
        let fixture_order = ToolOrder::new(vec![ToolName::new("zebra"), ToolName::new("beta")]);
        let mut fixture = vec![
            ToolDefinition::new("alpha").description("A tool"),
            ToolDefinition::new("beta").description("B tool"),
            ToolDefinition::new("zebra").description("Z tool"),
            ToolDefinition::new("delta").description("D tool"),
            ToolDefinition::new("charlie").description("C tool"),
        ];

        fixture_order.sort(&mut fixture);

        let actual: Vec<String> = fixture.iter().map(|t| t.name.to_string()).collect();
        // zebra and beta come first (in that order), rest alphabetically
        let expected = vec!["zebra", "beta", "alpha", "charlie", "delta"];

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_custom_order_with_refs() {
        let fixture_order = ToolOrder::new(vec![ToolName::new("write"), ToolName::new("read")]);
        let tools = [
            ToolDefinition::new("read").description("Read tool"),
            ToolDefinition::new("write").description("Write tool"),
            ToolDefinition::new("patch").description("Patch tool"),
        ];
        let mut fixture: Vec<&ToolDefinition> = tools.iter().collect();

        fixture_order.sort_refs(&mut fixture);

        let actual: Vec<String> = fixture.iter().map(|t| t.name.to_string()).collect();
        let expected = vec!["write", "read", "patch"];

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_custom_order_empty_list() {
        let fixture_order = ToolOrder::new(vec![]);
        let mut fixture = vec![
            ToolDefinition::new("zebra").description("Z tool"),
            ToolDefinition::new("alpha").description("A tool"),
            ToolDefinition::new("beta").description("B tool"),
        ];

        fixture_order.sort(&mut fixture);

        let actual: Vec<String> = fixture.iter().map(|t| t.name.to_string()).collect();
        // Should fall back to alphabetical
        let expected = vec!["alpha", "beta", "zebra"];

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_from_tool_list_exact_order() {
        let fixture = vec![
            ToolName::new("write"),
            ToolName::new("read"),
            ToolName::new("sage"),
            ToolName::new("patch"),
            ToolName::new("sem_search"),
        ];

        let actual = ToolOrder::from_tool_list(&fixture);

        let names: Vec<String> = actual.tools.iter().map(|t| t.to_string()).collect();
        // Should maintain exact order as specified
        assert_eq!(names[0], "write");
        assert_eq!(names[1], "read");
        assert_eq!(names[2], "sage");
        assert_eq!(names[3], "patch");
        assert_eq!(names[4], "sem_search");
    }

    #[test]
    fn test_from_tool_list_with_mcp_tools() {
        let fixture = vec![
            ToolName::new("read"),
            ToolName::new("mcp_github"),
            ToolName::new("write"),
            ToolName::new("mcp_slack"),
            ToolName::new("patch"),
        ];

        let actual = ToolOrder::from_tool_list(&fixture);

        let names: Vec<String> = actual.tools.iter().map(|t| t.to_string()).collect();
        // Should maintain exact order as specified, no special rules
        assert_eq!(names[0], "read");
        assert_eq!(names[1], "mcp_github");
        assert_eq!(names[2], "write");
        assert_eq!(names[3], "mcp_slack");
        assert_eq!(names[4], "patch");
    }

    #[test]
    fn test_from_tool_list_empty() {
        let fixture: Vec<ToolName> = vec![];

        let actual = ToolOrder::from_tool_list(&fixture);

        assert_eq!(actual, ToolOrder::new(vec![]));
    }

    #[test]
    fn test_from_tool_list_with_glob_patterns() {
        let fixture = vec![
            ToolName::new("read"),
            ToolName::new("fs_*"), // Glob pattern - preserved
            ToolName::new("write"),
            ToolName::new("mcp_*"), // Glob pattern - preserved
            ToolName::new("patch"),
        ];

        let actual = ToolOrder::from_tool_list(&fixture);

        let names: Vec<String> = actual.tools.iter().map(|t| t.to_string()).collect();
        // All tools and patterns preserved
        assert_eq!(names.len(), 5);
        assert_eq!(names[0], "read");
        assert_eq!(names[1], "fs_*");
        assert_eq!(names[2], "write");
        assert_eq!(names[3], "mcp_*");
        assert_eq!(names[4], "patch");
    }

    #[test]
    fn test_custom_order_with_glob_pattern_matching() {
        let fixture_order = ToolOrder::new(vec![
            ToolName::new("read"),
            ToolName::new("fs_*"),
            ToolName::new("shell"),
        ]);
        let mut fixture = vec![
            ToolDefinition::new("shell").description("Shell tool"),
            ToolDefinition::new("fs_write").description("FS Write"),
            ToolDefinition::new("read").description("Read tool"),
            ToolDefinition::new("fs_read").description("FS Read"),
        ];

        fixture_order.sort(&mut fixture);

        let actual: Vec<String> = fixture.iter().map(|t| t.name.to_string()).collect();
        // read (pos 0), fs_read and fs_write (both match fs_* at pos 1,
        // alphabetically sorted), shell (pos 2)
        let expected = vec!["read", "fs_read", "fs_write", "shell"];

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_is_empty() {
        let empty = ToolOrder::new(vec![]);
        let non_empty = ToolOrder::new(vec![ToolName::new("read")]);

        assert!(empty.tools.is_empty());
        assert!(!non_empty.tools.is_empty());
    }

    #[test]
    fn test_weight_lookup_optimization() {
        // Test that the HashMap optimization works correctly
        let fixture_order = ToolOrder::new(vec![
            ToolName::new("alpha"),
            ToolName::new("beta"),
            ToolName::new("gamma"),
        ]);

        // Verify weights are stored correctly
        assert_eq!(fixture_order.get_weight(&ToolName::new("alpha")), Some(0));
        assert_eq!(fixture_order.get_weight(&ToolName::new("beta")), Some(1));
        assert_eq!(fixture_order.get_weight(&ToolName::new("gamma")), Some(2));
        assert_eq!(fixture_order.get_weight(&ToolName::new("delta")), None);
    }

    #[test]
    fn test_pattern_matching_optimization() {
        // Test that patterns are pre-compiled
        let fixture_order = ToolOrder::new(vec![
            ToolName::new("exact_match"),
            ToolName::new("prefix_*"),
            ToolName::new("*_suffix"),
        ]);

        // Exact matches should be in weights
        assert_eq!(
            fixture_order.get_weight(&ToolName::new("exact_match")),
            Some(0)
        );

        // Pattern matches should work
        assert_eq!(
            fixture_order.get_weight(&ToolName::new("prefix_test")),
            Some(1)
        );
        assert_eq!(
            fixture_order.get_weight(&ToolName::new("prefix_foo")),
            Some(1)
        );
        assert_eq!(
            fixture_order.get_weight(&ToolName::new("test_suffix")),
            Some(2)
        );

        // Non-matches should return None
        assert_eq!(fixture_order.get_weight(&ToolName::new("no_match")), None);
    }

    #[test]
    fn test_clone_is_cheap() {
        // Test that cloning ToolOrder is cheap (Arc is cloned, not the data)
        let fixture_order = ToolOrder::new(vec![
            ToolName::new("alpha"),
            ToolName::new("beta"),
            ToolName::new("gamma"),
        ]);

        let cloned = fixture_order.clone();

        // Both should have the same Arc pointers (cheap clone)
        assert_eq!(
            Arc::as_ptr(&fixture_order.weights),
            Arc::as_ptr(&cloned.weights)
        );
        assert_eq!(
            Arc::as_ptr(&fixture_order.patterns),
            Arc::as_ptr(&cloned.patterns)
        );
    }
}
