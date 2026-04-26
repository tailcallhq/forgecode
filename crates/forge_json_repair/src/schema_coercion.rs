use schemars::Schema;
use serde::de::Error as _;
use serde_json::Value;

use crate::json_repair;

/// Coerces a JSON value to match the expected types defined in a JSON schema.
///
/// This function recursively traverses the JSON value and the schema,
/// converting string values to the expected types (e.g., "42" -> 42) when the
/// schema indicates a different type is expected.
///
/// # Arguments
///
/// * `value` - The JSON value to coerce
/// * `schema` - The JSON schema defining expected types
///
/// # Errors
///
/// Returns the original value if coercion is not possible or the schema doesn't
/// specify type constraints.
pub fn coerce_to_schema(value: Value, schema: &Schema) -> Value {
    coerce_value_with_schema(value, schema, schema)
}

fn coerce_value_with_schema(value: Value, schema: &Schema, root_schema: &Schema) -> Value {
    // In schemars 1.0, Schema is a wrapper around serde_json::Value
    // It can be either a boolean or an object
    match schema.as_value() {
        Value::Bool(_) => value, // Boolean schemas don't provide type info for coercion
        Value::Object(schema_obj) => {
            coerce_value_with_schema_object(value, schema_obj, root_schema)
        }
        _ => value, // Shouldn't happen, but handle gracefully
    }
}

fn coerce_value_with_schema_object(
    value: Value,
    schema: &serde_json::Map<String, Value>,
    root_schema: &Schema,
) -> Value {
    // Handle $ref schemas by resolving references
    if let Some(Value::String(reference)) = schema.get("$ref") {
        // Resolve $ref against root schema definitions
        // schemars uses format: "#/$defs/TypeName" or "#/definitions/TypeName"
        if let Some(def_name) = reference
            .strip_prefix("#/$defs/")
            .or_else(|| reference.strip_prefix("#/definitions/"))
            && let Some(def_schema) = root_schema
                .as_value()
                .as_object()
                .and_then(|obj| obj.get("$defs").or_else(|| obj.get("definitions")))
                .and_then(|defs| defs.as_object())
                .and_then(|defs| defs.get(def_name))
        {
            // Convert the Value to a Schema wrapper
            if let Ok(schema) = serde_json::from_value::<Schema>(def_schema.clone()) {
                return coerce_value_with_schema(value, &schema, root_schema);
            }
        }
    }

    // Coerce empty strings to null for nullable schemas.
    // LLMs often send "" for optional parameters instead of omitting them or
    // sending null. When the schema has "nullable": true (OpenAPI 3.0 style),
    // an empty string should be treated as null.
    if let Value::String(s) = &value
        && s.is_empty()
        && is_nullable(schema)
    {
        return Value::Null;
    }
    // Handle anyOf/oneOf schemas by trying each sub-schema
    if let Some(any_of) = schema.get("anyOf").and_then(|v| v.as_array()) {
        // Try each sub-schema in anyOf until one succeeds
        for sub_schema_value in any_of {
            if let Ok(sub_schema) = serde_json::from_value::<Schema>(sub_schema_value.clone()) {
                let result = coerce_value_with_schema(value.clone(), &sub_schema, root_schema);
                if result != value {
                    return result;
                }
            }
        }
    }
    if let Some(one_of) = schema.get("oneOf").and_then(|v| v.as_array()) {
        // Try each sub-schema in oneOf until one succeeds
        for sub_schema_value in one_of {
            if let Ok(sub_schema) = serde_json::from_value::<Schema>(sub_schema_value.clone()) {
                let result = coerce_value_with_schema(value.clone(), &sub_schema, root_schema);
                if result != value {
                    return result;
                }
            }
        }
    }
    if let Some(all_of) = schema.get("allOf").and_then(|v| v.as_array()) {
        // Apply all schemas in sequence
        let mut result = value;
        for sub_schema_value in all_of {
            if let Ok(sub_schema) = serde_json::from_value::<Schema>(sub_schema_value.clone()) {
                result = coerce_value_with_schema(result, &sub_schema, root_schema);
            }
        }
        return result;
    }

    // Handle objects with properties
    if let Value::Object(mut map) = value {
        if let Some(Value::Object(properties)) = schema.get("properties") {
            for (key, val) in map.iter_mut() {
                if let Some(prop_schema_value) = properties.get(key)
                    && let Ok(prop_schema) =
                        serde_json::from_value::<Schema>(prop_schema_value.clone())
                {
                    let coerced = coerce_value_with_schema(val.clone(), &prop_schema, root_schema);
                    *val = coerced;
                }
            }
        }
        return Value::Object(map);
    }

    // Handle arrays
    if let Value::Array(arr) = value {
        // Check for prefixItems first (JSON Schema 2020-12 for tuples)
        if let Some(prefix_items_value) = schema.get("prefixItems")
            && let Some(item_schemas) = prefix_items_value.as_array()
        {
            // Array of schemas (tuple validation)
            return Value::Array(
                arr.into_iter()
                    .enumerate()
                    .map(|(i, item)| {
                        if let Some(schema_value) = item_schemas.get(i)
                            && let Ok(schema) =
                                serde_json::from_value::<Schema>(schema_value.clone())
                        {
                            return coerce_value_with_schema(item, &schema, root_schema);
                        }
                        item
                    })
                    .collect(),
            );
        }

        // Check for items (older JSON Schema drafts)
        if let Some(items_schema_value) = schema.get("items") {
            // Check if it's a single schema or array of schemas
            if items_schema_value.is_object() || items_schema_value.is_boolean() {
                // Single schema for all items
                if let Ok(item_schema) =
                    serde_json::from_value::<Schema>(items_schema_value.clone())
                {
                    return Value::Array(
                        arr.into_iter()
                            .map(|item| coerce_value_with_schema(item, &item_schema, root_schema))
                            .collect(),
                    );
                }
            } else if let Some(item_schemas) = items_schema_value.as_array() {
                // Array of schemas (tuple validation)
                return Value::Array(
                    arr.into_iter()
                        .enumerate()
                        .map(|(i, item)| {
                            if let Some(schema_value) = item_schemas.get(i)
                                && let Ok(schema) =
                                    serde_json::from_value::<Schema>(schema_value.clone())
                            {
                                return coerce_value_with_schema(item, &schema, root_schema);
                            }
                            item
                        })
                        .collect(),
                );
            }
        }
        return Value::Array(arr);
    }

    // If schema has specific instance types, try to coerce the value
    if let Some(type_value) = schema.get("type") {
        return coerce_by_type(value, type_value, schema, root_schema);
    }

    value
}

fn coerce_by_type(
    value: Value,
    type_value: &Value,
    schema: &serde_json::Map<String, Value>,
    root_schema: &Schema,
) -> Value {
    // type can be a string or an array of strings
    let target_types: Vec<&str> = match type_value {
        Value::String(s) => vec![s.as_str()],
        Value::Array(arr) => arr.iter().filter_map(|v| v.as_str()).collect(),
        _ => return value,
    };

    // If the value already matches one of the target types, return as-is
    if type_matches(&value, &target_types) {
        return value;
    }

    // Try coercion if value is a string
    if let Value::String(s) = &value {
        for target_type in target_types {
            if let Some(coerced) = try_coerce_string(s, target_type, schema, root_schema) {
                return coerced;
            }
        }
    }

    value
}

/// Checks if a schema is marked as nullable via the OpenAPI 3.0 "nullable"
/// extension. This is set by schemars when `option_nullable = true` for
/// `Option<T>` fields.
fn is_nullable(schema: &serde_json::Map<String, Value>) -> bool {
    schema
        .get("nullable")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

fn type_matches(value: &Value, target_types: &[&str]) -> bool {
    target_types.iter().any(|t| match *t {
        "null" => value.is_null(),
        "boolean" => value.is_boolean(),
        "object" => value.is_object(),
        "array" => value.is_array(),
        "number" => value.is_number(),
        "string" => value.is_string(),
        "integer" => value.is_i64() || value.is_u64(),
        _ => false,
    })
}

fn try_coerce_string(
    s: &str,
    target_type: &str,
    schema: &serde_json::Map<String, Value>,
    root_schema: &Schema,
) -> Option<Value> {
    match target_type {
        "integer" => {
            // Try to parse as i64
            if let Ok(num) = s.parse::<i64>() {
                return Some(Value::Number(num.into()));
            }
            // Try to parse as u64
            if let Ok(num) = s.parse::<u64>() {
                return Some(Value::Number(num.into()));
            }
            None
        }
        "number" => {
            // Try to parse as integer first
            if let Ok(num) = s.parse::<i64>() {
                return Some(Value::Number(num.into()));
            }
            // Then try float
            if let Ok(num) = s.parse::<f64>()
                && let Some(json_num) = serde_json::Number::from_f64(num)
            {
                return Some(Value::Number(json_num));
            }
            None
        }
        "boolean" => match s.trim().to_lowercase().as_str() {
            "true" => Some(Value::Bool(true)),
            "false" => Some(Value::Bool(false)),
            _ => None,
        },
        "null" => {
            if s.trim().to_lowercase() == "null" {
                Some(Value::Null)
            } else {
                None
            }
        }
        "string" => {
            // Keep as string
            None
        }
        "object" => {
            // Try to parse the string as a JSON object
            if let Ok(parsed) = try_parse_json_string(s)
                && parsed.is_object()
            {
                return Some(parsed);
            }
            None
        }
        "array" => {
            // Try to parse the string as a JSON array
            if let Ok(parsed) = try_parse_json_string(s)
                && parsed.is_array()
            {
                // Recursively coerce array items using the schema
                return Some(coerce_array_value(parsed, schema, root_schema));
            }

            // If direct parsing fails, try to extract array portion from the string
            // This handles cases like: "[\"item\"]{\n}" or "garbage[\"item\"]more"
            if let Some(extracted) = extract_array_from_string(s) {
                // Recursively coerce the extracted array items
                return Some(coerce_array_value(extracted, schema, root_schema));
            }

            None
        }
        _ => None,
    }
}

/// Helper function to recursively coerce array items based on the schema
fn coerce_array_value(
    value: Value,
    schema: &serde_json::Map<String, Value>,
    root_schema: &Schema,
) -> Value {
    if let Value::Array(arr) = value {
        // Check for prefixItems first (JSON Schema 2020-12 for tuples)
        if let Some(prefix_items_value) = schema.get("prefixItems")
            && let Some(item_schemas) = prefix_items_value.as_array()
        {
            // Array of schemas (tuple validation)
            return Value::Array(
                arr.into_iter()
                    .enumerate()
                    .map(|(i, item)| {
                        if let Some(schema_value) = item_schemas.get(i)
                            && let Ok(schema) =
                                serde_json::from_value::<Schema>(schema_value.clone())
                        {
                            return coerce_value_with_schema(item, &schema, root_schema);
                        }
                        item
                    })
                    .collect(),
            );
        }

        // Check if schema defines array item types (older JSON Schema draft)
        if let Some(items_schema_value) = schema.get("items") {
            // Check if it's a single schema or array of schemas
            if items_schema_value.is_object() || items_schema_value.is_boolean() {
                // Single schema for all items
                if let Ok(item_schema) =
                    serde_json::from_value::<Schema>(items_schema_value.clone())
                {
                    return Value::Array(
                        arr.into_iter()
                            .map(|item| coerce_value_with_schema(item, &item_schema, root_schema))
                            .collect(),
                    );
                }
            } else if let Some(item_schemas) = items_schema_value.as_array() {
                // Array of schemas (tuple validation)
                return Value::Array(
                    arr.into_iter()
                        .enumerate()
                        .map(|(i, item)| {
                            if let Some(schema_value) = item_schemas.get(i)
                                && let Ok(schema) =
                                    serde_json::from_value::<Schema>(schema_value.clone())
                            {
                                return coerce_value_with_schema(item, &schema, root_schema);
                            }
                            item
                        })
                        .collect(),
                );
            }
        }
        Value::Array(arr)
    } else {
        value
    }
}

/// Attempts to parse a string as JSON, JSON5, or repairable JSON, and unwraps
/// nested JSON strings when needed.
fn try_parse_json_string(s: &str) -> Result<Value, serde_json::Error> {
    let mut parsed = parse_json_like_value(s)?;

    for _ in 0..4 {
        let Value::String(inner) = &parsed else {
            return Ok(parsed);
        };

        let Ok(next) = parse_json_like_value(inner) else {
            return Ok(parsed);
        };

        parsed = next;
    }

    Ok(parsed)
}

fn parse_json_like_value(s: &str) -> Result<Value, serde_json::Error> {
    // First try parsing as-is (valid JSON)
    if let Ok(parsed) = serde_json::from_str::<Value>(s) {
        return Ok(parsed);
    }

    // If that fails, try parsing as JSON5 (handles single quotes, comments, etc.)
    if let Ok(parsed) = serde_json5::from_str::<Value>(s) {
        return Ok(parsed);
    }

    // Finally, fall back to Forge's JSON repair for malformed-but-recoverable
    // payloads such as persisted double-encoded tool arguments.
    json_repair(s).map_err(|e| serde_json::Error::custom(e.to_string()))
}

/// Extracts an array from a string that may contain garbage before/after the
/// array
///
/// # Examples
///
/// - `"[\"item\"]{\n}"` -> `["item"]`
/// - `"garbage[\"item\"]"` -> `["item"]`
/// - `"prefix[1,2,3]suffix"` -> `[1,2,3]`
///
/// This function is more permissive than standard JSON parsing - it will
/// extract arrays that have trailing or leading garbage. It requires the string
/// to at least look like it contains array-like content (quotes, commas, or
/// brackets after '[').
fn extract_array_from_string(s: &str) -> Option<Value> {
    // Find the first '[' and try to extract array from there
    let start_idx = s.find('[')?;

    // Check if there's anything after '[' that looks like array content
    // This helps us avoid extracting arrays from clearly invalid strings like
    // "[invalid json"
    let after_bracket = s.get(start_idx + 1..).unwrap_or("");
    let has_array_like_content = after_bracket.contains('"')
        || after_bracket.contains(',')
        || after_bracket.contains(']')
        || after_bracket.chars().next().is_some_and(|c| c.is_numeric());

    if !has_array_like_content {
        return None;
    }

    // Try to find matching closing bracket by parsing incrementally
    // Start from the opening bracket and try increasingly shorter substrings.
    // We iterate over valid char boundaries to avoid panicking on multi-byte
    // UTF-8 characters where byte offsets can land inside a character.
    for (end_idx, _) in s.char_indices().rev() {
        if end_idx <= start_idx {
            break;
        }
        let Some(candidate) = s.get(start_idx..end_idx) else {
            continue;
        };

        // Try to repair and parse this candidate
        if let Ok(parsed) = crate::json_repair::<Value>(candidate)
            && parsed.is_array()
        {
            return Some(parsed);
        }
    }

    // Also try the full string as a last resort (end at s.len() which is
    // always a valid boundary)
    let candidate = s.get(start_idx..)?;
    if let Ok(parsed) = crate::json_repair::<Value>(candidate)
        && parsed.is_array()
    {
        return Some(parsed);
    }

    None
}

#[cfg(test)]
mod tests {
    #![allow(dead_code)]
    use schemars::{JsonSchema, schema_for};
    use serde::{Deserialize, Serialize};
    use serde_json::json;

    use super::*;

    // Test structs with JsonSchema derive
    #[derive(JsonSchema)]
    #[allow(dead_code)]
    struct AgeData {
        age: i64,
    }

    #[derive(JsonSchema)]
    #[allow(dead_code)]
    struct RangeData {
        start: i64,
        end: i64,
    }

    #[derive(JsonSchema)]
    #[allow(dead_code)]
    struct PriceData {
        price: f64,
    }

    #[derive(JsonSchema)]
    #[allow(dead_code)]
    struct BooleanData {
        active: bool,
        disabled: bool,
    }

    #[derive(JsonSchema)]
    #[allow(dead_code)]
    struct UserData {
        age: i64,
        score: f64,
    }

    #[derive(JsonSchema)]
    #[allow(dead_code)]
    struct UserWrapper {
        user: UserData,
    }

    #[derive(JsonSchema)]
    #[allow(dead_code)]
    struct NumbersData {
        numbers: Vec<i64>,
    }

    #[derive(JsonSchema)]
    #[allow(dead_code)]
    struct MixedData {
        name: String,
        age: i64,
        active: bool,
    }

    #[derive(JsonSchema)]
    #[allow(dead_code)]
    struct PathData {
        path: String,
        start_line: i64,
        end_line: i64,
    }

    #[derive(JsonSchema)]
    #[allow(dead_code)]
    struct IntOrNull {
        value: Option<i64>,
    }

    #[derive(JsonSchema, Deserialize, Serialize)]
    #[allow(dead_code)]
    #[serde(untagged)]
    enum IntOrBool {
        Int(i64),
        Bool(bool),
    }

    #[derive(JsonSchema)]
    #[allow(dead_code)]
    struct IntOrBoolData {
        value: IntOrBool,
    }

    #[derive(JsonSchema)]
    #[allow(dead_code)]
    struct AllOfIntNumber {
        value: i64,
    }

    #[derive(JsonSchema)]
    #[allow(dead_code)]
    struct CoordinatesData {
        coordinates: [f64; 3],
    }

    #[derive(JsonSchema)]
    #[allow(dead_code)]
    struct MixedTupleData {
        data: (String, i64, bool),
    }

    #[derive(JsonSchema)]
    #[allow(dead_code)]
    struct TupleItems {
        items: [i64; 2],
    }

    #[derive(JsonSchema)]
    #[allow(dead_code)]
    struct ExtraItemsData {
        items: Vec<serde_json::Value>,
    }

    #[derive(JsonSchema)]
    #[allow(dead_code)]
    struct NestedUnionData {
        nested: IntOrNull,
    }

    #[derive(JsonSchema)]
    #[allow(dead_code)]
    struct NullData {
        value: Option<()>,
    }

    #[derive(JsonSchema)]
    #[allow(dead_code)]
    struct BoolData {
        value: bool,
    }

    #[derive(JsonSchema)]
    #[allow(dead_code)]
    struct LargeIntData {
        value: i64,
    }

    #[derive(JsonSchema)]
    #[allow(dead_code)]
    struct UnsignedIntData {
        value: u64,
    }

    #[derive(JsonSchema)]
    #[allow(dead_code)]
    struct ArrayData {
        items: Vec<i64>,
    }

    #[derive(JsonSchema)]
    #[allow(dead_code)]
    struct EditsData {
        edits: Vec<serde_json::Value>,
    }

    #[derive(JsonSchema)]
    #[allow(dead_code)]
    struct ConfigData {
        config: std::collections::BTreeMap<String, serde_json::Value>,
    }

    #[derive(JsonSchema)]
    #[allow(dead_code)]
    struct DataArray {
        data: Vec<serde_json::Value>,
    }

    #[derive(JsonSchema)]
    #[allow(dead_code)]
    struct ItemsArray {
        items: Vec<serde_json::Value>,
    }

    #[derive(JsonSchema)]
    #[allow(dead_code)]
    struct ConfigWithComments {
        config: std::collections::BTreeMap<String, serde_json::Value>,
    }

    #[derive(JsonSchema)]
    #[allow(dead_code)]
    struct ItemsTrailingComma {
        items: Vec<serde_json::Value>,
    }

    #[derive(JsonSchema)]
    #[allow(dead_code)]
    struct MultiPatchData {
        edits: Vec<serde_json::Value>,
    }

    #[test]
    fn test_coerce_string_to_integer() {
        let fixture = json!({"age": "42"});
        let schema = schema_for!(AgeData);
        let actual = coerce_to_schema(fixture, &schema);
        let expected = json!({"age": 42});
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_coerce_multiple_string_integers() {
        let fixture = json!({"start": "100", "end": "200"});
        let schema = schema_for!(RangeData);
        let actual = coerce_to_schema(fixture, &schema);
        let expected = json!({"start": 100, "end": 200});
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_coerce_string_to_number_float() {
        let fixture = json!({"price": "19.99"});
        let schema = schema_for!(PriceData);
        let actual = coerce_to_schema(fixture, &schema);
        let expected = json!({"price": 19.99});
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_coerce_string_to_boolean() {
        let fixture = json!({"active": "true", "disabled": "false"});
        let schema = schema_for!(BooleanData);
        let actual = coerce_to_schema(fixture, &schema);
        let expected = json!({"active": true, "disabled": false});
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_no_coercion_when_types_match() {
        let fixture = json!({"age": 42});
        let schema = schema_for!(AgeData);
        let actual = coerce_to_schema(fixture, &schema);
        let expected = json!({"age": 42});
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_no_coercion_for_invalid_strings() {
        let fixture = json!({"age": "not_a_number"});
        let schema = schema_for!(AgeData);
        let actual = coerce_to_schema(fixture, &schema);
        let expected = json!({"age": "not_a_number"});
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_coerce_nested_objects() {
        let fixture = json!({"user": {"age": "30", "score": "95.5"}});
        let schema = schema_for!(UserWrapper);
        let actual = coerce_to_schema(fixture, &schema);
        let expected = json!({"user": {"age": 30, "score": 95.5}});
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_coerce_array_items() {
        let fixture = json!({"numbers": ["1", "2", "3"]});
        let schema = schema_for!(NumbersData);
        let actual = coerce_to_schema(fixture, &schema);
        let expected = json!({"numbers": [1, 2, 3]});
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_preserve_non_string_values() {
        let fixture = json!({"name": "John", "age": 42, "active": true});
        let schema = schema_for!(MixedData);
        let actual = coerce_to_schema(fixture, &schema);
        let expected = json!({"name": "John", "age": 42, "active": true});
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_read_tool_line_numbers() {
        // Simulate the exact case from the task: read tool with string line numbers
        let fixture = json!({
            "path": "/Users/amit/code-forge/crates/forge_main/src/ui.rs",
            "start_line": "2255",
            "end_line": "2285"
        });

        let schema = schema_for!(PathData);
        let actual = coerce_to_schema(fixture, &schema);
        let expected = json!({
            "path": "/Users/amit/code-forge/crates/forge_main/src/ui.rs",
            "start_line": 2255,
            "end_line": 2285
        });
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_coerce_any_of_union_types() {
        // Test coercing string to integer
        let fixture = json!({"value": "42"});
        let schema = schema_for!(IntOrNull);
        let actual = coerce_to_schema(fixture, &schema);
        let expected = json!({"value": 42});
        assert_eq!(actual, expected);

        // Test preserving null
        let fixture = json!({"value": null});
        let actual = coerce_to_schema(fixture, &schema);
        let expected = json!({"value": null});
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_coerce_one_of_union_types() {
        // Test coercing string to integer
        let fixture = json!({"value": "123"});
        let schema = schema_for!(IntOrBoolData);
        let actual = coerce_to_schema(fixture, &schema);
        let expected = json!({"value": 123});
        assert_eq!(actual, expected);

        // Test coercing string to boolean
        let fixture = json!({"value": "true"});
        let actual = coerce_to_schema(fixture, &schema);
        let expected = json!({"value": true});
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_coerce_all_of_composition() {
        // Test coercing string to integer via allOf composition
        let fixture = json!({"value": "42"});
        let schema = schema_for!(AllOfIntNumber);
        let actual = coerce_to_schema(fixture, &schema);
        let expected = json!({"value": 42});
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_any_of_preserves_original_when_no_match() {
        // Test that anyOf preserves original value when no subschema matches
        // Note: oneOf behaves similarly
        let fixture = json!({"value": "not_a_number"});
        let schema = schema_for!(IntOrBoolData);
        let actual = coerce_to_schema(fixture, &schema);
        let expected = json!({"value": "not_a_number"});
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_any_of_with_number_coercion() {
        // Test anyOf with number coercion
        let fixture = json!({"value": "2.14"});
        let schema = schema_for!(IntOrNull);
        let actual = coerce_to_schema(fixture, &schema);
        // The anyOf schema tries each subschema; since "2.14" can't be parsed as i64,
        // it returns the original value
        let expected = json!({"value": "2.14"});
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_array_with_tuple_schema() {
        // Test array with tuple schema (SingleOrVec::Vec)
        let fixture = json!({"coordinates": ["1.5", "2.5", "3.5"]});
        let schema = schema_for!(CoordinatesData);
        let actual = coerce_to_schema(fixture, &schema);
        let expected = json!({"coordinates": [1.5, 2.5, 3.5]});
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_array_with_tuple_schema_mixed_types() {
        // Test array with tuple schema with mixed types
        let fixture = json!({"data": ["name", "42", "true"]});
        let schema = schema_for!(MixedTupleData);
        let actual = coerce_to_schema(fixture, &schema);
        let expected = json!({"data": ["name", 42, true]});
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_array_with_tuple_schema_extra_items() {
        // Test that Vec<serde_json::Value> doesn't coerce items (no type constraints)
        let fixture = json!({"items": ["1", "2", "3", "4"]});
        let schema = schema_for!(ExtraItemsData);
        let actual = coerce_to_schema(fixture, &schema);
        let expected = json!({"items": ["1", "2", "3", "4"]});
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_nested_any_of_in_object() {
        // Test coercing in nested object with anyOf
        let fixture = json!({"nested": {"value": "42"}});
        let schema = schema_for!(NestedUnionData);
        let actual = coerce_to_schema(fixture, &schema);
        let expected = json!({"nested": {"value": 42}});
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_coerce_string_to_null() {
        // Test coercing "null" string to null
        let fixture = json!({"value": "null"});
        let schema = schema_for!(NullData);
        let actual = coerce_to_schema(fixture, &schema);
        let expected = json!({"value": null});
        assert_eq!(actual, expected);

        // Test that "NULL" (uppercase) also works
        let fixture = json!({"value": "NULL"});
        let actual = coerce_to_schema(fixture, &schema);
        let expected = json!({"value": null});
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_coerce_boolean_case_insensitive() {
        // Test that boolean coercion is case-insensitive
        let schema = schema_for!(BoolData);

        // Test various case variations
        for (input, expected) in [
            ("true", true),
            ("TRUE", true),
            ("True", true),
            ("false", false),
            ("FALSE", false),
            ("False", false),
        ] {
            let fixture = json!({"value": input});
            let actual = coerce_to_schema(fixture, &schema);
            let expected = json!({"value": expected});
            assert_eq!(actual, expected);
        }
    }

    #[test]
    fn test_coerce_large_integer() {
        // Test coercing large integers that fit in i64
        let schema = schema_for!(LargeIntData);

        // Test coercing large positive integer
        let fixture = json!({"value": "9223372036854775807"}); // i64::MAX
        let actual = coerce_to_schema(fixture, &schema);
        let expected = json!({"value": 9223372036854775807i64});
        assert_eq!(actual, expected);

        // Test coercing large negative integer
        let fixture = json!({"value": "-9223372036854775808"}); // i64::MIN
        let actual = coerce_to_schema(fixture, &schema);
        let expected = json!({"value": -9223372036854775808i64});
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_coerce_unsigned_integer() {
        // Test coercing unsigned integers (u64)
        let schema = schema_for!(UnsignedIntData);

        // Test coercing large unsigned integer that doesn't fit in i64
        let fixture = json!({"value": "18446744073709551615"}); // u64::MAX
        let actual = coerce_to_schema(fixture, &schema);
        let expected = json!({"value": 18446744073709551615u64});
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_coerce_string_to_array() {
        // Test coercing a JSON array string to an actual array
        let fixture = json!({"items": "[1, 2, 3]"});
        let schema = schema_for!(ArrayData);
        let actual = coerce_to_schema(fixture, &schema);
        let expected = json!({"items": [1, 2, 3]});
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_coerce_python_style_string_to_array() {
        // Test coercing a Python-style array string to an actual array
        let fixture = json!({"edits": "[{'content': 'test', 'operation': 'replace'}]"});
        let schema = schema_for!(EditsData);
        let actual = coerce_to_schema(fixture, &schema);
        let expected = json!({"edits": [{"content": "test", "operation": "replace"}]});
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_coerce_python_style_string_to_object() {
        // Test coercing a Python-style object string to an actual object
        let fixture = json!({"config": "{'key': 'value', 'number': 42}"});
        let schema = schema_for!(ConfigData);
        let actual = coerce_to_schema(fixture, &schema);
        let expected = json!({"config": {"key": "value", "number": 42}});
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_coerce_double_encoded_string_to_object() {
        let fixture = json!({"config": r#""{\"key\":\"value\",\"number\":42}""#});
        let schema = schema_for!(ConfigData);
        let actual = coerce_to_schema(fixture, &schema);
        let expected = json!({"config": {"key": "value", "number": 42}});
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_repairs_invalid_json_string_when_schema_expects_array() {
        // Invalid JSON-like array strings are repaired into arrays when the schema
        // expects one.
        let fixture = json!({"data": "[invalid json"});
        let schema = schema_for!(DataArray);
        let actual = coerce_to_schema(fixture, &schema);
        let expected = json!({"data": ["invalid json"]});
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_coerce_json5_with_comments() {
        // Test coercing JSON5 with comments
        let fixture = json!({"config": r#"{
            // This is a comment
            "key": "value",
            "number": 42,
        }"#});
        let schema = schema_for!(ConfigWithComments);
        let actual = coerce_to_schema(fixture, &schema);
        let expected = json!({"config": {"key": "value", "number": 42}});
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_coerce_json5_with_trailing_commas() {
        // Test coercing JSON5 with trailing commas
        let fixture = json!({"items": "[1, 2, 3,]"});
        let schema = schema_for!(ItemsTrailingComma);
        let actual = coerce_to_schema(fixture, &schema);
        let expected = json!({"items": [1, 2, 3]});
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_coerce_multi_patch_python_style() {
        // Test coercing exact Python-style input from error
        // This matches multi_patch tool call format with nested objects
        let python_style = r#"[{'content': 'use schemars::schema::{InstanceType, RootSchema, Schema, SchemaObject, SingleOrVec};', 'operation': 'replace', 'path': 'crates/forge_json_repair/src/schema_coercion.rs'}, {'content': 'fn coerce_value_with_schema(value: Value, schema: &Schema) -> Value {', 'operation': 'replace', 'path': 'crates/forge_json_repair/src/schema_coercion.rs'}]"#;

        let fixture = json!({"edits": python_style});
        let schema = schema_for!(MultiPatchData);
        let actual = coerce_to_schema(fixture, &schema);

        // Should coerce string to an array of objects
        assert!(actual["edits"].is_array());
        let edits = actual["edits"].as_array().unwrap();
        assert_eq!(edits.len(), 2);

        // Verify first edit object
        assert_eq!(
            edits[0]["content"],
            "use schemars::schema::{InstanceType, RootSchema, Schema, SchemaObject, SingleOrVec};"
        );
        assert_eq!(edits[0]["operation"], "replace");
        assert_eq!(
            edits[0]["path"],
            "crates/forge_json_repair/src/schema_coercion.rs"
        );

        // Verify second edit object
        assert_eq!(
            edits[1]["content"],
            "fn coerce_value_with_schema(value: Value, schema: &Schema) -> Value {"
        );
        assert_eq!(edits[1]["operation"], "replace");
        assert_eq!(
            edits[1]["path"],
            "crates/forge_json_repair/src/schema_coercion.rs"
        );
    }

    // Tests for array extraction with garbage
    #[derive(JsonSchema)]
    #[allow(dead_code)]
    struct AgentInput {
        tasks: Vec<String>,
    }

    #[test]
    fn test_coerce_malformed_string_array_with_trailing_garbage() {
        // This is the exact case from the issue: string that looks like an array but
        // has trailing garbage
        let fixture = json!({
            "tasks": "[\"Find where the main function is defined in the code-forge codebase. Search for main function definitions and entry points.\"]{\n}"
        });

        let schema = schema_for!(AgentInput);
        let actual = coerce_to_schema(fixture, &schema);

        // Should extract the array portion and ignore the trailing garbage
        let expected = json!({
            "tasks": ["Find where the main function is defined in the code-forge codebase. Search for main function definitions and entry points."]
        });

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_coerce_string_array_with_leading_garbage() {
        // Test with leading garbage before the array
        let fixture = json!({
            "tasks": "garbage[\"task1\", \"task2\"]"
        });

        let schema = schema_for!(AgentInput);
        let actual = coerce_to_schema(fixture, &schema);

        // Should extract the array portion from the string
        let expected = json!({
            "tasks": ["task1", "task2"]
        });

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_coerce_string_array_with_both_garbage() {
        // Test with garbage on both ends
        let fixture = json!({
            "tasks": "prefix[\"task1\"]suffix"
        });

        let schema = schema_for!(AgentInput);
        let actual = coerce_to_schema(fixture, &schema);

        // Should extract the array portion
        let expected = json!({
            "tasks": ["task1"]
        });

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_repair_then_coerce_full_payload() {
        // Test the full flow: repair the JSON structure, then coerce types
        let malformed = r#"{"tasks": "[\"Find main function\"]{\n}"}"#;

        // First repair the JSON structure
        let repaired: Value = crate::json_repair(malformed).expect("Should repair JSON");

        // Then coerce to schema
        let schema = schema_for!(AgentInput);
        let actual = coerce_to_schema(repaired, &schema);

        let expected = json!({
            "tasks": ["Find main function"]
        });

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_preserve_already_valid_array() {
        // Ensure we don't break valid arrays
        let fixture = json!({
            "tasks": ["task1", "task2"]
        });

        let schema = schema_for!(AgentInput);
        let actual = coerce_to_schema(fixture, &schema);

        let expected = json!({
            "tasks": ["task1", "task2"]
        });

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_coerce_string_array_without_garbage() {
        // Valid JSON array string
        let fixture = json!({
            "tasks": "[\"task1\", \"task2\"]"
        });

        let schema = schema_for!(AgentInput);
        let actual = coerce_to_schema(fixture, &schema);

        let expected = json!({
            "tasks": ["task1", "task2"]
        });

        assert_eq!(actual, expected);
    }

    // Test nested structures
    #[derive(JsonSchema)]
    #[allow(dead_code)]
    struct SearchQuery {
        query: String,
        use_case: String,
    }

    #[derive(JsonSchema)]
    #[allow(dead_code)]
    struct SemanticSearchInput {
        queries: Vec<SearchQuery>,
        extensions: Vec<String>,
    }

    #[test]
    fn test_coerce_nested_array_of_objects_with_garbage() {
        // Test array of objects with trailing garbage
        let fixture = json!({
            "queries": "[{\"query\": \"test\", \"use_case\": \"find\"}]garbage",
            "extensions": "[\".rs\"]"
        });

        let schema = schema_for!(SemanticSearchInput);
        let actual = coerce_to_schema(fixture, &schema);

        let expected = json!({
            "queries": [{"query": "test", "use_case": "find"}],
            "extensions": [".rs"]
        });

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_coerce_nested_array_with_string_numbers() {
        // Test that nested coercion works - string numbers inside objects inside arrays
        #[derive(JsonSchema)]
        #[allow(dead_code)]
        struct Item {
            id: i64,
            name: String,
        }

        #[derive(JsonSchema)]
        #[allow(dead_code)]
        struct ItemList {
            items: Vec<Item>,
        }

        let fixture = json!({
            "items": "[{\"id\": \"42\", \"name\": \"test\"}]extra"
        });

        let schema = schema_for!(ItemList);
        let actual = coerce_to_schema(fixture, &schema);

        // The id should be coerced from string "42" to number 42
        let expected = json!({
            "items": [{"id": 42, "name": "test"}]
        });

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_coerce_deeply_nested_arrays() {
        // Test arrays containing objects containing arrays
        #[derive(JsonSchema)]
        #[allow(dead_code)]
        struct DeepItem {
            tags: Vec<String>,
        }

        #[derive(JsonSchema)]
        #[allow(dead_code)]
        struct DeepList {
            items: Vec<DeepItem>,
        }

        let fixture = json!({
            "items": "[{\"tags\": [\"tag1\", \"tag2\"]}]garbage"
        });

        let schema = schema_for!(DeepList);
        let actual = coerce_to_schema(fixture, &schema);

        let expected = json!({
            "items": [{"tags": ["tag1", "tag2"]}]
        });

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_coerce_empty_string_to_null_for_nullable_field() {
        // Simulates LLM sending "" for a nullable string field (e.g., file_type in
        // fs_search). The schema uses "nullable: true" (OpenAPI 3.0 style).
        #[derive(JsonSchema)]
        #[allow(dead_code)]
        struct NullableStringData {
            required_field: String,
            #[schemars(default)]
            optional_field: Option<String>,
        }

        // Generate schema with option_nullable=true (matching project settings)
        let settings = schemars::generate::SchemaSettings::default()
            .with_transform(schemars::transform::AddNullable::default())
            .into_generator();
        let schema = settings.into_root_schema_for::<NullableStringData>();

        let fixture = json!({
            "required_field": "value",
            "optional_field": ""
        });
        let actual = coerce_to_schema(fixture, &schema);
        let expected = json!({
            "required_field": "value",
            "optional_field": null
        });
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_preserve_non_empty_string_for_nullable_field() {
        // Non-empty strings should NOT be converted to null, even for nullable fields
        #[derive(JsonSchema)]
        #[allow(dead_code)]
        struct NullableStringData {
            optional_field: Option<String>,
        }

        let settings = schemars::generate::SchemaSettings::default()
            .with_transform(schemars::transform::AddNullable::default())
            .into_generator();
        let schema = settings.into_root_schema_for::<NullableStringData>();
        let fixture = json!({"optional_field": "rust"});
        let actual = coerce_to_schema(fixture, &schema);
        let expected = json!({"optional_field": "rust"});
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_preserve_empty_string_for_required_field() {
        // Empty strings should NOT be converted to null for non-nullable fields
        #[derive(JsonSchema)]
        #[allow(dead_code)]
        struct RequiredStringData {
            name: String,
        }

        let schema = schema_for!(RequiredStringData);

        let fixture = json!({"name": ""});
        let actual = coerce_to_schema(fixture, &schema);
        let expected = json!({"name": ""});
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_coerce_empty_string_to_null_for_nullable_integer() {
        // Empty string for a nullable integer should become null
        #[derive(JsonSchema)]
        #[allow(dead_code)]
        struct NullableIntData {
            count: Option<u32>,
        }

        let settings = schemars::generate::SchemaSettings::default()
            .with_transform(schemars::transform::AddNullable::default())
            .into_generator();
        let schema = settings.into_root_schema_for::<NullableIntData>();

        let fixture = json!({"count": ""});
        let actual = coerce_to_schema(fixture, &schema);
        let expected = json!({"count": null});
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_extract_array_from_string_with_multibyte_chars() {
        // Multi-byte UTF-8 characters (like arrows and emojis) should not
        // cause panics when extract_array_from_string iterates over byte
        // positions. The function must only slice at valid char boundaries.
        let input = "prefix → [1, 2, 3] suffix";
        let result = extract_array_from_string(input);
        assert!(result.is_some());
        let arr = result.unwrap();
        assert!(arr.is_array());
        assert_eq!(arr.as_array().unwrap().len(), 3);
    }

    #[test]
    fn test_extract_array_from_string_with_emoji_prefix() {
        // Emoji characters are 4 bytes each, many byte positions inside them
        // are invalid char boundaries.
        let input = "🔑🔒 [4, 5, 6]";
        let result = extract_array_from_string(input);
        assert!(result.is_some());
        let arr = result.unwrap();
        assert!(arr.is_array());
        assert_eq!(arr.as_array().unwrap().len(), 3);
    }

    #[test]
    fn test_extract_array_from_string_with_multibyte_inside_array() {
        // Multi-byte chars inside the array value itself
        let input = r#"["αβγ", "δεζ"]"#;
        let result = extract_array_from_string(input);
        assert!(result.is_some());
        let arr = result.unwrap();
        assert!(arr.is_array());
    }
}
