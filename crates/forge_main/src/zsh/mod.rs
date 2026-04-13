//! ZSH shell integration.
//!
//! This module provides all ZSH-related functionality including:
//! - Plugin generation and installation
//! - Theme generation
//! - Shell diagnostics
//! - Right prompt (rprompt) display
//! - Prompt styling utilities

pub(crate) mod paste;
mod plugin;
mod rprompt;
mod style;

/// Normalizes shell script content for cross-platform compatibility.
///
/// Strips carriage returns (`\r`) that appear when `include_str!` or
/// `include_dir!` embed files on Windows (where `git core.autocrlf=true`
/// converts LF to CRLF on checkout). Zsh cannot parse `\r` in scripts.
pub(crate) fn normalize_script(content: &str) -> String {
    content.replace("\r\n", "\n").replace('\r', "\n")
}

pub use plugin::{
    generate_zsh_plugin, generate_zsh_theme, run_zsh_doctor, run_zsh_keyboard,
    setup_zsh_integration,
};
pub use rprompt::ZshRPrompt;
