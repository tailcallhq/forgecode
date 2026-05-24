use derive_setters::Setters;
use schemars::Schema;
use schemars::generate::SchemaGenerator;
use schemars::transform::{Transform, transform_subschemas};
use serde::{Deserialize, Serialize};

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
}
