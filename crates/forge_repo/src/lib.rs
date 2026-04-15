mod agent;
mod agent_definition;
mod context_engine;
mod conversation;
mod database;
mod forge_repo;
mod fs_snap;
mod fuzzy_search;
mod provider;
mod skill;
mod validation;

mod proto_generated {
    tonic::include_proto!("forge.v1");
}

// Expose conversation persistence building blocks for benchmarks and focused validation.
pub use conversation::ConversationRepositoryImpl;
pub use database::{DatabasePool, PoolConfig};
pub use forge_repo::*;
