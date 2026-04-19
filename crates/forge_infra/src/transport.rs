use bytes::Bytes;
use futures::TryStreamExt;
use launchdarkly_sdk_transport::{ByteStream, HttpTransport, ResponseFuture, TransportError};
use reqwest::Client;

fn to_http_headers(
    headers: &reqwest::header::HeaderMap,
) -> Result<http::HeaderMap, TransportError> {
    headers
        .iter()
        .try_fold(http::HeaderMap::new(), |mut mapped, (name, value)| {
            let header_name = http::header::HeaderName::from_bytes(name.as_str().as_bytes())
                .map_err(|error| {
                    TransportError::new(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("Invalid response header name '{}': {}", name, error),
                    ))
                })?;
            let header_value =
                http::header::HeaderValue::from_bytes(value.as_bytes()).map_err(|error| {
                    TransportError::new(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("Invalid response header '{}': {}", name, error),
                    ))
                })?;
            mapped.insert(header_name, header_value);
            Ok(mapped)
        })
}

/// Reqwest-based HTTP transport for eventsource-client.
#[derive(Clone)]
pub struct ReqwestTransport {
    client: Client,
}

impl ReqwestTransport {
    /// Create a new ReqwestTransport from a reqwest Client.
    pub fn new(client: Client) -> Self {
        Self { client }
    }
}

impl HttpTransport for ReqwestTransport {
    fn request(&self, request: http::Request<Option<Bytes>>) -> ResponseFuture {
        let client = self.client.clone();

        Box::pin(async move {
            // Convert http::Request to reqwest::Request
            let method = reqwest::Method::from_bytes(request.method().as_str().as_bytes())
                .map_err(|e| {
                    TransportError::new(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        format!("Invalid HTTP method: {}", e),
                    ))
                })?;

            let url = request.uri().to_string();
            let mut reqwest_request = client.request(method, &url);

            // Add headers
            for (name, value) in request.headers() {
                if let Ok(value_str) = value.to_str() {
                    reqwest_request = reqwest_request.header(name.as_str(), value_str);
                }
            }

            // Add body if present
            if let Some(body) = request.body() {
                reqwest_request = reqwest_request.body(body.clone());
            }

            // Execute the request
            let response = reqwest_request.send().await.map_err(|e| {
                TransportError::new(std::io::Error::other(format!("Request failed: {}", e)))
            })?;

            // Convert reqwest::Response to http::Response<ByteStream>
            let status = response.status();
            let response_headers = to_http_headers(response.headers())?;

            // Create a byte stream from the response body
            let byte_stream: ByteStream = Box::pin(response.bytes_stream().map_err(|e| {
                TransportError::new(std::io::Error::other(format!("Stream error: {}", e)))
            }));

            let mut http_response = http::Response::builder()
                .status(status.as_u16())
                .body(byte_stream)
                .map_err(|e| {
                    TransportError::new(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("Failed to build response: {}", e),
                    ))
                })?;

            *http_response.headers_mut() = response_headers;

            Ok(http_response)
        })
    }
}

#[cfg(test)]
mod tests {
    use reqwest::header::{HeaderMap, HeaderValue};

    use super::to_http_headers;

    #[test]
    fn test_to_http_headers_preserves_content_type_for_sse() {
        let mut fixture = HeaderMap::new();
        fixture.insert(
            "content-type",
            HeaderValue::from_static("text/event-stream"),
        );
        fixture.insert("cache-control", HeaderValue::from_static("no-cache"));

        let actual = to_http_headers(&fixture).unwrap();

        assert_eq!(
            actual.get("content-type").unwrap(),
            &http::HeaderValue::from_static("text/event-stream")
        );
        assert_eq!(
            actual.get("cache-control").unwrap(),
            &http::HeaderValue::from_static("no-cache")
        );
    }
}
