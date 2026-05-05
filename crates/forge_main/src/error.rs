use forge_domain::ProviderId;

/// Errors specific to UI operations
#[derive(Debug, thiserror::Error)]
pub enum UIError {
    /// No authentication methods are available for a provider
    #[error(
        "No authentication methods are configured for provider '{provider}'. \
         Please check your provider configuration."
    )]
    NoAuthMethodsAvailable { provider: ProviderId },

    /// User selected an authentication method that could not be found
    #[error(
        "The selected authentication method is no longer available. \
         Please try again or check your provider configuration."
    )]
    AuthMethodNotFound,

    /// Display data is missing a header line - occurs when the data source
    /// (agents, models, or providers list) produces empty output after
    /// formatting
    #[error(
        "Unable to display the selection list - the data appears to be empty. \
         This can happen if the agents, models, or providers list could not be retrieved"
    )]
    MissingHeaderLine,
}

/// Checks if an error is a cursor position timeout error.
///
/// These errors occur when crossterm's cursor position query times out.
/// They are non-fatal and can be safely suppressed during shutdown.
///
/// See: plans/2026-05-04-forge-cursor-error-investigation.md
pub fn is_cursor_error(err: &(impl std::error::Error + ?Sized)) -> bool {
    let msg = err.to_string();
    msg.contains("cursor position could not be read")
        || msg.contains("cursor position could not be read within a normal duration")
        || (msg.contains("Resource temporarily unavailable") && msg.contains("os error 35"))
}

/// Checks if an error chain contains only cursor position errors.
///
/// If the entire error chain consists of cursor position errors, the operation
/// can be considered successful for practical purposes.
pub fn is_cursor_only_error(err: &anyhow::Error) -> bool {
    // Check the main error - anyhow::Error implements AsRef<dyn Error + 'static>
    let main_err: &(dyn std::error::Error + 'static) = err.as_ref();
    if !is_cursor_error(main_err) {
        return false;
    }

    // Check all chained errors
    let mut source = err.source();
    while let Some(e) = source {
        if !is_cursor_error(e) {
            return false;
        }
        source = e.source();
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_cursor_error_timeout() {
        // Test detection of cursor timeout error
        let err = std::io::Error::new(
            std::io::ErrorKind::Other,
            "The cursor position could not be read within a normal duration",
        );
        assert!(is_cursor_error(&err));
    }

    #[test]
    fn test_is_cursor_error_resource_unavailable() {
        // Test detection of resource unavailable error
        let err = std::io::Error::new(
            std::io::ErrorKind::Other,
            "Resource temporarily unavailable (os error 35)",
        );
        assert!(is_cursor_error(&err));
    }

    #[test]
    fn test_is_cursor_error_not_cursor() {
        // Test that non-cursor errors are not detected
        let err = std::io::Error::new(std::io::ErrorKind::NotFound, "File not found");
        assert!(!is_cursor_error(&err));
    }

    #[test]
    fn test_is_cursor_error_partial_match() {
        // Test that partial matches don't trigger (need both parts)
        let err = std::io::Error::new(
            std::io::ErrorKind::Other,
            "Resource temporarily unavailable (but not the cursor one)",
        );
        assert!(!is_cursor_error(&err));
    }
}
