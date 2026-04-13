use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;
use forge_app::{EnvironmentInfra, FileReaderInfra, TemplateService};
use forge_domain::Template;
use futures::future;
use handlebars::Handlebars;
use tokio::sync::{OnceCell, RwLock};

#[derive(Clone)]
pub struct ForgeTemplateService<F> {
    hb: Arc<OnceCell<RwLock<Handlebars<'static>>>>,
    infra: Arc<F>,
}

impl<F: EnvironmentInfra + FileReaderInfra> ForgeTemplateService<F> {
    pub fn new(infra: Arc<F>) -> Self {
        Self { hb: Arc::new(OnceCell::new()), infra }
    }

    /// Returns a reference to the lazily-initialized Handlebars RwLock,
    /// creating the instance on the first call.
    async fn get_hb(&self) -> &RwLock<Handlebars<'static>> {
        self.hb
            .get_or_init(|| async { RwLock::new(forge_app::TemplateEngine::handlebar_instance()) })
            .await
    }

    /// Reads multiple template files in parallel and returns their names and
    /// contents.
    ///
    /// Takes a list of file paths and the current working directory, then reads
    /// all files concurrently using async futures. Returns a vector of
    /// (name, content) tuples.
    async fn read_all(
        &self,
        file_paths: &[PathBuf],
        cwd: &Path,
    ) -> anyhow::Result<Vec<(String, String)>> {
        let futures = file_paths.iter().map(|template_path| async {
            let template_name = template_path
                .file_name()
                .and_then(|name| name.to_str())
                .with_context(|| format!("Invalid filename: {}", template_path.display()))?
                .to_string();
            let template_path = cwd.join(template_path.clone());
            let content = self.infra.read_utf8(&template_path).await?;
            Ok::<_, anyhow::Error>((template_name, content))
        });

        future::join_all(futures)
            .await
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
    }
}

/// Compiles a template based on the filename and content.
///
/// If the filename ends with ".md", it compiles the content as a Handlebars
/// template. Otherwise, it creates a raw string template.
fn compile_template(name: &str, content: &str) -> anyhow::Result<handlebars::template::Template> {
    if name.ends_with(".md") {
        handlebars::Template::compile(content).map_err(Into::into)
    } else {
        let mut template = handlebars::template::Template::new();
        template
            .elements
            .push(handlebars::template::TemplateElement::RawString(
                content.to_string(),
            ));
        template.name = Some(name.to_owned());
        Ok(template)
    }
}

#[async_trait::async_trait]
impl<F: EnvironmentInfra + FileReaderInfra> TemplateService for ForgeTemplateService<F> {
    async fn register_template(&self, path: PathBuf) -> anyhow::Result<()> {
        let cwd = &self.infra.get_environment().cwd;

        // Discover and filter unregistered templates in one pass
        let guard = self.get_hb().await.read().await;
        let path = if path.is_absolute() {
            path.to_string_lossy().to_string()
        } else {
            cwd.join(path).to_string_lossy().to_string()
        };
        let unregistered_files: Vec<_> = glob::glob(&format!("{path}/*"))?
            .filter_map(|entry| entry.ok())
            .filter(|p| p.is_file())
            .filter(|p| {
                p.file_name()
                    .and_then(|name| name.to_str())
                    .map(|name| guard.get_template(name).is_none())
                    .unwrap_or(true) // Keep files with invalid names for error
                // handling
            })
            .collect();
        drop(guard);

        // Read all files concurrently
        let templates = self.read_all(&unregistered_files, cwd.as_path()).await?;

        // Register all templates if any were found
        if !templates.is_empty() {
            let mut guard = self.get_hb().await.write().await;
            for (name, content) in templates {
                let template = compile_template(&name, &content)?;
                guard.register_template(&name, template);
            }
        }

        Ok(())
    }

    async fn render_template<V: serde::Serialize + Send + Sync>(
        &self,
        template: Template<V>,
        object: &V,
    ) -> anyhow::Result<String> {
        let rendered = self
            .get_hb()
            .await
            .read()
            .await
            .render_template(&template.template, object)?;
        Ok(rendered)
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use serde_json::json;

    use super::*;
    use crate::attachment::tests::MockCompositeService;

    #[tokio::test]
    async fn test_render_simple_template() {
        // Fixture: Create template service and data
        let service = ForgeTemplateService::new(Arc::new(MockCompositeService::new()));
        let data = json!({
            "name": "Forge",
            "version": "1.0",
            "features": ["templates", "rendering", "handlebars"]
        });

        // Actual: Render a simple template
        let template = "App: {{name}} v{{version}} - Features: {{#each features}}{{this}}{{#unless @last}}, {{/unless}}{{/each}}";
        let actual = service
            .render_template(Template::new(template), &data)
            .await
            .unwrap();

        // Expected: Result should match the expected string
        let expected = "App: Forge v1.0 - Features: templates, rendering, handlebars";
        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn test_render_partial_system_info() {
        // Fixture: Create template service and data
        let service = ForgeTemplateService::new(Arc::new(MockCompositeService::new()));
        let data = json!({
            "env": {
                "os": "test-os",
                "cwd": "/test/path",
                "shell": "/bin/test",
                "home": "/home/test"
            },
            "current_time": "2024-01-01 12:00:00 UTC",
            "files": [
                {"path": "file1.txt", "is_dir": false},
                {"path": "file2.txt", "is_dir": false}
            ]
        });

        // Actual: Render the partial-system-info template
        let actual = service
            .render_template(Template::new("{{> forge-partial-system-info.md }}"), &data)
            .await
            .unwrap();

        // Expected: Result should contain the rendered system info with substituted
        // values
        assert!(actual.contains("<operating_system>test-os</operating_system>"));
        assert!(actual.contains("file1.txt"));
        assert!(actual.contains("file2.txt"));
    }

    #[test]
    fn test_compile_template_hbs_file() {
        // Fixture: Create a handlebars template content and test data
        let name = "test.md";
        let content = "Hello {{name}}!";
        let test_data = json!({"name": "World"});

        // Actual: Compile the template and render it
        let template = compile_template(name, content).unwrap();
        let mut hb = Handlebars::new();
        hb.register_template("test", template);
        let actual = hb.render("test", &test_data).unwrap();

        // Expected: Should render the handlebars template with substituted values
        let expected = "Hello World!";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_compile_template_raw_file() {
        // Fixture: Create a raw template content with handlebars-like syntax
        let name = "test.txt";
        let content = "This is raw content with {{variables}} that won't be processed";
        let test_data = json!({"variables": "should not substitute"});

        // Actual: Compile the template and render it
        let template = compile_template(name, content).unwrap();
        let mut hb = Handlebars::new();
        hb.register_template("test", template);
        let actual = hb.render("test", &test_data).unwrap();

        // Expected: Should render the raw content without any substitution
        let expected = "This is raw content with {{variables}} that won't be processed";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_compile_template_invalid_hbs() {
        // Fixture: Create invalid handlebars content
        let name = "invalid.md";
        let content = "{{#if unclosed";

        // Actual: Try to compile the invalid template
        let actual = compile_template(name, content);

        // Expected: Should return an error
        assert!(actual.is_err());
        let error_msg = actual.unwrap_err().to_string();
        // The error should indicate a handlebars syntax issue
        assert!(error_msg.contains("handlebars syntax") || error_msg.contains("Template error"));
    }

    #[test]
    fn test_compile_template_empty_content() {
        // Fixture: Create empty content for both file types
        let hbs_name = "empty.md";
        let raw_name = "empty.txt";
        let content = "";
        let test_data = json!({});

        // Actual: Compile both templates and render them
        let hbs_template = compile_template(hbs_name, content).unwrap();
        let raw_template = compile_template(raw_name, content).unwrap();

        let mut hb = Handlebars::new();
        hb.register_template("hbs_test", hbs_template);
        hb.register_template("raw_test", raw_template);

        let hbs_actual = hb.render("hbs_test", &test_data).unwrap();
        let raw_actual = hb.render("raw_test", &test_data).unwrap();

        // Expected: Both should render as empty strings
        assert_eq!(hbs_actual, "");
        assert_eq!(raw_actual, "");
    }

    #[test]
    fn test_compile_template_case_sensitivity() {
        // Fixture: Create templates with different case extensions
        let uppercase_name = "test.HBS";
        let lowercase_name = "test.md";
        let content = "Hello {{name}}!";
        let test_data = json!({"name": "World"});

        // Actual: Compile both templates and render them
        let uppercase_template = compile_template(uppercase_name, content).unwrap();
        let lowercase_template = compile_template(lowercase_name, content).unwrap();

        let mut hb = Handlebars::new();
        hb.register_template("uppercase", uppercase_template);
        hb.register_template("lowercase", lowercase_template);

        let uppercase_actual = hb.render("uppercase", &test_data).unwrap();
        let lowercase_actual = hb.render("lowercase", &test_data).unwrap();

        // Expected: Only lowercase .md should process handlebars syntax
        assert_eq!(uppercase_actual, "Hello {{name}}!"); // Raw string, no substitution
        assert_eq!(lowercase_actual, "Hello World!"); // Handlebars processed
    }

    #[tokio::test]
    async fn test_read_template_files_parallel_empty() {
        use std::path::Path;

        // Fixture: Create service and empty file list
        let service = ForgeTemplateService::new(Arc::new(MockCompositeService::new()));
        let file_paths: Vec<PathBuf> = vec![];
        let temp_path = Path::new("/tmp");

        // Actual: Read files in parallel with empty list
        let actual = service.read_all(&file_paths, temp_path).await.unwrap();

        // Expected: Should return empty vector
        assert_eq!(actual.len(), 0);
    }
}
