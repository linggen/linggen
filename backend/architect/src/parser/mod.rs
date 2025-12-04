//! Language-specific import extraction using Tree-sitter
//!
//! This module provides parsers for different programming languages to extract
//! import/dependency information from source files.

mod go;
mod python;
mod rust;
mod typescript;

pub use go::GoParser;
pub use python::PythonParser;
pub use rust::RustParser;
pub use typescript::TypeScriptParser;

use anyhow::Result;

/// Represents an extracted import from source code
#[derive(Debug, Clone)]
pub struct ImportInfo {
    /// The module path being imported (e.g., "crate::foo::bar", "./utils", "fmt")
    pub module_path: String,
    /// Whether this is a module declaration (Rust-specific: mod foo;)
    pub is_mod_decl: bool,
    /// Whether this is a re-export (pub use, export from)
    pub is_reexport: bool,
}

/// Common trait for language-specific import extraction
pub trait ImportExtractor: Send + Sync {
    /// Extract imports from source code
    fn extract_imports(&self, source: &str) -> Result<Vec<ImportInfo>>;

    /// Get the language name for this extractor
    fn language(&self) -> &'static str;

    /// Get file extensions this parser handles
    fn extensions(&self) -> &'static [&'static str];
}

/// Multi-language import extractor that delegates to language-specific parsers
pub struct MultiLanguageExtractor {
    rust: RustParser,
    typescript: TypeScriptParser,
    go: GoParser,
    python: PythonParser,
}

impl MultiLanguageExtractor {
    /// Create a new multi-language extractor
    pub fn new() -> Self {
        Self {
            rust: RustParser::new(),
            typescript: TypeScriptParser::new(),
            go: GoParser::new(),
            python: PythonParser::new(),
        }
    }

    /// Get the appropriate parser for a file extension
    pub fn parser_for_extension(&self, ext: &str) -> Option<&dyn ImportExtractor> {
        let ext_lower = ext.to_lowercase();
        let ext_str = ext_lower.as_str();

        if self.rust.extensions().contains(&ext_str) {
            Some(&self.rust)
        } else if self.typescript.extensions().contains(&ext_str) {
            Some(&self.typescript)
        } else if self.go.extensions().contains(&ext_str) {
            Some(&self.go)
        } else if self.python.extensions().contains(&ext_str) {
            Some(&self.python)
        } else {
            None
        }
    }

    /// Extract imports from a file, auto-detecting language from extension
    pub fn extract_imports(&self, source: &str, extension: &str) -> Result<Vec<ImportInfo>> {
        match self.parser_for_extension(extension) {
            Some(parser) => parser.extract_imports(source),
            None => Ok(Vec::new()), // Unknown language, no imports
        }
    }

    /// Get all supported extensions
    pub fn supported_extensions(&self) -> Vec<&'static str> {
        let mut exts = Vec::new();
        exts.extend_from_slice(self.rust.extensions());
        exts.extend_from_slice(self.typescript.extensions());
        exts.extend_from_slice(self.go.extensions());
        exts.extend_from_slice(self.python.extensions());
        exts
    }
}

impl Default for MultiLanguageExtractor {
    fn default() -> Self {
        Self::new()
    }
}
