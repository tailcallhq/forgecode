use std::collections::BTreeMap;

use derive_setters::Setters;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, derive_more::From, Error)]
pub enum Error {
    #[error("{0}")]
    Response(Box<ErrorResponse>),

    #[error("Invalid Status Code: {0}")]
    InvalidStatusCode(u16),
}

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
#[serde(untagged)]
pub enum ErrorCode {
    String(String),
    Number(u16),
}

impl ErrorCode {
    pub fn as_number(&self) -> Option<u16> {
        match self {
            ErrorCode::String(s) => s.parse::<u16>().ok(),
            ErrorCode::Number(code) => Some(*code),
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            ErrorCode::String(s) => Some(s),
            ErrorCode::Number(_) => None,
        }
    }
}

#[derive(Default, Debug, Deserialize, Serialize, Clone, Setters)]
#[setters(strip_option)]
pub struct ErrorResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<Box<ErrorResponse>>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub errno: Option<i32>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<ErrorCode>,

    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, serde_json::Value>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub syscall: Option<String>,

    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub type_of: Option<serde_json::Value>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub param: Option<serde_json::Value>,
}

impl ErrorResponse {
    /// Deeply introspects the error structure to determine the ErrorCode
    pub fn get_code_deep(&self) -> Option<&ErrorCode> {
        if let Some(ref code) = self.code {
            return Some(code);
        }
        if let Some(ref error) = self.error {
            return error.get_code_deep();
        }
        None
    }
}

impl std::fmt::Display for ErrorResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        serde_json::to_string(self)
            .map_err(|_| std::fmt::Error)?
            .fmt(f)
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn test_error_code_as_number() {
        // Test with numeric error code
        let code_number = ErrorCode::Number(404);
        assert_eq!(code_number.as_number(), Some(404));

        // Test with string error code containing a valid number
        let code_string_numeric = ErrorCode::String("500".to_string());
        assert_eq!(code_string_numeric.as_number(), Some(500));

        // Test with string error code containing a non-numeric value
        let code_string_non_numeric = ErrorCode::String("ERR_STREAM_PREMATURE_CLOSE".to_string());
        assert_eq!(code_string_non_numeric.as_number(), None);
    }

    #[test]
    fn test_error_code_as_str() {
        // Test with string error code
        let code_string = ErrorCode::String("ERR_STREAM_PREMATURE_CLOSE".to_string());
        assert_eq!(code_string.as_str(), Some("ERR_STREAM_PREMATURE_CLOSE"));

        // Test with numeric error code
        let code_number = ErrorCode::Number(404);
        assert_eq!(code_number.as_str(), None);
    }

    #[test]
    fn test_get_code_deep_direct() {
        // Test with an error that has a direct code
        let error_code = ErrorCode::Number(404);

        // Use derived setters for a cleaner initialization
        let fixture = ErrorResponse::default()
            .message("Error message".to_string())
            .code(error_code);

        let actual = fixture.get_code_deep();

        // Create a new ErrorCode to compare against
        let expected_code = ErrorCode::Number(404);
        assert_eq!(actual, Some(&expected_code));
    }

    #[test]
    fn test_get_code_deep_nested() {
        // Test with an error that has no direct code but has an inner error with a code
        let error_code = ErrorCode::String("ERR_STREAM_PREMATURE_CLOSE".to_string());

        // Use derived setters for cleaner initialization
        let inner_error = ErrorResponse::default()
            .message("Inner error".to_string())
            .code(error_code);

        let fixture = ErrorResponse::default()
            .error(Box::new(inner_error))
            .message("Outer error".to_string());

        let actual = fixture.get_code_deep();

        // Create a new ErrorCode to compare against
        let expected_code = ErrorCode::String("ERR_STREAM_PREMATURE_CLOSE".to_string());
        assert_eq!(actual, Some(&expected_code));
    }

    #[test]
    fn test_get_code_deep_no_code() {
        // Test with an error that has no code and no inner error
        let fixture = ErrorResponse::default().message("Error message".to_string());

        let actual = fixture.get_code_deep();
        let expected = None;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_get_code_deep_multiple_nested() {
        // Test with deeply nested errors
        let error_code = ErrorCode::Number(500);

        let deepest_error = ErrorResponse::default()
            .message("Deepest error".to_string())
            .code(error_code);

        let middle_error = ErrorResponse::default()
            .error(Box::new(deepest_error))
            .message("Middle error".to_string());
        // No code here, should find deepest

        let fixture = ErrorResponse::default()
            .error(Box::new(middle_error))
            .message("Outer error".to_string());
        // No code here, should find deepest

        let actual = fixture.get_code_deep();

        // Create a new ErrorCode to compare against
        let expected_code = ErrorCode::Number(500);
        assert_eq!(actual, Some(&expected_code));
    }
}
