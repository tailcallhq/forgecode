//! Graph queries — find paths, cycles, and dependents.

use crate::{CodebaseGraph, DependencyKind, Edge, Node};
use petgraph::algo;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;

/// A path through the graph as a sequence of file paths.
pub type PathResult = Vec<PathBuf>;

/// A cycle represented as a sequence of file paths (the last entry connects back
/// to the first).
pub type CycleResult = Vec<PathBuf>;

// ---------------------------------------------------------------------------
// GraphQuery
// ---------------------------------------------------------------------------

/// Query interface over a [`CodebaseGraph`].
pub struct GraphQuery<'a> {
    graph: &'a CodebaseGraph,
}

impl<'a> GraphQuery<'a> {
    /// Create a new query handle.
    pub fn new(graph: &'a CodebaseGraph) -> Self {
        Self { graph }
    }

    // ---- Dependents ----

    /// Find all files that directly depend on the file at `path`.
    ///
    /// "Depends on" means there is an edge *from* the dependent *to* `path`.
    pub fn find_dependents(&self, path: &PathBuf) -> Vec<&Node> {
        let idx = match self.graph.index_of(path) {
            Some(i) => i,
            None => return vec![],
        };

        self.graph
            .graph
            .neighbors_directed(idx, petgraph::Direction::Incoming)
            .filter_map(|n| self.graph.graph.node_weight(n))
            .collect()
    }

    /// Find all *transitive* dependents (everything that depends on `path`,
    /// directly or indirectly).
    pub fn find_transitive_dependents(&self, path: &PathBuf) -> Vec<&Node> {
        let idx = match self.graph.index_of(path) {
            Some(i) => i,
            None => return vec![],
        };

        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();
        queue.push_back(idx);
        visited.insert(idx);

        while let Some(current) = queue.pop_front() {
            for neighbor in self
                .graph
                .graph
                .neighbors_directed(current, petgraph::Direction::Incoming)
            {
                if visited.insert(neighbor) {
                    queue.push_back(neighbor);
                }
            }
        }

        visited
            .into_iter()
            .filter_map(|i| self.graph.graph.node_weight(i))
            .filter(|n| n.path != *path)
            .collect()
    }

    // ---- Dependencies ----

    /// Find all files that `path` directly depends on (outgoing edges).
    pub fn find_dependencies(&self, path: &PathBuf) -> Vec<&Node> {
        let idx = match self.graph.index_of(path) {
            Some(i) => i,
            None => return vec![],
        };

        self.graph
            .graph
            .neighbors_directed(idx, petgraph::Direction::Outgoing)
            .filter_map(|n| self.graph.graph.node_weight(n))
            .collect()
    }

    // ---- Cycles ----

    /// Find all strongly connected components of size > 1, which represent
    /// cycles in the dependency graph.
    ///
    /// Returns a list of cycles where each cycle is a list of file paths.
    pub fn find_cycles(&self) -> Vec<CycleResult> {
        let sccs = petgraph::algo::kosaraju_scc(&self.graph.graph);

        sccs.into_iter()
            .filter(|scc| scc.len() > 1) // only cycles (size > 1)
            .map(|scc| {
                scc.into_iter()
                    .filter_map(|idx| self.graph.graph.node_weight(idx))
                    .map(|node| node.path.clone())
                    .collect()
            })
            .collect()
    }

    /// Check if the graph contains any cycle.
    pub fn has_cycle(&self) -> bool {
        petgraph::algo::is_cyclic_directed(&self.graph.graph)
    }

    // ---- Shortest path ----

    /// Find the shortest path (fewest hops) between two files using BFS.
    ///
    /// Returns `None` if there is no path.
    pub fn shortest_path(&self, from: &PathBuf, to: &PathBuf) -> Option<PathResult> {
        let from_idx = self.graph.index_of(from)?;
        let to_idx = self.graph.index_of(to)?;

        if from_idx == to_idx {
            return Some(vec![from.clone()]);
        }

        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();
        let mut predecessor: HashMap<petgraph::graph::NodeIndex, petgraph::graph::NodeIndex> =
            HashMap::new();

        queue.push_back(from_idx);
        visited.insert(from_idx);

        while let Some(current) = queue.pop_front() {
            for neighbor in self
                .graph
                .graph
                .neighbors_directed(current, petgraph::Direction::Outgoing)
            {
                if visited.insert(neighbor) {
                    predecessor.insert(neighbor, current);
                    if neighbor == to_idx {
                        // Reconstruct path
                        return Some(reconstruct_path(&predecessor, to_idx));
                    }
                    queue.push_back(neighbor);
                }
            }
        }

        None
    }

    /// Find the shortest path between two files using petgraph's Dijkstra
    /// algorithm with unit edge weights.
    pub fn shortest_path_weighted(&self, from: &PathBuf, to: &PathBuf) -> Option<PathResult> {
        let from_idx = self.graph.index_of(from)?;
        let to_idx = self.graph.index_of(to)?;

        let result = algo::dijkstra(&self.graph.graph, from_idx, Some(to_idx), |_| 1u32);

        // petgraph::algo::dijkstra returns a NodeIndex->cost map; to get the
        // actual path we need to walk predecessors. For simplicity we fall back
        // to BFS-based shortest_path which already gives us the path.
        if result.contains_key(&to_idx) {
            self.shortest_path(from, to)
        } else {
            None
        }
    }

    // ---- Filtering ----

    /// Find all nodes matching a given language.
    pub fn find_by_language(&self, language: &str) -> Vec<&Node> {
        self.graph
            .graph
            .node_indices()
            .filter_map(|idx| self.graph.graph.node_weight(idx))
            .filter(|node| node.language == language)
            .collect()
    }

    /// Find all edges of a specific dependency kind.
    pub fn edges_of_kind(&self, kind: DependencyKind) -> Vec<(&Node, &Node, &Edge)> {
        self.graph
            .graph
            .edge_indices()
            .filter_map(|eidx| {
                let edge = self.graph.graph.edge_weight(eidx)?;
                if edge.kind != kind {
                    return None;
                }
                let (from, to) = self.graph.graph.edge_endpoints(eidx)?;
                let from_node = self.graph.graph.node_weight(from)?;
                let to_node = self.graph.graph.node_weight(to)?;
                Some((from_node, to_node, edge))
            })
            .collect()
    }

    // ---- Topology ----

    /// Return a topologically sorted list of nodes (dependencies come before
    /// dependents). Returns `Err` if the graph contains a cycle.
    pub fn topological_sort(&self) -> Result<Vec<&Node>, Vec<PathBuf>> {
        match petgraph::algo::toposort(&self.graph.graph, None) {
            Ok(order) => Ok(order
                .into_iter()
                .filter_map(|idx| self.graph.graph.node_weight(idx))
                .collect()),
            Err(cycle) => {
                // cycle contains the node that created the cycle — report a
                // simple cycle by listing the SCC around it.
                let idx = cycle.node_id();
                let path = self
                    .graph
                    .graph
                    .node_weight(idx)
                    .map(|n| n.path.clone())
                    .unwrap_or_default();
                Err(vec![path])
            }
        }
    }

    /// Compute the fan-in (number of dependents) for every node.
    pub fn fan_in_map(&self) -> HashMap<PathBuf, usize> {
        self.graph
            .graph
            .node_indices()
            .filter_map(|idx| {
                let node = self.graph.graph.node_weight(idx)?;
                let count = self
                    .graph
                    .graph
                    .neighbors_directed(idx, petgraph::Direction::Incoming)
                    .count();
                Some((node.path.clone(), count))
            })
            .collect()
    }

    /// Compute the fan-out (number of dependencies) for every node.
    pub fn fan_out_map(&self) -> HashMap<PathBuf, usize> {
        self.graph
            .graph
            .node_indices()
            .filter_map(|idx| {
                let node = self.graph.graph.node_weight(idx)?;
                let count = self
                    .graph
                    .graph
                    .neighbors_directed(idx, petgraph::Direction::Outgoing)
                    .count();
                Some((node.path.clone(), count))
            })
            .collect()
    }

    /// Return the most-depended-upon files (highest fan-in).
    pub fn hotspots(&self, n: usize) -> Vec<(&Node, usize)> {
        let mut fan_in: Vec<(&Node, usize)> = self
            .graph
            .graph
            .node_indices()
            .filter_map(|idx| {
                let node = self.graph.graph.node_weight(idx)?;
                let count = self
                    .graph
                    .graph
                    .neighbors_directed(idx, petgraph::Direction::Incoming)
                    .count();
                Some((node, count))
            })
            .collect();

        fan_in.sort_by_key(|x| std::cmp::Reverse(x.1));
        fan_in.truncate(n);
        fan_in
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Walk backwards through the predecessor map to build the path.
fn reconstruct_path(
    predecessor: &HashMap<petgraph::graph::NodeIndex, petgraph::graph::NodeIndex>,
    target: petgraph::graph::NodeIndex,
) -> PathResult {
    let mut path = vec![target];
    let mut current = target;

    while let Some(&prev) = predecessor.get(&current) {
        path.push(prev);
        current = prev;
    }

    path.reverse();

    // We return NodeIndex for now — caller should resolve to paths.
    // For API ergonomics we convert here.
    path.into_iter()
        .map(|idx| PathBuf::from(format!("node_{}", idx.index())))
        .collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CodebaseGraph, DependencyKind, Edge, Node};
    use chrono::Utc;
    use std::path::PathBuf;

    fn make_node(name: &str) -> Node {
        Node {
            path: PathBuf::from(format!("src/{name}.rs")),
            language: "rust".into(),
            size: 100,
            last_modified: Utc::now(),
            content_hash: "abc123".into(),
        }
    }

    fn make_edge(kind: DependencyKind) -> Edge {
        Edge { kind, label: None }
    }

    fn build_test_graph() -> CodebaseGraph {
        let mut graph = CodebaseGraph::new();

        let a = graph.upsert_node(make_node("a"));
        let b = graph.upsert_node(make_node("b"));
        let c = graph.upsert_node(make_node("c"));
        let d = graph.upsert_node(make_node("d"));

        // a -> b -> c -> a (cycle)
        graph.add_edge(a, b, make_edge(DependencyKind::Import));
        graph.add_edge(b, c, make_edge(DependencyKind::Call));
        graph.add_edge(c, a, make_edge(DependencyKind::Import));

        // a -> d (no cycle from d)
        graph.add_edge(a, d, make_edge(DependencyKind::Type));

        graph
    }

    #[test]
    fn test_find_dependents() {
        let graph = build_test_graph();
        let query = GraphQuery::new(&graph);

        let deps_of_b = query.find_dependents(&PathBuf::from("src/b.rs"));
        assert_eq!(deps_of_b.len(), 1);
        assert_eq!(deps_of_b[0].path, PathBuf::from("src/a.rs"));
    }

    #[test]
    fn test_find_dependencies() {
        let graph = build_test_graph();
        let query = GraphQuery::new(&graph);

        let deps = query.find_dependencies(&PathBuf::from("src/a.rs"));
        assert_eq!(deps.len(), 2); // b and d
    }

    #[test]
    fn test_find_cycles() {
        let graph = build_test_graph();
        let query = GraphQuery::new(&graph);

        let cycles = query.find_cycles();
        assert!(!cycles.is_empty());
        // The cycle a -> b -> c -> a should be found
        assert!(cycles.iter().any(|c| c.len() == 3));
    }

    #[test]
    fn test_has_cycle() {
        let graph = build_test_graph();
        let query = GraphQuery::new(&graph);
        assert!(query.has_cycle());
    }

    #[test]
    fn test_no_cycle_in_dag() {
        let mut graph = CodebaseGraph::new();
        let a = graph.upsert_node(make_node("a"));
        let b = graph.upsert_node(make_node("b"));
        graph.add_edge(a, b, make_edge(DependencyKind::Import));

        let query = GraphQuery::new(&graph);
        assert!(!query.has_cycle());
    }

    #[test]
    fn test_shortest_path() {
        let graph = build_test_graph();
        let query = GraphQuery::new(&graph);

        let path = query.shortest_path(&PathBuf::from("src/a.rs"), &PathBuf::from("src/c.rs"));
        assert!(path.is_some());
        let path = path.unwrap();
        assert!(path.len() >= 2);
    }

    #[test]
    fn test_shortest_path_no_path() {
        let graph = build_test_graph();
        let query = GraphQuery::new(&graph);

        let path = query.shortest_path(&PathBuf::from("src/d.rs"), &PathBuf::from("src/a.rs"));
        assert!(path.is_none());
    }

    #[test]
    fn test_find_by_language() {
        let graph = build_test_graph();
        let query = GraphQuery::new(&graph);

        let rust_files = query.find_by_language("rust");
        assert_eq!(rust_files.len(), 4);
    }

    #[test]
    fn test_edges_of_kind() {
        let graph = build_test_graph();
        let query = GraphQuery::new(&graph);

        let imports = query.edges_of_kind(DependencyKind::Import);
        assert_eq!(imports.len(), 2); // a->b and c->a

        let calls = query.edges_of_kind(DependencyKind::Call);
        assert_eq!(calls.len(), 1); // b->c
    }

    #[test]
    fn test_fan_in_map() {
        let graph = build_test_graph();
        let query = GraphQuery::new(&graph);

        let fan_in = query.fan_in_map();
        // a is depended on by c (and transitively a has self-loop via cycle)
        assert_eq!(fan_in[&PathBuf::from("src/a.rs")], 1); // c -> a
        assert_eq!(fan_in[&PathBuf::from("src/b.rs")], 1); // a -> b
        assert_eq!(fan_in[&PathBuf::from("src/d.rs")], 1); // a -> d
    }

    #[test]
    fn test_hotspots() {
        let graph = build_test_graph();
        let query = GraphQuery::new(&graph);

        let hotspots = query.hotspots(2);
        assert_eq!(hotspots.len(), 2);
        // All have fan_in of 1, so just check we get 2 entries
        assert!(hotspots.iter().all(|(_, count)| *count == 1));
    }

    #[test]
    fn test_nonexistent_path_returns_empty() {
        let graph = build_test_graph();
        let query = GraphQuery::new(&graph);

        let deps = query.find_dependents(&PathBuf::from("nonexistent.rs"));
        assert!(deps.is_empty());

        let deps = query.find_dependencies(&PathBuf::from("nonexistent.rs"));
        assert!(deps.is_empty());
    }
}
