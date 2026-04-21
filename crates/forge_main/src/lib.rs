pub mod banner;
mod cli;
mod clipboard;
mod completer;
mod conversation_selector;
mod display_constants;
mod editor;
mod error;
mod fish;
mod highlighter;
mod info;
mod input;
mod model;
mod oauth_callback;
mod porcelain;
mod prompt;
mod sandbox;
mod state;
mod stream_renderer;
mod sync_display;
mod title_display;
mod tools_display;
pub mod tracker;
mod ui;
mod utils;
mod vscode;
mod zsh;

mod update;

use std::sync::LazyLock;

/// Application version, set by `build.rs` from git tags or `APP_VERSION` env var.
const VERSION: &str = env!("CARGO_PKG_VERSION");

pub use cli::{Cli, ListCommand, ListCommandGroup, TopLevelCommand};
pub use sandbox::Sandbox;
pub use title_display::*;
pub use ui::UI;

pub static TRACKER: LazyLock<forge_tracker::Tracker> =
    LazyLock::new(forge_tracker::Tracker::default);
