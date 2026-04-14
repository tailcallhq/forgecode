mod anthropic;
mod bedrock;
mod bedrock_cache;
mod bedrock_sanitize_ids;
mod chat;
mod event;
mod google;
#[cfg(test)]
mod mock_server;
mod openai;
mod openai_responses;
mod opencode;
mod provider_repo;
mod retry;
mod utils;

pub use chat::*;
pub use provider_repo::*;

/// Trait for converting types into domain types
pub(crate) trait IntoDomain {
    type Domain;
    fn into_domain(self) -> Self::Domain;
}

/// Trait for converting from domain types
trait FromDomain<T> {
    fn from_domain(value: T) -> anyhow::Result<Self>
    where
        Self: Sized;
}
