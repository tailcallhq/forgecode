use std::borrow::Cow;
use std::collections::BTreeMap;
use std::future::Future;
use std::str::FromStr;
use std::sync::{Arc, OnceLock, RwLock};

use backon::{ExponentialBuilder, Retryable};
use forge_app::McpClientInfra;
use forge_domain::{
    Environment, Image, McpHttpServer, McpServerConfig, ToolDefinition, ToolName, ToolOutput,
};
use http::{HeaderName, HeaderValue, header};
use rmcp::model::{CallToolRequestParam, ClientInfo, Implementation, InitializeRequestParam};
use rmcp::service::RunningService;
use rmcp::transport::sse_client::SseClientConfig;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::{SseClientTransport, StreamableHttpClientTransport, TokioChildProcess};
use rmcp::{RoleClient, ServiceExt};
use schemars::Schema;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

use crate::error::Error;

const VERSION: &str = match option_env!("APP_VERSION") {
    Some(val) => val,
    None => env!("CARGO_PKG_VERSION"),
};

type RmcpClient = RunningService<RoleClient, InitializeRequestParam>;

#[derive(Clone)]
pub struct ForgeMcpClient {
    client: Arc<RwLock<Option<Arc<RmcpClient>>>>,
    config: McpServerConfig,
    env_vars: BTreeMap<String, String>,
    environment: Environment,
    resolved_config: Arc<OnceLock<anyhow::Result<McpServerConfig>>>,
}

impl ForgeMcpClient {
    pub fn new(
        config: McpServerConfig,
        env_vars: &BTreeMap<String, String>,
        environment: Environment,
    ) -> Self {
        Self {
            client: Default::default(),
            config,
            env_vars: env_vars.clone(),
            environment,
            resolved_config: Arc::new(OnceLock::new()),
        }
    }

    /// Gets the resolved configuration, lazily initializing templates if needed
    fn get_resolved_config(&self) -> anyhow::Result<&McpServerConfig> {
        self.resolved_config
            .get_or_init(|| match &self.config {
                McpServerConfig::Http(http) => {
                    resolve_http_templates(http.clone(), &self.env_vars).map(McpServerConfig::Http)
                }
                x => Ok(x.clone()),
            })
            .as_ref()
            .map_err(|e| anyhow::anyhow!("{e}"))
    }

    fn client_info(&self) -> ClientInfo {
        ClientInfo {
            protocol_version: Default::default(),
            capabilities: Default::default(),
            client_info: Implementation {
                name: "Forge".to_string(),
                version: VERSION.to_string(),
                icons: None,
                title: None,
                website_url: None,
            },
        }
    }

    /// Connects to the MCP server. If `force` is true, it will reconnect even
    /// if already connected.
    async fn connect(&self) -> anyhow::Result<Arc<RmcpClient>> {
        if let Some(client) = self.get_client() {
            Ok(client.clone())
        } else {
            let client = self.create_connection().await?;
            self.set_client(client.clone());
            Ok(client.clone())
        }
    }

    fn get_client(&self) -> Option<Arc<RmcpClient>> {
        self.client.read().ok().and_then(|guard| guard.clone())
    }

    fn set_client(&self, client: Arc<RmcpClient>) {
        if let Ok(mut guard) = self.client.write() {
            *guard = Some(client);
        }
    }

    async fn create_connection(&self) -> anyhow::Result<Arc<RmcpClient>> {
        let config = self.get_resolved_config()?;
        let client = match config {
            McpServerConfig::Stdio(stdio) => {
                let mut cmd = Command::new(stdio.command.clone());

                for (key, value) in &stdio.env {
                    cmd.env(key, value);
                }

                cmd.args(&stdio.args).kill_on_drop(true);

                // Use builder pattern to capture stderr
                let (transport, stderr) = TokioChildProcess::builder(cmd)
                    .stderr(std::process::Stdio::piped())
                    .spawn()?;

                // Spawn a task to drain stderr to prevent buffer overflow
                // If stderr fills up, the child process will block
                if let Some(stderr) = stderr {
                    tokio::spawn(async move {
                        let mut reader = BufReader::new(stderr).lines();
                        while let Ok(Some(line)) = reader.next_line().await {
                            tracing::warn!("MCP server stderr: {}", line);
                        }
                    });
                }
                Arc::new(self.client_info().serve(transport).await?)
            }
            McpServerConfig::Http(http) => {
                // Check if OAuth is explicitly disabled
                if http.is_oauth_disabled() {
                    // OAuth explicitly disabled - only try standard connection
                    Arc::new(self.create_standard_http_connection(http).await?)
                } else if let Some(oauth_config) = http.oauth_config() {
                    // OAuth explicitly configured - use it directly
                    // Do NOT allow interactive auth during normal connection
                    self.create_oauth_connection(http, oauth_config, false)
                        .await?
                } else {
                    // Auto-detect: try standard first, fall back to OAuth on auth errors
                    match self.create_standard_http_connection(http).await {
                        Ok(client) => Arc::new(client),
                        Err(e) => {
                            let error_str = e.to_string().to_lowercase();
                            if error_str.contains("401")
                                || error_str.contains("unauthorized")
                                || error_str.contains("authentication required")
                                || error_str.contains("auth required")
                                || error_str.contains("oauth")
                            {
                                tracing::info!(
                                    "Standard connection failed with auth error for: {}, trying stored credentials",
                                    http.url
                                );
                                // Try OAuth with stored credentials (non-interactive)
                                // If stored credentials exist, use them; otherwise error
                                let default_config = forge_domain::McpOAuthConfig::default();
                                self.create_oauth_connection(http, &default_config, false)
                                    .await?
                            } else {
                                return Err(e);
                            }
                        }
                    }
                }
            }
        };

        Ok(client)
    }

    /// Create a standard HTTP connection without OAuth
    async fn create_standard_http_connection(
        &self,
        http: &McpHttpServer,
    ) -> anyhow::Result<RmcpClient> {
        // Try HTTP first, fall back to SSE if it fails
        let client = self.reqwest_client(http)?;
        let transport = StreamableHttpClientTransport::with_client(
            client.clone(),
            StreamableHttpClientTransportConfig::with_uri(http.url.clone()),
        );
        match self.client_info().serve(transport).await {
            Ok(client) => Ok(client),
            Err(_e) => {
                let transport = SseClientTransport::start_with_client(
                    client,
                    SseClientConfig { sse_endpoint: http.url.clone().into(), ..Default::default() },
                )
                .await?;
                Ok(self.client_info().serve(transport).await?)
            }
        }
    }

    /// Create an OAuth-enabled connection using rmcp's OAuth support.
    ///
    /// Uses rmcp's `AuthorizationManager` and `OAuthState` state machine which
    /// properly handle:
    /// 1. OAuth metadata discovery via RFC 8414
    /// 2. Dynamic client registration via RFC 7591
    /// 3. PKCE challenge/verifier generation and validation
    /// 4. CSRF state parameter generation and validation
    /// 5. Authorization code exchange for tokens
    /// 6. Token refresh via refresh_token grant
    /// 7. Token persistence via `CredentialStore` trait
    ///
    /// # Arguments
    /// * `allow_interactive` - If true, will open browser for user
    ///   authentication if no stored credentials exist. If false, returns an
    ///   error instead.
    async fn create_oauth_connection(
        &self,
        http: &McpHttpServer,
        oauth_config: &forge_domain::McpOAuthConfig,
        allow_interactive: bool,
    ) -> anyhow::Result<Arc<RmcpClient>> {
        use rmcp::transport::auth::{AuthorizationManager, OAuthState};

        use crate::auth::McpTokenStorage;

        let credential_store = McpTokenStorage::new(http.url.clone(), self.environment.clone());

        // First, try to use cached credentials with auto-refresh
        let mut auth_manager = AuthorizationManager::new(&http.url)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to create OAuth manager: {}", e))?;

        auth_manager.set_credential_store(credential_store);

        // Try to load and use stored credentials (with automatic token refresh)
        match auth_manager.initialize_from_store().await {
            Ok(true) => {
                // Stored credentials loaded. Try to get a valid access token
                // (this auto-refreshes if expired and refresh_token is available)
                match auth_manager.get_access_token().await {
                    Ok(token) => {
                        tracing::debug!("Using stored/refreshed OAuth token for: {}", http.url);
                        return self.connect_with_token(http, &token).await;
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Stored token invalid for {}: {}, re-authenticating",
                            http.url,
                            e
                        );
                    }
                }
            }
            Ok(false) => {
                tracing::info!("No stored credentials for: {}", http.url);
            }
            Err(e) => {
                tracing::warn!("Failed to load stored credentials for {}: {}", http.url, e);
            }
        }

        // No valid cached credentials
        if !allow_interactive {
            // Interactive auth not allowed - return error with instructions
            return Err(anyhow::anyhow!(
                "MCP server '{}' requires authentication. Run 'mcp login <name>' to authenticate.",
                http.url
            ));
        }

        // Interactive auth allowed - start full OAuth authorization flow
        // Create a fresh OAuthState to run the browser-based flow
        let mut oauth_state = OAuthState::new(&http.url, None)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to initialize OAuth state: {}", e))?;

        let redirect_uri = oauth_config
            .redirect_uri
            .clone()
            .unwrap_or_else(|| "http://127.0.0.1:8765/callback".to_string());

        let scopes: Vec<&str> = oauth_config.scopes.iter().map(|s| s.as_str()).collect();

        // start_authorization discovers metadata, registers client, generates PKCE +
        // CSRF state
        oauth_state
            .start_authorization(&scopes, &redirect_uri, Some("Forge"))
            .await
            .map_err(|e| anyhow::anyhow!("OAuth authorization flow failed: {}", e))?;

        // Get the authorization URL (includes PKCE challenge and CSRF state)
        let auth_url = oauth_state
            .get_authorization_url()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to get authorization URL: {}", e))?;

        tracing::info!("Starting OAuth authentication for MCP server: {}", http.url);

        // Parse redirect URI to get port for callback server
        let redirect_url: url::Url = redirect_uri
            .parse()
            .map_err(|e| anyhow::anyhow!("Invalid redirect URI: {}", e))?;
        let port = redirect_url.port().unwrap_or(8765);

        // Start local callback server, open browser, wait for redirect
        let (code, state) = self.run_oauth_callback_server(port, &auth_url).await?;

        // Exchange authorization code for tokens (validates CSRF state internally)
        // rmcp's OAuthState handles PKCE verifier inclusion in the token request
        oauth_state
            .handle_callback(&code, &state)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to exchange authorization code: {}", e))?;

        // Get the access token from the completed OAuth flow
        let access_token = oauth_state
            .get_access_token()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to get access token after OAuth: {}", e))?;

        // Save credentials for future use via our persistent store
        let credentials = oauth_state
            .get_credentials()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to get credentials: {}", e))?;

        {
            use rmcp::transport::auth::CredentialStore;
            let save_store = McpTokenStorage::new(http.url.clone(), self.environment.clone());
            let stored = rmcp::transport::auth::StoredCredentials {
                client_id: credentials.0,
                token_response: credentials.1,
            };
            save_store
                .save(stored)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to save credentials: {}", e))?;
        }

        tracing::info!(
            "OAuth authentication successful for MCP server: {}",
            http.url
        );

        self.connect_with_token(http, &access_token).await
    }

    /// Connect to an MCP server using a bearer token.
    ///
    /// Uses StreamableHTTP transport only - does NOT fall back to SSE
    /// since SSE transport doesn't support auth headers in the same way.
    /// Auth errors are transport-independent so falling back to SSE
    /// with the same auth issue would be pointless.
    async fn connect_with_token(
        &self,
        http: &McpHttpServer,
        token: &str,
    ) -> anyhow::Result<Arc<RmcpClient>> {
        let client = self.reqwest_client(http)?;
        let transport = StreamableHttpClientTransport::with_client(
            client,
            StreamableHttpClientTransportConfig::with_uri(http.url.clone()).auth_header(token),
        );

        Ok(Arc::new(self.client_info().serve(transport).await?))
    }

    /// Runs a local HTTP server to receive the OAuth callback, opens the
    /// browser, and returns the authorization code and state.
    async fn run_oauth_callback_server(
        &self,
        port: u16,
        auth_url: &str,
    ) -> anyhow::Result<(String, String)> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind(format!("127.0.0.1:{}", port))
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "Failed to start OAuth callback server on port {}: {}. \
                 Is another process using this port?",
                    port,
                    e
                )
            })?;

        tracing::info!("OAuth callback server listening on port {}", port);

        // Open browser
        if let Err(e) = open::that(auth_url) {
            tracing::warn!(
                "Failed to open browser: {}. Please open this URL manually:\n{}",
                e,
                auth_url
            );
            eprintln!(
                "\nPlease open this URL in your browser to authenticate:\n{}\n",
                auth_url
            );
        } else {
            eprintln!("\nOpening browser for OAuth authentication...\n");
        }

        // Wait for callback with timeout
        let timeout = tokio::time::Duration::from_secs(300); // 5 minutes
        let (mut stream, _addr) = tokio::time::timeout(timeout, listener.accept())
            .await
            .map_err(|_| anyhow::anyhow!("OAuth callback timed out after 5 minutes"))?
            .map_err(|e| anyhow::anyhow!("Failed to accept OAuth callback: {}", e))?;

        // Read the HTTP request
        let mut buf = vec![0u8; 4096];
        let n = stream.read(&mut buf).await?;
        let request = String::from_utf8_lossy(&buf[..n]);

        // Parse the request line to extract query parameters
        let first_line = request.lines().next().unwrap_or("");
        let path = first_line.split_whitespace().nth(1).unwrap_or("/");

        // Parse query parameters
        let query_start = path.find('?').unwrap_or(path.len());
        let query_string = &path[query_start..];
        let params: std::collections::HashMap<String, String> =
            url::form_urlencoded::parse(query_string.trim_start_matches('?').as_bytes())
                .into_owned()
                .collect();

        let code = params
            .get("code")
            .ok_or_else(|| {
                let error = params.get("error").map(|e| e.as_str()).unwrap_or("unknown");
                let desc = params
                    .get("error_description")
                    .map(|d| d.as_str())
                    .unwrap_or("No description");
                anyhow::anyhow!("OAuth error: {} - {}", error, desc)
            })?
            .clone();

        let state = params
            .get("state")
            .ok_or_else(|| anyhow::anyhow!("Missing state parameter in OAuth callback"))?
            .clone();

        // Send styled success response with auto-close
        let response_body = r#"<!doctype html><html><head><title>Forge - Authorization Successful</title><meta charset="utf-8"></head><body style="font-family:-apple-system,BlinkMacSystemFont,sans-serif;display:flex;align-items:center;justify-content:center;min-height:100vh;margin:0;background:#111827;color:#f9fafb;"><div style="text-align:center;padding:2rem;"><h1 style="margin-bottom:0.75rem;">Authorization Successful</h1><p style="color:#d1d5db;">You can close this window and return to Forge.</p></div><script>setTimeout(()=>window.close(),2000)</script></body></html>"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            response_body.len(),
            response_body
        );
        let _ = stream.write_all(response.as_bytes()).await;

        Ok((code, state))
    }

    fn reqwest_client(&self, config: &McpHttpServer) -> anyhow::Result<reqwest::Client> {
        let mut headers = header::HeaderMap::new();
        for (key, value) in config.headers.iter() {
            headers.insert(HeaderName::from_str(key)?, HeaderValue::from_str(value)?);
        }

        let client = reqwest::Client::builder().default_headers(headers);
        Ok(client.build()?)
    }

    async fn list(&self) -> anyhow::Result<Vec<ToolDefinition>> {
        let client = self.connect().await?;
        let tools = client.list_tools(None).await?;
        Ok(tools
            .tools
            .into_iter()
            .filter_map(|tool| {
                Some(
                    ToolDefinition::new(tool.name)
                        .description(tool.description.unwrap_or_default())
                        .input_schema(
                            serde_json::from_value::<Schema>(Value::Object(
                                tool.input_schema.as_ref().clone(),
                            ))
                            .ok()?,
                        ),
                )
            })
            .collect())
    }

    async fn call(&self, tool_name: &ToolName, input: &Value) -> anyhow::Result<ToolOutput> {
        let client = self.connect().await?;
        let result = client
            .call_tool(CallToolRequestParam {
                name: Cow::Owned(tool_name.to_string()),
                arguments: if let Value::Object(args) = input {
                    Some(args.clone())
                } else {
                    None
                },
            })
            .await?;

        let tool_contents: Vec<ToolOutput> = result
            .content
            .into_iter()
            .map(|content| match content.raw {
                rmcp::model::RawContent::Text(raw_text_content) => {
                    Ok(ToolOutput::text(raw_text_content.text))
                }
                rmcp::model::RawContent::Image(raw_image_content) => Ok(ToolOutput::image(
                    Image::new_base64(raw_image_content.data, raw_image_content.mime_type.as_str()),
                )),
                rmcp::model::RawContent::Resource(_) => {
                    Err(Error::UnsupportedMcpResponse("Resource").into())
                }
                rmcp::model::RawContent::ResourceLink(_) => {
                    Err(Error::UnsupportedMcpResponse("ResourceLink").into())
                }
                rmcp::model::RawContent::Audio(_) => {
                    Err(Error::UnsupportedMcpResponse("Audio").into())
                }
            })
            .collect::<anyhow::Result<Vec<ToolOutput>>>()?;

        Ok(ToolOutput::from(tool_contents.into_iter())
            .is_error(result.is_error.unwrap_or_default()))
    }

    async fn attempt_with_retry<T, F>(&self, call: impl Fn() -> F) -> anyhow::Result<T>
    where
        F: Future<Output = anyhow::Result<T>>,
    {
        call.retry(
            ExponentialBuilder::default()
                .with_max_times(5)
                .with_jitter(),
        )
        .when(|err| {
            let is_transport = err
                .downcast_ref::<rmcp::ServiceError>()
                .map(|e| {
                    matches!(
                        e,
                        rmcp::ServiceError::TransportSend(_) | rmcp::ServiceError::TransportClosed
                    )
                })
                .unwrap_or(false);

            if is_transport && let Ok(mut guard) = self.client.write() {
                guard.take();
            }

            is_transport
        })
        .await
    }
}

#[async_trait::async_trait]
impl McpClientInfra for ForgeMcpClient {
    async fn list(&self) -> anyhow::Result<Vec<ToolDefinition>> {
        self.attempt_with_retry(|| self.list()).await
    }

    async fn call(&self, tool_name: &ToolName, input: Value) -> anyhow::Result<ToolOutput> {
        self.attempt_with_retry(|| self.call(tool_name, &input))
            .await
    }
}

/// Resolves mustache templates in McpHttpServer headers using Handlebars
/// and provided environment variables
fn resolve_http_templates(
    mut http: McpHttpServer,
    env_vars: &BTreeMap<String, String>,
) -> anyhow::Result<McpHttpServer> {
    let handlebars = forge_app::TemplateEngine::handlebar_instance();

    // Create template data with env variables nested under "env"
    let template_data = serde_json::json!({"env": env_vars});

    // Resolve templates in headers
    for (_, value) in http.headers.iter_mut() {
        // Try to render the template, but keep original value if it fails
        if let Ok(resolved) = handlebars.render_template(value, &template_data) {
            *value = resolved;
        }
    }

    Ok(http)
}

/// Trigger OAuth authentication for a specific MCP server URL.
///
/// Runs the full OAuth flow: metadata discovery, dynamic registration,
/// browser-based authorization, and token persistence.
///
/// # Arguments
/// * `server_url` - The URL of the MCP server to authenticate with
/// * `env` - The environment for file system paths
pub async fn mcp_auth(server_url: &str, env: &Environment) -> anyhow::Result<()> {
    use rmcp::transport::auth::{CredentialStore, OAuthState};

    use crate::auth::McpTokenStorage;

    // Start fresh OAuth flow via OAuthState
    let mut oauth_state = OAuthState::new(server_url, None)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to initialize OAuth state: {}", e))?;

    let redirect_uri = "http://127.0.0.1:8765/callback";

    oauth_state
        .start_authorization(&[], redirect_uri, Some("Forge"))
        .await
        .map_err(|e| anyhow::anyhow!("OAuth authorization flow failed: {}", e))?;

    let auth_url = oauth_state
        .get_authorization_url()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to get authorization URL: {}", e))?;

    // Start callback server and open browser
    let listener = tokio::net::TcpListener::bind("127.0.0.1:8765")
        .await
        .map_err(|e| anyhow::anyhow!("Failed to start OAuth callback server: {}", e))?;

    if let Err(e) = open::that(&auth_url) {
        tracing::warn!("Failed to open browser: {}", e);
        eprintln!(
            "\nPlease open this URL in your browser to authenticate:\n{}\n",
            auth_url
        );
    } else {
        eprintln!("\nOpening browser for OAuth authentication...\n");
    }

    let timeout = tokio::time::Duration::from_secs(300);
    let (mut stream, _) = tokio::time::timeout(timeout, listener.accept())
        .await
        .map_err(|_| anyhow::anyhow!("OAuth callback timed out after 5 minutes"))?
        .map_err(|e| anyhow::anyhow!("Failed to accept OAuth callback: {}", e))?;

    // Read HTTP request and parse callback params
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut buf = vec![0u8; 4096];
    let n = stream.read(&mut buf).await?;
    let request = String::from_utf8_lossy(&buf[..n]);
    let first_line = request.lines().next().unwrap_or("");
    let path = first_line.split_whitespace().nth(1).unwrap_or("/");
    let query_start = path.find('?').unwrap_or(path.len());
    let params: std::collections::HashMap<String, String> =
        url::form_urlencoded::parse(path[query_start..].trim_start_matches('?').as_bytes())
            .into_owned()
            .collect();

    let code = params
        .get("code")
        .ok_or_else(|| {
            let error = params.get("error").map(|e| e.as_str()).unwrap_or("unknown");
            let desc = params
                .get("error_description")
                .map(|d| d.as_str())
                .unwrap_or("No description");
            anyhow::anyhow!("OAuth error: {} - {}", error, desc)
        })?
        .clone();

    let state = params
        .get("state")
        .ok_or_else(|| anyhow::anyhow!("Missing state parameter in OAuth callback"))?
        .clone();

    // Send styled response
    let body = r#"<!doctype html><html><head><title>Forge - Authorization Successful</title></head><body style="font-family:-apple-system,sans-serif;display:flex;align-items:center;justify-content:center;min-height:100vh;margin:0;background:#111827;color:#f9fafb;"><div style="text-align:center;"><h1>Authorization Successful</h1><p style="color:#d1d5db;">You can close this window and return to Forge.</p></div><script>setTimeout(()=>window.close(),2000)</script></body></html>"#;
    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    let _ = stream.write_all(resp.as_bytes()).await;

    // Exchange code for tokens
    oauth_state
        .handle_callback(&code, &state)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to exchange authorization code: {}", e))?;

    // Save credentials
    let credentials = oauth_state
        .get_credentials()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to get credentials: {}", e))?;

    let save_store = McpTokenStorage::new(server_url.to_string(), env.clone());
    let stored = rmcp::transport::auth::StoredCredentials {
        client_id: credentials.0,
        token_response: credentials.1,
    };
    save_store
        .save(stored)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to save credentials: {}", e))?;

    Ok(())
}

/// Remove stored OAuth credentials for a specific MCP server.
///
/// # Arguments
/// * `server_url` - The URL of the MCP server to remove credentials for
/// * `env` - The environment for file system paths
pub async fn mcp_logout(server_url: &str, env: &Environment) -> anyhow::Result<()> {
    use crate::auth::McpTokenStorage;
    let storage = McpTokenStorage::new(server_url.to_string(), env.clone());
    storage.remove_credentials().await
}

/// Remove all stored MCP OAuth credentials.
///
/// # Arguments
/// * `env` - The environment for file system paths
pub async fn mcp_logout_all(env: &Environment) -> anyhow::Result<()> {
    use crate::auth::McpCredentialStore;
    let path = McpCredentialStore::credential_path(env);
    if path.exists() {
        tokio::fs::remove_file(&path).await?;
    }
    Ok(())
}

/// Get the auth status for a specific MCP server.
///
/// Returns one of: "authenticated", "expired", "not_authenticated"
///
/// # Arguments
/// * `server_url` - The URL of the MCP server
/// * `env` - The environment for file system paths
pub async fn mcp_auth_status(server_url: &str, env: &Environment) -> String {
    use crate::auth::McpTokenStorage;
    let storage = McpTokenStorage::new(server_url.to_string(), env.clone());
    match storage.load_credentials().await {
        Ok(Some(entry)) => {
            if let Some(expires_at) = entry.tokens.expires_at {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                if expires_at <= now {
                    if entry.tokens.refresh_token.is_some() {
                        "expired (has refresh token)".to_string()
                    } else {
                        "expired".to_string()
                    }
                } else {
                    "authenticated".to_string()
                }
            } else {
                "authenticated".to_string()
            }
        }
        Ok(None) => "not authenticated".to_string(),
        Err(_) => "unknown (error reading credentials)".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn test_resolve_http_templates_with_env() {
        let env_vars = BTreeMap::from([
            ("GH_TOKEN".to_string(), "secret_token_123".to_string()),
            ("API_KEY".to_string(), "api_key_456".to_string()),
        ]);

        let http = McpHttpServer {
            url: "https://api.example.com".to_string(),
            headers: BTreeMap::from([
                (
                    "Authorization".to_string(),
                    "Bearer {{env.GH_TOKEN}}".to_string(),
                ),
                ("X-API-Key".to_string(), "{{env.API_KEY}}".to_string()),
                ("Content-Type".to_string(), "application/json".to_string()),
            ]),
            timeout: None,
            disable: false,
            oauth: Default::default(),
        };

        let resolved = resolve_http_templates(http, &env_vars).unwrap();

        assert_eq!(
            resolved.headers.get("Authorization"),
            Some(&"Bearer secret_token_123".to_string())
        );
        assert_eq!(
            resolved.headers.get("X-API-Key"),
            Some(&"api_key_456".to_string())
        );
        assert_eq!(
            resolved.headers.get("Content-Type"),
            Some(&"application/json".to_string())
        );
    }

    #[test]
    fn test_resolve_http_templates_missing_env_var() {
        let env_vars = BTreeMap::new(); // Empty env vars

        let http = McpHttpServer {
            url: "https://api.example.com".to_string(),
            headers: BTreeMap::from([(
                "Authorization".to_string(),
                "Bearer {{env.MISSING_VAR}}".to_string(),
            )]),
            timeout: None,
            disable: false,
            oauth: Default::default(),
        };

        let resolved = resolve_http_templates(http, &env_vars).unwrap();

        // Should keep original value if template rendering fails
        assert_eq!(
            resolved.headers.get("Authorization"),
            Some(&"Bearer {{env.MISSING_VAR}}".to_string())
        );
    }

    #[test]
    fn test_resolve_http_templates_preserves_url_and_disable() {
        let env_vars = BTreeMap::from([("TOKEN".to_string(), "test".to_string())]);

        let http = McpHttpServer {
            url: "https://test.example.com".to_string(),
            headers: BTreeMap::from([("Auth".to_string(), "{{env.TOKEN}}".to_string())]),
            timeout: None,
            disable: true,
            oauth: Default::default(),
        };

        let resolved = resolve_http_templates(http, &env_vars).unwrap();

        assert_eq!(resolved.url, "https://test.example.com");
        assert_eq!(resolved.disable, true);
        assert_eq!(resolved.headers.get("Auth"), Some(&"test".to_string()));
    }
}
