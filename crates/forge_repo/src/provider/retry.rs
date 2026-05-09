use forge_app::domain::Error as DomainError;
use forge_app::dto::anthropic::Error as AnthropicError;
use forge_app::dto::openai::{Error, ErrorCode, ErrorResponse};
use forge_config::RetryConfig;

const TRANSPORT_ERROR_CODES: [&str; 3] = ["ERR_STREAM_PREMATURE_CLOSE", "ECONNRESET", "ETIMEDOUT"];
const OPENAI_OVERLOADED_ERROR_CODE: &str = "server_is_overloaded";

pub fn into_retry(error: anyhow::Error, retry_config: &RetryConfig) -> anyhow::Error {
    if let Some(code) = get_req_status_code(&error)
        .or(get_event_req_status_code(&error))
        .or(get_api_status_code(&error))
        && retry_config.status_codes.contains(&code)
    {
        return DomainError::Retryable(error).into();
    }

    if is_api_transport_error(&error)
        || is_req_transport_error(&error)
        || is_event_transport_error(&error)
        || is_empty_error(&error)
        || is_anthropic_overloaded_error(&error)
        || is_openai_overloaded_error(&error)
    {
        return DomainError::Retryable(error).into();
    }

    error
}

/// Checks if the error is an Anthropic `overloaded_error`, which arrives as an
/// SSE event payload rather than an HTTP status code and must be retried.
fn is_anthropic_overloaded_error(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<AnthropicError>()
        .is_some_and(|e| matches!(e, AnthropicError::OverloadedError { .. }))
}

pub(crate) fn get_api_status_code(error: &anyhow::Error) -> Option<u16> {
    error.downcast_ref::<Error>().and_then(|error| match error {
        Error::Response(error) => error
            .get_code_deep()
            .as_ref()
            .and_then(|code| code.as_number()),
        Error::InvalidStatusCode(code) => Some(*code),
    })
}

fn get_req_status_code(error: &anyhow::Error) -> Option<u16> {
    error
        .downcast_ref::<reqwest::Error>()
        .and_then(|error| error.status())
        .map(|status| status.as_u16())
}

fn get_event_req_status_code(error: &anyhow::Error) -> Option<u16> {
    error
        .downcast_ref::<forge_eventsource::Error>()
        .and_then(|error| match error {
            forge_eventsource::Error::InvalidStatusCode(_, response) => {
                Some(response.status().as_u16())
            }
            forge_eventsource::Error::InvalidContentType(_, response) => {
                Some(response.status().as_u16())
            }
            _ => None,
        })
}

#[derive(Clone, Copy)]
enum RetryableApiErrorCode {
    Transport,
    OpenAIOverloaded,
}

impl RetryableApiErrorCode {
    fn matches(self, code: &ErrorCode) -> bool {
        let Some(code) = code.as_str() else {
            return false;
        };

        match self {
            RetryableApiErrorCode::Transport => TRANSPORT_ERROR_CODES.contains(&code),
            RetryableApiErrorCode::OpenAIOverloaded => code == OPENAI_OVERLOADED_ERROR_CODE,
        }
    }
}

fn has_error_code(error: &ErrorResponse, retryable_code: RetryableApiErrorCode) -> bool {
    // Check if the current level has a matching error code.
    let has_direct_code = error
        .code
        .as_ref()
        .is_some_and(|code| retryable_code.matches(code));

    if has_direct_code {
        return true;
    }

    // Recursively check nested errors.
    error
        .error
        .as_deref()
        .is_some_and(|nested| has_error_code(nested, retryable_code))
}

fn is_api_transport_error(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<Error>()
        .is_some_and(|error| match error {
            Error::Response(error) => has_error_code(error, RetryableApiErrorCode::Transport),
            _ => false,
        })
}

fn is_openai_overloaded_error(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<Error>()
        .is_some_and(|error| match error {
            Error::Response(error) => {
                has_error_code(error, RetryableApiErrorCode::OpenAIOverloaded)
            }
            _ => false,
        })
}

fn is_empty_error(error: &anyhow::Error) -> bool {
    error.downcast_ref::<Error>().is_some_and(|e| match e {
        Error::Response(error) => {
            error.message.is_none() && error.code.is_none() && error.error.is_none()
        }
        _ => false,
    })
}

fn is_req_transport_error(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<reqwest::Error>()
        .is_some_and(|e| e.is_timeout() || e.is_connect() || e.is_request())
}

fn is_event_transport_error(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<forge_eventsource::Error>()
        .is_some_and(|e| matches!(e, forge_eventsource::Error::Transport(_)))
}

#[cfg(test)]
mod tests {
    use anyhow::anyhow;
    use forge_app::dto::openai::{Error, ErrorCode, ErrorResponse};

    use super::*;

    // Helper function to check if an error is retryable
    fn is_retryable(error: anyhow::Error) -> bool {
        if let Some(domain_error) = error.downcast_ref::<DomainError>() {
            matches!(domain_error, DomainError::Retryable(_))
        } else {
            false
        }
    }

    // Fixture functions
    fn fixture_retry_config(codes: Vec<u16>) -> RetryConfig {
        RetryConfig::default().status_codes(codes)
    }

    fn fixture_response_error(code: Option<u16>) -> anyhow::Error {
        let error = if let Some(code) = code {
            ErrorResponse::default().code(ErrorCode::Number(code))
        } else {
            ErrorResponse::default()
        };
        anyhow::Error::from(Error::Response(Box::new(error)))
    }

    fn fixture_transport_error(code: &str) -> anyhow::Error {
        let error = ErrorResponse::default().code(ErrorCode::String(code.to_string()));
        anyhow::Error::from(Error::Response(Box::new(error)))
    }

    fn fixture_nested_transport_error(code: &str, depth: usize) -> anyhow::Error {
        let mut error = ErrorResponse::default().code(ErrorCode::String(code.to_string()));
        for _ in 0..depth {
            error = ErrorResponse::default().error(Box::new(error));
        }
        anyhow::Error::from(Error::Response(Box::new(error)))
    }

    #[test]
    fn test_into_retry_with_status_codes() {
        let retry_config = fixture_retry_config(vec![429, 500, 502, 503, 504]);

        // Retryable status codes
        for code in [429, 500, 502, 503, 504] {
            let error = fixture_response_error(Some(code));
            assert!(is_retryable(into_retry(error, &retry_config)));
        }

        // Non-retryable status codes
        for code in [400, 401, 403, 404] {
            let error = fixture_response_error(Some(code));
            assert!(!is_retryable(into_retry(error, &retry_config)));
        }

        // Empty retry config - nothing is retryable by status code
        let empty_config = fixture_retry_config(vec![]);
        let error = fixture_response_error(Some(500));
        assert!(!is_retryable(into_retry(error, &empty_config)));

        // String status code that parses to retryable number
        let error = ErrorResponse::default().code(ErrorCode::String("429".to_string()));
        let error = anyhow::Error::from(Error::Response(Box::new(error)));
        assert!(is_retryable(into_retry(error, &retry_config)));

        // String status code that parses to non-retryable number
        let error = ErrorResponse::default().code(ErrorCode::String("404".to_string()));
        let error = anyhow::Error::from(Error::Response(Box::new(error)));
        assert!(!is_retryable(into_retry(error, &retry_config)));
    }

    #[test]
    fn test_into_retry_with_invalid_status_code() {
        let retry_config = fixture_retry_config(vec![429, 500, 503]);

        // Matching InvalidStatusCode
        let error = anyhow::Error::from(Error::InvalidStatusCode(503));
        assert!(is_retryable(into_retry(error, &retry_config)));

        // Non-matching InvalidStatusCode
        let error = anyhow::Error::from(Error::InvalidStatusCode(400));
        assert!(!is_retryable(into_retry(error, &retry_config)));
    }

    #[test]
    fn test_into_retry_with_transport_errors() {
        let retry_config = fixture_retry_config(vec![]);

        // Known transport error codes
        for code in ["ERR_STREAM_PREMATURE_CLOSE", "ECONNRESET", "ETIMEDOUT"] {
            let error = fixture_transport_error(code);
            assert!(is_retryable(into_retry(error, &retry_config)));
        }

        // Nested transport errors
        for depth in [1, 2, 3] {
            let error = fixture_nested_transport_error("ECONNRESET", depth);
            assert!(is_retryable(into_retry(error, &retry_config)));
        }

        // Unknown transport code - not retryable
        let error = fixture_transport_error("UNKNOWN_ERROR");
        assert!(!is_retryable(into_retry(error, &retry_config)));

        // Nested unknown code - not retryable
        let error = fixture_nested_transport_error("UNKNOWN", 2);
        assert!(!is_retryable(into_retry(error, &retry_config)));
    }

    #[test]
    fn test_into_retry_with_edge_cases() {
        let retry_config = fixture_retry_config(vec![]);

        // Empty error is retryable
        let error = anyhow::Error::from(Error::Response(Box::new(ErrorResponse::default())));
        assert!(is_retryable(into_retry(error, &retry_config)));

        // Generic error is not retryable
        let error = anyhow!("Generic error");
        assert!(!is_retryable(into_retry(error, &retry_config)));

        // Non-Response error is not empty
        let error = anyhow::Error::from(Error::InvalidStatusCode(404));
        assert!(!is_empty_error(&error));
    }

    #[test]
    fn test_has_transport_error_code_with_known_codes() {
        let transport_codes = ["ERR_STREAM_PREMATURE_CLOSE", "ECONNRESET", "ETIMEDOUT"];

        for code in transport_codes {
            let error = ErrorResponse::default().code(ErrorCode::String(code.to_string()));
            assert!(
                has_error_code(&error, RetryableApiErrorCode::Transport),
                "Code {code} should be transport error"
            );
        }

        let error = ErrorResponse::default().code(ErrorCode::String("UNKNOWN".to_string()));
        assert!(!has_error_code(&error, RetryableApiErrorCode::Transport));

        let error = ErrorResponse::default();
        assert!(!has_error_code(&error, RetryableApiErrorCode::Transport));

        // Nested transport codes
        let nested = ErrorResponse::default().code(ErrorCode::String("ECONNRESET".to_string()));
        let error = ErrorResponse::default().error(Box::new(nested));
        assert!(has_error_code(&error, RetryableApiErrorCode::Transport));

        // is_empty_error
        let error = anyhow::Error::from(Error::Response(Box::new(ErrorResponse::default())));
        assert!(is_empty_error(&error));

        let error = anyhow::Error::from(Error::Response(Box::new(
            ErrorResponse::default().message("Error".to_string()),
        )));
        assert!(!is_empty_error(&error));

        let error = anyhow::Error::from(Error::Response(Box::new(
            ErrorResponse::default().code(ErrorCode::Number(500)),
        )));
        assert!(!is_empty_error(&error));

        let nested = ErrorResponse::default().message("Nested".to_string());
        let error = anyhow::Error::from(Error::Response(Box::new(
            ErrorResponse::default().error(Box::new(nested)),
        )));
        assert!(!is_empty_error(&error));

        // is_api_transport_error
        let error = fixture_transport_error("ETIMEDOUT");
        assert!(is_api_transport_error(&error));

        let error = fixture_transport_error("INVALID_REQUEST");
        assert!(!is_api_transport_error(&error));

        // Generic error handlers return defaults
        let error = anyhow!("Generic error");
        assert!(!is_api_transport_error(&error));
        assert!(!is_req_transport_error(&error));
        assert!(!is_event_transport_error(&error));
        assert!(get_api_status_code(&error).is_none());
        assert!(get_req_status_code(&error).is_none());
        assert!(get_event_req_status_code(&error).is_none());
    }

    #[test]
    fn test_openai_server_overloaded_error_is_retryable() {
        let retry_config = fixture_retry_config(vec![]);

        let error = anyhow::Error::from(Error::Response(Box::new(
            ErrorResponse::default()
                .code(ErrorCode::String("server_is_overloaded".to_string()))
                .message(
                    "Our servers are currently overloaded. Please try again later.".to_string(),
                ),
        )));

        assert!(is_retryable(into_retry(error, &retry_config)));

        let error = anyhow::Error::from(Error::Response(Box::new(
            ErrorResponse::default().code(ErrorCode::String("rate_limit".to_string())),
        )));

        assert!(!is_retryable(into_retry(error, &retry_config)));
    }

    #[test]
    fn test_anthropic_overloaded_error_is_retryable() {
        let retry_config = fixture_retry_config(vec![]);

        // overloaded_error arriving as an SSE event must be retried
        let error = anyhow::Error::from(AnthropicError::OverloadedError {
            message: "Overloaded".to_string(),
        });
        assert!(is_retryable(into_retry(error, &retry_config)));

        // Generic errors are still not retryable
        let error = anyhow!("Generic error");
        assert!(!is_retryable(into_retry(error, &retry_config)));
    }

    #[tokio::test]
    async fn test_incomplete_message_is_retryable() {
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (_socket, _) = listener.accept().await.unwrap();
        });

        let req_err = reqwest::Client::new()
            .get(format!("http://{addr}"))
            .send()
            .await
            .unwrap_err();

        let retry_config = fixture_retry_config(vec![]);
        assert!(is_retryable(into_retry(req_err.into(), &retry_config)));
    }
}
