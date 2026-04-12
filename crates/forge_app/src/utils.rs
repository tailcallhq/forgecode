use std::path::Path;

use crate::{Match, MatchResult};

/// Formats a path for display, converting absolute paths to relative when
/// possible
///
/// If the path starts with the current working directory, returns a
/// relative path. Otherwise, returns the original absolute path.
///
/// # Arguments
/// * `path` - The path to format
/// * `cwd` - The current working directory path
///
/// # Returns
/// * A formatted path string
pub fn format_display_path(path: &Path, cwd: &Path) -> String {
    // Try to create a relative path for display if possible
    let display_path = if path.starts_with(cwd) {
        match path.strip_prefix(cwd) {
            Ok(rel_path) => rel_path.display().to_string(),
            Err(_) => path.display().to_string(),
        }
    } else {
        path.display().to_string()
    };

    if display_path.is_empty() {
        ".".to_string()
    } else {
        display_path
    }
}

/// Truncates a key string for display purposes
///
/// If the key length is 20 characters or less, returns it unchanged.
/// Otherwise, shows the first 13 characters and last 4 characters with "..." in
/// between.
///
/// # Arguments
/// * `key` - The key string to truncate
///
/// # Returns
/// * A truncated version of the key for safe display
pub use forge_domain::truncate_key;

pub fn format_match(matched: &Match, base_dir: &Path) -> String {
    match &matched.result {
        Some(MatchResult::Error(err)) => format!("Error reading {}: {}", matched.path, err),
        Some(MatchResult::Found { line_number, line }) => {
            let path = format_display_path(Path::new(&matched.path), base_dir);
            match line_number {
                Some(num) => format!("{}:{}:{}", path, num, line),
                None => format!("{}:{}", path, line),
            }
        }
        Some(MatchResult::Count { count }) => {
            format!(
                "{}:{}",
                format_display_path(Path::new(&matched.path), base_dir),
                count
            )
        }
        Some(MatchResult::FileMatch) => format_display_path(Path::new(&matched.path), base_dir),
        Some(MatchResult::ContextMatch { line_number, line, before_context, after_context }) => {
            let path = format_display_path(Path::new(&matched.path), base_dir);
            let mut output = String::new();

            // Add before context lines
            for ctx_line in before_context {
                output.push_str(&format!("{}-{}\n", path, ctx_line));
            }

            // Add the match line
            match line_number {
                Some(num) => output.push_str(&format!("{}:{}:{}", path, num, line)),
                None => output.push_str(&format!("{}:{}", path, line)),
            }

            // Add after context lines
            for ctx_line in after_context {
                output.push_str(&format!("\n{}-{}", path, ctx_line));
            }

            output
        }
        None => format_display_path(Path::new(&matched.path), base_dir),
    }
}

/// Computes SHA-256 hash of the given content
///
/// General-purpose utility function that computes a SHA-256 hash of string
/// content. Returns a consistent hexadecimal representation that can be used
/// for content comparison, caching, or change detection.
///
/// # Arguments
/// * `content` - The content string to hash
///
/// # Returns
/// * A hexadecimal string representation of the SHA-256 hash
pub fn compute_hash(content: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    hex::encode(hasher.finalize())
}

// Merges strict-mode incompatible `allOf` branches into a single schema object.
fn flatten_all_of_schema(map: &mut serde_json::Map<String, serde_json::Value>) {
    let Some(serde_json::Value::Array(all_of)) = map.remove("allOf") else {
        return;
    };

    for sub_schema in all_of {
        let serde_json::Value::Object(source) = sub_schema else {
            continue;
        };

        merge_schema_object(map, source);
    }
}

fn merge_schema_object(
    target: &mut serde_json::Map<String, serde_json::Value>,
    mut source: serde_json::Map<String, serde_json::Value>,
) {
    flatten_all_of_schema(&mut source);

    for (key, value) in source {
        match target.get_mut(&key) {
            Some(existing) => merge_schema_keyword(existing, value, &key),
            None => {
                target.insert(key, value);
            }
        }
    }
}

fn merge_schema_keyword(target: &mut serde_json::Value, source: serde_json::Value, key: &str) {
    match (key, target, source) {
        (
            "properties" | "$defs" | "definitions" | "patternProperties",
            serde_json::Value::Object(target_map),
            serde_json::Value::Object(source_map),
        ) => merge_named_schema_map(target_map, source_map),
        (
            "required",
            serde_json::Value::Array(target_values),
            serde_json::Value::Array(source_values),
        ) => merge_required_arrays(target_values, source_values),
        (
            "enum",
            serde_json::Value::Array(target_values),
            serde_json::Value::Array(source_values),
        ) => merge_enum_arrays(target_values, source_values),
        (_, serde_json::Value::Object(target_map), serde_json::Value::Object(source_map)) => {
            merge_schema_object(target_map, source_map);
        }
        ("description" | "title", _, _) => {}
        (_, target_value, source_value) if *target_value == source_value => {}
        _ => {}
    }
}

fn merge_named_schema_map(
    target: &mut serde_json::Map<String, serde_json::Value>,
    source: serde_json::Map<String, serde_json::Value>,
) {
    for (key, value) in source {
        match target.get_mut(&key) {
            Some(existing) => merge_schema_keyword(existing, value, "schema"),
            None => {
                target.insert(key, value);
            }
        }
    }
}

fn merge_required_arrays(target: &mut Vec<serde_json::Value>, source: Vec<serde_json::Value>) {
    for value in source {
        if !target.contains(&value) {
            target.push(value);
        }
    }

    if target.iter().all(|value| value.as_str().is_some()) {
        target.sort_by(|left, right| left.as_str().cmp(&right.as_str()));
    }
}

fn merge_enum_arrays(target: &mut Vec<serde_json::Value>, source: Vec<serde_json::Value>) {
    target.retain(|value| source.contains(value));
}

fn normalize_named_schema_keyword(
    map: &mut serde_json::Map<String, serde_json::Value>,
    key: &str,
    strict_mode: bool,
) {
    let Some(serde_json::Value::Object(named_schemas)) = map.get_mut(key) else {
        return;
    };

    for schema in named_schemas.values_mut() {
        enforce_strict_schema(schema, strict_mode);
    }
}

fn normalize_schema_keyword(
    map: &mut serde_json::Map<String, serde_json::Value>,
    key: &str,
    strict_mode: bool,
) {
    let Some(schema) = map.get_mut(key) else {
        return;
    };

    match schema {
        serde_json::Value::Object(_) | serde_json::Value::Array(_) => {
            enforce_strict_schema(schema, strict_mode);
        }
        serde_json::Value::Bool(_) => {}
        _ => {}
    }
}

fn normalize_schema_keywords(
    map: &mut serde_json::Map<String, serde_json::Value>,
    strict_mode: bool,
) {
    for key in ["properties", "$defs", "definitions", "patternProperties"] {
        normalize_named_schema_keyword(map, key, strict_mode);
    }

    for key in [
        "items",
        "contains",
        "not",
        "if",
        "then",
        "else",
        "additionalProperties",
        "additionalItems",
        "unevaluatedProperties",
    ] {
        normalize_schema_keyword(map, key, strict_mode);
    }

    for key in ["allOf", "anyOf", "oneOf", "prefixItems"] {
        normalize_schema_keyword(map, key, strict_mode);
    }
}

fn is_supported_openai_string_format(format: &str) -> bool {
    matches!(
        format,
        "date-time"
            | "time"
            | "date"
            | "duration"
            | "email"
            | "hostname"
            | "ipv4"
            | "ipv6"
            | "uuid"
    )
}

fn normalize_string_format_keyword(
    map: &mut serde_json::Map<String, serde_json::Value>,
    strict_mode: bool,
) {
    if !strict_mode {
        return;
    }

    let Some(format) = map.get("format").and_then(|value| value.as_str()) else {
        return;
    };

    if !is_supported_openai_string_format(format) {
        map.remove("format");
    }
}

fn is_object_schema(map: &serde_json::Map<String, serde_json::Value>) -> bool {
    map.get("type")
        .and_then(|value| value.as_str())
        .is_some_and(|ty| ty == "object")
        || map.contains_key("properties")
        || map.contains_key("required")
        || map.contains_key("additionalProperties")
}

fn normalize_additional_properties(
    map: &mut serde_json::Map<String, serde_json::Value>,
    strict_mode: bool,
) {
    match map.get_mut("additionalProperties") {
        Some(serde_json::Value::Object(additional_props_map)) => {
            let has_combiners = additional_props_map.contains_key("anyOf")
                || additional_props_map.contains_key("oneOf")
                || additional_props_map.contains_key("allOf");

            if !additional_props_map.contains_key("type") && !has_combiners {
                additional_props_map.insert(
                    "type".to_string(),
                    serde_json::Value::String("object".to_string()),
                );
            }

            let mut additional_props =
                serde_json::Value::Object(std::mem::take(additional_props_map));
            enforce_strict_schema(&mut additional_props, strict_mode);
            map.insert("additionalProperties".to_string(), additional_props);
        }
        Some(serde_json::Value::Bool(_)) => {}
        Some(_) => {
            map.insert(
                "additionalProperties".to_string(),
                serde_json::Value::Bool(false),
            );
        }
        None => {
            map.insert(
                "additionalProperties".to_string(),
                serde_json::Value::Bool(false),
            );
        }
    }
}

/// Normalizes a JSON schema to meet LLM provider requirements
///
/// Many LLM providers (OpenAI, Anthropic) require that all object types in JSON
/// schemas explicitly set `additionalProperties: false`. This function
/// recursively processes the schema to add this requirement.
///
/// Additionally, for OpenAI compatibility, it ensures:
/// - All objects have a `properties` field (even if empty)
/// - All objects have a `required` array with all property keys
/// - `allOf` branches are merged into a single schema object when strict mode
///   is enabled
///
/// # Arguments
/// * `schema` - The JSON schema to normalize (will be modified in place)
/// * `strict_mode` - If true, adds `properties`, `required`, and `allOf`
///   flattening for OpenAI compatibility
pub fn enforce_strict_schema(schema: &mut serde_json::Value, strict_mode: bool) {
    match schema {
        serde_json::Value::Object(map) => {
            if strict_mode {
                flatten_all_of_schema(map);
                // Remove unsupported keywords that OpenAI/Codex doesn't allow
                map.remove("propertyNames");
            }

            normalize_string_format_keyword(map, strict_mode);

            let is_object = is_object_schema(map);

            // If this looks like an object schema but has no explicit type, add it
            // OpenAI requires all schemas to have a type when they represent objects
            if is_object && !map.contains_key("type") {
                map.insert(
                    "type".to_string(),
                    serde_json::Value::String("object".to_string()),
                );
            }

            if is_object {
                if strict_mode && !map.contains_key("properties") {
                    map.insert(
                        "properties".to_string(),
                        serde_json::Value::Object(serde_json::Map::new()),
                    );
                }

                normalize_additional_properties(map, strict_mode);

                if strict_mode {
                    let required_keys = map
                        .get("properties")
                        .and_then(|value| value.as_object())
                        .map(|props| {
                            let mut keys = props.keys().cloned().collect::<Vec<_>>();
                            keys.sort();
                            keys
                        })
                        .unwrap_or_default();

                    let required_values = required_keys
                        .into_iter()
                        .map(serde_json::Value::String)
                        .collect::<Vec<_>>();

                    map.insert(
                        "required".to_string(),
                        serde_json::Value::Array(required_values),
                    );
                }
            }

            if strict_mode
                && map
                    .get("nullable")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
            {
                map.remove("nullable");

                if let Some(serde_json::Value::Array(enum_values)) = map.get_mut("enum") {
                    enum_values.retain(|v| !v.is_null());
                }

                let description = map.remove("description");
                let non_null_branch = serde_json::Value::Object(std::mem::take(map));
                let null_branch = serde_json::json!({"type": "null"});

                if let Some(desc) = description {
                    map.insert("description".to_string(), desc);
                }
                map.insert(
                    "anyOf".to_string(),
                    serde_json::Value::Array(vec![non_null_branch, null_branch]),
                );
            }

            normalize_schema_keywords(map, strict_mode);
        }
        serde_json::Value::Array(items) => {
            for value in items {
                enforce_strict_schema(value, strict_mode);
            }
        }
        _ => {}
    }
}

/// Returns true if the Content-Type header indicates binary (non-text) content.
///
/// This utility helps detect binary content types commonly returned by HTTP
/// responses. It's useful for tools that handle text content but need to detect
/// and reject binary data.
///
/// # Arguments
/// * `content_type` - The Content-Type header value (e.g., "text/html",
///   "application/octet-stream")
///
/// # Examples
///
/// ```
/// use forge_app::utils::is_binary_content_type;
///
/// // Text content types are not binary
/// assert!(!is_binary_content_type("text/html"));
/// assert!(!is_binary_content_type("application/json"));
///
/// // Binary content types are detected
/// assert!(is_binary_content_type("image/png"));
/// assert!(is_binary_content_type("application/octet-stream"));
/// ```
pub fn is_binary_content_type(content_type: &str) -> bool {
    let ct = content_type.to_lowercase();
    // Allow text/* and common text-based types
    if ct.starts_with("text/")
        || ct.contains("json")
        || ct.contains("xml")
        || ct.contains("javascript")
        || ct.contains("ecmascript")
        || ct.contains("yaml")
        || ct.contains("toml")
        || ct.contains("csv")
        || ct.contains("html")
        || ct.contains("svg")
        || ct.contains("markdown")
        || ct.is_empty()
    {
        return false;
    }
    // Everything else (application/gzip, application/octet-stream, image/*,
    // audio/*, video/*, etc.)
    true
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use serde_json::json;

    use super::*;

    #[test]
    fn test_normalize_json_schema_anthropic_mode() {
        let mut schema = json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" }
            }
        });

        enforce_strict_schema(&mut schema, false);

        assert_eq!(schema["additionalProperties"], json!(false));
        // In non-strict mode, required field is not added
        assert_eq!(schema.get("required"), None);
    }

    #[test]
    fn test_normalize_json_schema_openai_strict_mode() {
        let mut schema = json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" },
                "age": { "type": "number" }
            }
        });

        enforce_strict_schema(&mut schema, true);

        assert_eq!(schema["additionalProperties"], json!(false));
        assert_eq!(schema["required"], json!(["age", "name"]));
    }

    #[test]
    fn test_normalize_json_schema_adds_empty_properties_in_strict_mode() {
        let mut schema = json!({
            "type": "object"
        });

        enforce_strict_schema(&mut schema, true);

        assert_eq!(schema["properties"], json!({}));
        assert_eq!(schema["additionalProperties"], json!(false));
        assert_eq!(schema["required"], json!([]));
    }

    #[test]
    fn test_normalize_json_schema_nested_objects() {
        let mut schema = json!({
            "type": "object",
            "properties": {
                "user": {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string" }
                    }
                }
            }
        });

        enforce_strict_schema(&mut schema, false);

        assert_eq!(schema["additionalProperties"], json!(false));
        assert_eq!(
            schema["properties"]["user"]["additionalProperties"],
            json!(false)
        );
    }

    #[test]
    fn test_dynamic_properties_schema_is_preserved_in_strict_mode() {
        let mut fixture = json!({
            "type": "object",
            "properties": {
                "pages": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "properties": {
                                "description": "Dynamic page properties",
                                "type": "object",
                                "additionalProperties": {
                                    "anyOf": [
                                        { "type": "string" },
                                        { "type": "number" },
                                        { "type": "null" }
                                    ]
                                },
                                "propertyNames": {
                                    "type": "string"
                                }
                            }
                        },
                        "additionalProperties": false
                    }
                }
            }
        });

        enforce_strict_schema(&mut fixture, true);

        let expected = json!({
            "type": "object",
            "properties": {
                "pages": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "properties": {
                                "description": "Dynamic page properties",
                                "type": "object",
                                "properties": {},
                                "additionalProperties": {
                                    "anyOf": [
                                        { "type": "string" },
                                        { "type": "number" },
                                        { "type": "null" }
                                    ]
                                },
                                "required": []
                            }
                        },
                        "additionalProperties": false,
                        "required": ["properties"]
                    }
                }
            },
            "additionalProperties": false,
            "required": ["pages"]
        });

        assert_eq!(fixture, expected);
    }

    #[test]
    fn test_all_of_is_flattened_in_strict_mode() {
        let mut fixture = json!({
            "type": "object",
            "properties": {
                "rich_text": {
                    "type": "array",
                    "items": {
                        "allOf": [
                            {
                                "type": "object",
                                "properties": {
                                    "text": { "type": "string" }
                                }
                            },
                            {
                                "description": "Rich text item"
                            }
                        ]
                    }
                }
            }
        });

        enforce_strict_schema(&mut fixture, true);

        let expected = json!({
            "type": "object",
            "properties": {
                "rich_text": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "text": { "type": "string" }
                        },
                        "description": "Rich text item",
                        "additionalProperties": false,
                        "required": ["text"]
                    }
                }
            },
            "additionalProperties": false,
            "required": ["rich_text"]
        });

        assert_eq!(fixture, expected);
    }

    #[test]
    fn test_all_of_is_preserved_in_non_strict_mode() {
        let mut fixture = json!({
            "type": "object",
            "properties": {
                "value": {
                    "allOf": [
                        { "type": "string" },
                        { "description": "A value" }
                    ]
                }
            }
        });

        enforce_strict_schema(&mut fixture, false);

        let expected = json!({
            "type": "object",
            "properties": {
                "value": {
                    "allOf": [
                        { "type": "string" },
                        { "description": "A value" }
                    ]
                }
            },
            "additionalProperties": false
        });

        assert_eq!(fixture, expected);
    }

    #[test]
    fn test_nullable_enum_converted_to_any_of_in_strict_mode() {
        // This matches what schemars AddNullable produces: nullable=true AND
        // null added to enum values array
        let mut schema = json!({
            "type": "object",
            "properties": {
                "output_mode": {
                    "description": "Output mode",
                    "nullable": true,
                    "type": "string",
                    "enum": ["content", "files_with_matches", "count", null]
                }
            }
        });

        enforce_strict_schema(&mut schema, true);

        let expected = json!({
            "type": "object",
            "properties": {
                "output_mode": {
                    "description": "Output mode",
                    "anyOf": [
                        { "type": "string", "enum": ["content", "files_with_matches", "count"] },
                        { "type": "null" }
                    ]
                }
            },
            "additionalProperties": false,
            "required": ["output_mode"]
        });

        assert_eq!(schema, expected);
    }

    #[test]
    fn test_nullable_string_converted_to_any_of_in_strict_mode() {
        let mut schema = json!({
            "type": "object",
            "properties": {
                "name": {
                    "description": "A name",
                    "nullable": true,
                    "type": "string"
                }
            }
        });

        enforce_strict_schema(&mut schema, true);

        let expected = json!({
            "type": "object",
            "properties": {
                "name": {
                    "description": "A name",
                    "anyOf": [
                        { "type": "string" },
                        { "type": "null" }
                    ]
                }
            },
            "additionalProperties": false,
            "required": ["name"]
        });

        assert_eq!(schema, expected);
    }

    #[test]
    fn test_nullable_not_converted_in_non_strict_mode() {
        let mut schema = json!({
            "type": "object",
            "properties": {
                "output_mode": {
                    "nullable": true,
                    "type": "string",
                    "enum": ["content", "files_with_matches", "count"]
                }
            }
        });

        enforce_strict_schema(&mut schema, false);

        // In non-strict mode, nullable should be preserved as-is
        assert_eq!(schema["properties"]["output_mode"]["nullable"], json!(true));
        assert!(schema["properties"]["output_mode"].get("anyOf").is_none());
    }

    #[test]
    fn test_schema_valued_additional_properties_is_normalized() {
        let mut schema = json!({
            "type": "object",
            "properties": {
                "metadata": {
                    "type": "object",
                    "additionalProperties": {
                        "type": "object",
                        "properties": {
                            "value": { "type": "string" }
                        }
                    }
                }
            }
        });

        enforce_strict_schema(&mut schema, true);

        // The additionalProperties schema should have been normalized
        // (additionalProperties: false added to nested schema)
        assert_eq!(
            schema["properties"]["metadata"]["additionalProperties"],
            json!({
                "type": "object",
                "properties": {
                    "value": { "type": "string" }
                },
                "additionalProperties": false,
                "required": ["value"]
            })
        );
    }

    #[test]
    fn test_notion_mcp_create_comment_schema() {
        // Simulates the actual Notion MCP create_comment schema that was failing
        let mut schema = json!({
            "type": "object",
            "properties": {
                "rich_text": {
                    "type": "array",
                    "items": {
                        "anyOf": [
                            {
                                "type": "object",
                                "description": "Text content",
                                "properties": {
                                    "text": {
                                        "type": "object",
                                        "properties": {
                                            "content": { "type": "string" }
                                        }
                                    }
                                }
                            },
                            {
                                "type": "object",
                                "description": "Mention content",
                                "properties": {
                                    "mention": {
                                        "type": "object",
                                        "properties": {
                                            "user": {
                                                "type": "object",
                                                "properties": {
                                                    "id": { "type": "string" }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        ]
                    }
                },
                "page_id": {
                    "type": "string"
                },
                "discussion_id": {
                    "type": "string"
                }
            }
        });

        enforce_strict_schema(&mut schema, true);

        // Verify the schema is now valid for OpenAI
        // 1. All objects should have type: "object"
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["properties"]["rich_text"]["type"], "array");

        // 2. Check that the anyOf items have proper types and additionalProperties:
        //    false
        let any_of = schema["properties"]["rich_text"]["items"]["anyOf"]
            .as_array()
            .unwrap();
        for branch in any_of {
            assert_eq!(branch["type"], "object");
            assert_eq!(branch["additionalProperties"], false);
            // All nested object properties should also have type and additionalProperties
            if let Some(props) = branch["properties"].as_object() {
                for (_, prop_schema) in props {
                    if let Some(obj) = prop_schema.as_object()
                        && obj.contains_key("properties")
                    {
                        assert!(
                            prop_schema["type"] == "object",
                            "Nested object should have type: object"
                        );
                    }
                }
            }
        }

        // 3. Verify additionalProperties: false at root level and for objects
        assert_eq!(schema["additionalProperties"], false);
        // Note: arrays don't get additionalProperties, only objects do
        assert_eq!(schema["properties"]["rich_text"]["type"], "array");

        // 4. Verify required fields are set
        let required = schema["required"].as_array().unwrap();
        assert!(required.contains(&json!("rich_text")));
        assert!(required.contains(&json!("page_id")));
        assert!(required.contains(&json!("discussion_id")));
    }

    #[test]
    fn test_property_names_is_removed_in_strict_mode() {
        // This test ensures we don't regress on propertyNames removal
        // propertyNames is a JSON Schema keyword that OpenAI/Codex doesn't support
        let mut schema = json!({
            "type": "object",
            "properties": {
                "dynamic": {
                    "type": "object",
                    "propertyNames": {
                        "type": "string",
                        "pattern": "^[a-z]+$"
                    },
                    "additionalProperties": {
                        "type": "string"
                    }
                }
            }
        });

        enforce_strict_schema(&mut schema, true);

        // propertyNames should be completely removed
        assert!(
            !schema["properties"]["dynamic"]
                .as_object()
                .unwrap()
                .contains_key("propertyNames"),
            "propertyNames must be removed in strict mode for OpenAI/Codex compatibility"
        );

        // The rest of the schema should be preserved
        assert_eq!(schema["properties"]["dynamic"]["type"], "object");
        assert_eq!(
            schema["properties"]["dynamic"]["additionalProperties"]["type"],
            "string"
        );
    }

    #[test]
    fn test_unsupported_format_is_removed_in_strict_mode() {
        let mut fixture = json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "format": "uri"
                }
            }
        });

        enforce_strict_schema(&mut fixture, true);

        let expected = json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string"
                }
            },
            "additionalProperties": false,
            "required": ["url"]
        });

        assert_eq!(fixture, expected);
    }

    #[test]
    fn test_supported_format_is_preserved_in_strict_mode() {
        let mut fixture = json!({
            "type": "object",
            "properties": {
                "timestamp": {
                    "type": "string",
                    "format": "date-time"
                }
            }
        });

        enforce_strict_schema(&mut fixture, true);

        let expected = json!({
            "type": "object",
            "properties": {
                "timestamp": {
                    "type": "string",
                    "format": "date-time"
                }
            },
            "additionalProperties": false,
            "required": ["timestamp"]
        });

        assert_eq!(fixture, expected);
    }

    /// Integration test that simulates the full Notion MCP workflow:
    /// 1. Schema arrives from MCP server (with propertyNames)
    /// 2. Gets normalized for OpenAI/Codex (propertyNames removed)
    /// 3. Serialized to JSON for API request
    #[test]
    fn test_notion_mcp_create_pages_full_schema() {
        // This is a realistic subset of the Notion MCP create_pages schema
        // that caused the original error
        let notion_mcp_schema = json!({
            "type": "object",
            "properties": {
                "pages": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "properties": {
                                "description": "Dynamic page properties",
                                "type": "object",
                                "propertyNames": {
                                    "type": "string"
                                },
                                "additionalProperties": {
                                    "anyOf": [
                                        { "type": "string" },
                                        { "type": "number" },
                                        { "type": "boolean" }
                                    ]
                                }
                            }
                        },
                        "required": ["properties"]
                    }
                }
            },
            "required": ["pages"]
        });

        // Step 1: Convert to Schema (like MCP client does)
        let schema_str = serde_json::to_string(&notion_mcp_schema).unwrap();
        let mut schema: serde_json::Value = serde_json::from_str(&schema_str).unwrap();

        // Step 2: Normalize for OpenAI/Codex strict mode
        enforce_strict_schema(&mut schema, true);

        // Step 3: Serialize for API request
        let api_request_json = serde_json::to_string(&schema).unwrap();

        // Verify: propertyNames should NOT be in the final JSON
        assert!(
            !api_request_json.contains("propertyNames"),
            "Final API request JSON must not contain 'propertyNames'. Schema: {}",
            api_request_json
        );

        // Verify: Schema structure is preserved
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["properties"]["pages"]["type"], "array");
        assert_eq!(
            schema["properties"]["pages"]["items"]["properties"]["properties"]["type"],
            "object"
        );

        // Verify: additionalProperties is normalized
        let additional_props = &schema["properties"]["pages"]["items"]["properties"]["properties"]
            ["additionalProperties"];
        assert!(additional_props.is_object() || additional_props.is_boolean());

        // Verify: Required fields are set
        let required = schema["required"].as_array().unwrap();
        assert!(required.contains(&json!("pages")));
    }
}
