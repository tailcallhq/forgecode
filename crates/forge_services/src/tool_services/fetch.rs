use std::env;
use std::fs;

use anyhow::{Context, anyhow};
use forge_app::{HttpResponse, NetFetchService, ResponseContext, is_binary_content_type};
use reqwest::{Client, Certificate, Url};
use tracing::warn;

/// Retrieves content from URLs as markdown or raw text. Enables access to
/// current online information including websites, APIs and documentation. Use
/// for obtaining up-to-date information beyond training data, verifying facts,
/// or retrieving specific online content. Handles HTTP/HTTPS and converts HTML
/// to readable markdown by default. Cannot access private/restricted resources
/// requiring authentication. Respects robots.txt and may be blocked by
/// anti-scraping measures. For large pages, returns the first 40,000 characters
/// and stores the complete content in a temporary file for subsequent access.
#[derive(Debug)]
pub struct ForgeFetch {
    client: Client,
}

impl Default for ForgeFetch {
    fn default() -> Self {
        Self::new()
    }
}

impl ForgeFetch {
    pub fn new() -> Self {
        // Explicitly disable hickory-dns so that DNS resolution is delegated to
        // the system resolver (or the configured HTTP proxy). The workspace
        // enables the `hickory-dns` cargo feature for reqwest, which causes
        // `Client::new()` to perform direct DNS lookups that bypass
        // HTTP_PROXY / HTTPS_PROXY — breaking connectivity in corporate proxy
        // environments.
        let mut builder = Client::builder().hickory_dns(false);

        // Load additional CA certificates so the fetch tool works in corporate
        // proxy environments that perform TLS interception (MITM). Without
        // these, rustls only trusts the webpki-roots bundle and rejects the
        // proxy's re-signed certificates.
        //
        // Certificates are loaded from, in order:
        //   1. FORGE_ROOT_CERT_PATHS  — comma-separated list of PEM/DER paths
        //   2. SSL_CERT_FILE          — single PEM bundle (common convention)
        //   3. NODE_EXTRA_CA_CERTS    — single PEM file (Node.js convention)
        //   4. REQUESTS_CA_BUNDLE     — single PEM bundle (Python convention)
        let cert_paths: Vec<String> = if let Ok(val) = env::var("FORGE_ROOT_CERT_PATHS") {
            val.split(',').map(|s| s.trim().to_string()).collect()
        } else if let Ok(val) = env::var("SSL_CERT_FILE") {
            vec![val]
        } else if let Ok(val) = env::var("NODE_EXTRA_CA_CERTS") {
            vec![val]
        } else if let Ok(val) = env::var("REQUESTS_CA_BUNDLE") {
            vec![val]
        } else {
            vec![]
        };

        for cert_path in &cert_paths {
            match fs::read(cert_path) {
                Ok(buf) => {
                    // A PEM file may contain multiple certificates (bundle).
                    // reqwest's Certificate::from_pem only parses the first one,
                    // so we use from_pem_bundle to load them all.
                    match Certificate::from_pem_bundle(&buf) {
                        Ok(certs) => {
                            for cert in certs {
                                builder = builder.add_root_certificate(cert);
                            }
                        }
                        Err(_) => {
                            // Fall back to single PEM, then DER
                            if let Ok(cert) = Certificate::from_pem(&buf) {
                                builder = builder.add_root_certificate(cert);
                            } else if let Ok(cert) = Certificate::from_der(&buf) {
                                builder = builder.add_root_certificate(cert);
                            } else {
                                warn!(
                                    "Failed to parse certificate from {}, skipping",
                                    cert_path
                                );
                            }
                        }
                    }
                }
                Err(err) => {
                    warn!(
                        "Failed to read certificate file {}: {}, skipping",
                        cert_path, err
                    );
                }
            }
        }

        let client = builder.build().expect("failed to build fetch HTTP client");
        Self { client }
    }
}

impl ForgeFetch {
    async fn check_robots_txt(&self, url: &Url) -> anyhow::Result<()> {
        let robots_url = format!("{}://{}/robots.txt", url.scheme(), url.authority());
        let robots_response = self.client.get(&robots_url).send().await;

        if let Ok(robots) = robots_response
            && robots.status().is_success()
        {
            let robots_content = robots.text().await.unwrap_or_default();
            let path = url.path();
            for line in robots_content.lines() {
                if let Some(disallowed) = line.strip_prefix("Disallow: ") {
                    let disallowed = disallowed.trim();
                    let disallowed = if !disallowed.starts_with('/') {
                        format!("/{disallowed}")
                    } else {
                        disallowed.to_string()
                    };
                    let path = if !path.starts_with('/') {
                        format!("/{path}")
                    } else {
                        path.to_string()
                    };
                    if path.starts_with(&disallowed) {
                        return Err(anyhow!(
                            "URL {url} cannot be fetched due to robots.txt restrictions"
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    async fn fetch_url(&self, url: &Url, force_raw: bool) -> anyhow::Result<HttpResponse> {
        self.check_robots_txt(url).await?;

        let response = self
            .client
            .get(url.as_str())
            .send()
            .await
            .map_err(|e| anyhow!("Failed to fetch URL {url}: {e}"))?;
        let code = response.status().as_u16();

        if !response.status().is_success() {
            return Err(anyhow!(
                "Failed to fetch {} - status code {}",
                url,
                response.status()
            ));
        }

        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();

        // Detect binary content types before attempting to read as text.
        // The fetch tool is designed for text/HTML content only.
        if is_binary_content_type(&content_type) {
            return Err(anyhow!(
                "URL {} returns binary content (Content-Type: {}). \
                 The fetch tool only handles text content. \
                 Use the shell tool with `curl -fLo <output_file> <url>` to download binary files.",
                url,
                content_type
            ));
        }

        let page_raw = response
            .text()
            .await
            .map_err(|e| anyhow!("Failed to read response content from {url}: {e}"))?;

        // Use floor_char_boundary to avoid panicking on multi-byte UTF-8 chars
        let sniff_end = if page_raw.len() >= 100 {
            // Find the nearest char boundary at or before byte index 100
            let mut end = 100;
            while end > 0 && !page_raw.is_char_boundary(end) {
                end -= 1;
            }
            end
        } else {
            page_raw.len()
        };
        let is_page_html = page_raw
            .get(..sniff_end)
            .map(|s| s.contains("<html"))
            .unwrap_or(false)
            || content_type.contains("text/html")
            || content_type.is_empty();

        if is_page_html && !force_raw {
            let content = html2md::parse_html(&page_raw);
            Ok(HttpResponse { content, context: ResponseContext::Raw, code, content_type })
        } else {
            Ok(HttpResponse {
                content: page_raw,
                context: ResponseContext::Parsed,
                code,
                content_type,
            })
        }
    }
}

#[async_trait::async_trait]
impl NetFetchService for ForgeFetch {
    async fn fetch(&self, url: String, raw: Option<bool>) -> anyhow::Result<HttpResponse> {
        let url = Url::parse(&url).with_context(|| format!("Failed to parse URL: {url}"))?;

        self.fetch_url(&url, raw.unwrap_or(false)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_binary_content_type_text_types_are_not_binary() {
        assert!(!is_binary_content_type("text/html"));
        assert!(!is_binary_content_type("text/plain"));
        assert!(!is_binary_content_type("text/css"));
        assert!(!is_binary_content_type("application/json"));
        assert!(!is_binary_content_type("application/xml"));
        assert!(!is_binary_content_type("application/javascript"));
        assert!(!is_binary_content_type("application/yaml"));
        assert!(!is_binary_content_type("image/svg+xml"));
        assert!(!is_binary_content_type("text/csv"));
        assert!(!is_binary_content_type("text/markdown"));
        assert!(!is_binary_content_type("")); // empty = unknown, allow
    }

    #[test]
    fn test_is_binary_content_type_binary_types_detected() {
        assert!(is_binary_content_type("application/gzip"));
        assert!(is_binary_content_type("application/x-gzip"));
        assert!(is_binary_content_type("application/octet-stream"));
        assert!(is_binary_content_type("application/zip"));
        assert!(is_binary_content_type("application/x-tar"));
        assert!(is_binary_content_type("application/pdf"));
        assert!(is_binary_content_type("image/png"));
        assert!(is_binary_content_type("image/jpeg"));
        assert!(is_binary_content_type("audio/mpeg"));
        assert!(is_binary_content_type("video/mp4"));
    }

    #[test]
    fn test_is_binary_content_type_case_insensitive() {
        assert!(!is_binary_content_type("Application/JSON"));
        assert!(!is_binary_content_type("TEXT/HTML; charset=utf-8"));
        assert!(is_binary_content_type("Application/Gzip"));
    }
}
