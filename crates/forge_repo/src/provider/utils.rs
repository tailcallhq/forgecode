use anyhow::Context as _;
use reqwest::header::HeaderMap;
use reqwest::{Response, StatusCode, Url};

/// Helper function to format HTTP request/response context for logging and
/// error reporting
pub(crate) fn format_http_context<U: AsRef<str>>(
    status: Option<StatusCode>,
    method: &str,
    url: U,
) -> String {
    if let Some(status) = status {
        format!("{} {} {}", status.as_u16(), method, url.as_ref())
    } else {
        format!("{} {}", method, url.as_ref())
    }
}

/// Reads an HTTP error response body and formats a human-readable reason.
///
/// Returns the status code as a `u16` and a formatted string of the form
/// `"<status> Reason: <body>"`, falling back to `"<status> Reason: [Unknown]"`
/// when the body cannot be read or is empty.
pub async fn read_http_error_reason(response: Response) -> (u16, String) {
    let status = response.status();
    let body = response.text().await.ok().filter(|b| !b.is_empty());
    let reason = match body {
        Some(b) => format!("{status} Reason: {b}"),
        None => format!("{status} Reason: [Unknown]"),
    };
    (status.as_u16(), reason)
}

/// Joins a base URL with a path, validating the path for security
///
/// # Errors
///
/// Returns an error if the path contains forbidden patterns or if URL parsing
/// fails
pub fn join_url(base_url: &str, path: &str) -> anyhow::Result<Url> {
    // Validate the path doesn't contain certain patterns
    if path.contains("://") || path.contains("..") {
        anyhow::bail!("Invalid path: Contains forbidden patterns");
    }

    // Remove leading slash to avoid double slashes
    let path = path.trim_start_matches('/');

    let url = Url::parse(base_url)
        .with_context(|| format!("Failed to parse base URL: {base_url}"))?
        .join(path)
        .with_context(|| format!("Failed to append {path} to base URL: {base_url}"))?;
    Ok(url)
}

/// Creates a HeaderMap from a vector of header key-value pairs
pub fn create_headers(headers: Vec<(String, String)>) -> HeaderMap {
    let mut header_map = HeaderMap::new();
    for (key, value) in headers {
        let header_name =
            reqwest::header::HeaderName::from_bytes(key.as_bytes()).expect("Invalid header name");
        let header_value = value.parse().expect("Invalid header value");
        header_map.insert(header_name, header_value);
    }
    header_map
}
