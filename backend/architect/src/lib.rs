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

    // Walk all source files once, then build the graph in 2 passes:
    // - Pass 1: add all nodes
    // - Pass 2: resolve imports and add edges
    //
    // This avoids dropping edges when the imported file hasn't been seen yet.
    let entries = walker.walk()?;

    struct FileEntry {
        abs_path: std::path::PathBuf,
        rel_path: String,
        extension: String,
        language: Language,
        folder: String,
        label: String,
    }

    let mut files: Vec<FileEntry> = Vec::with_capacity(entries.len());

    for entry in entries {
        let path = entry.path().to_path_buf();
        let extension = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default()
            .to_string();

        // Detect language from extension
        let language = match extension.as_str() {
            "rs" => Language::Rust,
            "ts" | "tsx" => Language::TypeScript,
            "js" | "jsx" | "mjs" | "cjs" => Language::JavaScript,
            "go" => Language::Go,
            "py" | "pyi" => Language::Python,
            _ => continue, // Skip unsupported files
        };

        let rel_path = path
            .strip_prefix(project_root)
            .unwrap_or(&path)
            .to_string_lossy()
            .to_string();

        let folder = path
            .parent()
            .and_then(|p| p.strip_prefix(project_root).ok())
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();

        let label = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();

        files.push(FileEntry {
            abs_path: path,
            rel_path,
            extension,
            language,
            folder,
            label,
        });
    }

    // Pass 1: add all nodes
    for f in &files {
        graph.add_node(FileNode {
            id: f.rel_path.clone(),
            label: f.label.clone(),
            language: f.language.clone(),
            folder: f.folder.clone(),
        });
    }

    // Pass 2: parse, resolve and add edges
    for f in &files {
        // Skip if no parser for this extension
        if extractor.parser_for_extension(&f.extension).is_none() {
            continue;
        }

        let content = std::fs::read_to_string(&f.abs_path)?;
        let imports = extractor.extract_imports(&content, &f.extension)?;

        tracing::debug!("File {}: extracted {} imports", f.rel_path, imports.len());

        for import in imports {
            tracing::debug!(
                "Resolving import '{}' from {}",
                import.module_path,
                f.rel_path
            );
            if let Some(target) = resolver.resolve(project_root, &f.abs_path, &import, &f.extension)
            {
                tracing::debug!("  ✓ Resolved to: {}", target);
                graph.add_edge(Edge {
                    source: f.rel_path.clone(),
                    target,
                    kind: if import.is_reexport {
                        EdgeKind::ReExport
                    } else {
                        EdgeKind::Import
                    },
                });
            } else {
                tracing::debug!("  ✗ Could not resolve (external or not found)");
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn typescript_edges_are_not_dropped_due_to_walk_order() {
        let temp = TempDir::new().unwrap();
        let src = temp.path().join("src");
        fs::create_dir_all(&src).unwrap();

        // Ensure the importer file name sorts before the imported file name in most directory walks.
        fs::write(
            src.join("a.ts"),
            "import { b } from './b';\nexport const a = b;\n",
        )
        .unwrap();
        fs::write(src.join("b.ts"), "export const b = 1;\n").unwrap();

        let graph = build_project_graph(temp.path()).unwrap();

        assert!(graph.get_node("src/a.ts").is_some());
        assert!(graph.get_node("src/b.ts").is_some());

        // We should have an edge from a.ts -> b.ts.
        assert!(
            graph
                .edges
                .iter()
                .any(|e| e.source == "src/a.ts" && e.target == "src/b.ts"),
            "expected an import edge from src/a.ts to src/b.ts"
        );
    }
}
