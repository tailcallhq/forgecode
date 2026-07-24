use derive_setters::Setters;
use schemars::Schema;
use schemars::generate::SchemaGenerator;
use schemars::transform::{Transform, transform_subschemas};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ToolName;

/// A schemars [`Transform`] that recursively removes the `title` field from
/// every schema node.
///
/// Rust type names are emitted as `title` by the `JsonSchema` derive. These
/// are internal implementation details and must not be forwarded to LLM
/// provider APIs.
#[derive(Debug, Clone, Default)]
pub struct RemoveSchemaTitles;

impl Transform for RemoveSchemaTitles {
    fn transform(&mut self, schema: &mut Schema) {
        if let Some(map) = schema.as_object_mut() {
            map.remove("title");
        }

        transform_subschemas(self, schema);
    }
}

/// A [`Transform`] that marks nullable schemas with `"nullable": true` while
/// keeping `enum` arrays internally consistent.
///
/// Schemars' built-in [`schemars::transform::AddNullable`] adds the
/// `"nullable": true` keyword and removes `"null"` from the `type` keyword,
/// but it does **not** remove `null` values from `enum` arrays. This leaves
/// schemas where the `enum` array contains `null` while `type` is e.g.
/// `"string"` — an invalid combination that strict JSON Schema validators
/// (such as Moonshot/Kimi) reject.
///
/// This transform delegates to [`AddNullable`](schemars::transform::AddNullable)
/// for the `type` keyword, then recursively strips `null` from every `enum`
/// array in a schema that carries `"nullable": true`. The `nullable` marker
/// alone fully conveys nullability in OpenAPI 3.0-style dialects, so `null`
/// in `enum` is redundant once the marker is present.
#[derive(Debug, Clone, Default)]
pub struct NormalizeNullable;

impl Transform for NormalizeNullable {
    fn transform(&mut self, schema: &mut Schema) {
        // Delegate type-keyword normalization (including subschema recursion)
        // to schemars' AddNullable.
        schemars::transform::AddNullable::default().transform(schema);

        // Recursively clean up enum arrays left inconsistent by AddNullable.
        remove_null_enum_values(schema);
        transform_subschemas(&mut remove_null_enum_values, schema);
    }
}

/// Removes `null` entries from the `enum` array of a schema that is marked
/// `nullable: true`. The `nullable` marker already conveys nullability, so
/// keeping `null` in `enum` is redundant and can make the schema invalid when
/// the `type` no longer includes `"null"`.
fn remove_null_enum_values(schema: &mut Schema) {
    if let Some(map) = schema.as_object_mut() {
        let is_nullable = map
            .get("nullable")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        if is_nullable {
            if let Some(Value::Array(enum_values)) = map.get_mut("enum") {
                enum_values.retain(|v| !v.is_null());

                // Drop the enum key if every value was null.
                if enum_values.is_empty() {
                    map.remove("enum");
                }
            }
        }
    }
}

/// Returns a [`SchemaGenerator`] whose settings include [`RemoveSchemaTitles`]
/// as a registered transform.
///
/// All schemas produced via this generator will never contain `title` fields,
/// eliminating the need for any post-hoc stripping.
pub fn tool_schema_generator() -> SchemaGenerator {
    schemars::generate::SchemaSettings::default()
        .with(|s| {
            s.transforms.push(Box::new(RemoveSchemaTitles));
        })
        .into_generator()
}

///
/// Refer to the specification over here:
/// https://glama.ai/blog/2024-11-25-model-context-protocol-quickstart
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Setters)]
#[setters(into, strip_option)]
pub struct ToolDefinition {
    pub name: ToolName,
    pub description: String,
    #[setters(skip)]
    pub input_schema: Schema,
}

impl ToolDefinition {
    /// Create a new ToolDefinition with an empty input schema.
    pub fn new<N: ToString>(name: N) -> Self {
        ToolDefinition {
            name: ToolName::new(name),
            description: String::new(),
            input_schema: tool_schema_generator().into_root_schema_for::<()>(),
        }
    }

    /// Sets the input schema.
    ///
    /// # Arguments
    /// * `input_schema` - The JSON schema describing accepted tool input
    pub fn input_schema(mut self, input_schema: impl Into<Schema>) -> Self {
        self.input_schema = input_schema.into();
        self
    }
}

pub trait ToolDescription {
    fn description(&self) -> String;
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use schemars::JsonSchema;
    use serde::Deserialize as SerdeDeserialize;

    use super::*;

    /// A struct with a Rust type name that schemars would emit as `title`.
    #[derive(SerdeDeserialize, JsonSchema)]
    #[allow(dead_code)]
    struct InternalPatchInput {
        old_string: String,
        nested: NestedInput,
    }

    #[derive(SerdeDeserialize, JsonSchema)]
    #[allow(dead_code)]
    struct NestedInput {
        value: String,
    }

    #[test]
    fn test_tool_schema_generator_strips_titles() {
        let r#gen = tool_schema_generator();
        let actual =
            serde_json::to_value(r#gen.into_root_schema_for::<InternalPatchInput>()).unwrap();

        assert_eq!(
            actual.pointer("/title"),
            None,
            "root title should be absent"
        );
        assert_eq!(
            actual.pointer("/properties/nested/title"),
            None,
            "nested title should be absent"
        );
    }

    #[test]
    fn test_tool_definition_new_has_no_title() {
        let fixture = ToolDefinition::new("patch");
        let actual = serde_json::to_value(&fixture.input_schema).unwrap();
        assert_eq!(actual.pointer("/title"), None);
    }

    #[test]
    fn test_tool_definition_round_trip_preserves_no_title() {
        let r#gen = tool_schema_generator();
        let schema = r#gen.into_root_schema_for::<InternalPatchInput>();
        let fixture = ToolDefinition::new("patch")
            .description("Patch a file")
            .input_schema(schema);

        // Serialise then deserialise and confirm no title leaks in
        let json_str = serde_json::to_string(&fixture).unwrap();
        let roundtripped: ToolDefinition = serde_json::from_str(&json_str).unwrap();
        let actual = serde_json::to_value(roundtripped.input_schema).unwrap();
        assert_eq!(actual.pointer("/title"), None);
        assert_eq!(actual.pointer("/properties/nested/title"), None);
    }

    #[test]
    fn test_tool_definition_serialization_has_no_title() {
        let r#gen = tool_schema_generator();
        let schema = r#gen.into_root_schema_for::<InternalPatchInput>();
        let fixture = ToolDefinition {
            name: ToolName::new("patch"),
            description: "Patch a file".to_string(),
            input_schema: schema,
        };
        let actual = serde_json::to_value(&fixture).unwrap();

        // Titles must be absent at every level regardless of the schema structure
        assert_eq!(actual.pointer("/input_schema/title"), None);
        assert_eq!(
            actual.pointer("/input_schema/$defs/NestedInput/title"),
            None
        );
    }

    #[test]
    fn test_normalize_nullable_strips_null_from_enum() {
        use schemars::json_schema;

        // Simulate what schemars produces for Option<EnumWithCustomSchema>:
        // type includes "null", and enum array contains null.
        let mut schema = json_schema!({
            "type": ["string", "null"],
            "enum": ["content", "files_with_matches", "count", null]
        });

        NormalizeNullable.transform(&mut schema);

        let actual = serde_json::to_value(&schema).unwrap();

        // nullable marker should be present
        assert_eq!(actual["nullable"], serde_json::Value::Bool(true));

        // type should no longer contain "null"
        assert_eq!(actual["type"], serde_json::json!("string"));

        // enum must NOT contain null — this is the bug fix
        let enum_values = actual["enum"].as_array().unwrap();
        assert!(
            !enum_values.contains(&serde_json::Value::Null),
            "enum must not contain null after NormalizeNullable"
        );
        assert_eq!(enum_values.len(), 3);
    }

    #[test]
    fn test_normalize_nullable_preserves_non_nullable_enum() {
        use schemars::json_schema;

        // A non-nullable enum should be left untouched.
        let mut schema = json_schema!({
            "type": "string",
            "enum": ["a", "b", "c"]
        });

        let expected = serde_json::to_value(&schema).unwrap();

        NormalizeNullable.transform(&mut schema);

        let actual = serde_json::to_value(&schema).unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_normalize_nullable_recurses_into_properties() {
        use schemars::json_schema;

        // Nullable enum nested inside a properties object.
        let mut schema = json_schema!({
            "type": "object",
            "properties": {
                "mode": {
                    "type": ["string", "null"],
                    "enum": ["fast", "slow", null]
                }
            }
        });

        NormalizeNullable.transform(&mut schema);

        let actual = serde_json::to_value(&schema).unwrap();
        let mode_enum = actual
            .pointer("/properties/mode/enum")
            .and_then(|v| v.as_array())
            .unwrap();

        assert!(
            !mode_enum.contains(&serde_json::Value::Null),
            "nested enum must not contain null"
        );
        assert_eq!(mode_enum.len(), 2);
    }
}
