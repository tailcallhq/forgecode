//! Terminal multiplexer bridge abstraction.
//!
//! Provides a [`MuxBridge`] trait that abstracts over terminal multiplexers
//! (tmux, zellij, etc.) and a concrete [`TmuxBridge`](tmux::TmuxBridge)
//! implementation that shells out to the `tmux` binary.

pub mod tmux;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// A single window/pane within a tmux session.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MuxWindow {
    pub id: String,
    pub name: String,
    pub active: bool,
}

/// A single tmux session containing zero or more windows.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MuxSession {
    pub id: String,
    pub name: String,
    pub windows: Vec<MuxWindow>,
}

/// Errors that can occur during mux operations.
#[derive(Debug, Error)]
pub enum MuxError {
    /// An I/O error from the underlying command invocation.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// The output from tmux could not be parsed.
    #[error("parse error: {0}")]
    Parse(String),

    /// The requested operation is not supported by this backend.
    #[error("not supported by this backend")]
    NotSupported,
}

/// Abstract interface for querying a terminal multiplexer.
#[async_trait::async_trait]
pub trait MuxBridge: Send + Sync {
    /// Return all currently active sessions.
    async fn sessions(&self) -> Result<Vec<MuxSession>, MuxError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify that session/window types round-trip through serde JSON.
    #[test]
    fn test_serde_roundtrip() {
        let session = MuxSession {
            id: "$0".into(),
            name: "work".into(),
            windows: vec![MuxWindow { id: "@1".into(), name: "editor".into(), active: true }],
        };

        let json = serde_json::to_string(&session).unwrap();
        let deserialized: MuxSession = serde_json::from_str(&json).unwrap();
        assert_eq!(session, deserialized);

        // Spot-check the JSON structure.
        assert!(json.contains("\"id\":\"$0\""));
        assert!(json.contains("\"name\":\"work\""));
        assert!(json.contains("\"active\":true"));
    }

    /// Display impls for MuxError.
    #[test]
    fn test_mux_error_display() {
        let io_err = MuxError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "tmux not found",
        ));
        assert!(io_err.to_string().contains("tmux not found"));

        let parse_err = MuxError::Parse("bad format".into());
        assert_eq!(parse_err.to_string(), "parse error: bad format");

        let not_supported = MuxError::NotSupported;
        assert_eq!(not_supported.to_string(), "not supported by this backend");
    }
}
