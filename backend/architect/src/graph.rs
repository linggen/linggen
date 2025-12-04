//! Graph data structures for file dependencies

use crate::Language;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A node representing a source file in the project graph
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileNode {
    /// Unique identifier (relative file path from project root)
    pub id: String,
    /// Display label (usually the file name)
    pub label: String,
    /// Programming language of the file
    pub language: Language,
    /// Parent folder path (relative to project root)
    pub folder: String,
}

/// Type of relationship between files
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EdgeKind {
    /// Direct import/use statement
    Import,
    /// Module declaration (mod foo;)
    ModuleDecl,
    /// Re-export (pub use)
    ReExport,
    /// Manual edge added by user
    Manual,
}

impl std::fmt::Display for EdgeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EdgeKind::Import => write!(f, "import"),
            EdgeKind::ModuleDecl => write!(f, "mod"),
            EdgeKind::ReExport => write!(f, "reexport"),
            EdgeKind::Manual => write!(f, "manual"),
        }
    }
}

/// An edge representing a dependency relationship between two files
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    /// Source file (the file that imports/uses)
    pub source: String,
    /// Target file (the file being imported/used)
    pub target: String,
    /// Type of relationship
    pub kind: EdgeKind,
}

/// The complete project dependency graph
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectGraph {
    /// Project identifier (usually the root path)
    pub project_id: String,
    /// All file nodes in the graph
    pub nodes: Vec<FileNode>,
    /// All edges (dependencies) in the graph
    pub edges: Vec<Edge>,
    /// Index for quick node lookup by id
    #[serde(skip)]
    node_index: HashMap<String, usize>,
    /// Timestamp when the graph was built
    pub built_at: Option<String>,
}

impl ProjectGraph {
    /// Create a new empty project graph
    pub fn new(project_id: String) -> Self {
        Self {
            project_id,
            nodes: Vec::new(),
            edges: Vec::new(),
            node_index: HashMap::new(),
            built_at: None,
        }
    }

    /// Add a node to the graph
    pub fn add_node(&mut self, node: FileNode) {
        if !self.node_index.contains_key(&node.id) {
            let idx = self.nodes.len();
            self.node_index.insert(node.id.clone(), idx);
            self.nodes.push(node);
        }
    }

    /// Add an edge to the graph
    pub fn add_edge(&mut self, edge: Edge) {
        // Only add edge if both source and target nodes exist
        if self.node_index.contains_key(&edge.source)
            && self.node_index.contains_key(&edge.target)
            && edge.source != edge.target
        {
            // Avoid duplicate edges
            let exists = self
                .edges
                .iter()
                .any(|e| e.source == edge.source && e.target == edge.target && e.kind == edge.kind);
            if !exists {
                self.edges.push(edge);
            }
        }
    }

    /// Get a node by its id
    pub fn get_node(&self, id: &str) -> Option<&FileNode> {
        self.node_index.get(id).map(|&idx| &self.nodes[idx])
    }

    /// Get all edges originating from a node
    pub fn get_outgoing_edges(&self, node_id: &str) -> Vec<&Edge> {
        self.edges.iter().filter(|e| e.source == node_id).collect()
    }

    /// Get all edges pointing to a node
    pub fn get_incoming_edges(&self, node_id: &str) -> Vec<&Edge> {
        self.edges.iter().filter(|e| e.target == node_id).collect()
    }

    /// Get the total number of nodes
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Get the total number of edges
    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

    /// Set the build timestamp
    pub fn set_built_at(&mut self, timestamp: String) {
        self.built_at = Some(timestamp);
    }

    /// Get neighbors of a node (1-hop)
    pub fn get_neighbors(&self, node_id: &str) -> Vec<&str> {
        let mut neighbors: Vec<&str> = Vec::new();

        // Outgoing edges (imports)
        for edge in &self.edges {
            if edge.source == node_id {
                neighbors.push(&edge.target);
            }
            if edge.target == node_id {
                neighbors.push(&edge.source);
            }
        }

        neighbors.sort();
        neighbors.dedup();
        neighbors
    }

    /// Get a subgraph containing only nodes within k hops of the given node
    pub fn get_neighborhood(&self, node_id: &str, k: usize) -> ProjectGraph {
        let mut subgraph = ProjectGraph::new(self.project_id.clone());
        let mut visited: HashMap<&str, usize> = HashMap::new();
        let mut queue: Vec<(&str, usize)> = vec![(node_id, 0)];

        while let Some((current, depth)) = queue.pop() {
            if depth > k || visited.contains_key(current) {
                continue;
            }
            visited.insert(current, depth);

            if let Some(node) = self.get_node(current) {
                subgraph.add_node(node.clone());

                if depth < k {
                    for neighbor in self.get_neighbors(current) {
                        if !visited.contains_key(neighbor) {
                            queue.push((neighbor, depth + 1));
                        }
                    }
                }
            }
        }

        // Add edges between nodes in the subgraph
        for edge in &self.edges {
            if visited.contains_key(edge.source.as_str())
                && visited.contains_key(edge.target.as_str())
            {
                subgraph.add_edge(edge.clone());
            }
        }

        subgraph
    }

    /// Filter graph by folder prefix
    pub fn filter_by_folder(&self, folder_prefix: &str) -> ProjectGraph {
        let mut subgraph = ProjectGraph::new(self.project_id.clone());

        for node in &self.nodes {
            if node.folder.starts_with(folder_prefix) || node.id.starts_with(folder_prefix) {
                subgraph.add_node(node.clone());
            }
        }

        for edge in &self.edges {
            if subgraph.node_index.contains_key(&edge.source)
                && subgraph.node_index.contains_key(&edge.target)
            {
                subgraph.add_edge(edge.clone());
            }
        }

        subgraph
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add_node() {
        let mut graph = ProjectGraph::new("test".to_string());
        graph.add_node(FileNode {
            id: "src/main.rs".to_string(),
            label: "main.rs".to_string(),
            language: Language::Rust,
            folder: "src".to_string(),
        });

        assert_eq!(graph.node_count(), 1);
        assert!(graph.get_node("src/main.rs").is_some());
    }

    #[test]
    fn test_add_edge() {
        let mut graph = ProjectGraph::new("test".to_string());
        graph.add_node(FileNode {
            id: "src/main.rs".to_string(),
            label: "main.rs".to_string(),
            language: Language::Rust,
            folder: "src".to_string(),
        });
        graph.add_node(FileNode {
            id: "src/lib.rs".to_string(),
            label: "lib.rs".to_string(),
            language: Language::Rust,
            folder: "src".to_string(),
        });
        graph.add_edge(Edge {
            source: "src/main.rs".to_string(),
            target: "src/lib.rs".to_string(),
            kind: EdgeKind::Import,
        });

        assert_eq!(graph.edge_count(), 1);
    }

    #[test]
    fn test_no_self_loops() {
        let mut graph = ProjectGraph::new("test".to_string());
        graph.add_node(FileNode {
            id: "src/main.rs".to_string(),
            label: "main.rs".to_string(),
            language: Language::Rust,
            folder: "src".to_string(),
        });
        graph.add_edge(Edge {
            source: "src/main.rs".to_string(),
            target: "src/main.rs".to_string(),
            kind: EdgeKind::Import,
        });

        assert_eq!(graph.edge_count(), 0);
    }

    #[test]
    fn test_get_neighbors() {
        let mut graph = ProjectGraph::new("test".to_string());
        graph.add_node(FileNode {
            id: "a.rs".to_string(),
            label: "a.rs".to_string(),
            language: Language::Rust,
            folder: "".to_string(),
        });
        graph.add_node(FileNode {
            id: "b.rs".to_string(),
            label: "b.rs".to_string(),
            language: Language::Rust,
            folder: "".to_string(),
        });
        graph.add_node(FileNode {
            id: "c.rs".to_string(),
            label: "c.rs".to_string(),
            language: Language::Rust,
            folder: "".to_string(),
        });
        graph.add_edge(Edge {
            source: "a.rs".to_string(),
            target: "b.rs".to_string(),
            kind: EdgeKind::Import,
        });
        graph.add_edge(Edge {
            source: "c.rs".to_string(),
            target: "a.rs".to_string(),
            kind: EdgeKind::Import,
        });

        let neighbors = graph.get_neighbors("a.rs");
        assert_eq!(neighbors.len(), 2);
        assert!(neighbors.contains(&"b.rs"));
        assert!(neighbors.contains(&"c.rs"));
    }
}
