//! Windows backend using Job Objects + restricted token (placeholder).
//!
//! On Windows, sandboxing requires Job Objects (for resource limits + kill-on-
//! close semantics) and a restricted access token (to drop privileges). The
//! `windows` crate provides the bindings; for now this backend is a stub that
//! falls through to `Disabled` until the implementation lands.
//!
//! Roadmap: implement via `JobObjectExtendedLimitInformation` + `CreateRestrictedToken`.

use crate::backend::Backend;
use crate::config::SandboxConfig;
use crate::{SandboxError, SandboxOutput};
use async_trait::async_trait;

#[derive(Clone)]
pub struct WindowsBackend {
    /// Windows backend is currently a passthrough. When the real implementation
    /// lands we'll gate this on actual availability.
    available: bool,
}

impl WindowsBackend {
    pub fn new() -> Result<Self, SandboxError> {
        // Real implementation pending — JobObjects + restricted tokens.
        Ok(Self { available: false })
    }
}

#[async_trait]
impl Backend for WindowsBackend {
    fn name(&self) -> &'static str {
        "windows-jobobject"
    }

    fn enforces_isolation(&self) -> bool {
        self.available
    }

    async fn run(&self, _config: &SandboxConfig) -> Result<SandboxOutput, SandboxError> {
        // Fall back to disabled until the real implementation ships.
        Err(SandboxError::BackendUnavailable("windows-jobobject"))
    }
}
