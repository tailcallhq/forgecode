//! # forge_graph
//!
//! Codebase dependency graph — scan, model, and query source-file relationships.
//!
//! This crate provides a directed graph representation of a codebase where:
//! - **Nodes** represent source files with metadata (path, language, size, last modified).
//! - **Edges** represent dependencies between files (imports, calls, type references).
//!
//! ## Usage
//!
//! ```no_run
//! use forge_graph::{GraphBuilder, GraphQuery};
//! use std::path::PathBuf;
//!
//! # async fn example() -> anyhow::Result<()> {
//! let mut builder = GraphBuilder::new(PathBuf::from("."));
//! let graph = builder.build().await?;
//!
//! let query = GraphQuery::new(&graph);
//! let cycles = query.find_cycles();
//! println!("Found {} cycles", cycles.len());
//! # Ok(())
//! # }
//! ```

pub mod builder;
pub mod query;

pub use builder::GraphBuilder;
pub use query::GraphQuery;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Unique identifier for a node in the graph.
pub type NodeIndex = petgraph::graph::NodeIndex;

/// The underlying directed graph type.
pub type InnerGraph = petgraph::graph::DiGraph<Node, Edge>;

// ---------------------------------------------------------------------------
// Node
// ---------------------------------------------------------------------------

/// A source file node in the dependency graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    /// Absolute or workspace-relative path to the source file.
    pub path: PathBuf,
    /// Detected programming language (e.g. "rust", "python", "typescript").
    pub language: String,
    /// File size in bytes.
    pub size: u64,
    /// Last-modified timestamp (filesystem mtime).
    pub last_modified: DateTime<Utc>,
    /// A content hash so we can detect unchanged files for incremental updates.
    pub content_hash: String,
}

// ---------------------------------------------------------------------------
// Edge
// ---------------------------------------------------------------------------

/// The kind of dependency relationship between two source files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DependencyKind {
    /// `use` / `import` / `from` statement.
    Import,
    /// Function or method call across files.
    Call,
    /// Type reference (struct, enum, trait alias, etc.).
    Type,
}

/// A directed edge representing a dependency from one file to another.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    /// What kind of dependency this edge represents.
    pub kind: DependencyKind,
    /// Optional human-readable label (e.g. the specific symbol referenced).
    pub label: Option<String>,
}

// ---------------------------------------------------------------------------
// CodebaseGraph
// ---------------------------------------------------------------------------

/// The main graph structure that holds the full codebase dependency map.
#[derive(Debug)]
pub struct CodebaseGraph {
    /// The underlying petgraph directed graph.
    pub graph: InnerGraph,
    /// Fast lookup from file path → node index.
    pub path_index: HashMap<PathBuf, NodeIndex>,
}

impl CodebaseGraph {
    /// Create an empty graph.
    pub fn new() -> Self {
        Self { graph: InnerGraph::new(), path_index: HashMap::new() }
    }

    /// Number of nodes (files) in the graph.
    pub fn node_count(&self) -> usize {
        self.graph.node_count()
    }

    /// Number of edges (dependencies) in the graph.
    pub fn edge_count(&self) -> usize {
        self.graph.edge_count()
    }

    /// Look up a node by its file path.
    pub fn node_by_path(&self, path: &PathBuf) -> Option<&Node> {
        self.path_index
            .get(path)
            .and_then(|&idx| self.graph.node_weight(idx))
    }

    /// Look up the node index for a given path.
    pub fn index_of(&self, path: &PathBuf) -> Option<NodeIndex> {
        self.path_index.get(path).copied()
    }

    /// Insert or update a node, returning its index.
    /// If the path already exists the metadata is updated in place.
    pub fn upsert_node(&mut self, node: Node) -> NodeIndex {
        if let Some(&existing) = self.path_index.get(&node.path) {
            if let Some(weight) = self.graph.node_weight_mut(existing) {
                *weight = node;
            }
            existing
        } else {
            let idx = self.graph.add_node(node.clone());
            self.path_index.insert(node.path, idx);
            idx
        }
    }

    /// Remove a node (and all its incident edges) by path.
    pub fn remove_node_by_path(&mut self, path: &PathBuf) -> Option<Node> {
        let idx = self.path_index.remove(path)?;
        self.graph.remove_node(idx)
    }

    /// Add a directed edge between two node indices.
    pub fn add_edge(&mut self, from: NodeIndex, to: NodeIndex, edge: Edge) {
        self.graph.add_edge(from, to, edge);
    }

    /// Clear the entire graph.
    pub fn clear(&mut self) {
        self.graph.clear();
        self.path_index.clear();
    }
}

impl Default for CodebaseGraph {
    fn default() -> Self {
        Self::new()
    }
}
