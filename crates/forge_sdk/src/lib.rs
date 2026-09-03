//! # ForgeSDK
//!
//! SDK for programmatic use of ForgeCode / HeliosLite AI coding agent.
//!
//! This crate provides a high-level API for embedding ForgeCode's AI coding
//! capabilities into your own applications.
//!
//! ## Quick Start
//!
//! ```rust,ignore
//! use forge_sdk::*;
//!
//! // Create an API instance
//! let cwd = std::path::PathBuf::from(".");
//! let config = ForgeConfig::from_cwd(cwd.clone())?;
//! let api = ForgeAPI::<ForgeServices>::init(cwd, config);
//!
//! // List available models
//! let models = api.models().await?;
//!
//! // Start a conversation
//! let conversation = Conversation::default();
//!
//! // Dispatch a message
//! let stream = api.dispatch(conversation, Agent::default()).await?;
//! ```
//!
//! ## Features
//!
//! - **40+ LLM providers** — OpenAI, Anthropic, Google, Ollama, and more
//! - **Session persistence** — SQLite-backed conversation store with WAL
//! - **Context compression** — 4-phase pipeline for long sessions
//! - **Local-first** — your code never leaves your machine
//!
//! ## Architecture
//!
//! The SDK is built on a hexagonal architecture with trait-based boundaries:
//!
//! - [`forge_domain`] — Core domain types (Agent, Conversation, Tool, etc.)
//! - [`forge_api`] — High-level API for conversation management
//! - [`forge_config`] — Configuration management

// Re-export the public API from forge_api
pub use forge_api::*;

// Re-export key domain types
pub use forge_domain::{
    Agent, AgentId, Context, Conversation, ConversationId, Model, ModelId, Provider, ProviderId,
    ToolCallFull, ToolResult,
};

// Re-export configuration
pub use forge_config::ForgeConfig;
