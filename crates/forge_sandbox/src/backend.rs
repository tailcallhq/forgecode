//! Backend trait and platform-aware dispatch.
//!
//! A [`Backend`] is a platform-specific sandbox implementation. The [`Sandbox`]
//! enum dispatches to the right one at runtime via [`Sandbox::for_platform`].

use crate::config::SandboxConfig;
use crate::{SandboxError, SandboxOutput};
use async_trait::async_trait;

/// Trait implemented by every sandbox backend.
///
/// One method: [`Backend::run`]. That's all we need — backends are
/// intentionally small so adding a new one (gVisor, Firecracker, …) is
/// trivial.
#[async_trait]
pub trait Backend: Send + Sync {
    /// Human-readable name of this backend.
    fn name(&self) -> &'static str;

    /// Whether this backend actually enforces OS-level isolation.
    /// The `Disabled` backend returns `false` here.
    fn enforces_isolation(&self) -> bool;

    /// Run the configured command inside the sandbox.
    async fn run(&self, config: &SandboxConfig) -> Result<SandboxOutput, SandboxError>;
}

/// Top-level sandbox handle. Dispatched to the platform-appropriate backend.
#[derive(Clone)]
pub enum Sandbox {
    Linux(crate::linux::LinuxBackend),
    MacOS(crate::macos::MacOsBackend),
    Windows(crate::windows::WindowsBackend),
    Disabled(crate::disabled::DisabledBackend),
}

impl Sandbox {
    /// Pick the best backend for the current platform.
    ///
    /// Falls back to [`Sandbox::Disabled`] if the platform-specific backend
    /// is unavailable (e.g., Landlock on Linux <5.13, sandbox-exec missing on
    /// macOS).
    pub fn for_platform() -> Self {
        #[cfg(target_os = "linux")]
        {
            if let Ok(backend) = crate::linux::LinuxBackend::new() {
                return Sandbox::Linux(backend);
            }
        }
        #[cfg(target_os = "macos")]
        {
            if let Ok(backend) = crate::macos::MacOsBackend::new() {
                return Sandbox::MacOS(backend);
            }
        }
        #[cfg(target_os = "windows")]
        {
            if let Ok(backend) = crate::windows::WindowsBackend::new() {
                return Sandbox::Windows(backend);
            }
        }
        Sandbox::Disabled(crate::disabled::DisabledBackend::new())
    }

    /// Convenience: name of the active backend.
    pub fn name(&self) -> &'static str {
        match self {
            Sandbox::Linux(b) => b.name(),
            Sandbox::MacOS(b) => b.name(),
            Sandbox::Windows(b) => b.name(),
            Sandbox::Disabled(b) => b.name(),
        }
    }

    /// Convenience: whether the active backend enforces isolation.
    pub fn enforces_isolation(&self) -> bool {
        match self {
            Sandbox::Linux(b) => b.enforces_isolation(),
            Sandbox::MacOS(b) => b.enforces_isolation(),
            Sandbox::Windows(b) => b.enforces_isolation(),
            Sandbox::Disabled(b) => b.enforces_isolation(),
        }
    }

    /// Run `config` inside the active backend.
    pub async fn run(&self, config: &SandboxConfig) -> Result<SandboxOutput, SandboxError> {
        config.validate().map_err(SandboxError::InvalidConfig)?;
        match self {
            Sandbox::Linux(b) => b.run(config).await,
            Sandbox::MacOS(b) => b.run(config).await,
            Sandbox::Windows(b) => b.run(config).await,
            Sandbox::Disabled(b) => b.run(config).await,
        }
    }
}
