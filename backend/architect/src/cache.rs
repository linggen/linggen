//! Graph caching and persistence

use crate::ProjectGraph;
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Status of the graph cache
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum CacheStatus {
    /// No cache exists
    Missing,
    /// Cache exists and is fresh
    Fresh,
    /// Cache exists but is stale (source files changed)
    Stale,
    /// Graph is currently being built
    Building,
    /// Cache exists but failed to load
    Error(String),
}

/// Metadata about the cached graph
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CacheMetadata {
    /// When the cache was created
    pub created_at: String,
    /// Number of nodes in the graph
    pub node_count: usize,
    /// Number of edges in the graph
    pub edge_count: usize,
    /// Hash of source file modification times for staleness detection
    pub source_hash: String,
}

/// Manages caching of project graphs
pub struct GraphCache {
    /// Base directory for storing cache files
    cache_dir: PathBuf,
}

impl GraphCache {
    /// Create a new graph cache manager
    pub fn new(cache_dir: &Path) -> Result<Self> {
        fs::create_dir_all(cache_dir).context("Failed to create cache directory")?;
        Ok(Self {
            cache_dir: cache_dir.to_path_buf(),
        })
    }

    /// Get the cache file path for a project
    fn cache_path(&self, project_id: &str) -> PathBuf {
        // Sanitize project_id to be a valid filename
        let safe_id = project_id
            .replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_");
        self.cache_dir.join(format!("{}.graph.json", safe_id))
    }

    /// Get the metadata file path for a project
    fn metadata_path(&self, project_id: &str) -> PathBuf {
        let safe_id = project_id
            .replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_");
        self.cache_dir.join(format!("{}.meta.json", safe_id))
    }

    /// Save a graph to the cache
    pub fn save(&self, graph: &ProjectGraph) -> Result<()> {
        let cache_path = self.cache_path(&graph.project_id);
        let metadata_path = self.metadata_path(&graph.project_id);

        // Serialize and write graph
        let graph_json = serde_json::to_string_pretty(graph)
            .context("Failed to serialize graph")?;
        fs::write(&cache_path, graph_json)
            .context("Failed to write graph cache")?;

        // Write metadata
        let metadata = CacheMetadata {
            created_at: chrono::Utc::now().to_rfc3339(),
            node_count: graph.node_count(),
            edge_count: graph.edge_count(),
            source_hash: self.compute_source_hash(&graph.project_id)?,
        };
        let metadata_json = serde_json::to_string_pretty(&metadata)
            .context("Failed to serialize metadata")?;
        fs::write(&metadata_path, metadata_json)
            .context("Failed to write metadata")?;

        tracing::info!(
            "Saved graph cache for {} ({} nodes, {} edges)",
            graph.project_id,
            graph.node_count(),
            graph.edge_count()
        );

        Ok(())
    }

    /// Load a graph from the cache
    pub fn load(&self, project_id: &str) -> Result<Option<ProjectGraph>> {
        let cache_path = self.cache_path(project_id);

        if !cache_path.exists() {
            return Ok(None);
        }

        let content = fs::read_to_string(&cache_path)
            .context("Failed to read graph cache")?;
        let graph: ProjectGraph = serde_json::from_str(&content)
            .context("Failed to deserialize graph")?;

        Ok(Some(graph))
    }

    /// Load cache metadata
    pub fn load_metadata(&self, project_id: &str) -> Result<Option<CacheMetadata>> {
        let metadata_path = self.metadata_path(project_id);

        if !metadata_path.exists() {
            return Ok(None);
        }

        let content = fs::read_to_string(&metadata_path)
            .context("Failed to read metadata")?;
        let metadata: CacheMetadata = serde_json::from_str(&content)
            .context("Failed to deserialize metadata")?;

        Ok(Some(metadata))
    }

    /// Check the status of the cache for a project
    pub fn status(&self, project_id: &str) -> CacheStatus {
        let cache_path = self.cache_path(project_id);

        if !cache_path.exists() {
            return CacheStatus::Missing;
        }

        // Load metadata to check staleness
        match self.load_metadata(project_id) {
            Ok(Some(metadata)) => {
                match self.compute_source_hash(project_id) {
                    Ok(current_hash) => {
                        if current_hash == metadata.source_hash {
                            CacheStatus::Fresh
                        } else {
                            CacheStatus::Stale
                        }
                    }
                    Err(_) => CacheStatus::Stale,
                }
            }
            Ok(None) => CacheStatus::Stale,
            Err(e) => CacheStatus::Error(e.to_string()),
        }
    }

    /// Delete the cache for a project
    pub fn delete(&self, project_id: &str) -> Result<()> {
        let cache_path = self.cache_path(project_id);
        let metadata_path = self.metadata_path(project_id);

        if cache_path.exists() {
            fs::remove_file(&cache_path)?;
        }
        if metadata_path.exists() {
            fs::remove_file(&metadata_path)?;
        }

        Ok(())
    }

    /// Compute a hash of source file modification times
    ///
    /// This is a simple staleness check - if any source file was modified
    /// after the cache was created, the cache is stale.
    fn compute_source_hash(&self, project_id: &str) -> Result<String> {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let project_path = Path::new(project_id);
        if !project_path.exists() {
            return Ok("missing".to_string());
        }

        let mut hasher = DefaultHasher::new();
        let mut file_count = 0u64;

        // Walk source files and hash their modification times
        for entry in walkdir::WalkDir::new(project_path)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();

            // Skip non-source files
            let is_source = path.extension()
                .and_then(|e| e.to_str())
                .map(|ext| matches!(ext, "rs" | "ts" | "tsx" | "js" | "jsx"))
                .unwrap_or(false);

            if !is_source {
                continue;
            }

            // Skip common ignored directories
            let skip = path.components().any(|c| {
                c.as_os_str().to_str()
                    .map(|s| matches!(s, "target" | "node_modules" | ".git" | "dist" | "build"))
                    .unwrap_or(false)
            });

            if skip {
                continue;
            }

            if let Ok(metadata) = fs::metadata(path) {
                if let Ok(modified) = metadata.modified() {
                    if let Ok(duration) = modified.duration_since(SystemTime::UNIX_EPOCH) {
                        duration.as_secs().hash(&mut hasher);
                        path.to_string_lossy().hash(&mut hasher);
                        file_count += 1;
                    }
                }
            }
        }

        file_count.hash(&mut hasher);
        let hash = hasher.finish();

        Ok(format!("{:016x}", hash))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Edge, EdgeKind, FileNode, Language};
    use tempfile::TempDir;

    fn create_test_graph() -> ProjectGraph {
        let mut graph = ProjectGraph::new("/test/project".to_string());
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
        graph
    }

    #[test]
    fn test_save_and_load() {
        let temp = TempDir::new().unwrap();
        let cache = GraphCache::new(temp.path()).unwrap();
        let graph = create_test_graph();

        cache.save(&graph).unwrap();

        let loaded = cache.load("/test/project").unwrap();
        assert!(loaded.is_some());
        let loaded = loaded.unwrap();
        assert_eq!(loaded.node_count(), 2);
        assert_eq!(loaded.edge_count(), 1);
    }

    #[test]
    fn test_missing_cache() {
        let temp = TempDir::new().unwrap();
        let cache = GraphCache::new(temp.path()).unwrap();

        let status = cache.status("/nonexistent/project");
        assert_eq!(status, CacheStatus::Missing);
    }

    #[test]
    fn test_delete_cache() {
        let temp = TempDir::new().unwrap();
        let cache = GraphCache::new(temp.path()).unwrap();
        let graph = create_test_graph();

        cache.save(&graph).unwrap();
        assert!(cache.load("/test/project").unwrap().is_some());

        cache.delete("/test/project").unwrap();
        assert!(cache.load("/test/project").unwrap().is_none());
    }
}
