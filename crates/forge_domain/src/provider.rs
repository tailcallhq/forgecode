use std::borrow::Cow;

use derive_more::{AsRef, Deref, From};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use strum_macros::{Display, EnumString};
use url::Url;

use crate::{ApiKey, AuthCredential, AuthDetails, Model, Template};

/// Distinguishes between different categories of providers
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Display, EnumString, Default,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum ProviderType {
    /// LLM providers for chat completions (default for backward compatibility)
    #[default]
    Llm,
    /// Context engine providers for code indexing and search
    ContextEngine,
}

/// --- IMPORTANT ---
/// The order of providers is important because that would be order in which the
/// providers will be resolved
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    JsonSchema,
    AsRef,
    Deref,
    Serialize,
    Deserialize,
)]
#[schemars(with = "String")]
#[serde(from = "String")]
pub struct ProviderId(Cow<'static, str>);

impl ProviderId {
    // Built-in provider constants
    pub const FORGE: ProviderId = ProviderId(Cow::Borrowed("forge"));
    pub const OPENAI: ProviderId = ProviderId(Cow::Borrowed("openai"));
    pub const OPEN_ROUTER: ProviderId = ProviderId(Cow::Borrowed("open_router"));
    pub const REQUESTY: ProviderId = ProviderId(Cow::Borrowed("requesty"));
    pub const ZAI: ProviderId = ProviderId(Cow::Borrowed("zai"));
    pub const ZAI_CODING: ProviderId = ProviderId(Cow::Borrowed("zai_coding"));
    pub const CEREBRAS: ProviderId = ProviderId(Cow::Borrowed("cerebras"));
    pub const XAI: ProviderId = ProviderId(Cow::Borrowed("xai"));
    pub const ANTHROPIC: ProviderId = ProviderId(Cow::Borrowed("anthropic"));
    pub const CLAUDE_CODE: ProviderId = ProviderId(Cow::Borrowed("claude_code"));
    pub const VERTEX_AI: ProviderId = ProviderId(Cow::Borrowed("vertex_ai"));
    pub const VERTEX_AI_ANTHROPIC: ProviderId = ProviderId(Cow::Borrowed("vertex_ai_anthropic"));
    pub const BIG_MODEL: ProviderId = ProviderId(Cow::Borrowed("big_model"));
    pub const AZURE: ProviderId = ProviderId(Cow::Borrowed("azure"));
    pub const GITHUB_COPILOT: ProviderId = ProviderId(Cow::Borrowed("github_copilot"));
    pub const OPENAI_COMPATIBLE: ProviderId = ProviderId(Cow::Borrowed("openai_compatible"));
    pub const OPENAI_RESPONSES_COMPATIBLE: ProviderId =
        ProviderId(Cow::Borrowed("openai_responses_compatible"));
    pub const ANTHROPIC_COMPATIBLE: ProviderId = ProviderId(Cow::Borrowed("anthropic_compatible"));
    pub const FORGE_SERVICES: ProviderId = ProviderId(Cow::Borrowed("forge_services"));
    pub const IO_INTELLIGENCE: ProviderId = ProviderId(Cow::Borrowed("io_intelligence"));
    pub const BEDROCK: ProviderId = ProviderId(Cow::Borrowed("bedrock"));
    pub const MINIMAX: ProviderId = ProviderId(Cow::Borrowed("minimax"));
    pub const CODEX: ProviderId = ProviderId(Cow::Borrowed("codex"));
    pub const OPENCODE_ZEN: ProviderId = ProviderId(Cow::Borrowed("opencode_zen"));
    pub const OPENCODE_GO: ProviderId = ProviderId(Cow::Borrowed("opencode_go"));
    pub const FIREWORKS_AI: ProviderId = ProviderId(Cow::Borrowed("fireworks-ai"));
    pub const NOVITA: ProviderId = ProviderId(Cow::Borrowed("novita"));
    pub const VIVGRID: ProviderId = ProviderId(Cow::Borrowed("vivgrid"));
    pub const GOOGLE_AI_STUDIO: ProviderId = ProviderId(Cow::Borrowed("google_ai_studio"));
    pub const MODAL: ProviderId = ProviderId(Cow::Borrowed("modal"));
    pub const ADAL: ProviderId = ProviderId(Cow::Borrowed("adal"));
    pub const XIAOMI_MIMO: ProviderId = ProviderId(Cow::Borrowed("xiaomi_mimo"));
    pub const NVIDIA: ProviderId = ProviderId(Cow::Borrowed("nvidia"));

    /// Returns all built-in provider IDs
    ///
    /// This includes all providers defined as constants in this implementation.
    pub fn built_in_providers() -> &'static [ProviderId] {
        &[
            ProviderId::FORGE,
            ProviderId::OPENAI,
            ProviderId::OPEN_ROUTER,
            ProviderId::REQUESTY,
            ProviderId::ZAI,
            ProviderId::ZAI_CODING,
            ProviderId::CEREBRAS,
            ProviderId::XAI,
            ProviderId::ANTHROPIC,
            ProviderId::CLAUDE_CODE,
            ProviderId::VERTEX_AI,
            ProviderId::VERTEX_AI_ANTHROPIC,
            ProviderId::BIG_MODEL,
            ProviderId::AZURE,
            ProviderId::GITHUB_COPILOT,
            ProviderId::OPENAI_COMPATIBLE,
            ProviderId::OPENAI_RESPONSES_COMPATIBLE,
            ProviderId::ANTHROPIC_COMPATIBLE,
            ProviderId::FORGE_SERVICES,
            ProviderId::IO_INTELLIGENCE,
            ProviderId::BEDROCK,
            ProviderId::MINIMAX,
            ProviderId::CODEX,
            ProviderId::OPENCODE_ZEN,
            ProviderId::OPENCODE_GO,
            ProviderId::FIREWORKS_AI,
            ProviderId::NOVITA,
            ProviderId::VIVGRID,
            ProviderId::GOOGLE_AI_STUDIO,
            ProviderId::MODAL,
            ProviderId::ADAL,
            ProviderId::XIAOMI_MIMO,
            ProviderId::NVIDIA,
        ]
    }

    /// Returns the display name for UI (UpperCamelCase with special handling
    /// for acronyms).
    ///
    /// This converts snake_case IDs to proper display names:
    /// - "openai" -> "OpenAI"
    /// - "open_router" -> "OpenRouter"
    /// - "xai" -> "XAI"
    fn display_name(&self) -> String {
        // Special cases for known providers with acronyms
        match self.0.as_ref() {
            "openai" => "OpenAI".to_string(),
            "xai" => "XAI".to_string(),
            "zai" => "ZAI".to_string(),
            "vertex_ai" => "VertexAI".to_string(),
            "vertex_ai_anthropic" => "VertexAIAnthropic".to_string(),
            "openai_compatible" => "OpenAICompatible".to_string(),
            "openai_responses_compatible" => "OpenAIResponsesCompatible".to_string(),
            "io_intelligence" => "IOIntelligence".to_string(),
            "minimax" => "MiniMax".to_string(),
            "codex" => "Codex".to_string(),
            "opencode_zen" => "OpenCode Zen".to_string(),
            "opencode_go" => "OpenCode Go".to_string(),
            "fireworks-ai" => "FireworksAI".to_string(),
            "novita" => "Novita".to_string(),
            "vivgrid" => "Vivgrid".to_string(),
            "google_ai_studio" => "GoogleAIStudio".to_string(),
            "modal" => "Modal".to_string(),
            "adal" => "AdaL".to_string(),
            "xiaomi_mimo" => "XiaomiMimo".to_string(),
            "nvidia" => "NVIDIA".to_string(),
            _ => {
                // For other providers, use UpperCamelCase conversion
                use convert_case::{Case, Casing};
                self.0.to_case(Case::UpperCamel)
            }
        }
    }
}

impl std::fmt::Display for ProviderId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.display_name())
    }
}

impl std::str::FromStr for ProviderId {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Check if it's a built-in provider first
        let provider = match s {
            "forge" => ProviderId::FORGE,
            "openai" => ProviderId::OPENAI,
            "open_router" => ProviderId::OPEN_ROUTER,
            "requesty" => ProviderId::REQUESTY,
            "zai" => ProviderId::ZAI,
            "zai_coding" => ProviderId::ZAI_CODING,
            "cerebras" => ProviderId::CEREBRAS,
            "xai" => ProviderId::XAI,
            "anthropic" => ProviderId::ANTHROPIC,
            "claude_code" => ProviderId::CLAUDE_CODE,
            "vertex_ai" => ProviderId::VERTEX_AI,
            "big_model" => ProviderId::BIG_MODEL,
            "azure" => ProviderId::AZURE,
            "github_copilot" => ProviderId::GITHUB_COPILOT,
            "openai_compatible" => ProviderId::OPENAI_COMPATIBLE,
            "openai_responses_compatible" => ProviderId::OPENAI_RESPONSES_COMPATIBLE,
            "anthropic_compatible" => ProviderId::ANTHROPIC_COMPATIBLE,
            "forge_services" => ProviderId::FORGE_SERVICES,
            "io_intelligence" => ProviderId::IO_INTELLIGENCE,
            "minimax" => ProviderId::MINIMAX,
            "codex" => ProviderId::CODEX,
            "opencode_go" => ProviderId::OPENCODE_GO,
            "fireworks-ai" => ProviderId::FIREWORKS_AI,
            "novita" => ProviderId::NOVITA,
            "vertex_ai_anthropic" => ProviderId::VERTEX_AI_ANTHROPIC,
            "bedrock" => ProviderId::BEDROCK,
            "opencode_zen" => ProviderId::OPENCODE_ZEN,
            "vivgrid" => ProviderId::VIVGRID,
            "google_ai_studio" => ProviderId::GOOGLE_AI_STUDIO,
            "modal" => ProviderId::MODAL,
            "adal" => ProviderId::ADAL,
            "xiaomi_mimo" => ProviderId::XIAOMI_MIMO,
            "nvidia" => ProviderId::NVIDIA,
            // For custom providers, use Cow::Owned to avoid memory leaks
            custom => ProviderId(Cow::Owned(custom.to_string())),
        };
        Ok(provider)
    }
}

impl From<String> for ProviderId {
    fn from(s: String) -> Self {
        std::str::FromStr::from_str(&s).unwrap()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ProviderResponse {
    OpenAI,
    OpenAIResponses,
    Anthropic,
    Bedrock,
    Google,
    OpenCode,
}

/// Represents the source of models for a provider
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ModelSource<T> {
    /// Can be a `Url` or a `Template`
    Url(T),
    Hardcoded(Vec<Model>),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Provider<T> {
    pub id: ProviderId,
    #[serde(default)]
    pub provider_type: ProviderType,
    pub response: Option<ProviderResponse>,
    pub url: T,
    pub models: Option<ModelSource<T>>,
    pub auth_methods: Vec<crate::AuthMethod>,
    #[serde(default)]
    pub url_params: Vec<crate::URLParamSpec>,
    pub credential: Option<AuthCredential>,
    /// Custom HTTP headers to include in API requests for this provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_headers: Option<std::collections::HashMap<String, String>>,
}

/// Type alias for a provider with template URLs (not yet rendered)
pub type ProviderTemplate = Provider<Template<crate::URLParameters>>;

impl<T> Provider<T> {
    pub fn is_configured(&self) -> bool {
        self.credential.is_some()
    }
    pub fn models(&self) -> Option<&ModelSource<T>> {
        self.models.as_ref()
    }
}

impl Provider<Url> {
    pub fn url(&self) -> &Url {
        &self.url
    }

    pub fn api_key(&self) -> Option<&ApiKey> {
        self.credential
            .as_ref()
            .and_then(|c| match &c.auth_details {
                AuthDetails::ApiKey(key) => Some(key),
                _ => None,
            })
    }
}

/// Enum for viewing providers in listings where both configured and
/// unconfigured.
#[derive(Debug, Clone, PartialEq, From)]
pub enum AnyProvider {
    Url(Provider<Url>),
    Template(ProviderTemplate),
}

impl AnyProvider {
    /// Returns whether this provider is configured
    pub fn is_configured(&self) -> bool {
        match self {
            AnyProvider::Url(p) => p.is_configured(),
            AnyProvider::Template(p) => p.is_configured(),
        }
    }

    pub fn provider_type(&self) -> &ProviderType {
        match self {
            AnyProvider::Url(p) => &p.provider_type,
            AnyProvider::Template(t) => &t.provider_type,
        }
    }

    pub fn id(&self) -> ProviderId {
        match self {
            AnyProvider::Url(p) => p.id.clone(),
            AnyProvider::Template(p) => p.id.clone(),
        }
    }

    /// Gets the response type
    pub fn response(&self) -> Option<&ProviderResponse> {
        match self {
            AnyProvider::Url(p) => p.response.as_ref(),
            AnyProvider::Template(p) => p.response.as_ref(),
        }
    }

    /// Gets the URL for this provider.
    ///
    /// For configured providers, returns the resolved URL. For template
    /// providers with no URL parameters (i.e. a hardcoded default URL in
    /// provider.json), parses and returns the template string as a URL.
    /// Returns `None` for template providers that require user-supplied URL
    /// parameters.
    pub fn url(&self) -> Option<Url> {
        match self {
            AnyProvider::Url(p) => Some(p.url().clone()),
            AnyProvider::Template(t) if t.url_params.is_empty() => Url::parse(&t.url.template).ok(),
            AnyProvider::Template(_) => None,
        }
    }
    pub fn url_params(&self) -> &[crate::URLParamSpec] {
        match self {
            AnyProvider::Url(p) => &p.url_params,
            AnyProvider::Template(p) => &p.url_params,
        }
    }

    /// Gets the authentication methods supported by this provider
    pub fn auth_methods(&self) -> &[crate::AuthMethod] {
        match self {
            AnyProvider::Url(p) => &p.auth_methods,
            AnyProvider::Template(p) => &p.auth_methods,
        }
    }

    /// Consumes self and returns the configured provider if this is a URL
    /// provider with credentials
    pub fn into_configured(self) -> Option<Provider<Url>> {
        match self {
            AnyProvider::Url(p) if p.is_configured() => Some(p),
            _ => None,
        }
    }
}

/// Represents a provider with its available models
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderModels {
    /// The provider identifier
    pub provider_id: ProviderId,
    /// Available models from this provider
    pub models: Vec<Model>,
}

#[cfg(test)]
mod test_helpers {
    use std::collections::HashMap;

    use super::*;

    fn make_credential(provider_id: ProviderId, key: &str) -> Option<AuthCredential> {
        Some(AuthCredential {
            id: provider_id,
            auth_details: AuthDetails::ApiKey(ApiKey::from(key.to_string())),
            url_params: HashMap::new(),
        })
    }

    /// Test helper for creating a ZAI provider
    pub(super) fn zai(key: &str) -> Provider<Url> {
        Provider {
            id: ProviderId::ZAI,
            provider_type: Default::default(),
            response: Some(ProviderResponse::OpenAI),
            url: Url::parse("https://api.z.ai/api/paas/v4/chat/completions").unwrap(),
            auth_methods: vec![crate::AuthMethod::ApiKey],
            url_params: vec![],
            credential: make_credential(ProviderId::ZAI, key),
            custom_headers: None,
            models: Some(ModelSource::Url(
                Url::parse("https://api.z.ai/api/paas/v4/models").unwrap(),
            )),
        }
    }

    /// Test helper for creating a ZAI Coding provider
    pub(super) fn zai_coding(key: &str) -> Provider<Url> {
        Provider {
            id: ProviderId::ZAI_CODING,
            provider_type: Default::default(),
            response: Some(ProviderResponse::OpenAI),
            url: Url::parse("https://api.z.ai/api/coding/paas/v4/chat/completions").unwrap(),
            auth_methods: vec![crate::AuthMethod::ApiKey],
            url_params: vec![],
            credential: make_credential(ProviderId::ZAI_CODING, key),
            custom_headers: None,
            models: Some(ModelSource::Url(
                Url::parse("https://api.z.ai/api/paas/v4/models").unwrap(),
            )),
        }
    }

    /// Test helper for creating an OpenAI provider
    pub(super) fn openai(key: &str) -> Provider<Url> {
        Provider {
            id: ProviderId::OPENAI,
            provider_type: Default::default(),
            response: Some(ProviderResponse::OpenAI),
            url: Url::parse("https://api.openai.com/v1/chat/completions").unwrap(),
            auth_methods: vec![crate::AuthMethod::ApiKey],
            url_params: vec![],
            credential: make_credential(ProviderId::OPENAI, key),
            custom_headers: None,
            models: Some(ModelSource::Url(
                Url::parse("https://api.openai.com/v1/models").unwrap(),
            )),
        }
    }

    /// Test helper for creating an XAI provider
    pub(super) fn xai(key: &str) -> Provider<Url> {
        Provider {
            id: ProviderId::XAI,
            provider_type: Default::default(),
            response: Some(ProviderResponse::OpenAI),
            url: Url::parse("https://api.x.ai/v1/chat/completions").unwrap(),
            auth_methods: vec![crate::AuthMethod::ApiKey],
            url_params: vec![],
            credential: make_credential(ProviderId::XAI, key),
            custom_headers: None,
            models: Some(ModelSource::Url(
                Url::parse("https://api.x.ai/v1/models").unwrap(),
            )),
        }
    }

    /// Test helper for creating a Vertex AI provider
    pub(super) fn vertex_ai(key: &str, project_id: &str, location: &str) -> Provider<Url> {
        let (chat_url, model_url) = if location == "global" {
            (
                format!(
                    "https://aiplatform.googleapis.com/v1/projects/{}/locations/{}/endpoints/openapi/chat/completions",
                    project_id, location
                ),
                format!(
                    "https://aiplatform.googleapis.com/v1/projects/{}/locations/{}/endpoints/openapi/models",
                    project_id, location
                ),
            )
        } else {
            (
                format!(
                    "https://{}-aiplatform.googleapis.com/v1/projects/{}/locations/{}/endpoints/openapi/chat/completions",
                    location, project_id, location
                ),
                format!(
                    "https://{}-aiplatform.googleapis.com/v1/projects/{}/locations/{}/endpoints/openapi/models",
                    location, project_id, location
                ),
            )
        };
        Provider {
            id: ProviderId::VERTEX_AI,
            provider_type: Default::default(),
            response: Some(ProviderResponse::OpenAI),
            url: Url::parse(&chat_url).unwrap(),
            auth_methods: vec![crate::AuthMethod::ApiKey],
            url_params: ["project_id", "location"]
                .iter()
                .map(|&s| s.to_string().into())
                .collect(),
            credential: make_credential(ProviderId::VERTEX_AI, key),
            custom_headers: None,
            models: Some(ModelSource::Url(Url::parse(&model_url).unwrap())),
        }
    }

    /// Test helper for creating an IO Intelligence provider
    pub(super) fn io_intelligence(key: &str) -> Provider<Url> {
        Provider {
            id: ProviderId::IO_INTELLIGENCE,
            provider_type: Default::default(),
            response: Some(ProviderResponse::OpenAI),
            url: Url::parse("https://api.intelligence.io.solutions/api/v1/chat/completions")
                .unwrap(),
            auth_methods: vec![crate::AuthMethod::ApiKey],
            url_params: vec![],
            credential: make_credential(ProviderId::IO_INTELLIGENCE, key),
            custom_headers: None,
            models: Some(ModelSource::Url(
                Url::parse("https://api.intelligence.io.solutions/api/v1/models").unwrap(),
            )),
        }
    }

    /// Test helper for creating an Azure provider
    pub(super) fn azure(
        key: &str,
        resource_name: &str,
        deployment_name: &str,
        api_version: &str,
    ) -> Provider<Url> {
        let chat_url = format!(
            "https://{}.openai.azure.com/openai/deployments/{}/chat/completions?api-version={}",
            resource_name, deployment_name, api_version
        );
        let model_url = format!(
            "https://{}.openai.azure.com/openai/models?api-version={}",
            resource_name, api_version
        );

        Provider {
            id: ProviderId::AZURE,
            provider_type: Default::default(),
            response: Some(ProviderResponse::OpenAI),
            url: Url::parse(&chat_url).unwrap(),
            auth_methods: vec![crate::AuthMethod::ApiKey],
            url_params: ["resource_name", "deployment_name", "api_version"]
                .iter()
                .map(|&s| s.to_string().into())
                .collect(),
            credential: make_credential(ProviderId::AZURE, key),
            custom_headers: None,
            models: Some(ModelSource::Url(Url::parse(&model_url).unwrap())),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::str::FromStr;

    use pretty_assertions::assert_eq;

    use super::test_helpers::*;
    use super::*;

    #[test]
    fn test_provider_id_display_name() {
        assert_eq!(ProviderId::OPENAI.to_string(), "OpenAI");
        assert_eq!(ProviderId::OPEN_ROUTER.to_string(), "OpenRouter");
        assert_eq!(ProviderId::ZAI.to_string(), "ZAI");
        assert_eq!(ProviderId::XAI.to_string(), "XAI");
        assert_eq!(ProviderId::ANTHROPIC.to_string(), "Anthropic");
        assert_eq!(ProviderId::GITHUB_COPILOT.to_string(), "GithubCopilot");
        assert_eq!(ProviderId::VERTEX_AI.to_string(), "VertexAI");
        assert_eq!(
            ProviderId::OPENAI_COMPATIBLE.to_string(),
            "OpenAICompatible"
        );
        assert_eq!(
            ProviderId::OPENAI_RESPONSES_COMPATIBLE.to_string(),
            "OpenAIResponsesCompatible"
        );
        assert_eq!(
            ProviderId::ANTHROPIC_COMPATIBLE.to_string(),
            "AnthropicCompatible"
        );
        assert_eq!(ProviderId::IO_INTELLIGENCE.to_string(), "IOIntelligence");
        assert_eq!(ProviderId::CODEX.to_string(), "Codex");
        assert_eq!(ProviderId::FIREWORKS_AI.to_string(), "FireworksAI");
        assert_eq!(ProviderId::VIVGRID.to_string(), "Vivgrid");
        assert_eq!(ProviderId::OPENCODE_ZEN.to_string(), "OpenCode Zen");
        assert_eq!(ProviderId::OPENCODE_GO.to_string(), "OpenCode Go");
        assert_eq!(ProviderId::GOOGLE_AI_STUDIO.to_string(), "GoogleAIStudio");
        assert_eq!(ProviderId::NVIDIA.to_string(), "NVIDIA");
    }

    #[test]
    fn test_codex_from_str() {
        let actual = ProviderId::from_str("codex").unwrap();
        let expected = ProviderId::CODEX;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_fireworks_ai_from_str() {
        let actual = ProviderId::from_str("fireworks-ai").unwrap();
        let expected = ProviderId::FIREWORKS_AI;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_vivgrid_from_str() {
        let actual = ProviderId::from_str("vivgrid").unwrap();
        let expected = ProviderId::VIVGRID;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_opencode_go_from_str() {
        let actual = ProviderId::from_str("opencode_go").unwrap();
        let expected = ProviderId::OPENCODE_GO;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_codex_in_built_in_providers() {
        let built_in = ProviderId::built_in_providers();
        assert!(built_in.contains(&ProviderId::CODEX));
        assert!(built_in.contains(&ProviderId::OPENAI_RESPONSES_COMPATIBLE));
        assert!(built_in.contains(&ProviderId::FIREWORKS_AI));
        assert!(built_in.contains(&ProviderId::VIVGRID));
        assert!(built_in.contains(&ProviderId::OPENCODE_GO));
        assert!(built_in.contains(&ProviderId::GOOGLE_AI_STUDIO));
        assert!(built_in.contains(&ProviderId::NVIDIA));
    }

    #[test]
    fn test_google_ai_studio_from_str() {
        let actual = ProviderId::from_str("google_ai_studio").unwrap();
        let expected = ProviderId::GOOGLE_AI_STUDIO;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_modal_from_str() {
        let actual = ProviderId::from_str("modal").unwrap();
        let expected = ProviderId::MODAL;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_adal_from_str() {
        let actual = ProviderId::from_str("adal").unwrap();
        let expected = ProviderId::ADAL;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_modal_display_name() {
        assert_eq!(ProviderId::MODAL.to_string(), "Modal");
    }

    #[test]
    fn test_modal_in_built_in_providers() {
        let built_in = ProviderId::built_in_providers();
        assert!(built_in.contains(&ProviderId::MODAL));
    }

    #[test]
    fn test_adal_display_name() {
        assert_eq!(ProviderId::ADAL.to_string(), "AdaL");
    }

    #[test]
    fn test_adal_in_built_in_providers() {
        let built_in = ProviderId::built_in_providers();
        assert!(built_in.contains(&ProviderId::ADAL));
    }

    #[test]
    fn test_xiaomi_mimo_from_str() {
        let actual = ProviderId::from_str("xiaomi_mimo").unwrap();
        let expected = ProviderId::XIAOMI_MIMO;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_xiaomi_mimo_display_name() {
        assert_eq!(ProviderId::XIAOMI_MIMO.to_string(), "XiaomiMimo");
    }

    #[test]
    fn test_xiaomi_mimo_in_built_in_providers() {
        let built_in = ProviderId::built_in_providers();
        assert!(built_in.contains(&ProviderId::XIAOMI_MIMO));
    }

    #[test]
    fn test_io_intelligence() {
        let fixture = "test_key";
        let actual = io_intelligence(fixture);
        let expected = Provider {
            id: ProviderId::IO_INTELLIGENCE,
            provider_type: Default::default(),
            response: Some(ProviderResponse::OpenAI),
            url: Url::from_str("https://api.intelligence.io.solutions/api/v1/chat/completions")
                .unwrap(),
            credential: Some(AuthCredential {
                id: ProviderId::IO_INTELLIGENCE,
                auth_details: AuthDetails::ApiKey(ApiKey::from(fixture.to_string())),
                url_params: HashMap::new(),
            }),
            auth_methods: vec![crate::AuthMethod::ApiKey],
            url_params: vec![],
            models: Some(ModelSource::Url(
                Url::from_str("https://api.intelligence.io.solutions/api/v1/models").unwrap(),
            )),
            custom_headers: None,
        };
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_xai() {
        let fixture = "test_key";
        let actual = xai(fixture);
        let expected = Provider {
            id: ProviderId::XAI,
            provider_type: Default::default(),
            response: Some(ProviderResponse::OpenAI),
            url: Url::from_str("https://api.x.ai/v1/chat/completions").unwrap(),
            credential: Some(AuthCredential {
                id: ProviderId::XAI,
                auth_details: AuthDetails::ApiKey(ApiKey::from(fixture.to_string())),
                url_params: HashMap::new(),
            }),
            auth_methods: vec![crate::AuthMethod::ApiKey],
            url_params: vec![],
            models: Some(ModelSource::Url(
                Url::from_str("https://api.x.ai/v1/models").unwrap(),
            )),
            custom_headers: None,
        };
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_is_xai_with_direct_comparison() {
        let fixture_xai = xai("key");
        assert_eq!(fixture_xai.id, ProviderId::XAI);

        let fixture_other = openai("key");
        assert_ne!(fixture_other.id, ProviderId::XAI);
    }

    #[test]
    fn test_zai_coding_to_chat_url() {
        let fixture = zai_coding("test_key");
        let actual = fixture.url.clone();
        let expected = Url::parse("https://api.z.ai/api/coding/paas/v4/chat/completions").unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_zai_coding_to_model_url() {
        let fixture = zai_coding("test_key");
        let actual = fixture.models.clone();
        let expected = Some(ModelSource::Url(
            Url::parse("https://api.z.ai/api/paas/v4/models").unwrap(),
        ));
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_regular_zai_to_chat_url() {
        let fixture = zai("test_key");
        let actual = fixture.url.clone();
        let expected = Url::parse("https://api.z.ai/api/paas/v4/chat/completions").unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_regular_zai_to_model_url() {
        let fixture = zai("test_key");
        let actual = fixture.models.clone();
        let expected = Some(ModelSource::Url(
            Url::parse("https://api.z.ai/api/paas/v4/models").unwrap(),
        ));
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_vertex_ai_global_location() {
        let fixture = vertex_ai("test_token", "forge-452914", "global");
        let actual = fixture.url.clone();
        let expected = Url::parse("https://aiplatform.googleapis.com/v1/projects/forge-452914/locations/global/endpoints/openapi/chat/completions").unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_vertex_ai_regular_location() {
        let fixture = vertex_ai("test_token", "test_project", "us-central1");
        let actual = fixture.url.clone();
        let expected = Url::parse("https://us-central1-aiplatform.googleapis.com/v1/projects/test_project/locations/us-central1/endpoints/openapi/chat/completions").unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_azure_provider() {
        let fixture = azure("test_key", "my-resource", "gpt-4", "2024-02-15-preview");

        // Check chat completion URL (url field now contains the chat completion URL)
        let actual_chat = fixture.url.clone();
        let expected_chat = Url::parse("https://my-resource.openai.azure.com/openai/deployments/gpt-4/chat/completions?api-version=2024-02-15-preview").unwrap();
        assert_eq!(actual_chat, expected_chat);

        // Check model URL
        let actual_model = fixture.models.clone();
        let expected_model = Some(ModelSource::Url(
            Url::parse(
                "https://my-resource.openai.azure.com/openai/models?api-version=2024-02-15-preview",
            )
            .unwrap(),
        ));
        assert_eq!(actual_model, expected_model);

        assert_eq!(fixture.id, ProviderId::AZURE);
        assert_eq!(fixture.response, Some(ProviderResponse::OpenAI));
    }

    #[test]
    fn test_azure_provider_with_different_params() {
        let fixture = azure("another_key", "east-us", "gpt-35-turbo", "2023-05-15");

        // Check chat completion URL
        let actual_chat = fixture.url.clone();
        let expected_chat = Url::parse("https://east-us.openai.azure.com/openai/deployments/gpt-35-turbo/chat/completions?api-version=2023-05-15").unwrap();
        assert_eq!(actual_chat, expected_chat);

        // Check model URL
        let actual_model = fixture.models.clone();
        let expected_model = Some(ModelSource::Url(
            Url::parse("https://east-us.openai.azure.com/openai/models?api-version=2023-05-15")
                .unwrap(),
        ));
        assert_eq!(actual_model, expected_model);
    }
}
