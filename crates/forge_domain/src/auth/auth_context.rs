use std::collections::HashMap;

use derive_more::{Deref, From};
use url::Url;

use super::{
    ApiKey, AuthorizationCode, DeviceCode, OAuthConfig, PkceVerifier, State, URLParam,
    URLParamSpec, URLParamValue, UserCode,
};

#[derive(Debug, Clone, PartialEq, Deref, From)]
pub struct URLParameters(HashMap<URLParam, URLParamValue>);

// API Key Flow

/// Request parameters for API key authentication
#[derive(Debug, Clone)]
pub struct ApiKeyRequest {
    pub required_params: Vec<URLParamSpec>,
    pub existing_params: Option<URLParameters>,
    pub api_key: Option<ApiKey>,
}

/// Response containing API key and URL parameters
#[derive(Debug, Clone)]
pub struct ApiKeyResponse {
    pub api_key: ApiKey,
    pub url_params: HashMap<URLParam, URLParamValue>,
}

// Authorization Code Flow

/// Authorization code OAuth authentication flow
#[derive(Debug, Clone)]
pub struct CodeAuthFlow;

/// Request parameters for authorization code flow
#[derive(Debug, Clone)]
pub struct CodeRequest {
    pub authorization_url: Url,
    pub state: State,
    pub pkce_verifier: Option<PkceVerifier>,
    pub oauth_config: OAuthConfig,
}

/// Response containing authorization code
#[derive(Debug, Clone)]
pub struct CodeResponse {
    pub code: AuthorizationCode,
}

// Device Code Flow

/// Device code OAuth authentication flow
#[derive(Debug, Clone)]
pub struct DeviceCodeAuthFlow;

/// Request parameters for device code flow
#[derive(Debug, Clone)]
pub struct DeviceCodeRequest {
    pub user_code: UserCode,
    pub device_code: DeviceCode,
    pub verification_uri: Url,
    pub verification_uri_complete: Option<Url>,
    pub expires_in: u64,
    pub interval: u64,
    pub oauth_config: OAuthConfig,
}

/// Response for device code flow
#[derive(Debug, Clone)]
pub struct DeviceCodeResponse;

/// Generic container that pairs a request with its corresponding response
#[derive(Debug, Clone)]
pub struct AuthContext<Request, Response> {
    pub request: Request,
    pub response: Response,
}

/// Represents different types of authentication requests
#[derive(Debug, Clone)]
pub enum AuthContextRequest {
    ApiKey(ApiKeyRequest),
    DeviceCode(DeviceCodeRequest),
    Code(CodeRequest),
}

/// Represents completed authentication flows with their request/response pairs
#[derive(Debug, Clone)]
pub enum AuthContextResponse {
    ApiKey(AuthContext<ApiKeyRequest, ApiKeyResponse>),
    DeviceCode(AuthContext<DeviceCodeRequest, DeviceCodeResponse>),
    Code(AuthContext<CodeRequest, CodeResponse>),
}

impl AuthContextResponse {
    /// Creates an API key authentication context
    pub fn api_key(
        request: ApiKeyRequest,
        api_key: impl ToString,
        url_params: HashMap<String, String>,
    ) -> Self {
        Self::ApiKey(AuthContext {
            request,
            response: ApiKeyResponse {
                api_key: api_key.to_string().into(),
                url_params: url_params
                    .into_iter()
                    .map(|(k, v)| (k.into(), v.into()))
                    .collect(),
            },
        })
    }

    /// Creates a device code authentication context
    pub fn device_code(request: DeviceCodeRequest) -> Self {
        Self::DeviceCode(AuthContext { request, response: DeviceCodeResponse })
    }

    /// Creates an authorization code authentication context
    pub fn code(request: CodeRequest, code: impl ToString) -> Self {
        Self::Code(AuthContext {
            request,
            response: CodeResponse { code: code.to_string().into() },
        })
    }
}
