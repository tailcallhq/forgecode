use anyhow::Context;
use forge_app::domain::ChatCompletionMessage;
use forge_app::dto::openai::Error;
use reqwest::Url;
use reqwest_eventsource::{Event, EventSource};
use serde::de::DeserializeOwned;
use tokio_stream::{Stream, StreamExt};
use tracing::debug;

use super::utils::{format_http_context, read_http_error_reason};

pub fn into_chat_completion_message<Response>(
    url: Url,
    source: EventSource,
) -> impl Stream<Item = anyhow::Result<ChatCompletionMessage>>
where
    Response: DeserializeOwned,
    ChatCompletionMessage: TryFrom<Response, Error = anyhow::Error>,
{
    source
        .take_while(|message| !matches!(message, Err(reqwest_eventsource::Error::StreamEnded)))
        .then(|event| async {
            match event {
                Ok(event) => match event {
                    Event::Open => None,
                    Event::Message(event) if ["[DONE]", ""].contains(&event.data.as_str()) => {
                        debug!("Received completion from Upstream");
                        None
                    }
                    Event::Message(message) => Some(
                        serde_json::from_str::<Response>(&message.data)
                            .with_context(|| {
                                format!(
                                    "Failed to parse provider response: {}",
                                    message.data
                                )
                            })
                            .and_then(|response| {
                                ChatCompletionMessage::try_from(response).with_context(|| {
                                    format!(
                                        "Failed to create completion message: {}",
                                        message.data
                                    )
                                })
                            }),
                    ),
                },
                Err(error) => match error {
                    reqwest_eventsource::Error::StreamEnded => None,
                    reqwest_eventsource::Error::InvalidStatusCode(_, response)
                    | reqwest_eventsource::Error::InvalidContentType(_, response) => {
                        let (code, reason) = read_http_error_reason(response).await;
                        Some(Err(Error::InvalidStatusCode(code)).with_context(|| reason))
                    }
                    error => {
                        tracing::error!(error = ?error, "Failed to receive chat completion event");
                        Some(Err(error.into()))
                    }
                },
            }
        })
        .filter_map(move |response| {
            response.map(|result| {
                result.with_context(|| format_http_context(None, "POST", url.clone()))
            })
        })
}

#[cfg(test)]
mod tests {
    use forge_app::dto::openai::Response;
    use reqwest_eventsource::RequestBuilderExt;
    use tokio_stream::StreamExt;
    use url::Url;

    use super::*;
    use crate::provider::mock_server::MockServer;

    #[tokio::test]
    async fn test_invalid_status_code_includes_response_body() -> anyhow::Result<()> {
        let mut fixture = MockServer::new().await;
        let error_body = r#"{"error":{"message":"The requested model is not supported.","code":"model_not_supported"}}"#;
        let mock = fixture.mock_post_error("/chat", error_body, 400).await;

        let url = Url::parse(&format!("{}/chat", fixture.url()))?;
        let source = reqwest::Client::new()
            .post(url.clone())
            .body("{}")
            .eventsource()?;

        let actual: Vec<_> = into_chat_completion_message::<Response>(url, source)
            .collect()
            .await;

        mock.assert_async().await;
        assert_eq!(actual.len(), 1);
        let err = actual[0].as_ref().unwrap_err();
        let err_str = format!("{:#}", err);
        assert!(err_str.contains("400 Bad Request Reason:"));
        assert!(err_str.contains("model_not_supported"));
        Ok(())
    }

    #[tokio::test]
    async fn test_invalid_content_type_includes_response_body() -> anyhow::Result<()> {
        let mut fixture = MockServer::new().await;
        let error_body = r#"{"error":{"message":"unexpected content type"}}"#;
        // Return 200 with application/json — triggers InvalidContentType
        let mock = fixture
            .mock_post_wrong_content_type("/chat", error_body)
            .await;

        let url = Url::parse(&format!("{}/chat", fixture.url()))?;
        let source = reqwest::Client::new()
            .post(url.clone())
            .body("{}")
            .eventsource()?;

        let actual: Vec<_> = into_chat_completion_message::<Response>(url, source)
            .collect()
            .await;

        mock.assert_async().await;
        assert_eq!(actual.len(), 1);
        let err = actual[0].as_ref().unwrap_err();
        let err_str = format!("{:#}", err);
        assert!(err_str.contains("200 OK Reason:"));
        assert!(err_str.contains("unexpected content type"));
        Ok(())
    }
}
