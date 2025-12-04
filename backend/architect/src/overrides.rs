//! User overrides for the project graph
//!
//! This module defines the data model for user customizations to the graph:
//! - Hidden edges (false positives from import detection)
//! - Manual edges (relationships that analysis can't detect)
//! - Node tags (semantic labels like "service", "publisher", etc.)
//!
//! In v1, these are stored in a simple JSON file per project.
//! In v2+, they can be migrated to redb for better performance and querying.

use crate::EdgeKind;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A manually added edge
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManualEdge {
    /// Source file path
    pub source: String,
    /// Target file path
    pub target: String,
    /// Kind of relationship
    pub kind: EdgeKind,
    /// Optional description
    pub description: Option<String>,
}

/// A tag applied to a node
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NodeTag {
    /// The tag name (e.g., "service", "publisher", "model")
    pub name: String,
    /// Optional color for visualization
    pub color: Option<String>,
}

/// User overrides for a project graph
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GraphOverrides {
    /// Project ID these overrides apply to
    pub project_id: String,

    /// Edges to hide (false positives)
    /// Key: "source_path|target_path"
    #[serde(default)]
    pub hidden_edges: Vec<HiddenEdge>,

    /// Manually added edges
    #[serde(default)]
    pub manual_edges: Vec<ManualEdge>,

    /// Tags applied to nodes
    /// Key: node path, Value: list of tags
    #[serde(default)]
    pub node_tags: HashMap<String, Vec<NodeTag>>,

    /// Last modified timestamp
    pub last_modified: Option<String>,
}

/// A hidden edge (false positive to ignore)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HiddenEdge {
    /// Source file path
    pub source: String,
    /// Target file path
    pub target: String,
    /// Reason for hiding (optional)
    pub reason: Option<String>,
}

impl GraphOverrides {
    /// Create new empty overrides for a project
    pub fn new(project_id: String) -> Self {
        Self {
            project_id,
            hidden_edges: Vec::new(),
            manual_edges: Vec::new(),
            node_tags: HashMap::new(),
            last_modified: None,
        }
    }

    /// Hide an edge (mark as false positive)
    pub fn hide_edge(&mut self, source: &str, target: &str, reason: Option<&str>) {
        // Don't add duplicates
        if !self.is_edge_hidden(source, target) {
            self.hidden_edges.push(HiddenEdge {
                source: source.to_string(),
                target: target.to_string(),
                reason: reason.map(|s| s.to_string()),
            });
            self.touch();
        }
    }

    /// Unhide an edge
    pub fn unhide_edge(&mut self, source: &str, target: &str) {
        self.hidden_edges.retain(|e| e.source != source || e.target != target);
        self.touch();
    }

    /// Check if an edge is hidden
    pub fn is_edge_hidden(&self, source: &str, target: &str) -> bool {
        self.hidden_edges
            .iter()
            .any(|e| e.source == source && e.target == target)
    }

    /// Add a manual edge
    pub fn add_manual_edge(&mut self, edge: ManualEdge) {
        // Don't add duplicates
        if !self.manual_edges.iter().any(|e| e.source == edge.source && e.target == edge.target) {
            self.manual_edges.push(edge);
            self.touch();
        }
    }

    /// Remove a manual edge
    pub fn remove_manual_edge(&mut self, source: &str, target: &str) {
        self.manual_edges.retain(|e| e.source != source || e.target != target);
        self.touch();
    }

    /// Add a tag to a node
    pub fn add_node_tag(&mut self, node_path: &str, tag: NodeTag) {
        let tags = self.node_tags.entry(node_path.to_string()).or_default();
        if !tags.iter().any(|t| t.name == tag.name) {
            tags.push(tag);
            self.touch();
        }
    }

    /// Remove a tag from a node
    pub fn remove_node_tag(&mut self, node_path: &str, tag_name: &str) {
        if let Some(tags) = self.node_tags.get_mut(node_path) {
            tags.retain(|t| t.name != tag_name);
            if tags.is_empty() {
                self.node_tags.remove(node_path);
            }
            self.touch();
        }
    }

    /// Get tags for a node
    pub fn get_node_tags(&self, node_path: &str) -> &[NodeTag] {
        self.node_tags
            .get(node_path)
            .map(|v| v.as_slice())
            .unwrap_or_default()
    }

    /// Update last modified timestamp
    fn touch(&mut self) {
        self.last_modified = Some(chrono::Utc::now().to_rfc3339());
    }

    /// Check if there are any overrides
    pub fn is_empty(&self) -> bool {
        self.hidden_edges.is_empty()
            && self.manual_edges.is_empty()
            && self.node_tags.is_empty()
    }
}

/// Storage trait for graph overrides
///
/// This allows swapping between JSON file storage and redb storage
pub trait OverridesStore {
    /// Load overrides for a project
    fn load(&self, project_id: &str) -> anyhow::Result<Option<GraphOverrides>>;

    /// Save overrides for a project
    fn save(&self, overrides: &GraphOverrides) -> anyhow::Result<()>;

    /// Delete overrides for a project
    fn delete(&self, project_id: &str) -> anyhow::Result<()>;
}

/// JSON file-based storage for overrides (v1 implementation)
pub struct JsonOverridesStore {
    base_dir: std::path::PathBuf,
}

impl JsonOverridesStore {
    /// Create a new JSON overrides store
    pub fn new(base_dir: &std::path::Path) -> anyhow::Result<Self> {
        std::fs::create_dir_all(base_dir)?;
        Ok(Self {
            base_dir: base_dir.to_path_buf(),
        })
    }

    fn overrides_path(&self, project_id: &str) -> std::path::PathBuf {
        let safe_id = project_id.replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_");
        self.base_dir.join(format!("{}.overrides.json", safe_id))
    }
}

impl OverridesStore for JsonOverridesStore {
    fn load(&self, project_id: &str) -> anyhow::Result<Option<GraphOverrides>> {
        let path = self.overrides_path(project_id);
        if !path.exists() {
            return Ok(None);
        }

        let content = std::fs::read_to_string(&path)?;
        let overrides: GraphOverrides = serde_json::from_str(&content)?;
        Ok(Some(overrides))
    }

    fn save(&self, overrides: &GraphOverrides) -> anyhow::Result<()> {
        let path = self.overrides_path(&overrides.project_id);
        let content = serde_json::to_string_pretty(overrides)?;
        std::fs::write(&path, content)?;
        Ok(())
    }

    fn delete(&self, project_id: &str) -> anyhow::Result<()> {
        let path = self.overrides_path(project_id);
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_hide_edge() {
        let mut overrides = GraphOverrides::new("test".to_string());

        overrides.hide_edge("a.rs", "b.rs", Some("false positive"));
        assert!(overrides.is_edge_hidden("a.rs", "b.rs"));
        assert!(!overrides.is_edge_hidden("b.rs", "a.rs"));

        overrides.unhide_edge("a.rs", "b.rs");
        assert!(!overrides.is_edge_hidden("a.rs", "b.rs"));
    }

    #[test]
    fn test_manual_edge() {
        let mut overrides = GraphOverrides::new("test".to_string());

        overrides.add_manual_edge(ManualEdge {
            source: "a.rs".to_string(),
            target: "b.rs".to_string(),
            kind: EdgeKind::Manual,
            description: Some("calls via event bus".to_string()),
        });

        assert_eq!(overrides.manual_edges.len(), 1);

        overrides.remove_manual_edge("a.rs", "b.rs");
        assert!(overrides.manual_edges.is_empty());
    }

    #[test]
    fn test_node_tags() {
        let mut overrides = GraphOverrides::new("test".to_string());

        overrides.add_node_tag(
            "src/service.rs",
            NodeTag {
                name: "service".to_string(),
                color: Some("#3b82f6".to_string()),
            },
        );

        let tags = overrides.get_node_tags("src/service.rs");
        assert_eq!(tags.len(), 1);
        assert_eq!(tags[0].name, "service");

        overrides.remove_node_tag("src/service.rs", "service");
        assert!(overrides.get_node_tags("src/service.rs").is_empty());
    }

    #[test]
    fn test_json_store() {
        let temp = TempDir::new().unwrap();
        let store = JsonOverridesStore::new(temp.path()).unwrap();

        let mut overrides = GraphOverrides::new("/test/project".to_string());
        overrides.hide_edge("a.rs", "b.rs", None);

        store.save(&overrides).unwrap();

        let loaded = store.load("/test/project").unwrap();
        assert!(loaded.is_some());
        assert!(loaded.unwrap().is_edge_hidden("a.rs", "b.rs"));

        store.delete("/test/project").unwrap();
        assert!(store.load("/test/project").unwrap().is_none());
    }
}
