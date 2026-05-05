//! Terminal utilities with graceful degradation for cursor position errors.
//!
//! The crossterm library uses a 2-second timeout when reading cursor position via
//! the CSI `ESC [ 6 n` escape sequence. In certain conditions (multiple concurrent sessions,
//! terminal not responding, non-interactive environments), this can fail with:
//! "The cursor position could not be read within a normal duration"
//!
//! This module provides wrapper functions that retry cursor operations with exponential
//! backoff and gracefully degrade when cursor position cannot be determined.
//!
//! See: plans/2026-05-04-forge-cursor-error-investigation.md

use std::io;
use std::time::Duration;

/// Default retry configuration
const MAX_ATTEMPTS: u32 = 3;
const BASE_DELAY_MS: u64 = 100;

/// Result type for cursor operations that can fail gracefully
pub type CursorResult<T> = Result<T, CursorError>;

/// Errors that can occur when reading cursor position
#[derive(Debug, Clone)]
pub enum CursorError {
    /// The cursor position could not be read within the timeout
    Timeout,
    /// The terminal is not in raw mode or not available
    NotAvailable,
    /// Generic I/O error
    Io(io::Error),
}

impl std::fmt::Display for CursorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CursorError::Timeout => write!(f, "The cursor position could not be read within a normal duration"),
            CursorError::NotAvailable => write!(f, "Terminal cursor position not available"),
            CursorError::Io(e) => write!(f, "I/O error reading cursor position: {}", e),
        }
    }
}

impl std::error::Error for CursorError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            CursorError::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for CursorError {
    fn from(err: io::Error) -> Self {
        // Check if this is the cursor timeout error
        if err.to_string().contains("cursor position could not be read") {
            CursorError::Timeout
        } else {
            CursorError::Io(err)
        }
    }
}

/// Gets the cursor position with retry logic and graceful degradation.
///
/// This function wraps `crossterm::cursor::position()` with:
/// - Retry logic with exponential backoff
/// - Logging of transient failures
/// - Graceful fallback to (0, 0) after max retries
///
/// Returns `(0, 0)` if cursor position cannot be determined after retries.
pub fn get_cursor_position_with_retry() -> (u16, u16) {
    get_cursor_position_with_config(MAX_ATTEMPTS, BASE_DELAY_MS)
}

/// Gets the cursor position with configurable retry behavior.
///
/// # Arguments
/// * `max_attempts` - Maximum number of retry attempts
/// * `delay_ms` - Base delay between retries in milliseconds
///
/// # Returns
/// * `(col, row)` on success
/// * `(0, 0)` on failure after all retries
pub fn get_cursor_position_with_config(max_attempts: u32, delay_ms: u64) -> (u16, u16) {
    let mut attempts = 0;

    loop {
        match crossterm::cursor::position() {
            Ok(pos) => return pos,
            Err(e) => {
                attempts += 1;
                if attempts >= max_attempts {
                    // Log the failure but don't crash - use fallback position
                    tracing::warn!(
                        error = %e,
                        attempts = attempts,
                        "Cursor position unavailable after {} attempts, using fallback (0, 0)",
                        attempts
                    );
                    return (0, 0);
                }

                // Exponential backoff: 100ms, 200ms, 400ms, ...
                let delay = Duration::from_millis(delay_ms * 2u64.pow(attempts - 1));
                tracing::debug!(
                    error = %e,
                    attempt = attempts,
                    "Cursor position read failed, retrying in {:?}",
                    delay
                );
                std::thread::sleep(delay);
            }
        }
    }
}

/// Attempts to get cursor position, returning None on failure.
///
/// This is a convenience function that returns `None` instead of a fallback position.
pub fn try_cursor_position() -> Option<(u16, u16)> {
    get_cursor_position_with_config(1, 0); // Single attempt, no retry
    match crossterm::cursor::position() {
        Ok(pos) => Some(pos),
        Err(_) => None,
    }
}

/// Checks if cursor position is currently available.
///
/// This performs a non-blocking check to see if the terminal can report cursor position.
pub fn is_cursor_position_available() -> bool {
    crossterm::cursor::position().is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cursor_error_display() {
        let timeout = CursorError::Timeout;
        assert!(timeout.to_string().contains("could not be read"));

        let not_avail = CursorError::NotAvailable;
        assert!(not_avail.to_string().contains("not available"));
    }

    #[test]
    fn test_cursor_error_from_io() {
        use std::io::ErrorKind;

        // Test timeout error detection
        let timeout_err = io::Error::new(
            ErrorKind::Other,
            "The cursor position could not be read within a normal duration",
        );
        let cursor_err: CursorError = timeout_err.into();
        assert!(matches!(cursor_err, CursorError::Timeout));

        // Test other I/O errors
        let other_err = io::Error::new(ErrorKind::NotFound, "test");
        let cursor_err: CursorError = other_err.into();
        assert!(matches!(cursor_err, CursorError::Io(_)));
    }
}
