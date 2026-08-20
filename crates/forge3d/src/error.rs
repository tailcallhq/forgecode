/// Errors emitted by the forge3d daemon.
///
/// All fallible operations in `forge3d` funnel through this enum. It is the
/// only error type callers need to know about; per-module errors are converted
/// via `From` impls defined next to each concern.
#[derive(Debug, thiserror::Error)]
pub enum Forge3Error {
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("lock error: {0}")]
    Lock(String),

    #[error("registry error: {0}")]
    Registry(String),

    #[error("protocol error: {0}")]
    Protocol(String),

    #[error("invalid frame length: {0}")]
    FrameLength(u32),

    #[error("agent '{0}' not registered")]
    UnknownAgent(String),

    #[error("alert '{0}' not found")]
    UnknownAlert(String),

    #[error("another forge3d daemon holds the lock (pid={0})")]
    LockHeld(u32),

    #[error("daemon already started on this socket")]
    AlreadyRunning,
}

impl Forge3Error {
    /// Stable error code used in JSON-RPC `error.code` fields.
    ///
    /// Values are negative and follow JSON-RPC 2.0 reserved ranges where
    /// possible, with internal errors in `-32000..-32099`.
    pub fn code(&self) -> i32 {
        match self {
            Forge3Error::Protocol(_) => -32600,
            Forge3Error::FrameLength(_) => -32601,
            Forge3Error::UnknownAgent(_) => -32010,
            Forge3Error::UnknownAlert(_) => -32011,
            Forge3Error::LockHeld(_) => -32012,
            Forge3Error::AlreadyRunning => -32013,
            Forge3Error::Lock(_) | Forge3Error::Registry(_) => -32014,
            Forge3Error::Io(_) | Forge3Error::Json(_) => -32603,
        }
    }
}

/// Convenience alias used throughout the crate.
pub type Result<T> = std::result::Result<T, Forge3Error>;

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    /// Monotonic counter used to assign unique alert ids without coordinating
    /// with SQLite. The actual uniqueness is enforced by the `alerts.id
    /// PRIMARY KEY` constraint; collisions are detected and re-keyed at
    /// insert time.
    #[derive(Debug, Default)]
    struct AlertCounter(AtomicU64);

    impl AlertCounter {
        fn next(&self) -> u64 {
            self.0.fetch_add(1, Ordering::Relaxed)
        }
    }

    #[test]
    fn code_is_stable_for_each_variant() {
        // Lock the numeric codes so clients can branch on them.
        assert_eq!(Forge3Error::Protocol("x".into()).code(), -32600);
        assert_eq!(Forge3Error::FrameLength(7).code(), -32601);
        assert_eq!(Forge3Error::UnknownAgent("a".into()).code(), -32010);
        assert_eq!(Forge3Error::UnknownAlert("b".into()).code(), -32011);
        assert_eq!(Forge3Error::LockHeld(1).code(), -32012);
        assert_eq!(Forge3Error::AlreadyRunning.code(), -32013);
    }

    #[test]
    fn counter_is_monotonic_per_instance() {
        let c = AlertCounter::default();
        assert_eq!(c.next(), 0);
        assert_eq!(c.next(), 1);
        assert_eq!(c.next(), 2);
    }
}
