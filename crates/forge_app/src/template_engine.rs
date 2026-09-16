use std::sync::LazyLock;

use forge_domain::Template;
use handlebars::{Handlebars, no_escape};
use include_dir::{Dir, include_dir};

static TEMPLATE_DIR: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/../../templates");

/// Creates a new Handlebars instance with all custom helpers registered.
///
/// This function configures a Handlebars instance with:
/// - The 'inc' helper for incrementing values (useful for 1-based indexing)
/// - The 'json' helper for serializing values to JSON strings
/// - The 'contains' helper for checking if an array contains a value
/// - Strict mode enabled
/// - No HTML escaping
/// - All embedded templates registered
///
/// This is useful for creating standalone Handlebars instances with consistent
/// configuration across the application.
fn create_handlebar() -> Handlebars<'static> {
    let mut hb = Handlebars::new();
    hb.set_strict_mode(true);
    hb.register_escape_fn(no_escape);

    // Register the 'inc' helper to increment index for 1-based numbering
    hb.register_helper(
        "inc",
        Box::new(
            |h: &handlebars::Helper,
             _: &handlebars::Handlebars,
             _: &handlebars::Context,
             _: &mut handlebars::RenderContext,
             out: &mut dyn handlebars::Output|
             -> handlebars::HelperResult {
                let value = h.param(0).and_then(|v| v.value().as_u64()).ok_or_else(|| {
                    handlebars::RenderErrorReason::ParamNotFoundForIndex("inc", 0)
                })?;
                out.write(&(value + 1).to_string())?;
                Ok(())
            },
        ),
    );

    // Register the 'json' helper to serialize context as JSON string
    hb.register_helper(
        "json",
        Box::new(
            |h: &handlebars::Helper,
             _: &handlebars::Handlebars,
             _: &handlebars::Context,
             _: &mut handlebars::RenderContext,
             out: &mut dyn handlebars::Output|
             -> handlebars::HelperResult {
                let value = h.param(0).ok_or_else(|| {
                    handlebars::RenderErrorReason::ParamNotFoundForIndex("json", 0)
                })?;
                let json_string = serde_json::to_string(value.value())
                    .map_err(|e| handlebars::RenderErrorReason::NestedError(Box::new(e)))?;
                out.write(&json_string)?;
                Ok(())
            },
        ),
    );

    // Register the 'contains' helper to check if array contains a value
    // This is used with #if blocks: {{#if (contains array "value")}}
    hb.register_helper(
        "contains",
        Box::new(
            |h: &handlebars::Helper,
             _r: &handlebars::Handlebars,
             _ctx: &handlebars::Context,
             _rc: &mut handlebars::RenderContext,
             out: &mut dyn handlebars::Output|
             -> handlebars::HelperResult {
                let array = h.param(0).ok_or_else(|| {
                    handlebars::RenderErrorReason::ParamNotFoundForIndex("contains", 0)
                })?;
                let search_value = h.param(1).ok_or_else(|| {
                    handlebars::RenderErrorReason::ParamNotFoundForIndex("contains", 1)
                })?;

                // Check if the array contains the value
                let contains = if let Some(arr) = array.value().as_array() {
                    arr.iter().any(|v| v == search_value.value())
                } else {
                    false
                };

                // Write "true" or empty string for handlebars to interpret as
                // boolean
                if contains {
                    out.write("true")?;
                }

                Ok(())
            },
        ),
    );

    // Register all embedded templates from the templates directory
    forge_embed::register_templates(&mut hb, &TEMPLATE_DIR);

    hb
}

/// Global template engine instance with all custom helpers and templates
/// registered.
///
/// This static instance is lazily initialized on first access and provides:
/// - The 'inc' helper for incrementing values (useful for 1-based indexing)
/// - The 'json' helper for serializing values to JSON strings
/// - The 'contains' helper for checking if an array contains a value
/// - Strict mode enabled
/// - No HTML escaping
/// - All embedded templates registered
///
/// Use this instance for template rendering throughout the application to avoid
/// creating multiple Handlebars instances.
static HANDLEBARS: LazyLock<Handlebars<'static>> = LazyLock::new(create_handlebar);

/// A wrapper around the Handlebars template engine providing a simplified API.
///
/// This struct provides a clean interface for template rendering using the
/// `Template` type from the domain layer.
pub struct TemplateEngine<'a> {
    handlebar: Handlebars<'a>,
}

impl Default for TemplateEngine<'_> {
    fn default() -> Self {
        Self { handlebar: HANDLEBARS.clone() }
    }
}

impl<'a> TemplateEngine<'a> {
    /// Renders a template with the provided data.
    pub fn render<V: serde::Serialize>(
        &self,
        template: impl Into<Template<V>>,
        data: &V,
    ) -> anyhow::Result<String> {
        let template = template.into();
        Ok(self.handlebar.render(&template.template, data)?)
    }

    /// Renders a template with the provided data.
    pub fn render_template<V: serde::Serialize>(
        &self,
        template: impl Into<Template<V>>,
        data: &V,
    ) -> anyhow::Result<String> {
        let template = template.into();
        Ok(self.handlebar.render_template(&template.template, data)?)
    }

    pub fn handlebar_instance() -> Handlebars<'static> {
        create_handlebar()
    }
}

#[cfg(test)]
mod tests {
    use serde::Serialize;
    use serde_json::json;

    use super::*;

    #[derive(Serialize)]
    struct TestData {
        items: Vec<String>,
        numbers: Vec<i32>,
    }

    #[test]
    fn test_contains_helper_with_string_array() {
        let hb = create_handlebar();
        let template = r#"{{#if (contains items "apple")}}found{{else}}not found{{/if}}"#;

        let fixture = TestData {
            items: vec![
                "apple".to_string(),
                "banana".to_string(),
                "cherry".to_string(),
            ],
            numbers: vec![],
        };

        let actual = hb.render_template(template, &fixture).unwrap();
        let expected = "found";

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_contains_helper_with_string_array_not_found() {
        let hb = create_handlebar();
        let template = r#"{{#if (contains items "orange")}}found{{else}}not found{{/if}}"#;

        let fixture = TestData {
            items: vec![
                "apple".to_string(),
                "banana".to_string(),
                "cherry".to_string(),
            ],
            numbers: vec![],
        };

        let actual = hb.render_template(template, &fixture).unwrap();
        let expected = "not found";

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_contains_helper_with_number_array() {
        let hb = create_handlebar();
        let template = r#"{{#if (contains numbers 42)}}found{{else}}not found{{/if}}"#;

        let fixture = TestData { items: vec![], numbers: vec![10, 20, 42, 50] };

        let actual = hb.render_template(template, &fixture).unwrap();
        let expected = "found";

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_contains_helper_with_number_array_not_found() {
        let hb = create_handlebar();
        let template = r#"{{#if (contains numbers 99)}}found{{else}}not found{{/if}}"#;

        let fixture = TestData { items: vec![], numbers: vec![10, 20, 42, 50] };

        let actual = hb.render_template(template, &fixture).unwrap();
        let expected = "not found";

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_contains_helper_with_empty_array() {
        let hb = create_handlebar();
        let template = r#"{{#if (contains items "apple")}}found{{else}}not found{{/if}}"#;

        let fixture = TestData { items: vec![], numbers: vec![] };

        let actual = hb.render_template(template, &fixture).unwrap();
        let expected = "not found";

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_contains_helper_with_json_value() {
        let hb = create_handlebar();
        let template = r#"{{#if (contains tags "rust")}}yes{{else}}no{{/if}}"#;

        let fixture = json!({
            "tags": ["rust", "python", "javascript"]
        });

        let actual = hb.render_template(template, &fixture).unwrap();
        let expected = "yes";

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_contains_helper_multiple_conditions() {
        let hb = create_handlebar();
        let template = r#"{{#if (contains items "apple")}}A{{/if}}{{#if (contains items "banana")}}B{{/if}}{{#if (contains items "cherry")}}C{{/if}}"#;

        let fixture = TestData {
            items: vec!["apple".to_string(), "cherry".to_string()],
            numbers: vec![],
        };

        let actual = hb.render_template(template, &fixture).unwrap();
        let expected = "AC";

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_contains_helper_with_non_array_value() {
        let hb = create_handlebar();
        let template = r#"{{#if (contains name "test")}}found{{else}}not found{{/if}}"#;

        let fixture = json!({
            "name": "test-value"
        });

        let actual = hb.render_template(template, &fixture).unwrap();
        let expected = "not found";

        assert_eq!(actual, expected);
    }
}
