use jsonrpsee::types::{ErrorObject, ErrorObjectOwned};

/// JSON-RPC error codes
pub struct ErrorCode;

impl ErrorCode {
    pub const PARSE_ERROR: i32 = -32700;
    pub const INVALID_REQUEST: i32 = -32600;
    pub const METHOD_NOT_FOUND: i32 = -32601;
    pub const INVALID_PARAMS: i32 = -32602;
    pub const INTERNAL_ERROR: i32 = -32603;
    pub const NOT_FOUND: i32 = -32001;
    pub const UNAUTHORIZED: i32 = -32002;
    pub const VALIDATION_FAILED: i32 = -32003;
}

/// Convert anyhow errors to JSON-RPC errors
pub fn map_error(err: anyhow::Error) -> ErrorObjectOwned {
    // Try to downcast to domain errors
    if let Some(domain_err) = err.downcast_ref::<forge_domain::Error>() {
        return map_domain_error(domain_err);
    }

    // Default: internal error
    ErrorObject::owned(
        ErrorCode::INTERNAL_ERROR,
        format!("Internal error: {err}"),
        None::<()>,
    )
}

fn map_domain_error(err: &forge_domain::Error) -> ErrorObjectOwned {
    match err {
        forge_domain::Error::ConversationNotFound(_)
        | forge_domain::Error::AgentUndefined(_)
        | forge_domain::Error::WorkspaceNotFound
        | forge_domain::Error::HeadAgentUndefined => {
            ErrorObject::owned(ErrorCode::NOT_FOUND, err.to_string(), None::<()>)
        }
        forge_domain::Error::ProviderNotAvailable { .. }
        | forge_domain::Error::EnvironmentVariableNotFound { .. }
        | forge_domain::Error::AuthTokenNotFound => {
            ErrorObject::owned(ErrorCode::UNAUTHORIZED, err.to_string(), None::<()>)
        }
        _ => {
            ErrorObject::owned(ErrorCode::INTERNAL_ERROR, err.to_string(), None::<()>)
        }
    }
}

/// Create a not-found error object
pub fn not_found(resource: &str, id: &str) -> ErrorObjectOwned {
    ErrorObject::owned(
        ErrorCode::NOT_FOUND,
        format!("{resource} not found: {id}"),
        Some(serde_json::json!({ "resource": resource, "id": id })),
    )
}
