//! Java import extraction using Tree-sitter

use super::{ImportExtractor, ImportInfo};
use anyhow::{Context, Result};
use std::sync::Mutex;
use tree_sitter::{Parser, Query, QueryCursor};

/// Java import extractor
pub struct JavaParser {
    parser: Mutex<Parser>,
    query: Query,
}

impl JavaParser {
    pub fn new() -> Self {
        let mut parser = Parser::new();
        let language = tree_sitter_java::LANGUAGE;
        parser
            .set_language(&language.into())
            .expect("Failed to set Java language");

        // Capture:
        // - import com.foo.Bar;
        // - import static com.foo.Bar.baz;
        // - import com.foo.*;
        //
        // We capture the full scoped_identifier and later normalize it.
        let query_source = r#"
            (import_declaration
              (scoped_identifier) @import_path)
        "#;

        let query =
            Query::new(&language.into(), query_source).expect("Failed to compile Java query");

        Self {
            parser: Mutex::new(parser),
            query,
        }
    }

    fn extract(&self, source: &str) -> Result<Vec<ImportInfo>> {
        let mut parser = self
            .parser
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let tree = parser.parse(source, None).context("Failed to parse source")?;

        let root_node = tree.root_node();
        let mut cursor = QueryCursor::new();
        let mut captures = cursor.captures(&self.query, root_node, source.as_bytes());

        let mut imports = Vec::new();
        use streaming_iterator::StreamingIterator;
        while let Some((m, capture_index)) = captures.next() {
            let capture = &m.captures[*capture_index];
            let raw = capture
                .node
                .utf8_text(source.as_bytes())
                .unwrap_or_default()
                .trim()
                .to_string();
            if raw.is_empty() {
                continue;
            }

            imports.push(ImportInfo {
                module_path: raw,
                is_mod_decl: false,
                is_reexport: false,
            });
        }

        Ok(imports)
    }
}

impl Default for JavaParser {
    fn default() -> Self {
        Self::new()
    }
}

impl ImportExtractor for JavaParser {
    fn extract_imports(&self, source: &str) -> Result<Vec<ImportInfo>> {
        self.extract(source)
    }

    fn language(&self) -> &'static str {
        "java"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["java"]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_imports() {
        let parser = JavaParser::new();
        let source = r#"
            package com.example.app;
            import java.util.List;
            import com.foo.Bar;
            import static com.foo.Util.baz;
            import com.foo.*;

            public class Main {}
        "#;

        let imports = parser.extract_imports(source).unwrap();
        let paths: Vec<_> = imports.iter().map(|i| i.module_path.as_str()).collect();

        assert!(paths.contains(&"java.util.List"));
        assert!(paths.contains(&"com.foo.Bar"));
        assert!(paths.contains(&"com.foo.Util.baz"));
        assert!(paths.contains(&"com.foo"));
    }
}

