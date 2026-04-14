//! Utility functions for the markdown renderer.

use std::sync::OnceLock;
use std::time::Duration;

/// Terminal theme mode (dark or light).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeMode {
    /// Dark terminal background.
    Dark,
    /// Light terminal background.
    Light,
}

/// Maximum time to wait for a terminal color query response.
const THEME_DETECT_TIMEOUT: Duration = Duration::from_millis(100);

/// Process-wide cache for the detected terminal theme mode.
static THEME_MODE: OnceLock<ThemeMode> = OnceLock::new();

/// Detects the terminal theme mode (dark or light), querying the terminal at
/// most once per process lifetime. Subsequent calls return the cached result.
/// Falls back to dark mode if the terminal does not respond within the timeout.
pub fn detect_theme_mode() -> ThemeMode {
    *THEME_MODE.get_or_init(|| {
        use terminal_colorsaurus::{QueryOptions, ThemeMode as ColorsaurusThemeMode, theme_mode};

        let mut opts = QueryOptions::default();
        opts.timeout = THEME_DETECT_TIMEOUT;
        match theme_mode(opts) {
            Ok(ColorsaurusThemeMode::Light) => ThemeMode::Light,
            Ok(ColorsaurusThemeMode::Dark) | Err(_) => ThemeMode::Dark,
        }
    })
}
