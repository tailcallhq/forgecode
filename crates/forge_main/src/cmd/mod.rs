//! Headless CLI subcommand handlers.
//!
//! These handlers run without the interactive UI state and are wired
//! into the top-level CLI dispatch in `crate::ui::UI::handle_subcommands`.
//! They return `anyhow::Result<()>` so they can be composed with the
//! existing error-handling path.

pub mod ghostty;
