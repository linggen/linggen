//! Rust import extraction using Tree-sitter

use super::{ImportExtractor, ImportInfo};
use anyhow::{Context, Result};
use std::sync::Mutex;
use tree_sitter::{Parser, Query, QueryCursor};

/// Rust import extractor
pub struct RustParser {
    parser: Mutex<Parser>,
    query: Query,
}

impl RustParser {
    /// Create a new Rust parser
    pub fn new() -> Self {
        let mut parser = Parser::new();
        let language = tree_sitter_rust::LANGUAGE;
        parser
            .set_language(&language.into())
            .expect("Failed to set Rust language");

        // Tree-sitter query for Rust imports:
        // - use declarations: use foo::bar;
        // - mod declarations: mod foo;
        // - extern crate: extern crate foo;
        let query_source = r#"
            ; use declarations
            (use_declaration
                argument: (scoped_identifier) @use_path)
            (use_declaration
                argument: (identifier) @use_path)
            (use_declaration
                argument: (use_as_clause
                    path: (scoped_identifier) @use_path))
            (use_declaration
                argument: (use_as_clause
                    path: (identifier) @use_path))
            (use_declaration
                argument: (scoped_use_list
                    path: (scoped_identifier) @use_path))
            (use_declaration
                argument: (scoped_use_list
                    path: (identifier) @use_path))
            (use_declaration
                argument: (use_wildcard
                    (scoped_identifier) @use_path))
            
            ; mod declarations
            (mod_item
                name: (identifier) @mod_name)
            
            ; extern crate
            (extern_crate_declaration
                name: (identifier) @extern_crate)
        "#;

        let query =
            Query::new(&language.into(), query_source).expect("Failed to compile Rust query");

        Self {
            parser: Mutex::new(parser),
            query,
        }
    }
}

impl Default for RustParser {
    fn default() -> Self {
        Self::new()
    }
}

impl ImportExtractor for RustParser {
    fn extract_imports(&self, source: &str) -> Result<Vec<ImportInfo>> {
        let mut parser = self
            .parser
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let tree = parser
            .parse(source, None)
            .context("Failed to parse source")?;

        let root_node = tree.root_node();
        let mut cursor = QueryCursor::new();
        let mut captures = cursor.captures(&self.query, root_node, source.as_bytes());

        let mut imports = Vec::new();

        // Use streaming iterator pattern for tree-sitter 0.24+
        use streaming_iterator::StreamingIterator;
        while let Some((m, capture_index)) = captures.next() {
            let capture = &m.captures[*capture_index];
            let capture_name = &self.query.capture_names()[capture.index as usize];
            let text = capture
                .node
                .utf8_text(source.as_bytes())
                .unwrap_or_default()
                .to_string();

            match capture_name.as_ref() {
                "use_path" => {
                    // Check if this is a pub use (re-export)
                    let is_reexport = capture
                        .node
                        .parent()
                        .and_then(|p| p.parent())
                        .map(|p| {
                            source[p.start_byte()..p.end_byte().min(p.start_byte() + 10)]
                                .starts_with("pub")
                        })
                        .unwrap_or(false);

                    imports.push(ImportInfo {
                        module_path: text,
                        is_mod_decl: false,
                        is_reexport,
                    });
                }
                "mod_name" => {
                    // Check if this is a mod declaration (mod foo;) vs inline module (mod foo { ... })
                    let parent = capture.node.parent();
                    if let Some(mod_item) = parent {
                        let mod_text = mod_item.utf8_text(source.as_bytes()).unwrap_or_default();
                        // Inline modules have braces, declarations end with semicolon
                        let is_declaration = mod_text.trim_end().ends_with(';');

                        if is_declaration {
                            imports.push(ImportInfo {
                                module_path: text,
                                is_mod_decl: true,
                                is_reexport: false,
                            });
                        }
                    }
                }
                "extern_crate" => {
                    imports.push(ImportInfo {
                        module_path: text,
                        is_mod_decl: false,
                        is_reexport: false,
                    });
                }
                _ => {}
            }
        }

        Ok(imports)
    }

    fn language(&self) -> &'static str {
        "rust"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["rs"]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_simple_use() {
        let parser = RustParser::new();
        let source = r#"
            use std::collections::HashMap;
            use crate::foo;
        "#;

        let imports = parser.extract_imports(source).unwrap();
        assert!(imports.len() >= 2);

        let paths: Vec<_> = imports.iter().map(|i| i.module_path.as_str()).collect();
        assert!(paths.contains(&"std::collections::HashMap"));
        assert!(paths.contains(&"crate::foo"));
    }

    #[test]
    fn test_extract_mod_declaration() {
        let parser = RustParser::new();
        let source = r#"
            mod foo;
            mod bar;
        "#;

        let imports = parser.extract_imports(source).unwrap();
        assert_eq!(imports.len(), 2);
        assert!(imports.iter().all(|i| i.is_mod_decl));
    }

    #[test]
    fn test_ignore_inline_module() {
        let parser = RustParser::new();
        let source = r#"
            mod tests {
                fn test_something() {}
            }
        "#;

        let imports = parser.extract_imports(source).unwrap();
        // Inline modules should not be treated as imports
        assert!(imports
            .iter()
            .all(|i| !i.is_mod_decl || i.module_path != "tests"));
    }

    #[test]
    fn test_extract_pub_use() {
        let parser = RustParser::new();
        let source = r#"
            pub use crate::foo::Bar;
        "#;

        let imports = parser.extract_imports(source).unwrap();
        assert!(imports.iter().any(|i| i.is_reexport));
    }
}
