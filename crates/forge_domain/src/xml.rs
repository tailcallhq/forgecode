/// Extracts a full XML tag (including brackets) by its name
pub fn extract_tag<'a>(text: &'a str, tag_name: &str) -> Option<&'a str> {
    let opening_pattern = format!(r"<{tag_name}(?:\s[^>]*?)?>");
    if let Ok(regex) = regex::Regex::new(&opening_pattern)
        && let Some(mat) = regex.find(text)
    {
        return Some(mat.as_str());
    }
    None
}

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

/// Cleans a user prompt by extracting content from <feedback> tags if present,
/// or stripping all XML tags and meta-information.
pub fn clean_user_prompt(text: &str) -> String {
    // 1. Try to extract content from <feedback> tag
    if let Some(content) = extract_tag_content(text, "feedback") {
        return content.to_string();
    }

    // 2. Remove known meta tags with their content
    let mut cleaned = remove_tag_with_prefix(text, "system_");
    cleaned = remove_tag_with_prefix(&cleaned, "context_");

    // 3. Strip all remaining tags but preserve newlines (unlike strip_xml_tags)
    let tag_pattern = regex::Regex::new(r"<[^>]*>").unwrap();
    let result = tag_pattern.replace_all(&cleaned, "").to_string();

    // Trim while preserving internal structure
    result.trim().to_string()
}

/// Extracts the value of an attribute from an XML tag
pub fn extract_attribute(tag: &str, attr_name: &str) -> Option<String> {
    let pattern = format!(r#"{attr_name}="([^"]*)""#, attr_name = attr_name);
    if let Ok(regex) = regex::Regex::new(&pattern)
        && let Some(captures) = regex.captures(tag)
    {
        return captures.get(1).map(|m| m.as_str().to_string());
    }
    None
}

/// Removes all XML/HTML tags from the text, keeping only the content between tags.
/// Multiple whitespace characters are collapsed into a single space.
pub fn strip_xml_tags(text: &str) -> String {
    let tag_pattern = regex::Regex::new(r"<[^>]*>").unwrap();
    let result = tag_pattern.replace_all(text, "").to_string();
    // Collapse multiple whitespace characters into a single space
    let re_whitespace = regex::Regex::new(r"\s+").unwrap();
    re_whitespace.replace_all(&result, " ").trim().to_string()
}

/// Extracts file paths from XML tags in the given text.
/// Supports tags: plan_created, file_created, file_overwritten, file_diff, file_removed.
pub fn extract_modified_files_from_output(text: &str) -> Vec<String> {
    let mut modified_files = Vec::new();
    let tags = [
        "plan_created",
        "file_created",
        "file_overwritten",
        "file_diff",
        "file_removed",
    ];

    for tag_name in tags {
        if let Some(tag) = extract_tag(text, tag_name)
            && let Some(path) = extract_attribute(tag, "path")
        {
            modified_files.push(path);
            // Return only the first matching tag to maintain parity with existing logic
            return modified_files;
        }
    }
    modified_files
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
    fn test_extract_tag_content_with_duplicate_closing_tags() {
        let fixture = "Some text <summary>1<summary>2</summary>3</summary> and more text";
        let actual = extract_tag_content(fixture, "summary");
        let expected = Some("1<summary>2</summary>3");
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
    fn test_clean_user_prompt_with_tags() {
        let fixture = "<feedback>add feature to determine recipe from images using vision llm models</feedback>\n::: <system_date>2026-05-16</system_date>\n::: ";
        let actual = clean_user_prompt(fixture);
        // Should extract ONLY feedback and trim
        let expected = "add feature to determine recipe from images using vision llm models";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_clean_user_prompt_without_feedback() {
        let fixture = "Just plain text <system_date>2026</system_date>";
        let actual = clean_user_prompt(fixture);
        // Should strip system_date and its tags
        let expected = "Just plain text";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_extract_modified_files_from_output() {
        let fixture = r#"Some text <file_created path="/abs/path/to/file.txt" /> more text"#;
        let actual = extract_modified_files_from_output(fixture);
        let expected = vec!["/abs/path/to/file.txt".to_string()];
        assert_eq!(actual, expected);

        let fixture = r#"<plan_created path="plan.md">Plan content</plan_created>"#;
        let actual = extract_modified_files_from_output(fixture);
        let expected = vec!["plan.md".to_string()];
        assert_eq!(actual, expected);

        let fixture = r#"<file_overwritten path="old.txt" /><file_created path="new.txt" />"#;
        let actual = extract_modified_files_from_output(fixture);
        // Priority: file_created > file_overwritten
        let expected = vec!["new.txt".to_string()];
        assert_eq!(actual, expected);

        let fixture = "No tags here";
        let actual = extract_modified_files_from_output(fixture);
        let expected: Vec<String> = vec![];
        assert_eq!(actual, expected);
    }
}
