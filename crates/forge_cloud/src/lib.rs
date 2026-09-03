//! Cloud task dispatching for HeliosLite agent runners.
//!
//! [`forge_cloud`] provides a trait-based abstraction for dispatching agent tasks
//! to cloud compute backends. The primary implementation targets
//! [Cloudflare Workers](https://developers.cloudflare.com/workers/), with a
//! local in-process fallback for development and offline use.
//!
//! # Architecture
//!
//! ```text
//! CloudRunner ──► CloudProvider (trait)
//!     │                ├── CloudflareWorkers
//!     │                └── LocalProvider
//!     │
//!     ├── dispatch()  → TaskHandle
//!     ├── status()    → TaskStatus
//!     ├── cancel()    → ()
//!     └── result()    → TaskResult
//! ```

pub mod provider;
pub mod runner;
pub mod task;

pub use provider::{CloudflareWorkers, LocalProvider};
pub use runner::CloudRunner;
pub use task::{CloudTask, TaskError, TaskResult, TaskStatus};

use serde::{Deserialize, Serialize};

/// Top-level configuration for cloud dispatch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudConfig {
    /// Which backend to use.
    pub backend: CloudBackend,
    /// Account / project identifiers.
    pub account_id: Option<String>,
    /// API token (typically via env).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_token: Option<String>,
    /// Default timeout for tasks (seconds).
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
    /// Maximum concurrent tasks.
    #[serde(default = "default_max_concurrency")]
    pub max_concurrency: usize,
}

fn default_timeout_secs() -> u64 {
    300
}

fn default_max_concurrency() -> usize {
    8
}

impl Default for CloudConfig {
    fn default() -> Self {
        Self {
            backend: CloudBackend::Local,
            account_id: None,
            api_token: None,
            timeout_secs: default_timeout_secs(),
            max_concurrency: default_max_concurrency(),
        }
    }
}

/// Supported cloud backends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloudBackend {
    /// Dispatch to Cloudflare Workers.
    CloudflareWorkers,
    /// Run tasks in-process (default for development).
    Local,
}
