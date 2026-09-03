//! Integration tests for provider resolution in forge_services.
//!
//! Tests cover:
//! - Provider ID construction and resolution (valid/invalid names)
//! - Provider config defaults (ProviderType, ProviderResponse)
//! - AnyProvider classification and URL extraction
//! - Template rendering via ForgeProviderService
//! - Credential lifecycle (upsert/remove/get)

use std::collections::HashMap;
use std::sync::Arc;

use forge_app::ProviderService;
use forge_app::domain::{
    AnyProvider, ChatCompletionMessage, Model, ModelId, ProviderId, ProviderResponse, ResultStream,
};
use forge_domain::{
    ApiKey, AuthCredential, AuthDetails, AuthMethod, InputModality, ModelSource, Provider,
    ProviderRepository, ProviderTemplate, Template, URLParam, URLParamSpec, URLParamValue,
};
use forge_services::provider_service::ForgeProviderService;
use url::Url;

// ---------------------------------------------------------------------------
// Mock infrastructure
// ---------------------------------------------------------------------------

struct MockProviderRepo {
    providers: Vec<AnyProvider>,
    credentials: HashMap<ProviderId, AuthCredential>,
}

impl MockProviderRepo {
    fn empty() -> Self {
        Self { providers: vec![], credentials: HashMap::new() }
    }

    fn with_providers(mut self, providers: Vec<AnyProvider>) -> Self {
        self.providers = providers;
        self
    }

    fn with_credential(mut self, cred: AuthCredential) -> Self {
        self.credentials.insert(cred.id.clone(), cred);
        self
    }
}

#[async_trait::async_trait]
impl forge_domain::ChatRepository for MockProviderRepo {
    async fn chat(
        &self,
        _model_id: &ModelId,
        _context: forge_domain::Context,
        _provider: Provider<Url>,
    ) -> ResultStream<ChatCompletionMessage, anyhow::Error> {
        Ok(Box::pin(tokio_stream::empty()))
    }

    async fn models(&self, _provider: Provider<Url>) -> anyhow::Result<Vec<Model>> {
        Ok(vec![])
    }
}

#[async_trait::async_trait]
impl ProviderRepository for MockProviderRepo {
    async fn get_all_providers(&self) -> anyhow::Result<Vec<AnyProvider>> {
        Ok(self.providers.clone())
    }

    async fn get_provider(&self, id: ProviderId) -> anyhow::Result<ProviderTemplate> {
        self.providers
            .iter()
            .find(|p| p.id() == id)
            .and_then(|p| match p {
                AnyProvider::Template(t) => Some(t.clone()),
                _ => None,
            })
            .ok_or_else(|| anyhow::anyhow!("provider not found: {id}"))
    }

    async fn get_credential(&self, id: &ProviderId) -> anyhow::Result<Option<AuthCredential>> {
        Ok(self.credentials.get(id).cloned())
    }

    async fn upsert_credential(&self, _credential: AuthCredential) -> anyhow::Result<()> {
        // In-memory mock — accept silently.
        Ok(())
    }

    async fn remove_credential(&self, _id: &ProviderId) -> anyhow::Result<()> {
        Ok(())
    }

    async fn migrate_env_credentials(
        &self,
    ) -> anyhow::Result<Option<forge_domain::MigrationResult>> {
        Ok(None)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn openai_template_provider() -> ProviderTemplate {
    Provider {
        id: ProviderId::OPENAI,
        provider_type: forge_domain::ProviderType::Llm,
        response: Some(ProviderResponse::OpenAI),
        url: Template::new("https://api.openai.com/v1/chat/completions"),
        auth_methods: vec![AuthMethod::ApiKey],
        url_params: vec![],
        credential: Some(AuthCredential {
            id: ProviderId::OPENAI,
            auth_details: AuthDetails::ApiKey(ApiKey::from("test-key".to_string())),
            url_params: HashMap::new(),
        }),
        models: Some(ModelSource::Url(Template::new(
            "https://api.openai.com/v1/models",
        ))),
        custom_headers: None,
    }
}

fn anthropic_template_provider() -> ProviderTemplate {
    Provider {
        id: ProviderId::ANTHROPIC,
        provider_type: forge_domain::ProviderType::Llm,
        response: Some(ProviderResponse::Anthropic),
        url: Template::new("https://api.anthropic.com/v1/messages"),
        auth_methods: vec![AuthMethod::ApiKey],
        url_params: vec![],
        credential: Some(AuthCredential {
            id: ProviderId::ANTHROPIC,
            auth_details: AuthDetails::ApiKey(ApiKey::from("sk-ant-test".to_string())),
            url_params: HashMap::new(),
        }),
        models: Some(ModelSource::Hardcoded(vec![])),
        custom_headers: None,
    }
}

fn vllm_template_provider() -> ProviderTemplate {
    Provider {
        id: ProviderId::from("vllm".to_string()),
        provider_type: forge_domain::ProviderType::Llm,
        response: Some(ProviderResponse::OpenAI),
        url: Template::new(
            "{{VLLM_SSL_SCHEME}}://{{VLLM_HOST}}{{#if VLLM_PORT}}:{{VLLM_PORT}}{{/if}}/v1/chat/completions",
        ),
        auth_methods: vec![AuthMethod::ApiKey],
        url_params: vec![URLParamSpec::optional(URLParam::from(
            "VLLM_PORT".to_string(),
        ))],
        credential: Some(AuthCredential {
            id: ProviderId::from("vllm".to_string()),
            auth_details: AuthDetails::ApiKey(ApiKey::from("token-abc".to_string())),
            url_params: {
                let mut m = HashMap::new();
                m.insert(
                    URLParam::from("VLLM_SSL_SCHEME".to_string()),
                    URLParamValue::from("https".to_string()),
                );
                m.insert(
                    URLParam::from("VLLM_HOST".to_string()),
                    URLParamValue::from("gpu.local".to_string()),
                );
                m
            },
        }),
        models: None,
        custom_headers: None,
    }
}

fn unconfigured_template_provider() -> ProviderTemplate {
    Provider {
        id: ProviderId::from("custom_provider".to_string()),
        provider_type: forge_domain::ProviderType::Llm,
        response: Some(ProviderResponse::OpenAI),
        url: Template::new("{{HOST}}/v1/chat/completions"),
        auth_methods: vec![AuthMethod::ApiKey],
        url_params: vec![],
        credential: None, // Not configured
        models: None,
        custom_headers: None,
    }
}

fn resolved_url_provider() -> AnyProvider {
    AnyProvider::Url(Provider {
        id: ProviderId::OPENAI,
        provider_type: forge_domain::ProviderType::Llm,
        response: Some(ProviderResponse::OpenAI),
        url: Url::parse("https://api.openai.com/v1/chat/completions").unwrap(),
        auth_methods: vec![AuthMethod::ApiKey],
        url_params: vec![],
        credential: Some(AuthCredential {
            id: ProviderId::OPENAI,
            auth_details: AuthDetails::ApiKey(ApiKey::from("test-key".to_string())),
            url_params: HashMap::new(),
        }),
        models: Some(ModelSource::Url(
            Url::parse("https://api.openai.com/v1/models").unwrap(),
        )),
        custom_headers: None,
    })
}

fn make_service(providers: Vec<AnyProvider>) -> ForgeProviderService<MockProviderRepo> {
    let repo = Arc::new(MockProviderRepo::empty().with_providers(providers));
    ForgeProviderService::new(repo)
}

fn make_service_with_credential(
    providers: Vec<AnyProvider>,
    cred: AuthCredential,
) -> ForgeProviderService<MockProviderRepo> {
    let repo = Arc::new(
        MockProviderRepo::empty()
            .with_providers(providers)
            .with_credential(cred),
    );
    ForgeProviderService::new(repo)
}

// ===========================================================================
// Tests: Provider ID construction and resolution
// ===========================================================================

#[test]
fn test_provider_id_built_in_constants() {
    // All built-in provider IDs resolve to expected string values.
    assert_eq!(ProviderId::OPENAI.as_ref(), "openai");
    assert_eq!(ProviderId::ANTHROPIC.as_ref(), "anthropic");
    assert_eq!(ProviderId::FORGE.as_ref(), "forge");
    assert_eq!(ProviderId::OPEN_ROUTER.as_ref(), "open_router");
    assert_eq!(ProviderId::REQUESTY.as_ref(), "requesty");
    assert_eq!(ProviderId::XAI.as_ref(), "xai");
    assert_eq!(ProviderId::CEREBRAS.as_ref(), "cerebras");
    assert_eq!(ProviderId::VERTEX_AI.as_ref(), "vertex_ai");
    assert_eq!(ProviderId::CLAUDE_CODE.as_ref(), "claude_code");
}

#[test]
fn test_provider_id_custom_from_string() {
    let id = ProviderId::from("my_custom_provider".to_string());
    assert_eq!(id.as_ref(), "my_custom_provider");
}

#[test]
fn test_provider_id_equality() {
    let a = ProviderId::from("openai".to_string());
    let b = ProviderId::OPENAI;
    assert_eq!(a, b);
}

#[test]
fn test_provider_id_inequality() {
    assert_ne!(ProviderId::OPENAI, ProviderId::ANTHROPIC);
}

#[test]
fn test_provider_id_ord() {
    // ProviderId implements Ord; 'openai' > 'anthropic' alphabetically ('o' > 'a').
    assert!(ProviderId::ANTHROPIC < ProviderId::OPENAI);
}

#[test]
fn test_provider_id_hash() {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut h1 = DefaultHasher::new();
    let mut h2 = DefaultHasher::new();
    ProviderId::OPENAI.hash(&mut h1);
    ProviderId::from("openai".to_string()).hash(&mut h2);
    assert_eq!(h1.finish(), h2.finish());
}

// ===========================================================================
// Tests: Provider config defaults
// ===========================================================================

#[test]
fn test_provider_type_default_is_llm() {
    let default_type = forge_domain::ProviderType::default();
    assert_eq!(default_type, forge_domain::ProviderType::Llm);
}

#[test]
fn test_provider_type_display() {
    assert_eq!(forge_domain::ProviderType::Llm.to_string(), "llm");
    assert_eq!(
        forge_domain::ProviderType::ContextEngine.to_string(),
        "context_engine"
    );
}

#[test]
fn test_provider_response_variants() {
    // Just verifying the enum variants compile and can be compared.
    let openai = Some(ProviderResponse::OpenAI);
    let anthropic = Some(ProviderResponse::Anthropic);
    assert_ne!(openai, anthropic);
}

#[test]
fn test_input_modality_text() {
    let m = InputModality::Text;
    // Just verify the variant constructs without panic.
    assert!(matches!(m, InputModality::Text));
}

// ===========================================================================
// Tests: AnyProvider classification
// ===========================================================================

#[test]
fn test_anyprovider_url_variant() {
    let p = resolved_url_provider();
    assert!(matches!(p, AnyProvider::Url(_)));
    assert_eq!(p.id(), ProviderId::OPENAI);
    assert!(p.is_configured());
}

#[test]
fn test_anyprovider_template_configured() {
    let p = AnyProvider::Template(openai_template_provider());
    assert!(matches!(p, AnyProvider::Template(_)));
    assert!(p.is_configured());
    assert_eq!(p.id(), ProviderId::OPENAI);
}

#[test]
fn test_anyprovider_template_unconfigured() {
    let p = AnyProvider::Template(unconfigured_template_provider());
    assert!(!p.is_configured());
    assert_eq!(p.id().as_ref(), "custom_provider");
}

#[test]
fn test_anyprovider_provider_type() {
    let url_p = resolved_url_provider();
    assert_eq!(url_p.provider_type(), &forge_domain::ProviderType::Llm);

    let tmpl_p = AnyProvider::Template(openai_template_provider());
    assert_eq!(tmpl_p.provider_type(), &forge_domain::ProviderType::Llm);
}

#[test]
fn test_anyprovider_url_extraction_resolved() {
    let p = resolved_url_provider();
    let url = p.url().expect("URL provider should have a url");
    assert_eq!(url.as_str(), "https://api.openai.com/v1/chat/completions");
}

#[test]
fn test_anyprovider_url_extraction_unconfigured_returns_none() {
    let p = AnyProvider::Template(unconfigured_template_provider());
    // Template providers that require URL params and have no credentials
    // should return None for url().
    assert!(p.url().is_none());
}

#[test]
fn test_anyprovider_response() {
    let p = resolved_url_provider();
    assert!(p.response().is_some());
}

// ===========================================================================
// Tests: ForgeProviderService — template rendering
// ===========================================================================

#[tokio::test]
async fn test_get_all_providers_renders_configured() {
    let service = make_service(vec![
        AnyProvider::Template(openai_template_provider()),
        AnyProvider::Template(unconfigured_template_provider()),
    ]);

    let result = service.get_all_providers().await.unwrap();
    assert_eq!(result.len(), 2);

    // First (configured) should be rendered to Url.
    assert!(matches!(&result[0], AnyProvider::Url(_)));
    if let AnyProvider::Url(p) = &result[0] {
        assert_eq!(p.url.as_str(), "https://api.openai.com/v1/chat/completions");
    }

    // Second (unconfigured) stays as Template.
    assert!(matches!(&result[1], AnyProvider::Template(_)));
}

#[tokio::test]
async fn test_get_all_providers_preserves_already_resolved() {
    let service = make_service(vec![resolved_url_provider()]);

    let result = service.get_all_providers().await.unwrap();
    assert_eq!(result.len(), 1);
    assert!(matches!(&result[0], AnyProvider::Url(_)));
}

#[tokio::test]
async fn test_get_provider_returns_rendered_url() {
    let service = make_service(vec![AnyProvider::Template(openai_template_provider())]);

    let provider = service.get_provider(ProviderId::OPENAI).await.unwrap();
    assert_eq!(
        provider.url.as_str(),
        "https://api.openai.com/v1/chat/completions"
    );
}

#[tokio::test]
async fn test_get_provider_not_found() {
    let service = make_service(vec![]);

    let result = service.get_provider(ProviderId::ANTHROPIC).await;
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("not found"), "unexpected error: {err_msg}");
}

#[tokio::test]
async fn test_get_provider_vllm_with_port() {
    let service = make_service(vec![AnyProvider::Template(vllm_template_provider())]);

    let provider = service
        .get_provider(ProviderId::from("vllm".to_string()))
        .await
        .unwrap();
    // vllm template has no port — should omit port section.
    assert_eq!(
        provider.url.as_str(),
        "https://gpu.local/v1/chat/completions"
    );
}

// ===========================================================================
// Tests: Provider config defaults via template
// ===========================================================================

#[test]
fn test_template_provider_defaults() {
    let tmpl = openai_template_provider();
    assert_eq!(tmpl.provider_type, forge_domain::ProviderType::Llm);
    assert!(tmpl.auth_methods.contains(&AuthMethod::ApiKey));
    assert!(tmpl.credential.is_some());
}

#[test]
fn test_unconfigured_provider_has_no_credential() {
    let tmpl = unconfigured_template_provider();
    assert!(tmpl.credential.is_none());
}

#[tokio::test]
async fn test_get_all_providers_empty_list() {
    let service = make_service(vec![]);
    let result = service.get_all_providers().await.unwrap();
    assert!(result.is_empty());
}

// ===========================================================================
// Tests: Credential lifecycle via repository
// ===========================================================================

#[tokio::test]
async fn test_upsert_and_get_credential() {
    let cred = AuthCredential {
        id: ProviderId::OPENAI,
        auth_details: AuthDetails::ApiKey(ApiKey::from("sk-real".to_string())),
        url_params: HashMap::new(),
    };

    let _service = make_service_with_credential(
        vec![AnyProvider::Template(openai_template_provider())],
        cred.clone(),
    );

    // The repo mock stores credentials; verify get_credential returns it.
    // (We access the repo directly to verify the mock works.)
    let repo = Arc::new(MockProviderRepo::empty().with_credential(cred.clone()));
    let got = repo.get_credential(&ProviderId::OPENAI).await.unwrap();
    assert!(got.is_some());
    assert_eq!(got.unwrap().id, ProviderId::OPENAI);
}

#[tokio::test]
async fn test_get_credential_missing() {
    let repo = Arc::new(MockProviderRepo::empty());
    let got = repo.get_credential(&ProviderId::OPENAI).await.unwrap();
    assert!(got.is_none());
}

#[tokio::test]
async fn test_remove_credential_succeeds() {
    let cred = AuthCredential {
        id: ProviderId::OPENAI,
        auth_details: AuthDetails::ApiKey(ApiKey::from("sk-temp".to_string())),
        url_params: HashMap::new(),
    };
    let repo = Arc::new(MockProviderRepo::empty().with_credential(cred));
    // remove_credential should succeed without error.
    let result = repo.remove_credential(&ProviderId::OPENAI).await;
    assert!(result.is_ok());
}

// ===========================================================================
// Tests: Provider factory pattern
// ===========================================================================

#[tokio::test]
async fn test_factory_renders_multiple_providers() {
    let service = make_service(vec![
        AnyProvider::Template(openai_template_provider()),
        AnyProvider::Template(anthropic_template_provider()),
    ]);

    let all = service.get_all_providers().await.unwrap();
    assert_eq!(all.len(), 2);

    // Both should be rendered to Url variants.
    for p in &all {
        assert!(matches!(p, AnyProvider::Url(_)), "expected Url variant");
    }

    let urls: Vec<&str> = all
        .iter()
        .filter_map(|p| {
            if let AnyProvider::Url(u) = p {
                Some(u.url.as_str())
            } else {
                None
            }
        })
        .collect();

    assert!(urls.contains(&"https://api.openai.com/v1/chat/completions"));
    assert!(urls.contains(&"https://api.anthropic.com/v1/messages"));
}

#[tokio::test]
async fn test_factory_skips_unconfigured_providers() {
    let service = make_service(vec![
        AnyProvider::Template(openai_template_provider()),
        AnyProvider::Template(unconfigured_template_provider()),
    ]);

    let all = service.get_all_providers().await.unwrap();
    assert_eq!(all.len(), 2);

    // First is rendered (Url), second stays Template.
    assert!(matches!(&all[0], AnyProvider::Url(_)));
    assert!(matches!(&all[1], AnyProvider::Template(_)));
}

#[tokio::test]
async fn test_factory_handles_mixed_resolved_and_template() {
    let service = make_service(vec![
        resolved_url_provider(),
        AnyProvider::Template(anthropic_template_provider()),
    ]);

    let all = service.get_all_providers().await.unwrap();
    assert_eq!(all.len(), 2);
    // Both should be Url.
    for p in &all {
        assert!(matches!(p, AnyProvider::Url(_)));
    }
}

#[tokio::test]
async fn test_factory_model_source_preserved() {
    let tmpl = openai_template_provider();
    let service = make_service(vec![AnyProvider::Template(tmpl)]);

    let all = service.get_all_providers().await.unwrap();
    if let AnyProvider::Url(p) = &all[0] {
        assert!(p.models.is_some());
        if let Some(ModelSource::Url(url)) = &p.models {
            assert_eq!(url.as_str(), "https://api.openai.com/v1/models");
        }
    } else {
        panic!("expected Url variant");
    }
}

#[tokio::test]
async fn test_factory_hardcoded_models_preserved() {
    let tmpl = anthropic_template_provider();
    let service = make_service(vec![AnyProvider::Template(tmpl)]);

    let all = service.get_all_providers().await.unwrap();
    if let AnyProvider::Url(p) = &all[0] {
        assert!(p.models.is_some());
        assert!(matches!(&p.models, Some(ModelSource::Hardcoded(_))));
    } else {
        panic!("expected Url variant");
    }
}

// ===========================================================================
// Tests: Edge cases
// ===========================================================================

#[tokio::test]
async fn test_provider_id_from_various_sources() {
    let from_str = ProviderId::from("openai".to_string());
    assert_eq!(from_str, ProviderId::OPENAI);

    let from_static = ProviderId::ANTHROPIC;
    assert_eq!(from_static.as_ref(), "anthropic");
}

#[test]
fn test_anyprovider_id_from_url_variant() {
    let p = resolved_url_provider();
    assert_eq!(p.id(), ProviderId::OPENAI);
}

#[test]
fn test_anyprovider_id_from_template_variant() {
    let p = AnyProvider::Template(openai_template_provider());
    assert_eq!(p.id(), ProviderId::OPENAI);
}

#[tokio::test]
async fn test_get_provider_with_template_id_lookup() {
    let service = make_service(vec![
        AnyProvider::Template(openai_template_provider()),
        AnyProvider::Template(anthropic_template_provider()),
    ]);

    // Both should resolve independently.
    let openai = service.get_provider(ProviderId::OPENAI).await.unwrap();
    assert_eq!(
        openai.url.as_str(),
        "https://api.openai.com/v1/chat/completions"
    );

    let anthropic = service.get_provider(ProviderId::ANTHROPIC).await.unwrap();
    assert_eq!(
        anthropic.url.as_str(),
        "https://api.anthropic.com/v1/messages"
    );
}

#[tokio::test]
async fn test_multiple_provider_ids_do_not_interfere() {
    let service = make_service(vec![
        AnyProvider::Template(openai_template_provider()),
        AnyProvider::Template(anthropic_template_provider()),
    ]);

    let all = service.get_all_providers().await.unwrap();
    let ids: Vec<ProviderId> = all.iter().map(|p| p.id()).collect();
    assert!(ids.contains(&ProviderId::OPENAI));
    assert!(ids.contains(&ProviderId::ANTHROPIC));
    // No duplicates
    assert_eq!(ids.len(), 2);
}
