mod compaction;
mod doom_loop;
mod pending_todos;
mod title_generation;
mod tracing;
pub mod verification_reminder;

pub use compaction::CompactionHandler;
pub use doom_loop::DoomLoopDetector;
use forge_domain::Hook;
pub use pending_todos::PendingTodosHandler;
pub use title_generation::TitleGenerationHandler;
pub use tracing::TracingHandler;

pub fn default() -> Hook {
    Hook::default().on_request(DoomLoopDetector::default())
}
