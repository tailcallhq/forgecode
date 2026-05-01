//! # Forge Loop
//!
//! Native loop and monitor functionality for ForgeCode.
//!
//! This crate provides:
//! - `$loop` command: Periodic autonomous execution
//! - `$monitor` command: Conditional/event-driven execution
//!
//! ## Usage
//!
//! ```rust,no_run
//! use forge_loop::{LoopScheduler, LoopConfig};
//! use std::time::Duration;
//!
//! # async fn example() {
//! let mut scheduler = LoopScheduler::new();
//!
//! let loop_id = scheduler.start_loop(
//!     Duration::from_secs(300),  // 5 minutes
//!     "continue working".to_string(),
//!     "conversation-id".to_string()
//! ).unwrap();
//!
//! println!("Started loop: {}", loop_id);
//! # }
//! ```

mod scheduler;
mod state;
mod executor;

pub use scheduler::{LoopScheduler, LoopConfig, LoopId, LoopStatus};
pub use state::{LoopState, LoopEntry, MonitorEntry, Condition};
pub use executor::LoopExecutor;
pub use thiserror::Error;

#[derive(Debug, Error)]
pub enum LoopError {
    #[error("Loop not found: {0}")]
    NotFound(LoopId),

    #[error("Invalid interval: {0}")]
    InvalidInterval(String),

    #[error("Execution failed: {0}")]
    ExecutionFailed(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, LoopError>;
