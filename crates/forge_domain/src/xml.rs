/// Extracts content between the specified XML-style tags
///
/// # Arguments
///
/// * `text` - The text to extract content from
/// * `tag_name` - The name of the XML tag (without angle brackets)
///
/// # Returns
///
/// * `Some(&str)` containing the extracted content if tags are found
/// * `None` if the tags are not found
pub fn extract_tag_content<'a>(text: &'a str, tag_name: &str) -> Option<&'a str> {
    let opening_tag = format!("<{tag_name}>",);
    let closing_tag = format!("</{tag_name}>");

    #[allow(clippy::collapsible_if)]
    if let Some(start_idx) = text.find(&opening_tag) {
        if let Some(end_idx) = text.rfind(&closing_tag) {
            let content_start = start_idx + opening_tag.len();
            if content_start < end_idx {
                return text.get(content_start..end_idx).map(|s| s.trim());
            }
        }
    }

    None
}

/// Removes content within XML-style tags that start with the specified prefix
pub fn remove_tag_with_prefix(text: &str, prefix: &str) -> String {
    // First, find all unique tag names that start with the prefix
    let tag_pattern = format!(r"<({prefix}[a-zA-Z0-9_-]*?)(?:\s[^>]*?)?>");
    let mut tag_names = Vec::new();

    if let Ok(regex) = regex::Regex::new(&tag_pattern) {
        for captures in regex.captures_iter(text) {
            if let Some(tag_name) = captures.get(1) {
                // Only add unique tag names to the list
                let tag_name = tag_name.as_str().to_string();
                if !tag_names.contains(&tag_name) {
                    tag_names.push(tag_name);
                }
            }
        }
    }

    // Now remove content for each tag name found
    let mut result = text.to_string();
    for tag_name in tag_names {
        // Create pattern to match complete tag including content
        let pattern = format!(r"<{tag_name}(?:\s[^>]*?)?>[\s\S]*?</{tag_name}>");

        if let Ok(regex) = regex::Regex::new(&pattern) {
            result = regex.replace_all(&result, "").to_string();
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn test_extract_tag_content() {
        let fixture = "Some text <summary>This is the important part</summary> and more text";
        let actual = extract_tag_content(fixture, "summary");
        let expected = Some("This is the important part");
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_extract_tag_content_no_tags() {
        let fixture = "Some text without any tags";
        let actual = extract_tag_content(fixture, "summary");
        let expected = None;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_extract_tag_content_with_different_tag() {
        let fixture = "Text with <custom>Custom content</custom> tags";
        let actual = extract_tag_content(fixture, "custom");
        let expected = Some("Custom content");
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_extract_tag_content_with_malformed_tags() {
        let fixture = "Text with <opening> but no closing tag";
        let actual = extract_tag_content(fixture, "opening");
        let expected = None;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_extract_tag_names_with_prefix() {
        let fixture = "<forge_tool>Something</forge_tool> <forge_tool_call>Content</forge_tool_call> <other>More</other>";
        let actual = remove_tag_with_prefix(fixture, "forge");
        // Check that both tool tags have been removed, leaving only <other> tag
        assert!(actual.contains("<other>More</other>"));
        assert!(!actual.contains("<forge_tool>"));
        assert!(!actual.contains("<forge_tool_call>"));
    }

    #[test]
    fn test_extract_tag_names_with_prefix_no_matches() {
        let fixture = "<other>Some content</other> <another>Other content</another>";
        let actual = remove_tag_with_prefix(fixture, "forge");
        let expected = "<other>Some content</other> <another>Other content</another>";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_extract_tag_names_with_prefix_nested() {
        let fixture = "<parent><forge_tool>Inner</forge_tool><forge_tool_call>Nested</forge_tool_call></parent>";
        let actual = remove_tag_with_prefix(fixture, "forge");
        let expected = "<parent></parent>";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_extract_tag_names_with_prefix_duplicates() {
        let fixture =
            "<forge_tool>First</forge_tool><other>Middle</other><forge_tool>Second</forge_tool>";
        let actual = remove_tag_with_prefix(fixture, "forge");
        let expected = "<other>Middle</other>";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_extract_tag_names_with_prefix_attributes() {
        let fixture = "<forge_tool id=\"1\">Content</forge_tool> <forge_tool_call class=\"important\">More</forge_tool_call>";
        let actual = remove_tag_with_prefix(fixture, "forge");
        // Check that both tool tags have been removed
        assert!(!actual.contains("<forge_tool"));
        assert!(!actual.contains("<forge_tool_call"));
        assert!(!actual.contains("Content"));
        assert!(!actual.contains("More"));
    }

    #[test]
    fn test_remove_tag_with_prefix() {
        let fixture = "<forge_task>Task details</forge_task> Regular text <forge_analysis>Analysis details</forge_analysis>";
        let actual = remove_tag_with_prefix(fixture, "forge_");
        let expected = " Regular text ";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_remove_tag_with_prefix_no_matching_tags() {
        let fixture = "<other>Content</other> <another>More content</another>";
        let actual = remove_tag_with_prefix(fixture, "forge_");
        let expected = "<other>Content</other> <another>More content</another>";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_with_duplicate_closing_tags() {
        let fixture = "<foo>1<foo>2</foo>3</foo>";
        let actual = extract_tag_content(fixture, "foo").unwrap();
        let expected = "1<foo>2</foo>3";
        assert_eq!(actual, expected);
    }
}
