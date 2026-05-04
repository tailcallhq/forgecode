use mockito::{Mock, Server, ServerGuard};

pub struct MockServer {
    server: ServerGuard,
}

impl MockServer {
    pub async fn new() -> Self {
        let server = Server::new_async().await;
        Self { server }
    }

    pub async fn mock_models(&mut self, body: serde_json::Value, status: usize) -> Mock {
        self.server
            .mock("GET", "/models")
            .with_status(status)
            .with_header("content-type", "application/json")
            .with_body(body.to_string())
            .create_async()
            .await
    }

    pub fn url(&self) -> String {
        self.server.url()
    }

    /// Mock any POST path returning the given status code and JSON body with
    /// `Content-Type: application/json`.  Used to simulate a provider API
    /// returning a non-2xx error so the SSE client fires `InvalidStatusCode`.
    pub async fn mock_post_error(&mut self, path: &str, body: &str, status: usize) -> Mock {
        self.server
            .mock("POST", path)
            .with_status(status)
            .with_header("content-type", "application/json")
            .with_body(body)
            .create_async()
            .await
    }

    /// Mock any POST path returning 200 with `Content-Type: application/json`
    /// (not `text/event-stream`).  Used to simulate an `InvalidContentType`
    /// error from the SSE client.
    pub async fn mock_post_wrong_content_type(&mut self, path: &str, body: &str) -> Mock {
        self.server
            .mock("POST", path)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(body)
            .create_async()
            .await
    }

    pub async fn mock_responses_stream(&mut self, events: Vec<String>, status: usize) -> Mock {
        let sse_body = events.join("\n\n");
        self.server
            .mock("POST", "/v1/responses")
            .with_status(status)
            .with_header("content-type", "text/event-stream")
            .with_header("cache-control", "no-cache")
            .with_body(sse_body)
            .create_async()
            .await
    }

    /// Mock SSE responses without `Content-Type: text/event-stream`.
    /// Simulates the Codex backend behavior where SSE data is returned
    /// as `application/octet-stream` instead of `text/event-stream`.
    pub async fn mock_codex_responses_stream(
        &mut self,
        path: &str,
        events: Vec<String>,
        status: usize,
    ) -> Mock {
        let sse_body = events.join("\n\n");
        self.server
            .mock("POST", path)
            .with_status(status)
            .with_header("content-type", "application/octet-stream")
            .with_header("cache-control", "no-cache")
            .with_body(sse_body)
            .create_async()
            .await
    }

    pub async fn mock_google_chat_stream(
        &mut self,
        model: &str,
        events: Vec<String>,
        status: usize,
    ) -> Mock {
        let mut sse_body = events.join("\n\n");
        sse_body.push_str("\n\n");
        let path = format!("/models/{}:streamGenerateContent", model);
        self.server
            .mock("POST", path.as_str())
            .match_query(mockito::Matcher::UrlEncoded("alt".into(), "sse".into()))
            .with_status(status)
            .with_header("content-type", "text/event-stream")
            .with_header("cache-control", "no-cache")
            .with_body(sse_body)
            .create_async()
            .await
    }
}

/// Normalize dynamic addresses in messages for testing/logging.
pub fn normalize_ports(input: String) -> String {
    use regex::Regex;

    let re_ip_port = Regex::new(r"127\.0\.0\.1:\d+").unwrap();
    let re_http = Regex::new(r"http://127\.0\.0\.1:\d+").unwrap();

    let normalized = re_http.replace_all(&input, "http://127.0.0.1:<port>");
    let normalized = re_ip_port.replace_all(&normalized, "127.0.0.1:<port>");

    normalized.to_string()
}
