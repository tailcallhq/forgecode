//! # helios-bot
//!
//! GitHub bot that responds to `@helios` mentions on issues and PRs.
//!
//! ## Two operating modes
//!
//! ### 1. Webhook server
//!
//! Listens for GitHub webhook events. When a comment with `@helios <request>`
//! is detected, queues the request for processing and posts back the agent's
//! response.
//!
//! ```text
//!   GitHub ──webhook──▶ helios-bot ──enqueue──▶ forge_main
//!                          │                        │
//!                          └──post reply──┐         │
//!                                         ▼         ▼
//!                                      GitHub (issue/PR comment)
//! ```
//!
//! ### 2. CLI runner
//!
//! `helios-bot run --repo owner/repo --request "..."` — one-shot mode for
//! CI / scripts. Reads the repo, runs the agent, prints the result.
//!
//! ## Authentication
//!
//! - GitHub App installation token, cached in memory, refreshed on 401
//! - Webhook payloads verified via HMAC-SHA256 against
//!   `GITHUB_WEBHOOK_SECRET`

#![allow(clippy::needless_return)]

mod agent;
mod cli;
mod github;
mod webhook;

use anyhow::Result;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    cli::run().await
}
