use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr, TcpListener};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use forge_domain::CodeRequest;
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};
use url::{Host, Url};

/// Maximum time to wait for the OAuth browser callback before giving up.
const OAUTH_CALLBACK_TIMEOUT: Duration = Duration::from_secs(300);
const CALLBACK_POLL_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, PartialEq, Eq)]
struct OAuthCallbackPayload {
    code: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum OAuthCallbackParseResult {
    PathMismatch,
    InvalidRequest(String),
    OAuthError(String),
    Success(OAuthCallbackPayload),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CallbackRequestDisposition {
    Continue {
        status_code: StatusCode,
        message: String,
    },
    Fail {
        status_code: StatusCode,
        message: String,
    },
    Complete(OAuthCallbackPayload),
}

/// Localhost OAuth callback server that waits for a browser redirect and
/// returns the authorization code.
pub(crate) struct LocalhostOAuthCallbackServer {
    redirect_uri: Url,
    server: Arc<Server>,
    shutdown: Arc<AtomicBool>,
    task: Option<tokio::task::JoinHandle<anyhow::Result<String>>>,
}

impl LocalhostOAuthCallbackServer {
    /// Starts a localhost OAuth callback server when the request uses a
    /// localhost redirect URI.
    ///
    /// Returns `Ok(None)` when the request is not configured for a localhost
    /// callback.
    ///
    /// # Errors
    ///
    /// Returns an error if the localhost redirect URI is invalid or the HTTP
    /// listener cannot be bound.
    pub(crate) fn start(request: &CodeRequest) -> anyhow::Result<Option<Self>> {
        let Some(redirect_uri) = localhost_oauth_redirect_uri(request) else {
            return Ok(None);
        };

        let listener = TcpListener::bind(localhost_oauth_bind_addr(&redirect_uri)?)?;
        let callback_path = redirect_uri.path().to_string();
        let expected_state = request.state.to_string();
        let server = Arc::new(Server::from_listener(listener, None).map_err(|e| {
            anyhow::anyhow!("Failed to start localhost OAuth callback server: {e}")
        })?);
        let shutdown = Arc::new(AtomicBool::new(false));
        let task = tokio::task::spawn_blocking({
            let server = Arc::clone(&server);
            let shutdown = Arc::clone(&shutdown);
            move || {
                wait_for_localhost_oauth_callback(server, callback_path, expected_state, shutdown)
            }
        });

        Ok(Some(Self {
            redirect_uri,
            server,
            shutdown,
            task: Some(task),
        }))
    }

    /// Returns the redirect URI the callback server is listening on.
    pub(crate) fn redirect_uri(&self) -> &Url {
        &self.redirect_uri
    }

    /// Waits for the browser callback and returns the authorization code.
    ///
    /// # Errors
    ///
    /// Returns an error when the background task fails or the callback request
    /// is invalid.
    pub(crate) async fn wait_for_code(mut self) -> anyhow::Result<String> {
        self.task
            .take()
            .expect("OAuth callback task should exist")
            .await
            .map_err(|e| anyhow::anyhow!("OAuth callback task failed: {e}"))?
    }
}

impl Drop for LocalhostOAuthCallbackServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        self.server.unblock();
    }
}

fn escape_html(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn localhost_oauth_redirect_uri(request: &CodeRequest) -> Option<Url> {
    request
        .oauth_config
        .redirect_uri
        .as_ref()
        .and_then(|uri| Url::parse(uri).ok())
        .filter(|uri| {
            uri.scheme() == "http"
                && uri.port().is_some()
                && (matches!(uri.host(), Some(Host::Domain("localhost")))
                    || uri.host().is_some_and(|host| match host {
                        Host::Ipv4(ip) => ip.is_loopback(),
                        Host::Ipv6(ip) => ip.is_loopback(),
                        Host::Domain(_) => false,
                    }))
        })
}

fn localhost_oauth_bind_addr(redirect_uri: &Url) -> anyhow::Result<SocketAddr> {
    let port = redirect_uri
        .port()
        .ok_or_else(|| anyhow::anyhow!("OAuth redirect URI is missing an explicit port"))?;

    match redirect_uri.host() {
        Some(Host::Domain("localhost")) => Ok(SocketAddr::from(([127, 0, 0, 1], port))),
        Some(Host::Ipv4(ip)) if ip.is_loopback() => Ok(SocketAddr::new(IpAddr::V4(ip), port)),
        Some(Host::Ipv6(ip)) if ip.is_loopback() => Ok(SocketAddr::new(IpAddr::V6(ip), port)),
        Some(_) => anyhow::bail!("OAuth redirect URI host must be localhost or loopback"),
        None => anyhow::bail!("OAuth redirect URI is missing a host"),
    }
}

fn oauth_callback_success_page() -> String {
    "<!doctype html><html><head><title>ForgeCode Authorization Successful</title><meta charset=\"utf-8\"></head><body style=\"font-family: -apple-system, BlinkMacSystemFont, sans-serif; display:flex; align-items:center; justify-content:center; min-height:100vh; margin:0; background:#111827; color:#f9fafb;\"><div style=\"text-align:center; padding:2rem;\"><h1 style=\"margin-bottom:0.75rem;\">Authorization Successful</h1><p style=\"color:#d1d5db;\">You can close this window and return to ForgeCode.</p></div></body></html>".to_string()
}

fn oauth_callback_error_page(message: &str) -> String {
    format!(
        "<!doctype html><html><head><title>ForgeCode Authorization Failed</title><meta charset=\"utf-8\"></head><body style=\"font-family: -apple-system, BlinkMacSystemFont, sans-serif; display:flex; align-items:center; justify-content:center; min-height:100vh; margin:0; background:#111827; color:#f9fafb;\"><div style=\"text-align:center; padding:2rem; max-width:42rem;\"><h1 style=\"margin-bottom:0.75rem; color:#fca5a5;\">Authorization Failed</h1><p style=\"color:#d1d5db;\">ForgeCode could not complete sign-in.</p><pre style=\"white-space:pre-wrap; word-break:break-word; margin-top:1rem; padding:1rem; border-radius:0.5rem; background:#1f2937; color:#fca5a5;\">{}</pre></div></body></html>",
        escape_html(message)
    )
}

fn html_response(status_code: StatusCode, body: String) -> Response<std::io::Cursor<Vec<u8>>> {
    Response::from_string(body)
        .with_status_code(status_code)
        .with_header(response_header("Content-Type", "text/html; charset=utf-8"))
        .with_header(response_header("Cache-Control", "no-store"))
        .with_header(response_header("X-Content-Type-Options", "nosniff"))
}

fn response_header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes())
        .expect("static HTTP header should be valid")
}

fn parse_oauth_callback_target(
    request_target: &str,
    expected_path: &str,
    expected_state: &str,
) -> OAuthCallbackParseResult {
    let Some(request_target) = request_target.strip_prefix('/') else {
        return OAuthCallbackParseResult::InvalidRequest(
            "Malformed OAuth callback request target".to_string(),
        );
    };

    let callback_url = match Url::parse(&format!("http://localhost/{request_target}")) {
        Ok(url) => url,
        Err(_) => {
            return OAuthCallbackParseResult::InvalidRequest(
                "Malformed OAuth callback request target".to_string(),
            );
        }
    };

    if callback_url.path() != expected_path {
        return OAuthCallbackParseResult::PathMismatch;
    }

    let params: HashMap<String, String> = callback_url.query_pairs().into_owned().collect();
    if let Some(error) = params.get("error") {
        let detail = params
            .get("error_description")
            .filter(|value| !value.trim().is_empty())
            .map(|value| format!(": {value}"))
            .unwrap_or_default();
        return OAuthCallbackParseResult::OAuthError(format!(
            "Authorization failed ({error}{detail})"
        ));
    }

    let Some(state) = params.get("state").filter(|value| !value.trim().is_empty()) else {
        return OAuthCallbackParseResult::InvalidRequest(
            "Missing OAuth state in callback".to_string(),
        );
    };
    if state != expected_state {
        return OAuthCallbackParseResult::InvalidRequest(
            "OAuth state mismatch. Please try again.".to_string(),
        );
    }

    let Some(code) = params
        .get("code")
        .filter(|value| !value.trim().is_empty())
        .cloned()
    else {
        return OAuthCallbackParseResult::InvalidRequest(
            "Missing authorization code in callback".to_string(),
        );
    };

    OAuthCallbackParseResult::Success(OAuthCallbackPayload { code })
}

fn classify_callback_request(
    request: &Request,
    expected_path: &str,
    expected_state: &str,
) -> CallbackRequestDisposition {
    if request
        .remote_addr()
        .is_some_and(|remote_addr| !remote_addr.ip().is_loopback())
    {
        return CallbackRequestDisposition::Continue {
            status_code: StatusCode(403),
            message: "Only loopback callback requests are accepted.".to_string(),
        };
    }

    if request.method() != &Method::Get {
        return CallbackRequestDisposition::Continue {
            status_code: StatusCode(405),
            message: "Only GET requests are supported for OAuth callbacks.".to_string(),
        };
    }

    match parse_oauth_callback_target(request.url(), expected_path, expected_state) {
        OAuthCallbackParseResult::PathMismatch => CallbackRequestDisposition::Continue {
            status_code: StatusCode(404),
            message: "Received a request for an unexpected callback path.".to_string(),
        },
        OAuthCallbackParseResult::InvalidRequest(message) => {
            CallbackRequestDisposition::Continue { status_code: StatusCode(400), message }
        }
        OAuthCallbackParseResult::OAuthError(message) => {
            CallbackRequestDisposition::Fail { status_code: StatusCode(400), message }
        }
        OAuthCallbackParseResult::Success(payload) => CallbackRequestDisposition::Complete(payload),
    }
}

fn respond_to_callback_request(
    request: Request,
    disposition: &CallbackRequestDisposition,
) -> anyhow::Result<()> {
    let response = match disposition {
        CallbackRequestDisposition::Continue { status_code, message }
        | CallbackRequestDisposition::Fail { status_code, message } => {
            html_response(*status_code, oauth_callback_error_page(message))
        }
        CallbackRequestDisposition::Complete(_) => {
            html_response(StatusCode(200), oauth_callback_success_page())
        }
    };

    request.respond(response)?;
    Ok(())
}

fn wait_for_localhost_oauth_callback(
    server: Arc<Server>,
    expected_path: String,
    expected_state: String,
    shutdown: Arc<AtomicBool>,
) -> anyhow::Result<String> {
    let deadline = Instant::now() + OAUTH_CALLBACK_TIMEOUT;

    loop {
        if shutdown.load(Ordering::Relaxed) {
            anyhow::bail!("OAuth callback listener was cancelled");
        }

        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Timed out waiting for OAuth callback after {} seconds",
                    OAUTH_CALLBACK_TIMEOUT.as_secs()
                )
            })?;

        let timeout = remaining.min(CALLBACK_POLL_INTERVAL);
        let Some(request) = server.recv_timeout(timeout)? else {
            continue;
        };

        let actual = classify_callback_request(&request, &expected_path, &expected_state);
        let _ = respond_to_callback_request(request, &actual);

        match actual {
            CallbackRequestDisposition::Continue { .. } => continue,
            CallbackRequestDisposition::Fail { message, .. } => anyhow::bail!(message),
            CallbackRequestDisposition::Complete(payload) => return Ok(payload.code),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    use std::thread;

    use forge_domain::{OAuthConfig, PkceVerifier, State};
    use pretty_assertions::assert_eq;

    use super::*;

    fn sample_code_request(authorization_url: &str) -> CodeRequest {
        CodeRequest {
            authorization_url: Url::parse(authorization_url).unwrap(),
            state: State::from("expected-state".to_string()),
            pkce_verifier: Some(PkceVerifier::from("verifier".to_string())),
            oauth_config: OAuthConfig {
                auth_url: Url::parse("https://auth.openai.com/oauth/authorize").unwrap(),
                token_url: Url::parse("https://auth.openai.com/oauth/token").unwrap(),
                client_id: "client-id".to_string().into(),
                scopes: vec!["openid".to_string()],
                redirect_uri: Some("http://localhost:1455/auth/callback".to_string()),
                use_pkce: true,
                token_refresh_url: None,
                custom_headers: None,
                extra_auth_params: None,
            },
        }
    }

    fn sample_callback_server() -> (Arc<Server>, SocketAddr, Arc<AtomicBool>) {
        let fixture = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = fixture.local_addr().unwrap();
        let server = Arc::new(Server::from_listener(fixture, None).unwrap());
        let shutdown = Arc::new(AtomicBool::new(false));
        (server, addr, shutdown)
    }

    fn send_http_request(addr: SocketAddr, request: &str) -> String {
        let mut fixture = TcpStream::connect(addr).unwrap();
        fixture.write_all(request.as_bytes()).unwrap();
        fixture.shutdown(Shutdown::Write).unwrap();

        let mut actual = String::new();
        fixture.read_to_string(&mut actual).unwrap();
        actual
    }

    #[test]
    fn extracts_localhost_redirect_uri_from_oauth_request() {
        let setup = sample_code_request(
            "https://auth.openai.com/oauth/authorize?client_id=test&redirect_uri=http%3A%2F%2Flocalhost%3A1455%2Fauth%2Fcallback&state=expected-state",
        );

        let actual = localhost_oauth_redirect_uri(&setup).unwrap();

        let expected = "http://localhost:1455/auth/callback";
        assert_eq!(actual.as_str(), expected);
    }

    #[test]
    fn extracts_ipv6_loopback_redirect_uri_from_oauth_request() {
        let mut setup = sample_code_request("https://auth.openai.com/oauth/authorize");
        setup.oauth_config.redirect_uri = Some("http://[::1]:1455/auth/callback".to_string());

        let actual = localhost_oauth_redirect_uri(&setup).unwrap();

        let expected = "http://[::1]:1455/auth/callback";
        assert_eq!(actual.as_str(), expected);
    }

    #[test]
    fn captures_authorization_code_from_localhost_callback() {
        let setup = sample_callback_server();
        let server = Arc::clone(&setup.0);
        let addr = setup.1;
        let shutdown = Arc::clone(&setup.2);
        let fixture = thread::spawn(move || {
            send_http_request(
                addr,
                "GET /auth/callback?code=auth-code&state=expected-state HTTP/1.1\r\nHost: localhost\r\n\r\n",
            )
        });

        let actual = wait_for_localhost_oauth_callback(
            server,
            "/auth/callback".to_string(),
            "expected-state".to_string(),
            shutdown,
        )
        .unwrap();

        let response = fixture.join().unwrap();
        let expected = "auth-code".to_string();
        assert_eq!(actual, expected);
        assert!(response.contains("200 OK"));
    }

    #[test]
    fn keeps_listening_after_invalid_state_until_valid_callback_arrives() {
        let setup = sample_callback_server();
        let server = Arc::clone(&setup.0);
        let addr = setup.1;
        let shutdown = Arc::clone(&setup.2);
        let fixture = thread::spawn(move || {
            let first = send_http_request(
                addr,
                "GET /auth/callback?code=auth-code&state=wrong-state HTTP/1.1\r\nHost: localhost\r\n\r\n",
            );
            let second = send_http_request(
                addr,
                "GET /auth/callback?code=auth-code&state=expected-state HTTP/1.1\r\nHost: localhost\r\n\r\n",
            );
            (first, second)
        });

        let actual = wait_for_localhost_oauth_callback(
            server,
            "/auth/callback".to_string(),
            "expected-state".to_string(),
            shutdown,
        )
        .unwrap();

        let responses = fixture.join().unwrap();
        let expected = "auth-code".to_string();
        assert_eq!(actual, expected);
        assert!(responses.0.contains("400 Bad Request"));
        assert!(responses.1.contains("200 OK"));
    }

    #[test]
    fn keeps_listening_after_invalid_method_until_valid_callback_arrives() {
        let setup = sample_callback_server();
        let server = Arc::clone(&setup.0);
        let addr = setup.1;
        let shutdown = Arc::clone(&setup.2);
        let fixture = thread::spawn(move || {
            let first = send_http_request(
                addr,
                "POST /auth/callback?code=auth-code&state=expected-state HTTP/1.1\r\nHost: localhost\r\nContent-Length: 0\r\n\r\n",
            );
            let second = send_http_request(
                addr,
                "GET /auth/callback?code=auth-code&state=expected-state HTTP/1.1\r\nHost: localhost\r\n\r\n",
            );
            (first, second)
        });

        let actual = wait_for_localhost_oauth_callback(
            server,
            "/auth/callback".to_string(),
            "expected-state".to_string(),
            shutdown,
        )
        .unwrap();

        let responses = fixture.join().unwrap();
        let expected = "auth-code".to_string();
        assert_eq!(actual, expected);
        assert!(responses.0.contains("405 Method Not Allowed"));
        assert!(responses.1.contains("200 OK"));
    }

    #[test]
    fn stops_when_provider_returns_terminal_oauth_error() {
        let setup = sample_callback_server();
        let server = Arc::clone(&setup.0);
        let addr = setup.1;
        let shutdown = Arc::clone(&setup.2);
        let fixture = thread::spawn(move || {
            send_http_request(
                addr,
                "GET /auth/callback?error=access_denied&error_description=user%20cancelled HTTP/1.1\r\nHost: localhost\r\n\r\n",
            )
        });

        let actual = wait_for_localhost_oauth_callback(
            server,
            "/auth/callback".to_string(),
            "expected-state".to_string(),
            shutdown,
        )
        .unwrap_err();

        let response = fixture.join().unwrap();
        let expected = "Authorization failed (access_denied: user cancelled)";
        assert_eq!(actual.to_string(), expected);
        assert!(response.contains("400 Bad Request"));
    }
}
