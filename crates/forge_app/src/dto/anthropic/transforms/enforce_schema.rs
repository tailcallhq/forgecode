use forge_domain::Transformer;

use crate::dto::anthropic::{OutputFormat, Request};
use crate::utils::enforce_strict_schema;

/// Transformer that normalizes output_format schema to meet Anthropic API
/// requirements.
///
/// Anthropic requires that all object types in JSON schemas must explicitly set
/// `additionalProperties: false`. This transformer recursively processes the
/// schema to add this requirement.
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
                // Use non-strict mode (false) for Anthropic - only adds
                // additionalProperties
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

        request
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use schemars::JsonSchema;
    use serde::Deserialize;

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
}
