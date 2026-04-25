mod compaction;
mod doom_loop;
mod external;
pub mod loader;
mod pending_todos;
mod title_generation;
mod tracing;
pub mod trust;

pub use compaction::CompactionHandler;
pub use doom_loop::DoomLoopDetector;
pub use external::{discover_hooks, ExternalHookInterceptor};
pub use loader::{load_and_verify_hooks, HookSummary};
pub use pending_todos::PendingTodosHandler;
pub use title_generation::TitleGenerationHandler;
pub use tracing::TracingHandler;
pub use trust::{
    HookTrustStatus, TrustStore, TrustedHook, compute_file_hash, discover_events,
    hooks_base_dir, relative_hook_path, trust_store_path, validate_hook_path,
    validate_hook_path_for_delete,
};
