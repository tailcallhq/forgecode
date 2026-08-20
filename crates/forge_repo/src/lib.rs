mod agent;
mod agent_definition;
mod codec;
mod context_engine;
mod conversation;
mod daemon_repo;
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

// Only expose forge_repo container
pub use conversation::{
    ForgeSnapshot, ForgeSnapshotManifest, ForgeSnapshotRow, SNAPSHOT_CONTRACT_VERSION,
    export_forge_snapshot, publish_snapshot_atomic,
};
pub use forge_repo::*;
