use forge_domain::Transformer;

use crate::dto::openai::{Request, ResponseFormat};
use crate::utils::enforce_strict_schema;

/// Normalizes tool schemas for OpenAI compatibility
/// Remove duplicate title and description from parameters.
pub struct NormalizeToolSchema;

/// Enforces strict JSON schema compatibility for tool parameters.
///
/// This is primarily used for OpenAI-compatible providers that require
/// nullable enum values to be rewritten to OpenAI-compatible strict schema.
pub struct EnforceStrictToolSchema;

/// Enforces strict JSON schema compatibility for response_format schemas.
///
/// This is used for OpenAI-compatible providers that require the same strict
/// JSON Schema subset for structured outputs as they do for tool parameters.
pub struct EnforceStrictResponseFormatSchema;

impl Transformer for NormalizeToolSchema {
    type Value = Request;

    fn transform(&mut self, mut request: Self::Value) -> Self::Value {
        if let Some(tools) = request.tools.as_mut() {
            for tool in tools.iter_mut() {
                if let Some(obj) = tool.function.parameters.as_object_mut() {
                    // Remove tool usage description and title from parameters property
                    obj.remove("description");
                    obj.remove("title");
                }
            }
        }
        request
    }
}

impl Transformer for EnforceStrictToolSchema {
    type Value = Request;

    fn transform(&mut self, mut request: Self::Value) -> Self::Value {
        if let Some(tools) = request.tools.as_mut() {
            for tool in tools.iter_mut() {
                enforce_strict_schema(&mut tool.function.parameters, true);
            }
        }
        request
    }
}

impl Transformer for EnforceStrictResponseFormatSchema {
    type Value = Request;

    fn transform(&mut self, mut request: Self::Value) -> Self::Value {
        let Some(response_format) = request.response_format.take() else {
            return request;
        };

        match response_format {
            ResponseFormat::Text => {
                request.response_format = Some(ResponseFormat::Text);
            }
            ResponseFormat::JsonSchema { name, schema } => {
                if let Ok(mut schema_value) = serde_json::to_value(&schema) {
                    enforce_strict_schema(&mut schema_value, true);

                    if let Ok(normalized_schema) = serde_json::from_value(schema_value) {
                        request.response_format = Some(ResponseFormat::JsonSchema {
                            name,
                            schema: Box::new(normalized_schema),
                        });
                    } else {
                        request.response_format = Some(ResponseFormat::JsonSchema { name, schema });
                    }
                } else {
                    request.response_format = Some(ResponseFormat::JsonSchema { name, schema });
                }
            }
        }

        request
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use serde_json::json;

    use super::*;
    use crate::dto::openai::{FunctionDescription, FunctionType, Tool};

    fn tool_fixture(parameters: serde_json::Value) -> Tool {
        Tool {
            r#type: FunctionType,
            function: FunctionDescription {
                name: "test_tool".to_string(),
                description: Some("Test tool description".to_string()),
                parameters,
            },
        }
    }

    #[test]
    fn test_normalize_removes_description_and_title() {
        let fixture = Request::default().tools(vec![tool_fixture(json!({
            "type": "object",
            "description": "Schema description",
            "title": "Schema title",
            "properties": {
                "param1": {"type": "string"}
            }
        }))]);

        let actual = NormalizeToolSchema.transform(fixture);

        let expected = json!({
            "type": "object",
            "properties": {
                "param1": {"type": "string"}
            }
        });

        assert_eq!(actual.tools.unwrap()[0].function.parameters, expected);
    }

    #[test]
    fn test_normalize_already_normalized() {
        let fixture = Request::default().tools(vec![tool_fixture(json!({
            "type": "object",
            "properties": {
                "param1": {"type": "string"}
            }
        }))]);

        let actual = NormalizeToolSchema.transform(fixture);

        let expected = json!({
            "type": "object",
            "properties": {
                "param1": {"type": "string"}
            }
        });

        assert_eq!(actual.tools.unwrap()[0].function.parameters, expected);
    }

    #[test]
    fn test_enforce_strict_converts_nullable_enum() {
        let fixture = Request::default().tools(vec![tool_fixture(json!({
            "type": "object",
            "properties": {
                "output_mode": {
                    "description": "Output mode",
                    "nullable": true,
                    "type": "string",
                    "enum": ["content", "files_with_matches", "count", null]
                }
            }
        }))]);

        let actual = EnforceStrictToolSchema.transform(fixture);

        let expected = json!({
            "type": "object",
            "properties": {
                "output_mode": {
                    "description": "Output mode",
                    "anyOf": [
                        {"type": "string", "enum": ["content", "files_with_matches", "count"]},
                        {"type": "null"}
                    ]
                }
            },
            "additionalProperties": false,
            "required": ["output_mode"]
        });

        assert_eq!(actual.tools.unwrap()[0].function.parameters, expected);
    }

    #[test]
    fn test_enforce_strict_response_format_converts_nullable_enum() {
        let fixture = Request::default().response_format(ResponseFormat::JsonSchema {
            name: "test_response".to_string(),
            schema: Box::new(
                schemars::Schema::try_from(json!({
                    "type": "object",
                    "properties": {
                        "result": {
                            "description": "Result",
                            "nullable": true,
                            "type": "string",
                            "enum": ["done", null]
                        }
                    }
                }))
                .unwrap(),
            ),
        });

        let actual = EnforceStrictResponseFormatSchema.transform(fixture);

        let actual_schema = match actual.response_format {
            Some(ResponseFormat::JsonSchema { schema, .. }) => {
                serde_json::to_value(schema).unwrap()
            }
            Some(ResponseFormat::Text) => panic!("Expected json_schema response format"),
            None => panic!("Expected response format to be preserved"),
        };
        let expected = json!({
            "type": "object",
            "properties": {
                "result": {
                    "description": "Result",
                    "anyOf": [
                        {"type": "string", "enum": ["done"]},
                        {"type": "null"}
                    ]
                }
            },
            "additionalProperties": false,
            "required": ["result"]
        });

        assert_eq!(actual_schema, expected);
    }
}
