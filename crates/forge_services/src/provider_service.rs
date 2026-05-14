use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use forge_app::ProviderService;
use forge_app::domain::{
    AnyProvider, ChatCompletionMessage, Model, ModelId, ProviderId, ResultStream,
};
use forge_domain::{
    AuthCredential, ChatRepository, Context, MigrationResult, ModelSource, Provider,
    ProviderRepository, ProviderTemplate,
};
use url::Url;

/// Service layer wrapper for ProviderRepository that handles template rendering
pub struct ForgeProviderService<R> {
    repository: Arc<R>,
}

impl<R> ForgeProviderService<R> {
    /// Creates a new ForgeProviderService instance
    pub fn new(repository: Arc<R>) -> Self {
        Self { repository }
    }

    /// Renders a URL template with provided parameters.
    ///
    /// Params present in the credential are passed through as strings.
    /// Optional specs that are absent from the credential entirely are
    /// inserted as JSON `null` so that `{{#if PARAM}}` blocks evaluate
    /// to false in the Handlebars template (e.g. a port not provided).
    fn render_url_template(
        &self,
        template: &str,
        params: &HashMap<forge_domain::URLParam, forge_domain::URLParamValue>,
        specs: &[forge_domain::URLParamSpec],
    ) -> Result<Url> {
        // Start with all stored params as string values.
        let mut template_data: HashMap<&str, serde_json::Value> = params
            .iter()
            .map(|(k, v)| {
                (
                    k.as_str(),
                    serde_json::Value::String(v.as_str().to_string()),
                )
            })
            .collect();

        // For optional specs that are entirely absent from the credential,
        // inject null so {{#if PARAM}} is falsy instead of erroring.
        for spec in specs {
            if spec.optional && !params.contains_key(&spec.name) {
                template_data.insert(spec.name.as_str(), serde_json::Value::Null);
            }
        }

        let handlebars = forge_app::TemplateEngine::handlebar_instance();
        let rendered = handlebars.render_template(template, &template_data)?;

        Ok(Url::parse(&rendered)?)
    }

    /// Renders a provider from template to fully resolved URLs
    fn render_provider(&self, template_provider: ProviderTemplate) -> Result<Provider<Url>> {
        let credential = template_provider
            .credential
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Provider has no credential"))?;

        // Render main URL
        let url = self.render_url_template(
            &template_provider.url.template,
            &credential.url_params,
            &template_provider.url_params,
        )?;

        // Render model source URLs
        let models = template_provider.models.as_ref().and_then(|m| match m {
            ModelSource::Url(template) => {
                let model_url = self
                    .render_url_template(
                        &template.template,
                        &credential.url_params,
                        &template_provider.url_params,
                    )
                    .ok();
                model_url.map(ModelSource::Url)
            }
            ModelSource::Hardcoded(list) => Some(ModelSource::Hardcoded(list.clone())),
        });

        Ok(Provider {
            id: template_provider.id,
            provider_type: template_provider.provider_type,
            response: template_provider.response,
            url,
            models,
            auth_methods: template_provider.auth_methods,
            url_params: template_provider.url_params,
            credential: template_provider.credential,
            custom_headers: template_provider.custom_headers,
        })
    }
}

#[async_trait::async_trait]
impl<R: ChatRepository + ProviderRepository> ProviderService for ForgeProviderService<R> {
    async fn chat(
        &self,
        model_id: &ModelId,
        context: Context,
        provider: Provider<Url>,
    ) -> ResultStream<ChatCompletionMessage, anyhow::Error> {
        // Repository builds client on each call (no caching at repository level)
        self.repository.chat(model_id, context, provider).await
    }

    async fn models(&self, provider: Provider<Url>) -> Result<Vec<Model>> {
        self.repository.models(provider).await
    }

    async fn get_all_providers(&self) -> Result<Vec<AnyProvider>> {
        let providers = self.repository.get_all_providers().await?;

        // Render configured providers from Template to Url
        let rendered_providers = providers
            .into_iter()
            .map(|provider| {
                // If provider is a Template with credentials, render it to Url
                if let AnyProvider::Template(template_provider) = &provider
                    && template_provider.is_configured()
                {
                    // Clone and render the provider
                    if let Ok(rendered) = self.render_provider(template_provider.clone()) {
                        return AnyProvider::Url(rendered);
                    }
                }
                // Otherwise return as-is
                provider
            })
            .collect();

        Ok(rendered_providers)
    }

    async fn get_provider(&self, id: ProviderId) -> Result<Provider<Url>> {
        let template_provider = self.repository.get_provider(id).await?;
        self.render_provider(template_provider)
    }

    async fn upsert_credential(&self, credential: AuthCredential) -> Result<()> {
        self.repository.upsert_credential(credential).await
    }

    async fn remove_credential(&self, id: &ProviderId) -> Result<()> {
        self.repository.remove_credential(id).await
    }

    async fn migrate_env_credentials(&self) -> Result<Option<MigrationResult>> {
        self.repository.migrate_env_credentials().await
    }
}

#[cfg(test)]
mod tests {
    use forge_app::domain::ProviderId;
    use forge_domain::{
        AuthDetails, AuthMethod, InputModality, ModelSource, ProviderType, Template,
    };
    use pretty_assertions::assert_eq;

    use super::*;

    // Mock repository for testing
    struct MockProviderRepository {
        models: Vec<Model>,
        providers: Vec<AnyProvider>,
    }

    impl MockProviderRepository {
        fn new(models: Vec<Model>) -> Self {
            Self { models, providers: vec![] }
        }

        fn with_providers(mut self, providers: Vec<AnyProvider>) -> Self {
            self.providers = providers;
            self
        }
    }

    #[async_trait::async_trait]
    impl ChatRepository for MockProviderRepository {
        async fn chat(
            &self,
            _model_id: &ModelId,
            _context: Context,
            _provider: Provider<Url>,
        ) -> ResultStream<ChatCompletionMessage, anyhow::Error> {
            Ok(Box::pin(tokio_stream::empty()))
        }

        async fn models(&self, _provider: Provider<Url>) -> Result<Vec<Model>> {
            Ok(self.models.clone())
        }
    }

    #[async_trait::async_trait]
    impl ProviderRepository for MockProviderRepository {
        async fn get_all_providers(&self) -> Result<Vec<AnyProvider>> {
            Ok(self.providers.clone())
        }

        async fn get_provider(&self, _id: ProviderId) -> Result<ProviderTemplate> {
            Ok(test_template_provider())
        }

        async fn get_credential(&self, _id: &ProviderId) -> Result<Option<AuthCredential>> {
            Ok(None)
        }

        async fn upsert_credential(&self, _credential: AuthCredential) -> Result<()> {
            Ok(())
        }

        async fn remove_credential(&self, _id: &ProviderId) -> Result<()> {
            Ok(())
        }

        async fn migrate_env_credentials(&self) -> Result<Option<MigrationResult>> {
            Ok(None)
        }
    }

    fn test_provider() -> Provider<Url> {
        Provider {
            id: ProviderId::OPENAI,
            provider_type: ProviderType::Llm,
            response: Some(forge_app::domain::ProviderResponse::OpenAI),
            url: Url::parse("https://api.openai.com/v1/chat/completions").unwrap(),
            auth_methods: vec![AuthMethod::ApiKey],
            url_params: vec![],
            credential: Some(AuthCredential {
                id: ProviderId::OPENAI,
                auth_details: AuthDetails::ApiKey(forge_domain::ApiKey::from(
                    "test-key".to_string(),
                )),
                url_params: HashMap::new(),
            }),
            models: Some(ModelSource::Url(
                Url::parse("https://api.openai.com/v1/models").unwrap(),
            )),
            custom_headers: None,
        }
    }

    fn test_template_provider() -> ProviderTemplate {
        Provider {
            id: ProviderId::OPENAI,
            provider_type: ProviderType::Llm,
            response: Some(forge_app::domain::ProviderResponse::OpenAI),
            url: Template::<forge_domain::URLParameters>::new(
                "https://api.openai.com/v1/chat/completions",
            ),
            auth_methods: vec![AuthMethod::ApiKey],
            url_params: vec![],
            credential: Some(AuthCredential {
                id: ProviderId::OPENAI,
                auth_details: AuthDetails::ApiKey(forge_domain::ApiKey::from(
                    "test-key".to_string(),
                )),
                url_params: HashMap::new(),
            }),
            models: Some(ModelSource::Url(
                Template::<forge_domain::URLParameters>::new("https://api.openai.com/v1/models"),
            )),
            custom_headers: None,
        }
    }

    fn test_model(id: &str) -> Model {
        Model {
            id: ModelId::from(id),
            name: Some(id.to_string()),
            description: None,
            context_length: Some(4096),
            tools_supported: Some(true),
            supports_parallel_tool_calls: Some(true),
            supports_reasoning: Some(false),
            input_modalities: vec![InputModality::Text],
        }
    }

    #[tokio::test]
    async fn test_models_delegates_to_repository() {
        let models = vec![test_model("gpt-4"), test_model("gpt-3.5-turbo")];
        let repository = Arc::new(MockProviderRepository::new(models.clone()));
        let service = ForgeProviderService::new(repository);
        let provider = test_provider();

        let actual = service.models(provider).await.unwrap();

        assert_eq!(actual, models);
    }

    #[tokio::test]
    async fn test_get_all_providers_renders_configured_providers() {
        let configured = test_template_provider();
        let unconfigured = Provider { credential: None, ..test_template_provider() };

        let repository = Arc::new(MockProviderRepository::new(vec![]).with_providers(vec![
            AnyProvider::Template(configured),
            AnyProvider::Template(unconfigured),
        ]));

        let service = ForgeProviderService::new(repository);
        let actual = service.get_all_providers().await.unwrap();

        assert_eq!(actual.len(), 2);
        assert!(matches!(actual[0], AnyProvider::Url(_)));
        assert!(matches!(actual[1], AnyProvider::Template(_)));

        if let AnyProvider::Url(provider) = &actual[0] {
            assert_eq!(
                provider.url.as_str(),
                "https://api.openai.com/v1/chat/completions"
            );
        }
    }

    #[test]
    fn test_render_url_template_optional_port_absent() {
        // VLLM_PORT is absent from the credential (user left it blank).
        // render_url_template must inject null so {{#if VLLM_PORT}} is falsy.
        let service = ForgeProviderService::new(Arc::new(MockProviderRepository::new(vec![])));
        let template = "{{VLLM_SSL_SCHEME}}://{{VLLM_HOST}}{{#if VLLM_PORT}}:{{VLLM_PORT}}{{/if}}/v1/chat/completions";

        let mut params = HashMap::new();
        params.insert(
            forge_domain::URLParam::from("VLLM_SSL_SCHEME".to_string()),
            forge_domain::URLParamValue::from("https".to_string()),
        );
        params.insert(
            forge_domain::URLParam::from("VLLM_HOST".to_string()),
            forge_domain::URLParamValue::from("my.server.url".to_string()),
        );
        // VLLM_PORT intentionally absent — not in the credential map at all.

        let specs = vec![forge_domain::URLParamSpec::optional(
            forge_domain::URLParam::from("VLLM_PORT".to_string()),
        )];

        let actual = service
            .render_url_template(template, &params, &specs)
            .unwrap();
        let expected = "https://my.server.url/v1/chat/completions";

        assert_eq!(actual.as_str(), expected);
    }

    #[test]
    fn test_render_url_template_optional_port_with_value() {
        // When VLLM_PORT has a value, it should appear in the URL.
        let service = ForgeProviderService::new(Arc::new(MockProviderRepository::new(vec![])));
        let template = "{{VLLM_SSL_SCHEME}}://{{VLLM_HOST}}{{#if VLLM_PORT}}:{{VLLM_PORT}}{{/if}}/v1/chat/completions";

        let mut params = HashMap::new();
        params.insert(
            forge_domain::URLParam::from("VLLM_SSL_SCHEME".to_string()),
            forge_domain::URLParamValue::from("https".to_string()),
        );
        params.insert(
            forge_domain::URLParam::from("VLLM_HOST".to_string()),
            forge_domain::URLParamValue::from("my.server.url".to_string()),
        );
        params.insert(
            forge_domain::URLParam::from("VLLM_PORT".to_string()),
            forge_domain::URLParamValue::from("8000".to_string()),
        );

        let specs = vec![forge_domain::URLParamSpec::optional(
            forge_domain::URLParam::from("VLLM_PORT".to_string()),
        )];

        let actual = service
            .render_url_template(template, &params, &specs)
            .unwrap();
        let expected = "https://my.server.url:8000/v1/chat/completions";

        assert_eq!(actual.as_str(), expected);
    }
}
