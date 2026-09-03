pub mod banner;
mod cli;
mod completer;
mod conversation_selector;
mod display_constants;
mod editor;
mod error;
mod highlighter;
mod info;
mod input;
mod logs;
mod model;
mod oauth_callback;
mod porcelain;
mod prompt;
mod sandbox;
mod state;
mod stream_renderer;
mod sync_display;
mod test_runner;
mod title_display;
mod tools_display;
pub mod tracker;
mod ui;
mod utils;
mod vscode;
mod zsh;

mod update;
use std::sync::LazyLock;

pub use cli::{Cli, ListCommand, ListCommandGroup, TopLevelCommand};
use forge_domain::{AgentId, Effort};
pub use sandbox::Sandbox;
pub use test_runner::{TestResult, TestRunner, parse_cargo_test_output};
pub use title_display::*;
pub use ui::UI;
pub use update::{DEFAULT_UPDATE_REPO, current_binary_prefix};

pub static TRACKER: LazyLock<forge_tracker::Tracker> =
    LazyLock::new(forge_tracker::Tracker::default);

/// Render the shell right prompt without constructing the full async app.
///
/// This is intentionally limited to static config/env data so `forge zsh
/// rprompt` stays cheap enough to run on every prompt redraw. Live
/// conversation/model enrichment remains available on the full UI path.
pub fn render_static_zsh_rprompt(config: &forge_config::ForgeConfig) -> String {
    let agent_id = std::env::var("_FORGE_ACTIVE_AGENT")
        .ok()
        .filter(|text| !text.trim().is_empty())
        .map(AgentId::new);

    let use_nerd_font = std::env::var("NERD_FONT")
        .or_else(|_| std::env::var("USE_NERD_FONT"))
        .map(|val| val == "1")
        .unwrap_or(true);

    let terminal_width = std::env::var("COLUMNS")
        .ok()
        .and_then(|s| s.parse::<usize>().ok());

    let reasoning_effort = config
        .reasoning
        .as_ref()
        .and_then(|reasoning| reasoning.effort.clone())
        .map(|effort| match effort {
            forge_config::Effort::None => Effort::None,
            forge_config::Effort::Minimal => Effort::Minimal,
            forge_config::Effort::Low => Effort::Low,
            forge_config::Effort::Medium => Effort::Medium,
            forge_config::Effort::High => Effort::High,
            forge_config::Effort::XHigh => Effort::XHigh,
            forge_config::Effort::Max => Effort::Max,
        });

    zsh::ZshRPrompt::from_config(config)
        .agent(agent_id)
        .reasoning_effort(reasoning_effort)
        .terminal_width(terminal_width)
        .use_nerd_font(use_nerd_font)
        .to_string()
}
