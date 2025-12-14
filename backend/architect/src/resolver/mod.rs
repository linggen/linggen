//! Language-specific import resolution
//!
//! This module provides resolvers for different programming languages to resolve
//! import paths to actual file paths in the project.

mod go;
mod java;
mod python;
mod rust;
mod typescript;

pub use go::GoResolver;
pub use java::JavaResolver;
pub use python::PythonResolver;
pub use rust::RustResolver;
pub use typescript::TypeScriptResolver;

use crate::parser::ImportInfo;
use crate::EdgeKind;
use std::path::Path;

/// The result of resolving an import to a local file edge.
#[derive(Debug, Clone)]
pub struct ResolvedImport {
    /// Target file path relative to project root
    pub target: String,
    /// Relationship kind to use for the graph edge
    pub kind: EdgeKind,
}

/// Common trait for language-specific import resolution
pub trait ImportResolver: Send + Sync {
    /// Resolve an import to a file path relative to project root
    ///
    /// Returns `Some(ResolvedImport)` if the import can be resolved to a local file,
    /// or `None` if it's an external dependency or cannot be resolved.
    fn resolve(
        &self,
        project_root: &Path,
        current_file: &Path,
        import: &ImportInfo,
    ) -> Option<ResolvedImport>;

    /// Get the language name for this resolver
    fn language(&self) -> &'static str;

    /// Get file extensions this resolver handles
    fn extensions(&self) -> &'static [&'static str];
}

/// Multi-language import resolver that delegates to language-specific resolvers
pub struct MultiLanguageResolver {
    rust: RustResolver,
    typescript: TypeScriptResolver,
    go: GoResolver,
    java: JavaResolver,
    python: PythonResolver,
}

impl MultiLanguageResolver {
    /// Create a new multi-language resolver
    pub fn new() -> Self {
        Self {
            rust: RustResolver::new(),
            typescript: TypeScriptResolver::new(),
            go: GoResolver::new(),
            java: JavaResolver::new(),
            python: PythonResolver::new(),
        }
    }

    /// Get the appropriate resolver for a file extension
    pub fn resolver_for_extension(&self, ext: &str) -> Option<&dyn ImportResolver> {
        let ext_lower = ext.to_lowercase();
        let ext_str = ext_lower.as_str();

        if self.rust.extensions().contains(&ext_str) {
            Some(&self.rust)
        } else if self.typescript.extensions().contains(&ext_str) {
            Some(&self.typescript)
        } else if self.go.extensions().contains(&ext_str) {
            Some(&self.go)
        } else if self.java.extensions().contains(&ext_str) {
            Some(&self.java)
        } else if self.python.extensions().contains(&ext_str) {
            Some(&self.python)
        } else {
            None
        }
    }

    /// Resolve an import, auto-detecting language from extension
    pub fn resolve(
        &self,
        project_root: &Path,
        current_file: &Path,
        import: &ImportInfo,
        extension: &str,
    ) -> Option<ResolvedImport> {
        self.resolver_for_extension(extension)
            .and_then(|resolver| resolver.resolve(project_root, current_file, import))
    }

    /// Get all supported extensions
    pub fn supported_extensions(&self) -> Vec<&'static str> {
        let mut exts = Vec::new();
        exts.extend_from_slice(self.rust.extensions());
        exts.extend_from_slice(self.typescript.extensions());
        exts.extend_from_slice(self.go.extensions());
        exts.extend_from_slice(self.java.extensions());
        exts.extend_from_slice(self.python.extensions());
        exts
    }
}

impl Default for MultiLanguageResolver {
    fn default() -> Self {
        Self::new()
    }
}
