use agent_client_protocol as acp;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("ACP protocol error: {0}")]
    Protocol(#[from] acp::Error),

    #[error("Forge application error: {0}")]
    Application(#[from] anyhow::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// Converts a domain Error into an acp::Error.
///
/// AGENTS.md forbids blanket `From` impls for domain error conversion.
/// Call this explicitly at each `.map_err()` site instead.
pub fn into_acp_error(error: Error) -> acp::Error {
    match error {
        Error::Protocol(error) => error,
        Error::Application(error) => {
            acp::Error::into_internal_error(error.as_ref() as &dyn std::error::Error)
        }
        Error::Io(error) => acp::Error::into_internal_error(&error),
    }
}

#[cfg(test)]
mod tests {
    use std::io;

    use agent_client_protocol as acp;

    use super::{Error, into_acp_error};

    #[test]
    fn preserves_protocol_errors() {
        let error = acp::Error::invalid_params();

        let actual = into_acp_error(Error::Protocol(error.clone()));

        assert_eq!(actual.code, error.code);
        assert_eq!(actual.message, error.message);
    }

    #[test]
    fn wraps_application_errors_as_internal_errors() {
        let actual = into_acp_error(Error::Application(anyhow::anyhow!("boom")));

        assert_eq!(actual.code, acp::ErrorCode::InternalError);
        assert_eq!(actual.message, "Internal error");
        assert_eq!(actual.data, Some(serde_json::Value::String("boom".to_string())));
    }

    #[test]
    fn wraps_io_errors_as_internal_errors() {
        let actual = into_acp_error(Error::Io(io::Error::other("disk failure")));

        assert_eq!(actual.code, acp::ErrorCode::InternalError);
        assert_eq!(actual.message, "Internal error");
        assert_eq!(
            actual.data,
            Some(serde_json::Value::String("disk failure".to_string()))
        );
    }
}
