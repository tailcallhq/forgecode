use forge_domain::Transformer;

use crate::dto::anthropic::{OutputFormat, Request};
use crate::utils::enforce_strict_schema;

/// Transformer that normalizes JSON schemas in the Anthropic request to meet
/// API requirements.
///
/// Two categories of schemas are processed:
///
/// 1. **`output_format` schema** — Anthropic requires all object types to
///    explicitly set `additionalProperties: false`.
///
/// 2. **Tool `input_schema`** — each tool's parameter schema is sanitized to
///    remove constructs that produce invalid JSON Schema, such as `null`
///    values in `enum` arrays that contradict the declared `type`. This is
///    essential when routing to Anthropic-compatible endpoints backed by
///    third-party models (e.g. Moonshot/Kimi) whose validators reject such
///    schemas.
///
/// # Example
///
/// Before normalization:
/// ```json
/// {
///   "type": "object",
///   "properties": { "name": { "type": "string" } }
/// }
/// ```
///
/// After normalization:
/// ```json
/// {
///   "type": "object",
///   "properties": { "name": { "type": "string" } },
///   "additionalProperties": false
/// }
/// ```
pub struct EnforceStrictObjectSchema;

impl Transformer for EnforceStrictObjectSchema {
    type Value = Request;

    fn transform(&mut self, mut request: Self::Value) -> Self::Value {
        if let Some(OutputFormat::JsonSchema { schema }) = request.output_format.take() {
            // Convert schema to JSON value for normalization
            if let Ok(mut schema_value) = serde_json::to_value(&schema) {
                // Use non-strict mode (false) for Anthropic - only adds additionalProperties
                enforce_strict_schema(&mut schema_value, false);

                // Convert back to RootSchema
                if let Ok(normalized_schema) = serde_json::from_value(schema_value) {
                    request.output_format =
                        Some(OutputFormat::JsonSchema { schema: normalized_schema });
                } else {
                    // If deserialization fails, keep the original schema
                    request.output_format = Some(OutputFormat::JsonSchema { schema });
                }
            } else {
                // If serialization fails, keep the original schema
                request.output_format = Some(OutputFormat::JsonSchema { schema });
            }
        }

        // Normalize each tool's input schema. Non-strict mode keeps the
        // `nullable` keyword for providers that support it while stripping
        // invalid constructs (e.g. null in enum) that strict validators reject.
        for tool in &mut request.tools {
            enforce_strict_schema(&mut tool.input_schema, false);
        }

        request
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use schemars::JsonSchema;
    use serde::Deserialize;

    use crate::dto::anthropic::ToolDefinition;

    use super::*;

    #[derive(Deserialize, JsonSchema)]
    #[schemars(title = "test_response")]
    #[allow(dead_code)]
    struct TestResponse {
        name: String,
        nested: NestedObject,
    }

    #[derive(Deserialize, JsonSchema)]
    #[allow(dead_code)]
    struct NestedObject {
        value: String,
    }

    #[test]
    fn test_normalize_output_schema_adds_additional_properties() {
        let schema = schemars::schema_for!(TestResponse);
        let fixture = Request::default().output_format(OutputFormat::JsonSchema { schema });

        let actual = EnforceStrictObjectSchema.transform(fixture);

        // Convert to JSON to check if additionalProperties was added
        if let Some(OutputFormat::JsonSchema { schema }) = actual.output_format {
            let schema_json = serde_json::to_value(&schema).unwrap();

            // Check top-level schema
            assert_eq!(
                schema_json["additionalProperties"],
                serde_json::Value::Bool(false),
                "Top-level additionalProperties should be false"
            );

            // Check nested object schema - it might be in definitions or $defs
            if let Some(nested_schema) = schema_json
                .get("properties")
                .and_then(|p| p.get("nested"))
                .and_then(|n| n.get("additionalProperties"))
            {
                assert_eq!(
                    nested_schema,
                    &serde_json::Value::Bool(false),
                    "Nested additionalProperties should be false"
                );
            } else if let Some(defs) = schema_json
                .get("$defs")
                .or_else(|| schema_json.get("definitions"))
            {
                // Check if NestedObject is in definitions
                if let Some(nested_def) = defs.get("NestedObject") {
                    assert_eq!(
                        nested_def["additionalProperties"],
                        serde_json::Value::Bool(false),
                        "NestedObject in definitions should have additionalProperties: false"
                    );
                }
            }
        } else {
            panic!("Expected output_format to be Some(OutputFormat::JsonSchema)");
        }
    }

    #[test]
    fn test_normalize_output_schema_preserves_none() {
        let fixture = Request::default();

        let actual = EnforceStrictObjectSchema.transform(fixture);

        assert_eq!(actual.output_format, None);
    }

    #[test]
    fn test_normalize_tool_schema_strips_null_from_enum() {
        // Simulate a tool input schema with a nullable enum that contains
        // null in the enum array — exactly what Moonshot/Kimi rejects.
        let fixture = Request::default().max_tokens(1u64).tools(vec![ToolDefinition {
            name: "fs_search".to_string(),
            description: Some("Search files".to_string()),
            cache_control: None,
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "output_mode": {
                        "type": "string",
                        "enum": ["content", "files_with_matches", "count", null],
                        "nullable": true
                    }
                }
            }),
        }]);

        let actual = EnforceStrictObjectSchema.transform(fixture);

        let tool = &actual.tools[0];
        let enum_values = tool
            .input_schema
            .pointer("/properties/output_mode/enum")
            .and_then(|v| v.as_array())
            .unwrap();

        assert!(
            !enum_values.contains(&serde_json::Value::Null),
            "enum must not contain null after normalization"
        );
        assert_eq!(enum_values.len(), 3);
    }
}
