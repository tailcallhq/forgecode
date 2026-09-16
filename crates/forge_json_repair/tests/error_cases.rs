use forge_json_repair::json_repair;
use pretty_assertions::assert_eq;

#[test]
fn test_error_cases() {
    // Empty string
    assert!(json_repair::<serde_json::Value>("").is_err());

    // Missing colon
    assert!(json_repair::<serde_json::Value>(r#"{"a","#).is_err());

    // Missing object key
    assert!(json_repair::<serde_json::Value>("{:2}").is_err());

    // Unexpected character after valid JSON
    assert!(json_repair::<serde_json::Value>(r#"{"a":2}{}"#).is_err());

    // Invalid unicode
    assert!(json_repair::<serde_json::Value>(r#""\u26""#).is_err());
    assert!(json_repair::<serde_json::Value>(r#""\uZ000""#).is_err());
}

#[test]
fn test_regex_single_slash() {
    // This test case triggers index out of bounds at line 765 and 771 in
    // parse_regex When self.i == 0, accessing self.chars.get(self.i - 1) causes
    // underflow After processing single '/', self.i becomes 2 but chars only
    // has length 1
    let fixture = "/";
    let actual = json_repair::<serde_json::Value>(fixture).unwrap();
    let expected = serde_json::json!("/");
    assert_eq!(actual, expected);
}

#[test]
fn test_regex_with_backslash_slash() {
    // Test regex with escaped slash at the end
    // parse_regex treats the regex as a string literal, so backslash is
    // preserved
    let fixture = r#"/a\/"#;
    let actual = json_repair::<serde_json::Value>(fixture).unwrap();
    let expected = serde_json::json!(r#"/a\/"#);
    assert_eq!(actual, expected);
}

#[test]
fn test_string_with_colon_at_start() {
    // This test case checks for potential index out of bounds at line 445
    // When self.i == 0 and we try to access self.chars.get(self.i - 1)
    let fixture = ":";
    let actual = json_repair::<serde_json::Value>(fixture).unwrap();
    let expected = serde_json::json!(":");
    assert_eq!(actual, expected);
}

#[test]
fn test_multibyte_unicode_missing_end_quote() {
    // Triggers index out of bounds in insert_before_last_whitespace_str.
    // The output buffer contains multi-byte UTF-8 characters (é = 2 bytes
    // each), so self.output.len() (byte count) > chars.len() (char count).
    // When the repair path calls insert_before_last_whitespace_str with
    // trailing whitespace, it initialises `index` from the byte length and
    // then indexes into a Vec<char> at that byte-length position,
    // panicking.
    let fixture = r#""café "#;
    let actual = json_repair::<serde_json::Value>(fixture).unwrap();
    let expected = serde_json::json!("café");
    assert_eq!(actual, expected);
}

#[test]
fn test_multibyte_unicode_missing_comma_in_object() {
    // Triggers index out of bounds in insert_before_last_whitespace_str (line
    // 459). parse_string first collects `"é,"` and hits the inner `"test"`.
    // The prev_non_whitespace char is `,`, so it retries with
    // stop_at_index=2 (the comma position). On retry it collects
    // str_content = `"é` (3 bytes, 2 chars) and hits stop_at_index, calling
    // insert_before_last_whitespace_str. That function sets index =
    // text.len() = 3 (byte count) and then accesses chars[index - 1] =
    // chars[2] on a Vec<char> of length 2 — panic.
    let fixture = "\"é,\"test\"";
    let actual = json_repair::<serde_json::Value>(fixture).unwrap();
    let expected = serde_json::json!(["é", "test"]);
    assert_eq!(actual, expected);
}

#[test]
fn test_multibyte_unicode_missing_closing_brace() {
    // Triggers index out of bounds in insert_before_last_whitespace_str (line
    // 384). A string with a multi-byte character followed by trailing
    // whitespace and no closing quote hits the "end of text, missing end
    // quote" repair path. str_content = `"🎉 ` (6 bytes, 3 chars).
    // insert_before_last_whitespace_str sets index = text.len() = 6 and
    // accesses chars[5] on a Vec<char> of length 3 — panic.
    let fixture = "\"🎉 ";
    let actual = json_repair::<serde_json::Value>(fixture).unwrap();
    let expected = serde_json::json!("🎉");
    assert_eq!(actual, expected);
}
