//! Fish shell integration.
//!
//! This module provides all Fish-related functionality including:
//! - Plugin generation and installation
//! - Theme generation
//! - Shell diagnostics

mod plugin;
mod rprompt;

pub use plugin::{
    generate_fish_plugin, generate_fish_theme, run_fish_doctor, setup_fish_integration,
    teardown_fish_integration,
};
pub use rprompt::FishRPrompt;
