use anyhow::Context;
use eventsource_client::{Event, SSE};
use forge_app::EventSource;
use forge_app::domain::ChatCompletionMessage;
use forge_app::dto::openai::Error;
use reqwest::{StatusCode, Url};
use serde::de::DeserializeOwned;
use tokio_stream::{Stream, StreamExt};
use tracing::debug;

use super::utils::format_http_context;

pub fn into_chat_completion_message<Response>(
    url: Url,
    source: EventSource,
) -> impl Stream<Item = anyhow::Result<ChatCompletionMessage>>
where
    Response: DeserializeOwned,
    ChatCompletionMessage: TryFrom<Response, Error = anyhow::Error>,
{
    source
        .then(move |event| {
            let url = url.clone();
            async move {
                match event {
                    Ok(SSE::Connected(_)) => None,
                    Ok(SSE::Comment(_)) => None,
                    Ok(SSE::Event(event)) => handle_event::<Response>(event, url).await,
                    Err(error) => handle_error(error, url).await,
                }
            }
        })
        .filter_map(|response| response)
}

async fn handle_event<Response>(
    event: Event,
    url: Url,
) -> Option<anyhow::Result<ChatCompletionMessage>>
where
    Response: DeserializeOwned,
    ChatCompletionMessage: TryFrom<Response, Error = anyhow::Error>,
{
    // Check for completion markers
    if ["[DONE]", ""].contains(&event.data.as_str()) {
        debug!("Received completion from Upstream");
        return None;
    }

    // Parse the JSON response
    let result = serde_json::from_str::<Response>(&event.data)
        .with_context(|| format!("Failed to parse provider response: {}", event.data))
        .and_then(|response| {
            ChatCompletionMessage::try_from(response)
                .with_context(|| format!("Failed to create completion message: {}", event.data))
        })
        .with_context(|| format_http_context(None, "POST", url));

    Some(result)
}

async fn handle_error(
    error: anyhow::Error,
    url: Url,
) -> Option<anyhow::Result<ChatCompletionMessage>> {
    let error_msg = error.to_string();

    // Check for specific error patterns from eventsource-client
    // The error types are different from reqwest-eventsource
    if error_msg.to_lowercase().contains("eof") || error_msg.contains("stream ended") {
        return None;
    }

    // Check for HTTP status errors that we might extract from the error message
    // eventsource-client wraps HTTP errors differently
    if error_msg.contains("UnexpectedResponse") {
        let status_code = extract_status_code(&error_msg);
        if let Some(status) = status_code {
            let status_display = StatusCode::from_u16(status)
                .map(|status| status.to_string())
                .unwrap_or_else(|_| status.to_string());
            let reason =
                extract_unexpected_response_reason(&error_msg).unwrap_or_else(|| error_msg.clone());
            return Some(
                Err(Error::InvalidStatusCode(status))
                    .with_context(|| format!("{} Reason: {}", status_display, reason))
                    .with_context(|| format_http_context(None, "POST", &url)),
            );
        }
    }

    tracing::error!(error = ?error, "Failed to receive chat completion event");
    Some(Err(error).with_context(|| format_http_context(None, "POST", url)))
}

/// Extract a status code from an error message string
fn extract_status_code(error_msg: &str) -> Option<u16> {
    // Look for patterns like "401 Unauthorized" or "status: 401"
    use regex::Regex;

    // Try to find a 3-digit status code in the error message
    let re = Regex::new(r"\b(\d{3})\b").ok()?;
    if let Some(captures) = re.captures(error_msg)
        && let Some(code) = captures.get(1)
    {
        return code.as_str().parse().ok();
    }
    None
}

fn extract_unexpected_response_reason(error_msg: &str) -> Option<String> {
    let body_marker = "body: ";
    let body_start = error_msg.find(body_marker)? + body_marker.len();
    let body = error_msg.get(body_start..)?;
    Some(body.trim_end_matches(')').to_string())
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use serde::{Deserialize, Serialize};
    use tokio_stream::StreamExt;

    use super::*;

    #[derive(Debug, Serialize)]
    struct FixtureApiErrorBody {
        r#type: String,
        message: String,
    }

    #[derive(Debug, thiserror::Error)]
    enum FixtureEventSourceError {
        #[error("UnexpectedResponse(status: {status}, body: {body})")]
        UnexpectedResponse {
            status: StatusCode,
            body: serde_json::Value,
        },
        #[error("eof")]
        Eof,
    }

    #[derive(Debug, Deserialize)]
    struct FixtureResponse;

    impl TryFrom<FixtureResponse> for ChatCompletionMessage {
        type Error = anyhow::Error;

        fn try_from(_value: FixtureResponse) -> Result<Self, Self::Error> {
            Ok(ChatCompletionMessage::default())
        }
    }

    #[tokio::test]
    async fn test_into_chat_completion_message_preserves_unexpected_response_error() {
        let url = Url::parse("https://example.com/v1/chat/completions").unwrap();
        let fixture = FixtureApiErrorBody {
            r#type: "error".to_string(),
            message: "Subscription quota exceeded".to_string(),
        };
        let reason = serde_json::to_value(fixture).unwrap();
        let source: EventSource = Box::pin(tokio_stream::iter(vec![Err(
            FixtureEventSourceError::UnexpectedResponse {
                status: StatusCode::TOO_MANY_REQUESTS,
                body: reason.clone(),
            }
            .into(),
        )]));

        let mut actual = Box::pin(into_chat_completion_message::<FixtureResponse>(url, source));
        let error = actual
            .next()
            .await
            .expect("stream should yield an error")
            .expect_err("stream item should be an error");
        let expected = vec![
            "POST https://example.com/v1/chat/completions".to_string(),
            format!("429 Too Many Requests Reason: {}", reason),
            "Invalid Status Code: 429".to_string(),
        ];
        let actual = error
            .chain()
            .map(|error| error.to_string())
            .collect::<Vec<_>>();

        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn test_into_chat_completion_message_ignores_eof_error() {
        let url = Url::parse("https://example.com/v1/chat/completions").unwrap();
        let source: EventSource = Box::pin(tokio_stream::iter(vec![Err(
            FixtureEventSourceError::Eof.into(),
        )]));

        let mut actual = Box::pin(into_chat_completion_message::<FixtureResponse>(url, source));
        let actual = actual.next().await;

        assert!(actual.is_none());
    }
}
