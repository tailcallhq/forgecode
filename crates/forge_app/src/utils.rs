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
    normalize_named_schema_keyword(map, "properties", strict_mode);

    for key in ["items", "additionalProperties", "allOf", "anyOf"] {
        normalize_schema_keyword(map, key, strict_mode);
    }

    if !strict_mode {
        for key in ["$defs", "definitions", "patternProperties"] {
            normalize_named_schema_keyword(map, key, strict_mode);
        }

        for key in [
            "oneOf",
            "prefixItems",
            "contains",
            "not",
            "if",
            "then",
            "else",
        ] {
            normalize_schema_keyword(map, key, strict_mode);
        }
    }
}

fn normalize_openai_schema_subset_keywords(map: &mut serde_json::Map<String, serde_json::Value>) {
    for key in [
        "$schema",
        "$id",
        "$anchor",
        "$comment",
        "$defs",
        "$ref",
        "additionalItems",
        "contains",
        "definitions",
        "dependentRequired",
        "dependentSchemas",
        "deprecated",
        "else",
        "examples",
        "exclusiveMaximum",
        "exclusiveMinimum",
        "if",
        "maxContains",
        "maxItems",
        "maxLength",
        "maxProperties",
        "maximum",
        "minContains",
        "minItems",
        "minLength",
        "minProperties",
        "multipleOf",
        "not",
        "pattern",
        "patternProperties",
        "prefixItems",
        "propertyNames",
        "readOnly",
        "then",
        "title",
        "unevaluatedItems",
        "unevaluatedProperties",
        "uniqueItems",
        "writeOnly",
    ] {
        map.remove(key);
    }

    if let Some(const_value) = map.remove("const")
        && !map.contains_key("enum")
    {
        map.insert(
            "enum".to_string(),
            serde_json::Value::Array(vec![const_value]),
        );
    }
}

fn normalize_one_of_keyword(
    map: &mut serde_json::Map<String, serde_json::Value>,
    strict_mode: bool,
) {
    if !strict_mode {
        return;
    }

    let Some(one_of) = map.remove("oneOf") else {
        return;
    };

    match map.get_mut("anyOf") {
        Some(serde_json::Value::Array(any_of)) => match one_of {
            serde_json::Value::Array(mut one_of) => any_of.append(&mut one_of),
            value => any_of.push(value),
        },
        _ => {
            map.insert("anyOf".to_string(), one_of);
        }
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

fn is_array_schema(map: &serde_json::Map<String, serde_json::Value>) -> bool {
    map.get("type")
        .and_then(|value| value.as_str())
        .is_some_and(|ty| ty == "array")
        || map.contains_key("items")
}

fn normalize_array_items(map: &mut serde_json::Map<String, serde_json::Value>, strict_mode: bool) {
    if strict_mode && is_array_schema(map) && !map.contains_key("items") {
        map.insert("items".to_string(), serde_json::json!({ "type": "string" }));
    }
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
/// - unsupported JSON Schema keywords are removed, matching Codex's limited
///   Responses API schema subset, while preserving `default` and `minimum`
///   values
/// - `const` is converted to a single-value `enum`
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
                // Match Codex's Responses API schema subset. Codex parses MCP
                // schemas into a typed representation that only serializes the
                // supported OpenAI fields; Forge keeps raw JSON schemas, so we
                // explicitly remove unsupported validation/meta keywords here.
                normalize_openai_schema_subset_keywords(map);
                // Convert oneOf to anyOf because the Responses API rejects oneOf
                // in tool parameter schemas while accepting equivalent anyOf
                // unions.
                normalize_one_of_keyword(map, strict_mode);
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
            } else if strict_mode
                && !map.contains_key("type")
                && !map.contains_key("anyOf")
                && !map.contains_key("allOf")
            {
                // In strict mode, OpenAI/Codex requires all property schemas to have a
                // 'type' key. External MCP tool schemas may define properties with only a
                // description and no type. Default such typeless leaf schemas to "string"
                // so the request is not rejected with "schema must have a 'type' key".
                map.insert(
                    "type".to_string(),
                    serde_json::Value::String("string".to_string()),
                );
            }

            normalize_array_items(map, strict_mode);

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

fn normalize_gemini_schema_subset_keywords(map: &mut serde_json::Map<String, serde_json::Value>) {
    if let Some(exclusive_minimum) = map.remove("exclusiveMinimum") {
        map.entry("minimum".to_string())
            .or_insert(exclusive_minimum);
    }

    if let Some(exclusive_maximum) = map.remove("exclusiveMaximum") {
        map.entry("maximum".to_string())
            .or_insert(exclusive_maximum);
    }

    for key in [
        "$schema",
        "$id",
        "$anchor",
        "$comment",
        "$defs",
        "$ref",
        "additionalItems",
        "additionalProperties",
        "definitions",
        "deprecated",
        "examples",
        "title",
        "unevaluatedItems",
        "unevaluatedProperties",
        "writeOnly",
        "readOnly",
    ] {
        map.remove(key);
    }
}

/// Sanitizes a JSON schema for Google/Gemini API compatibility.
///
/// The Gemini API uses OpenAPI 3.0-style function declarations rather than raw
/// JSON Schema, and has several restrictions that differ from standard JSON
/// Schema:
///
/// - **Integer/number enums are rejected**: Gemini requires all enum values to
///   be strings. Integer and number type enums are converted to string enums.
/// - **Arrays require `items`**: Gemini rejects array schemas without an
///   `items` field. A default `{ "type": "string" }` is added if missing,
///   unless the array has a combiner (`anyOf`/`oneOf`/`allOf`).
/// - **Non-object types must not have `properties`/`required`**: Gemini rejects
///   `properties` and `required` fields on non-object schemas (e.g., strings
///   with properties). These are stripped.
/// - **`required` must reference existing `properties`**: Gemini rejects
///   `required` entries that don't have corresponding entries in `properties`.
///   The `required` array is filtered to only include fields present in
///   `properties`.
/// - **Unsupported JSON Schema metadata and references are rejected**:
///   `$schema`, `$defs`, `$ref`, `title`, and `additionalProperties` are
///   removed.
/// - **Exclusive bounds are rejected**: `exclusiveMinimum` and
///   `exclusiveMaximum` are converted to `minimum` and `maximum` when the
///   inclusive bound is not already present.
/// - **`const` is rejected**: Converted to single-value `enum` (OpenAPI 3.0
///   style).
/// - **Nullable types**: `{ "type": ["string", "null"] }` is converted to `{
///   "type": "string", "nullable": true }` (OpenAPI 3.0 style).
pub fn sanitize_gemini_schema(schema: &mut serde_json::Value) {
    match schema {
        serde_json::Value::Object(map) => {
            normalize_gemini_schema_subset_keywords(map);

            // Convert const to enum
            if let Some(const_value) = map.remove("const")
                && !map.contains_key("enum")
            {
                map.insert(
                    "enum".to_string(),
                    serde_json::Value::Array(vec![const_value]),
                );
            }

            // Handle type arrays — convert to OpenAPI 3.0 compatible format.
            // OpenAPI 3.0 doesn't support type arrays, so we convert them:
            // - ["string", "null"] -> type: "string", nullable: true
            // - ["string", "number"] -> anyOf: [{type: string}, {type: number}]
            // - ["string", "number", "null"] -> anyOf: [{type: string}, {type: number}],
            //   nullable: true
            if map.contains_key("type") && map["type"].is_array() {
                let types = map.remove("type").unwrap();
                if let serde_json::Value::Array(type_arr) = types {
                    let has_null = type_arr.iter().any(|t| t == "null");
                    let non_null_types: Vec<serde_json::Value> =
                        type_arr.into_iter().filter(|t| *t != "null").collect();

                    if non_null_types.is_empty() {
                        // Only null type
                        map.insert(
                            "type".to_string(),
                            serde_json::Value::String("null".to_string()),
                        );
                    } else if non_null_types.len() == 1 {
                        // Single non-null type: ["string", "null"] -> type: "string", nullable:
                        // true
                        map.insert(
                            "type".to_string(),
                            non_null_types.into_iter().next().unwrap(),
                        );
                        if has_null {
                            map.insert("nullable".to_string(), serde_json::Value::Bool(true));
                        }
                    } else {
                        // Multiple non-null types: convert to anyOf
                        let any_of_items: Vec<serde_json::Value> = non_null_types
                            .into_iter()
                            .map(|t| serde_json::json!({ "type": t }))
                            .collect();
                        map.insert("anyOf".to_string(), serde_json::Value::Array(any_of_items));
                        if has_null {
                            map.insert("nullable".to_string(), serde_json::Value::Bool(true));
                        }
                    }
                }
            }

            // Handle anyOf with null type — elevate null to nullable.
            // { anyOf: [{type: string, ...}, {type: null}] } -> { type: string, nullable:
            // true, ... } { anyOf: [{type: string, ...}, {type: number, ...},
            // {type: null}] } -> { anyOf: [{type: string}, {type: number}], nullable: true
            // }
            if let Some(serde_json::Value::Array(any_of)) = map.remove("anyOf") {
                let (null_schemas, non_null_schemas): (Vec<_>, Vec<_>) =
                    any_of.into_iter().partition(|s| {
                        s.as_object().is_some_and(|o| {
                            o.len() == 1 && o.get("type").is_some_and(|t| t == "null")
                        })
                    });

                if !null_schemas.is_empty() && non_null_schemas.len() == 1 {
                    // Single non-null branch with nullable: merge into this schema
                    let mut merged = non_null_schemas.into_iter().next().unwrap();
                    if let serde_json::Value::Object(merged_map) = &mut merged {
                        // Copy current schema's keys into the merged branch
                        // (anyOf was already removed, so we copy everything else)
                        let current_keys: Vec<(String, serde_json::Value)> =
                            map.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
                        for (key, value) in current_keys {
                            merged_map.entry(key).or_insert(value);
                        }
                    }
                    map.insert("nullable".to_string(), serde_json::Value::Bool(true));
                    // Put the merged schema back into map
                    if let serde_json::Value::Object(merged_map) = merged {
                        for (key, value) in merged_map {
                            map.insert(key, value);
                        }
                    }
                } else {
                    // Either no null schemas, or multiple non-null schemas:
                    // put anyOf back, possibly with nullable
                    if !null_schemas.is_empty() {
                        map.insert("nullable".to_string(), serde_json::Value::Bool(true));
                    }
                    map.insert(
                        "anyOf".to_string(),
                        serde_json::Value::Array(non_null_schemas),
                    );
                }
            }

            // Convert integer/number enum values to strings (Gemini rejects integer
            // enums). Only change the type when there's an enum — a bare integer/number
            // type without enum is valid for Gemini.
            let has_numeric_type_with_enum = map
                .get("type")
                .and_then(|v| v.as_str())
                .is_some_and(|t| t == "integer" || t == "number")
                && map.contains_key("enum");

            if has_numeric_type_with_enum {
                map.insert(
                    "type".to_string(),
                    serde_json::Value::String("string".to_string()),
                );
            }

            // Convert any numeric enum values to strings
            if let Some(serde_json::Value::Array(enum_values)) = map.get_mut("enum") {
                *enum_values = enum_values
                    .iter()
                    .map(|v| match v {
                        serde_json::Value::Number(n) => serde_json::Value::String(n.to_string()),
                        other => other.clone(),
                    })
                    .collect();
            }

            // Handle array schemas: ensure items field is present
            let is_array_type = map
                .get("type")
                .and_then(|v| v.as_str())
                .is_some_and(|t| t == "array");

            let has_combiner =
                map.contains_key("anyOf") || map.contains_key("oneOf") || map.contains_key("allOf");

            if is_array_type && !has_combiner {
                match map.get_mut("items") {
                    None => {
                        // No items at all — add a default string items schema
                        map.insert("items".to_string(), serde_json::json!({ "type": "string" }));
                    }
                    Some(serde_json::Value::Object(items_map)) => {
                        // Items exists but may be empty — ensure it has at least a
                        // type if it has no schema-defining keywords
                        let has_schema_intent = items_map.contains_key("type")
                            || items_map.contains_key("$ref")
                            || items_map.contains_key("enum")
                            || items_map.contains_key("const")
                            || items_map.contains_key("anyOf")
                            || items_map.contains_key("oneOf")
                            || items_map.contains_key("allOf")
                            || items_map.contains_key("properties")
                            || items_map.contains_key("additionalProperties")
                            || items_map.contains_key("patternProperties")
                            || items_map.contains_key("required")
                            || items_map.contains_key("not")
                            || items_map.contains_key("if")
                            || items_map.contains_key("then")
                            || items_map.contains_key("else");

                        if !has_schema_intent {
                            items_map.insert(
                                "type".to_string(),
                                serde_json::Value::String("string".to_string()),
                            );
                        }
                    }
                    _ => {} // items is an array or other — leave as-is
                }
            }

            // Remove properties/required from non-object types (unless it has a
            // combiner, which overrides the type)
            let has_explicit_type = map.contains_key("type");
            let type_is_not_object = map
                .get("type")
                .and_then(|v| v.as_str())
                .is_some_and(|t| t != "object");

            if has_explicit_type && type_is_not_object && !has_combiner {
                map.remove("properties");
                map.remove("required");
            }

            // Filter required array to only include fields present in properties
            let property_keys: Option<Vec<String>> = map
                .get("properties")
                .and_then(|v| v.as_object())
                .map(|props| props.keys().cloned().collect());

            if let (Some(property_keys), Some(serde_json::Value::Array(required))) =
                (property_keys, map.get_mut("required"))
            {
                required.retain(|v| {
                    v.as_str()
                        .is_some_and(|field| property_keys.iter().any(|k| k == field))
                });
            }

            // Recursively sanitize all nested schemas
            for key in ["properties", "$defs", "definitions", "patternProperties"] {
                if let Some(serde_json::Value::Object(named_schemas)) = map.get_mut(key) {
                    for value in named_schemas.values_mut() {
                        sanitize_gemini_schema(value);
                    }
                }
            }

            for key in [
                "items",
                "contains",
                "not",
                "if",
                "then",
                "else",
                "additionalItems",
                "unevaluatedProperties",
            ] {
                if let Some(value) = map.get_mut(key) {
                    sanitize_gemini_schema(value);
                }
            }

            for key in ["allOf", "anyOf", "oneOf", "prefixItems"] {
                if let Some(serde_json::Value::Array(items)) = map.get_mut(key) {
                    for value in items.iter_mut() {
                        sanitize_gemini_schema(value);
                    }
                }
            }
        }
        serde_json::Value::Array(items) => {
            for value in items.iter_mut() {
                sanitize_gemini_schema(value);
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
    fn test_strict_schema_preserves_default_values() {
        let mut fixture = json!({
            "type": "object",
            "properties": {
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of records",
                    "default": 10,
                    "minimum": 0
                },
                "output_mode": {
                    "type": "string",
                    "enum": ["content", "files_with_matches"],
                    "default": "content"
                }
            }
        });

        enforce_strict_schema(&mut fixture, true);

        let actual = fixture;
        let expected = json!({
            "type": "object",
            "properties": {
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of records",
                    "default": 10,
                    "minimum": 0
                },
                "output_mode": {
                    "type": "string",
                    "enum": ["content", "files_with_matches"],
                    "default": "content"
                }
            },
            "additionalProperties": false,
            "required": ["limit", "output_mode"]
        });
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_typeless_property_gets_string_type_in_strict_mode() {
        // MCP tool schemas from external servers (e.g. Affine) may define properties
        // with only a description and no type key. The OpenAI/Codex endpoint rejects
        // such schemas with "schema must have a 'type' key". This test verifies that
        // enforce_strict_schema defaults typeless leaf properties to "string".
        let mut schema = json!({
            "type": "object",
            "properties": {
                "content": {
                    "description": "The content of the comment"
                },
                "author": {
                    "description": "The author name"
                }
            }
        });

        enforce_strict_schema(&mut schema, true);

        let actual = schema.clone();
        let expected = json!({
            "type": "object",
            "properties": {
                "content": {
                    "description": "The content of the comment",
                    "type": "string"
                },
                "author": {
                    "description": "The author name",
                    "type": "string"
                }
            },
            "additionalProperties": false,
            "required": ["author", "content"]
        });

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_typeless_property_not_modified_in_non_strict_mode() {
        // In non-strict mode, typeless properties should not be modified.
        let mut schema = json!({
            "type": "object",
            "properties": {
                "content": {
                    "description": "The content of the comment"
                }
            }
        });

        enforce_strict_schema(&mut schema, false);

        // In non-strict mode, no type should be injected
        assert_eq!(schema["properties"]["content"]["type"], json!(null));
        assert_eq!(
            schema["properties"]["content"]["description"],
            json!("The content of the comment")
        );
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

    // === sanitize_gemini_schema tests ===

    #[test]
    fn test_gemini_strips_dollar_schema() {
        let mut schema = json!({
            "$schema": "http://json-schema.org/draft-07/schema#",
            "type": "object",
            "properties": {
                "name": { "type": "string" }
            }
        });

        sanitize_gemini_schema(&mut schema);

        assert!(!schema.as_object().unwrap().contains_key("$schema"));
    }

    #[test]
    fn test_gemini_removes_additional_properties() {
        let mut schema = json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" }
            },
            "additionalProperties": false
        });

        sanitize_gemini_schema(&mut schema);

        assert!(
            !schema
                .as_object()
                .unwrap()
                .contains_key("additionalProperties")
        );
    }

    #[test]
    fn test_gemini_removes_nested_additional_properties() {
        let mut schema = json!({
            "type": "object",
            "properties": {
                "metadata": {
                    "type": "object",
                    "additionalProperties": {
                        "type": "string"
                    }
                }
            }
        });

        sanitize_gemini_schema(&mut schema);

        let metadata = &schema["properties"]["metadata"];
        assert!(
            !metadata
                .as_object()
                .unwrap()
                .contains_key("additionalProperties")
        );
    }

    #[test]
    fn test_gemini_converts_integer_enum_to_string() {
        let mut schema = json!({
            "type": "object",
            "properties": {
                "priority": {
                    "type": "integer",
                    "enum": [1, 2, 3]
                }
            }
        });

        sanitize_gemini_schema(&mut schema);

        let priority = &schema["properties"]["priority"];
        assert_eq!(priority["type"], "string");
        assert_eq!(priority["enum"], json!(["1", "2", "3"]));
    }

    #[test]
    fn test_gemini_converts_number_enum_to_string() {
        let mut schema = json!({
            "type": "object",
            "properties": {
                "rate": {
                    "type": "number",
                    "enum": [1.5, 2.5, 3.5]
                }
            }
        });

        sanitize_gemini_schema(&mut schema);

        let rate = &schema["properties"]["rate"];
        assert_eq!(rate["type"], "string");
        assert_eq!(rate["enum"], json!(["1.5", "2.5", "3.5"]));
    }

    #[test]
    fn test_gemini_preserves_string_enum() {
        let mut schema = json!({
            "type": "object",
            "properties": {
                "mode": {
                    "type": "string",
                    "enum": ["fast", "slow"]
                }
            }
        });

        sanitize_gemini_schema(&mut schema);

        assert_eq!(schema["properties"]["mode"]["type"], "string");
        assert_eq!(
            schema["properties"]["mode"]["enum"],
            json!(["fast", "slow"])
        );
    }

    #[test]
    fn test_gemini_adds_items_to_array_without_items() {
        let mut schema = json!({
            "type": "object",
            "properties": {
                "tags": {
                    "type": "array"
                }
            }
        });

        sanitize_gemini_schema(&mut schema);

        assert_eq!(schema["properties"]["tags"]["items"]["type"], "string");
    }

    #[test]
    fn test_gemini_adds_type_to_empty_items() {
        let mut schema = json!({
            "type": "object",
            "properties": {
                "items": {
                    "type": "array",
                    "items": {}
                }
            }
        });

        sanitize_gemini_schema(&mut schema);

        // Empty items should get type: "string"
        assert_eq!(schema["properties"]["items"]["items"]["type"], "string");
    }

    #[test]
    fn test_gemini_preserves_items_with_schema_intent() {
        let mut schema = json!({
            "type": "object",
            "properties": {
                "items": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "name": { "type": "string" }
                        }
                    }
                }
            }
        });

        sanitize_gemini_schema(&mut schema);

        // Items should still have its own type, not replaced with "string"
        assert_eq!(schema["properties"]["items"]["items"]["type"], "object");
    }

    #[test]
    fn test_gemini_removes_properties_from_non_object_types() {
        let mut schema = json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "properties": {
                        "invalid": { "type": "string" }
                    },
                    "required": ["invalid"]
                }
            }
        });

        sanitize_gemini_schema(&mut schema);

        let name = &schema["properties"]["name"];
        assert!(!name.as_object().unwrap().contains_key("properties"));
        assert!(!name.as_object().unwrap().contains_key("required"));
    }

    #[test]
    fn test_gemini_preserves_properties_on_object_type() {
        let mut schema = json!({
            "type": "object",
            "properties": {
                "config": {
                    "type": "object",
                    "properties": {
                        "key": { "type": "string" }
                    },
                    "required": ["key"]
                }
            }
        });

        sanitize_gemini_schema(&mut schema);

        let config = &schema["properties"]["config"];
        assert!(config.as_object().unwrap().contains_key("properties"));
        assert!(config.as_object().unwrap().contains_key("required"));
    }

    #[test]
    fn test_gemini_filters_required_to_existing_properties() {
        let mut schema = json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" },
                "age": { "type": "integer" }
            },
            "required": ["name", "age", "nonexistent"]
        });

        sanitize_gemini_schema(&mut schema);

        let required = schema["required"].as_array().unwrap();
        assert!(required.contains(&json!("name")));
        assert!(required.contains(&json!("age")));
        assert!(!required.contains(&json!("nonexistent")));
    }

    #[test]
    fn test_gemini_converts_const_to_enum() {
        let mut schema = json!({
            "type": "object",
            "properties": {
                "role": {
                    "const": "admin"
                }
            }
        });

        sanitize_gemini_schema(&mut schema);

        let role = &schema["properties"]["role"];
        assert!(!role.as_object().unwrap().contains_key("const"));
        assert_eq!(role["enum"], json!(["admin"]));
    }

    #[test]
    fn test_gemini_does_not_override_existing_enum_with_const() {
        let mut schema = json!({
            "type": "object",
            "properties": {
                "role": {
                    "const": "admin",
                    "enum": ["admin", "user"]
                }
            }
        });

        sanitize_gemini_schema(&mut schema);

        let role = &schema["properties"]["role"];
        // Existing enum should be preserved, const removed
        assert!(!role.as_object().unwrap().contains_key("const"));
        assert_eq!(role["enum"], json!(["admin", "user"]));
    }

    #[test]
    fn test_gemini_converts_nullable_type_array() {
        let mut schema = json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": ["string", "null"]
                }
            }
        });

        sanitize_gemini_schema(&mut schema);

        let name = &schema["properties"]["name"];
        assert_eq!(name["type"], "string");
        assert_eq!(name["nullable"], true);
    }

    #[test]
    fn test_gemini_array_with_anyof_does_not_get_default_items() {
        let mut schema = json!({
            "type": "object",
            "properties": {
                "values": {
                    "type": "array",
                    "anyOf": [
                        { "type": "string" }
                    ]
                }
            }
        });

        sanitize_gemini_schema(&mut schema);

        // Should NOT add items since it has anyOf
        let values = &schema["properties"]["values"];
        assert!(!values.as_object().unwrap().contains_key("items"));
    }

    #[test]
    fn test_gemini_full_complex_schema() {
        let mut schema = json!({
            "$schema": "http://json-schema.org/draft-07/schema#",
            "type": "object",
            "properties": {
                "priority": {
                    "type": "integer",
                    "enum": [1, 2, 3]
                },
                "tags": {
                    "type": "array"
                },
                "name": {
                    "type": "string",
                    "properties": {
                        "invalid": { "type": "string" }
                    }
                },
                "status": {
                    "const": "active"
                },
                "metadata": {
                    "type": "object",
                    "additionalProperties": {
                        "type": "string"
                    }
                }
            },
            "required": ["priority", "tags", "nonexistent_field"],
            "additionalProperties": false
        });

        sanitize_gemini_schema(&mut schema);

        // $schema removed
        assert!(!schema.as_object().unwrap().contains_key("$schema"));

        // additionalProperties removed at all levels
        assert!(
            !schema
                .as_object()
                .unwrap()
                .contains_key("additionalProperties")
        );
        // metadata's additionalProperties also removed
        assert!(
            !schema["properties"]["metadata"]
                .as_object()
                .unwrap()
                .contains_key("additionalProperties")
        );

        // integer enum converted to string
        assert_eq!(schema["properties"]["priority"]["type"], "string");
        assert_eq!(
            schema["properties"]["priority"]["enum"],
            json!(["1", "2", "3"])
        );

        // array without items gets default items
        assert_eq!(schema["properties"]["tags"]["items"]["type"], "string");

        // properties removed from non-object type (string)
        assert!(
            !schema["properties"]["name"]
                .as_object()
                .unwrap()
                .contains_key("properties")
        );

        // const converted to enum
        assert!(
            !schema["properties"]["status"]
                .as_object()
                .unwrap()
                .contains_key("const")
        );
        assert_eq!(schema["properties"]["status"]["enum"], json!(["active"]));

        // required filtered to only existing properties
        let required = schema["required"].as_array().unwrap();
        assert!(required.contains(&json!("priority")));
        assert!(required.contains(&json!("tags")));
        assert!(!required.contains(&json!("nonexistent_field")));
    }

    #[test]
    fn test_gemini_nested_integer_enum_in_array_items() {
        let mut schema = json!({
            "type": "object",
            "properties": {
                "items": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "level": {
                                "type": "integer",
                                "enum": [1, 2, 3]
                            }
                        }
                    }
                }
            }
        });

        sanitize_gemini_schema(&mut schema);

        let level = &schema["properties"]["items"]["items"]["properties"]["level"];
        assert_eq!(level["type"], "string");
        assert_eq!(level["enum"], json!(["1", "2", "3"]));
    }

    #[test]
    fn test_gemini_converts_multi_type_array_with_null() {
        // Should become: anyOf: [{type: string}, {type: number}], nullable: true
        let mut schema = json!({
            "type": "object",
            "properties": {
                "multiTypeField": {
                    "type": ["string", "number", "null"]
                }
            }
        });

        sanitize_gemini_schema(&mut schema);

        let field = &schema["properties"]["multiTypeField"];
        assert!(field.as_object().unwrap().contains_key("anyOf"));
        assert_eq!(field["nullable"], true);
        let any_of = field["anyOf"].as_array().unwrap();
        assert_eq!(any_of.len(), 2);
        assert_eq!(any_of[0]["type"], "string");
        assert_eq!(any_of[1]["type"], "number");
    }

    #[test]
    fn test_gemini_converts_multi_type_array_without_null() {
        // Should become: anyOf: [{type: string}, {type: number}]
        let mut schema = json!({
            "type": "object",
            "properties": {
                "multiTypeField": {
                    "type": ["string", "number"]
                }
            }
        });

        sanitize_gemini_schema(&mut schema);

        let field = &schema["properties"]["multiTypeField"];
        assert!(field.as_object().unwrap().contains_key("anyOf"));
        assert!(!field.as_object().unwrap().contains_key("nullable"));
        let any_of = field["anyOf"].as_array().unwrap();
        assert_eq!(any_of.len(), 2);
        assert_eq!(any_of[0]["type"], "string");
        assert_eq!(any_of[1]["type"], "number");
    }

    #[test]
    fn test_gemini_anyof_null_elevation_single_branch() {
        // Should become: type: string, nullable: true, enum: [a,b,c]
        let mut schema = json!({
            "type": "object",
            "properties": {
                "field": {
                    "anyOf": [
                        { "type": "string", "enum": ["a", "b", "c"] },
                        { "type": "null" }
                    ]
                }
            }
        });

        sanitize_gemini_schema(&mut schema);

        let field = &schema["properties"]["field"];
        assert_eq!(field["type"], "string");
        assert_eq!(field["nullable"], true);
        assert_eq!(field["enum"], json!(["a", "b", "c"]));
        assert!(!field.as_object().unwrap().contains_key("anyOf"));
    }

    #[test]
    fn test_gemini_anyof_null_elevation_multiple_branches() {
        // Should become: anyOf: [{...non-null...}], nullable: true
        let mut schema = json!({
            "type": "object",
            "properties": {
                "field": {
                    "anyOf": [
                        { "type": "string" },
                        { "type": "number" },
                        { "type": "null" }
                    ]
                }
            }
        });

        sanitize_gemini_schema(&mut schema);

        let field = &schema["properties"]["field"];
        assert!(field.as_object().unwrap().contains_key("anyOf"));
        assert_eq!(field["nullable"], true);
        let any_of = field["anyOf"].as_array().unwrap();
        assert_eq!(any_of.len(), 2);
        assert_eq!(any_of[0]["type"], "string");
        assert_eq!(any_of[1]["type"], "number");
    }

    #[test]
    fn test_gemini_deeply_nested_const_in_anyof() {
        let mut schema = json!({
            "type": "object",
            "properties": {
                "nested": {
                    "type": "object",
                    "properties": {
                        "deeplyNested": {
                            "anyOf": [
                                {
                                    "type": "object",
                                    "properties": {
                                        "value": { "const": "specific value" }
                                    }
                                },
                                { "type": "string" }
                            ]
                        }
                    }
                }
            }
        });

        sanitize_gemini_schema(&mut schema);

        let deep_value = &schema["properties"]["nested"]["properties"]["deeplyNested"];
        // The anyOf with null should be preserved, and const converted to enum
        let first_branch = &deep_value["anyOf"][0];
        assert!(!first_branch.as_object().unwrap().contains_key("const"));
        assert_eq!(
            first_branch["properties"]["value"]["enum"],
            json!(["specific value"])
        );
    }

    #[test]
    fn test_gemini_preserves_description_and_format() {
        let mut schema = json!({
            "type": "object",
            "description": "A user object",
            "properties": {
                "id": {
                    "type": "number",
                    "description": "The user ID"
                },
                "name": {
                    "type": "string",
                    "description": "The user's full name"
                },
                "email": {
                    "type": "string",
                    "format": "email",
                    "description": "The user's email address"
                }
            },
            "required": ["id", "name"]
        });

        sanitize_gemini_schema(&mut schema);

        assert_eq!(schema["description"], "A user object");
        assert_eq!(schema["properties"]["id"]["description"], "The user ID");
        assert_eq!(
            schema["properties"]["name"]["description"],
            "The user's full name"
        );
        assert_eq!(schema["properties"]["email"]["format"], "email");
        assert_eq!(
            schema["properties"]["email"]["description"],
            "The user's email address"
        );
    }

    #[test]
    fn test_gemini_sanitizes_fetch_schema_exclusive_bounds_from_csv_failure() {
        let mut fixture = json!({
            "description": "Parameters for fetching a URL.",
            "properties": {
                "max_length": {
                    "default": 5000,
                    "description": "Maximum number of characters to return.",
                    "exclusiveMaximum": 1000000,
                    "exclusiveMinimum": 0,
                    "title": "Max Length",
                    "type": "integer"
                },
                "start_index": {
                    "default": 0,
                    "description": "On return output starting at this character index.",
                    "minimum": 0,
                    "title": "Start Index",
                    "type": "integer"
                },
                "url": {
                    "description": "URL to fetch",
                    "format": "uri",
                    "minLength": 1,
                    "title": "Url",
                    "type": "string"
                }
            },
            "required": ["url"],
            "title": "Fetch",
            "type": "object"
        });

        sanitize_gemini_schema(&mut fixture);

        let actual = fixture;
        let expected = json!({
            "description": "Parameters for fetching a URL.",
            "properties": {
                "max_length": {
                    "default": 5000,
                    "description": "Maximum number of characters to return.",
                    "maximum": 1000000,
                    "minimum": 0,
                    "type": "integer"
                },
                "start_index": {
                    "default": 0,
                    "description": "On return output starting at this character index.",
                    "minimum": 0,
                    "type": "integer"
                },
                "url": {
                    "description": "URL to fetch",
                    "format": "uri",
                    "minLength": 1,
                    "type": "string"
                }
            },
            "required": ["url"],
            "type": "object"
        });
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_gemini_sanitizes_defs_refs_from_notion_schema_csv_failure() {
        let mut fixture = json!({
            "$defs": {
                "richTextRequest": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "text": {"type": "string"}
                    }
                }
            },
            "type": "object",
            "properties": {
                "comment": {
                    "$ref": "#/$defs/richTextRequest"
                },
                "children": {
                    "type": "array",
                    "items": {
                        "$ref": "#/$defs/richTextRequest"
                    }
                }
            }
        });

        sanitize_gemini_schema(&mut fixture);

        let actual = fixture;
        let expected = json!({
            "type": "object",
            "properties": {
                "comment": {},
                "children": {
                    "type": "array",
                    "items": {}
                }
            }
        });
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_gemini_adds_array_items_for_fibery_where_csv_failure() {
        let mut fixture = json!({
            "type": "object",
            "properties": {
                "q_where": {
                    "description": "Filter conditions",
                    "type": "array"
                }
            },
            "required": ["q_where"]
        });

        sanitize_gemini_schema(&mut fixture);

        let actual = fixture;
        let expected = json!({
            "type": "object",
            "properties": {
                "q_where": {
                    "description": "Filter conditions",
                    "type": "array",
                    "items": {"type": "string"}
                }
            },
            "required": ["q_where"]
        });
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_gemini_nested_const_in_anyof_complex() {
        let mut schema = json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" },
                "age": { "type": "number" },
                "contact": {
                    "anyOf": [
                        {
                            "type": "object",
                            "properties": {
                                "type": { "type": "string", "const": "email" },
                                "value": { "type": "string" }
                            },
                            "required": ["type", "value"],
                            "additionalProperties": false
                        },
                        {
                            "type": "object",
                            "properties": {
                                "type": { "type": "string", "const": "phone" },
                                "value": { "type": "string" }
                            },
                            "required": ["type", "value"],
                            "additionalProperties": false
                        }
                    ]
                }
            },
            "required": ["name", "age", "contact"],
            "additionalProperties": false,
            "$schema": "http://json-schema.org/draft-07/schema#"
        });

        sanitize_gemini_schema(&mut schema);

        // $schema removed
        assert!(!schema.as_object().unwrap().contains_key("$schema"));
        // Root additionalProperties removed
        assert!(
            !schema
                .as_object()
                .unwrap()
                .contains_key("additionalProperties")
        );
        // const converted to enum inside anyOf
        let contact = &schema["properties"]["contact"];
        assert!(contact.as_object().unwrap().contains_key("anyOf"));
        // anyOf branch additionalProperties removed
        let first_branch = &contact["anyOf"][0];
        assert!(
            !first_branch
                .as_object()
                .unwrap()
                .contains_key("additionalProperties")
        );
        // const in anyOf branches converted to enum
        assert!(
            !first_branch["properties"]["type"]
                .as_object()
                .unwrap()
                .contains_key("const")
        );
        assert_eq!(first_branch["properties"]["type"]["enum"], json!(["email"]));
    }

    #[test]
    fn test_gemini_empty_object_preserved_when_nested() {
        let mut schema = json!({
            "type": "object",
            "properties": {
                "url": { "type": "string", "description": "URL to navigate to" },
                "launchOptions": {
                    "type": "object",
                    "description": "PuppeteerJS LaunchOptions"
                },
                "allowDangerous": {
                    "type": "boolean",
                    "description": "Allow dangerous options"
                }
            },
            "required": ["url", "launchOptions"]
        });

        sanitize_gemini_schema(&mut schema);

        let launch_options = &schema["properties"]["launchOptions"];
        assert_eq!(launch_options["type"], "object");
        assert_eq!(launch_options["description"], "PuppeteerJS LaunchOptions");
    }

    #[test]
    fn test_gemini_removes_required_from_non_object_types() {
        let mut schema = json!({
            "type": "object",
            "properties": {
                "data": {
                    "type": "array",
                    "items": { "type": "string" },
                    "required": ["invalid"]
                }
            }
        });

        sanitize_gemini_schema(&mut schema);

        let data = &schema["properties"]["data"];
        assert!(!data.as_object().unwrap().contains_key("required"));
    }

    #[test]
    fn test_gemini_nested_non_object_removal() {
        let mut schema = json!({
            "type": "object",
            "properties": {
                "outer": {
                    "type": "object",
                    "properties": {
                        "inner": {
                            "type": "number",
                            "properties": { "bad": { "type": "string" } },
                            "required": ["bad"]
                        }
                    }
                }
            }
        });

        sanitize_gemini_schema(&mut schema);

        let inner = &schema["properties"]["outer"]["properties"]["inner"];
        assert_eq!(inner["type"], "number");
        assert!(!inner.as_object().unwrap().contains_key("properties"));
        assert!(!inner.as_object().unwrap().contains_key("required"));
    }

    #[test]
    fn test_gemini_2d_array_empty_inner_items() {
        let mut schema = json!({
            "type": "object",
            "properties": {
                "values": {
                    "type": "array",
                    "items": {
                        "type": "array",
                        "items": {}
                    }
                }
            }
        });

        sanitize_gemini_schema(&mut schema);

        // Inner items should get default type: "string"
        assert_eq!(
            schema["properties"]["values"]["items"]["items"]["type"],
            "string"
        );
    }

    #[test]
    fn test_gemini_2d_array_missing_inner_items() {
        let mut schema = json!({
            "type": "object",
            "properties": {
                "data": {
                    "type": "array",
                    "items": {
                        "type": "array"
                    }
                }
            }
        });

        sanitize_gemini_schema(&mut schema);

        // Inner array should get items with default type
        assert_eq!(
            schema["properties"]["data"]["items"]["items"]["type"],
            "string"
        );
    }

    #[test]
    fn test_gemini_3d_nested_arrays() {
        let mut schema = json!({
            "type": "object",
            "properties": {
                "matrix": {
                    "type": "array",
                    "items": {
                        "type": "array",
                        "items": {
                            "type": "array"
                        }
                    }
                }
            }
        });

        sanitize_gemini_schema(&mut schema);

        // Deepest array should get items with default type
        assert_eq!(
            schema["properties"]["matrix"]["items"]["items"]["items"]["type"],
            "string"
        );
    }

    #[test]
    fn test_gemini_nested_array_preserves_existing_item_types() {
        let mut schema = json!({
            "type": "object",
            "properties": {
                "numbers": {
                    "type": "array",
                    "items": {
                        "type": "array",
                        "items": { "type": "number" }
                    }
                }
            }
        });

        sanitize_gemini_schema(&mut schema);

        // Should preserve the explicit type
        assert_eq!(
            schema["properties"]["numbers"]["items"]["items"]["type"],
            "number"
        );
    }

    #[test]
    fn test_gemini_mixed_nested_structures() {
        let mut schema = json!({
            "type": "object",
            "properties": {
                "spreadsheetData": {
                    "type": "object",
                    "properties": {
                        "rows": {
                            "type": "array",
                            "items": {
                                "type": "array",
                                "items": {}
                            }
                        }
                    }
                }
            }
        });

        sanitize_gemini_schema(&mut schema);

        assert_eq!(
            schema["properties"]["spreadsheetData"]["properties"]["rows"]["items"]["items"]["type"],
            "string"
        );
    }

    #[test]
    fn test_gemini_combiner_nodes_no_sibling_type_or_items() {
        // sibling type or items added during sanitize
        let mut schema = json!({
            "type": "object",
            "properties": {
                "edits": {
                    "type": "array",
                    "items": {
                        "anyOf": [
                            {
                                "type": "object",
                                "properties": {
                                    "old_string": { "type": "string" },
                                    "new_string": { "type": "string" }
                                },
                                "required": ["old_string", "new_string"]
                            },
                            {
                                "type": "object",
                                "properties": {
                                    "old_string": { "type": "string" },
                                    "new_string": { "type": "string" },
                                    "replace_all": { "type": "boolean" }
                                },
                                "required": ["old_string", "new_string"]
                            }
                        ]
                    }
                }
            },
            "required": ["edits"]
        });

        sanitize_gemini_schema(&mut schema);

        let edits = &schema["properties"]["edits"]["items"];
        // Items with anyOf should NOT have a type added
        assert!(!edits.as_object().unwrap().contains_key("type"));
        // The anyOf should still be present
        assert!(edits.as_object().unwrap().contains_key("anyOf"));
    }

    #[test]
    fn test_gemini_combiner_nodes_no_extra_keys() {
        // during sanitize beyond what was originally there
        let mut schema = json!({
            "type": "object",
            "properties": {
                "value": {
                    "oneOf": [{ "type": "string" }, { "type": "boolean" }]
                },
                "meta": {
                    "allOf": [
                        { "type": "object", "properties": { "a": { "type": "string" } } },
                        { "type": "object", "properties": { "b": { "type": "string" } } }
                    ]
                }
            }
        });

        sanitize_gemini_schema(&mut schema);

        let value = &schema["properties"]["value"];
        // oneOf should not have extra type or items added
        assert!(!value.as_object().unwrap().contains_key("type"));
        assert!(!value.as_object().unwrap().contains_key("items"));
        assert!(value.as_object().unwrap().contains_key("oneOf"));
    }

    #[test]
    fn test_gemini_nested_objects_and_arrays() {
        let mut schema = json!({
            "type": "object",
            "properties": {
                "users": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "id": { "type": "number" },
                            "name": { "type": "string" }
                        },
                        "additionalProperties": false
                    }
                }
            },
            "additionalProperties": false
        });

        sanitize_gemini_schema(&mut schema);

        // Root additionalProperties removed
        assert!(
            !schema
                .as_object()
                .unwrap()
                .contains_key("additionalProperties")
        );
        // Nested additionalProperties in items removed
        let items = &schema["properties"]["users"]["items"];
        assert!(
            !items
                .as_object()
                .unwrap()
                .contains_key("additionalProperties")
        );
        // But properties should be preserved
        assert!(items["properties"]["id"]["type"] == "number");
        assert!(items["properties"]["name"]["type"] == "string");
    }

    #[test]
    fn test_gemini_explicit_null_type() {
        let mut schema = json!({
            "type": "object",
            "properties": {
                "nullableField": {
                    "type": ["string", "null"]
                },
                "explicitNullField": {
                    "type": "null"
                }
            }
        });

        sanitize_gemini_schema(&mut schema);

        // nullableField: ["string", "null"] -> type: "string", nullable: true
        assert_eq!(schema["properties"]["nullableField"]["type"], "string");
        assert_eq!(schema["properties"]["nullableField"]["nullable"], true);
        // explicitNullField: type "null" should stay as-is
        assert_eq!(schema["properties"]["explicitNullField"]["type"], "null");
    }

    #[test]
    fn test_gemini_required_filter_on_nested_objects() {
        // Test that required filtering works recursively on nested objects
        let mut schema = json!({
            "type": "object",
            "properties": {
                "outer": {
                    "type": "object",
                    "properties": {
                        "valid": { "type": "string" }
                    },
                    "required": ["valid", "nonexistent"]
                }
            }
        });

        sanitize_gemini_schema(&mut schema);

        let outer = &schema["properties"]["outer"];
        let required = outer["required"].as_array().unwrap();
        assert!(required.contains(&json!("valid")));
        assert!(!required.contains(&json!("nonexistent")));
    }

    #[test]
    fn test_gemini_string_enum_preserved() {
        let mut schema = json!({
            "type": "object",
            "properties": {
                "kind": {
                    "type": "string",
                    "enum": ["text", "code", "image"]
                }
            },
            "required": ["kind"],
            "additionalProperties": false,
            "$schema": "http://json-schema.org/draft-07/schema#"
        });

        sanitize_gemini_schema(&mut schema);

        // $schema removed, additionalProperties removed
        assert!(!schema.as_object().unwrap().contains_key("$schema"));
        assert!(
            !schema
                .as_object()
                .unwrap()
                .contains_key("additionalProperties")
        );
        // String enum preserved
        assert_eq!(schema["properties"]["kind"]["type"], "string");
        assert_eq!(
            schema["properties"]["kind"]["enum"],
            json!(["text", "code", "image"])
        );
    }

    #[test]
    fn test_gemini_non_empty_object_preserved() {
        let mut schema = json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" }
            }
        });

        sanitize_gemini_schema(&mut schema);

        assert_eq!(schema["type"], "object");
        assert_eq!(schema["properties"]["name"]["type"], "string");
    }
}
