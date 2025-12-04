//! Architect: File dependency graph analysis for Linggen
//!
//! This crate provides functionality to build and analyze file-level dependency graphs
//! using Tree-sitter for parsing source code and extracting import relationships.

pub mod cache;
pub mod graph;
pub mod overrides;
pub mod parser;
pub mod resolver;
pub mod walker;

pub use cache::{CacheMetadata, CacheStatus, GraphCache};
pub use graph::{Edge, EdgeKind, FileNode, ProjectGraph};
pub use overrides::{
    GraphOverrides, HiddenEdge, JsonOverridesStore, ManualEdge, NodeTag, OverridesStore,
};
pub use parser::{ImportExtractor, ImportInfo, MultiLanguageExtractor};
pub use resolver::{ImportResolver, MultiLanguageResolver};
pub use walker::ProjectWalker;

use anyhow::Result;
use std::path::Path;

/// Build a project graph from a project root directory.
///
/// This is the main entry point for building a file dependency graph.
/// It walks the project directory, parses source files with Tree-sitter,
/// extracts imports, and resolves them to file paths.
pub fn build_project_graph(project_root: &Path) -> Result<ProjectGraph> {
    let walker = ProjectWalker::new(project_root);
    let mut graph = ProjectGraph::new(project_root.to_string_lossy().to_string());
    let extractor = MultiLanguageExtractor::new();
    let resolver = MultiLanguageResolver::new();

    // Walk all source files
    for entry in walker.walk()? {
        let path = entry.path();
        let extension = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default();

        let relative_path = path
            .strip_prefix(project_root)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();

        // Detect language from extension
        let language = match extension {
            "rs" => Language::Rust,
            "ts" | "tsx" => Language::TypeScript,
            "js" | "jsx" | "mjs" | "cjs" => Language::JavaScript,
            "go" => Language::Go,
            "py" | "pyi" => Language::Python,
            _ => continue, // Skip unsupported files
        };

        // Add node for this file
        let folder = path
            .parent()
            .and_then(|p| p.strip_prefix(project_root).ok())
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();

        graph.add_node(FileNode {
            id: relative_path.clone(),
            label: path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default(),
            language: language.clone(),
            folder,
        });

        // Skip if no parser for this extension
        if extractor.parser_for_extension(extension).is_none() {
            continue;
        }

        // Parse and extract imports
        let content = std::fs::read_to_string(path)?;
        let imports = extractor.extract_imports(&content, extension)?;

        // Resolve imports to file paths and add edges
        for import in imports {
            if let Some(target) = resolver.resolve(project_root, path, &import, extension) {
                graph.add_edge(Edge {
                    source: relative_path.clone(),
                    target,
                    kind: if import.is_reexport {
                        EdgeKind::ReExport
                    } else {
                        EdgeKind::Import
                    },
                });
            }
        }
    }

    Ok(graph)
}

/// Supported programming languages for analysis
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Language {
    Rust,
    TypeScript,
    JavaScript,
    Go,
    Python,
}

impl std::fmt::Display for Language {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Language::Rust => write!(f, "rust"),
            Language::TypeScript => write!(f, "typescript"),
            Language::JavaScript => write!(f, "javascript"),
            Language::Go => write!(f, "go"),
            Language::Python => write!(f, "python"),
        }
    }
}
