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
