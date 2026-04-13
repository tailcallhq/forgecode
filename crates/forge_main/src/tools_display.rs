use std::collections::HashSet;

use convert_case::{Case, Casing};
use forge_api::{ToolName, ToolsOverview};

use crate::info::Info;

/// Formats the tools overview for display using the Info component,
/// organized by categories with availability checkboxes.
pub fn format_tools(agent_tools: &[ToolName], overview: &ToolsOverview) -> Info {
    let mut info = Info::new();
    let agent_tools = agent_tools.iter().collect::<HashSet<_>>();
    let checkbox = |tool_name: &ToolName| -> &str {
        if agent_tools.contains(tool_name) {
            "[✓]"
        } else {
            "[ ]"
        }
    };

    // System tools section
    info = info.add_title("SYSTEM");
    for tool in &overview.system {
        info = info.add_value(format!("{} {}", checkbox(&tool.name), tool.name));
    }

    // Agents section
    info = info.add_title("AGENTS");
    for tool in &overview.agents {
        info = info.add_value(format!("{} {}", checkbox(&tool.name), tool.name));
    }

    // MCP tools section
    if !overview.mcp.get_servers().is_empty() {
        for (server_name, tools) in overview.mcp.get_servers().iter() {
            let title = (*server_name).to_case(Case::UpperSnake);
            info = info.add_title(title);

            for tool in tools {
                info = info.add_value(format!("{} {}", checkbox(&tool.name), tool.name));
            }
        }
    }

    // Failed MCP servers section
    if !overview.mcp.get_failures().is_empty() {
        info = info.add_title("FAILED MCP SERVERS");
        for (server_name, error) in overview.mcp.get_failures().iter() {
            // Truncate error message for readability in list view
            // Use 'mcp show <name>' for full error details
            let truncated_error = truncate_error(error);
            info = info.add_value(format!("[✗] {server_name} - {truncated_error}"));
        }
    }

    info
}

/// Truncates an error message to at most 80 characters for display.
///
/// If the message exceeds 80 characters, the first 77 characters are kept
/// followed by "...". Uses character-based counting to avoid panicking on
/// multi-byte UTF-8 strings.
fn truncate_error(error: &str) -> String {
    if error.chars().count() > 80 {
        let truncated: String = error.chars().take(77).collect();
        format!("{truncated}...")
    } else {
        error.to_string()
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn test_truncate_error_short_message() {
        let fixture = "Connection refused";
        let actual = truncate_error(fixture);
        let expected = "Connection refused";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_truncate_error_long_ascii_message() {
        let fixture = "A".repeat(100);
        let actual = truncate_error(&fixture);
        assert_eq!(actual.chars().count(), 80);
        assert!(actual.ends_with("..."));
    }

    #[test]
    fn test_truncate_error_multibyte_chars_no_panic() {
        // Error containing multi-byte UTF-8 chars (→ is 3 bytes)
        let fixture = "Error: struct → prioritizes struct definitions, trait → prioritizes traits, impl → prioritizes impls, and more details follow here";
        let actual = truncate_error(fixture);
        // Should not panic and should truncate correctly by char count
        assert!(actual.chars().count() <= 80);
        assert!(actual.ends_with("..."));
    }

    #[test]
    fn test_truncate_error_emoji_no_panic() {
        // Error containing 4-byte emojis - 90 emoji chars > 80 limit
        let fixture = "🚀".repeat(90);
        let actual = truncate_error(&fixture);
        assert_eq!(actual.chars().count(), 80);
        assert!(actual.ends_with("..."));
    }

    #[test]
    fn test_truncate_error_exactly_80_chars() {
        let fixture = "A".repeat(80);
        let actual = truncate_error(&fixture);
        let expected = "A".repeat(80);
        assert_eq!(actual, expected);
    }
}
