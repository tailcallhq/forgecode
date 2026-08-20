//! `forge_repo_map` — a tree-sitter based repository structure map.
//!
//! Walks a repository directory, parses source files using tree-sitter
//! language grammars, and builds a compact symbol/structure map
//! (files → top-level functions, types, structs, enums, traits, imports, etc.)
//! that an agent can consume as repo context.
//!
//! # Example
//!
//! ```ignore
//! use forge_repo_map::RepoMapBuilder;
//!
//! let map = RepoMapBuilder::new()
//!     .cwd("/path/to/repo".into())
//!     .build()?;
//!
//! // Render as compact text
//! println!("{}", map.to_text());
//!
//! // Or as JSON
//! println!("{}", map.to_json()?);
//! # Ok::<_, anyhow::Error>(())
//! ```

pub mod builder;
pub mod parser;
pub mod types;

pub use builder::RepoMapBuilder;
pub use types::RepoMap;
