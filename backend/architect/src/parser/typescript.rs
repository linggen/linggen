//! TypeScript/JavaScript import extraction using Tree-sitter

use super::{ImportExtractor, ImportInfo};
use anyhow::{Context, Result};
use std::sync::Mutex;
use tree_sitter::{Parser, Query, QueryCursor};

/// TypeScript/JavaScript import extractor
pub struct TypeScriptParser {
    ts_parser: Mutex<Parser>,
    tsx_parser: Mutex<Parser>,
    ts_query: Query,
    tsx_query: Query,
}

impl TypeScriptParser {
    /// Create a new TypeScript parser
    pub fn new() -> Self {
        // TypeScript parser
        let mut ts_parser = Parser::new();
        let ts_language = tree_sitter_typescript::LANGUAGE_TYPESCRIPT;
        ts_parser
            .set_language(&ts_language.into())
            .expect("Failed to set TypeScript language");

        // TSX parser (for React files)
        let mut tsx_parser = Parser::new();
        let tsx_language = tree_sitter_typescript::LANGUAGE_TSX;
        tsx_parser
            .set_language(&tsx_language.into())
            .expect("Failed to set TSX language");

        // Tree-sitter query for TypeScript/JavaScript imports
        let query_source = r#"
            ; ES6 imports: import foo from './bar'
            (import_statement
                source: (string) @import_path)
            
            ; Dynamic imports: import('./foo')
            (call_expression
                function: (import)
                arguments: (arguments (string) @import_path))
            
            ; Require calls: require('./foo')
            (call_expression
                function: (identifier) @_func (#eq? @_func "require")
                arguments: (arguments (string) @import_path))
            
            ; Export from: export { foo } from './bar'
            (export_statement
                source: (string) @reexport_path)
        "#;

        let ts_query =
            Query::new(&ts_language.into(), query_source).expect("Failed to compile TS query");
        let tsx_query =
            Query::new(&tsx_language.into(), query_source).expect("Failed to compile TSX query");

        Self {
            ts_parser: Mutex::new(ts_parser),
            tsx_parser: Mutex::new(tsx_parser),
            ts_query,
            tsx_query,
        }
    }

    fn extract_with_parser(
        &self,
        parser: &Mutex<Parser>,
        query: &Query,
        source: &str,
    ) -> Result<Vec<ImportInfo>> {
        let mut parser = parser
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let tree = parser
            .parse(source, None)
            .context("Failed to parse source")?;

        let root_node = tree.root_node();
        let mut cursor = QueryCursor::new();
        let mut captures = cursor.captures(query, root_node, source.as_bytes());

        let mut imports = Vec::new();

        use streaming_iterator::StreamingIterator;
        while let Some((m, capture_index)) = captures.next() {
            let capture = &m.captures[*capture_index];
            let capture_name = &query.capture_names()[capture.index as usize];

            // Get the raw text and strip quotes
            let raw_text = capture
                .node
                .utf8_text(source.as_bytes())
                .unwrap_or_default();
            let text = raw_text.trim_matches(|c| c == '"' || c == '\'' || c == '`');

            match capture_name.as_ref() {
                "import_path" => {
                    imports.push(ImportInfo {
                        module_path: text.to_string(),
                        is_mod_decl: false,
                        is_reexport: false,
                    });
                }
                "reexport_path" => {
                    imports.push(ImportInfo {
                        module_path: text.to_string(),
                        is_mod_decl: false,
                        is_reexport: true,
                    });
                }
                _ => {}
            }
        }

        Ok(imports)
    }

    /// Extract imports, choosing parser based on whether content looks like TSX
    pub fn extract_imports_for_extension(
        &self,
        source: &str,
        extension: &str,
    ) -> Result<Vec<ImportInfo>> {
        let ext_lower = extension.to_lowercase();
        if ext_lower == "tsx" || ext_lower == "jsx" {
            self.extract_with_parser(&self.tsx_parser, &self.tsx_query, source)
        } else {
            self.extract_with_parser(&self.ts_parser, &self.ts_query, source)
        }
    }
}

impl Default for TypeScriptParser {
    fn default() -> Self {
        Self::new()
    }
}

impl ImportExtractor for TypeScriptParser {
    fn extract_imports(&self, source: &str) -> Result<Vec<ImportInfo>> {
        // Default to TS parser; use extract_imports_for_extension for TSX
        self.extract_with_parser(&self.ts_parser, &self.ts_query, source)
    }

    fn language(&self) -> &'static str {
        "typescript"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["ts", "tsx", "js", "jsx", "mjs", "cjs"]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_es6_import() {
        let parser = TypeScriptParser::new();
        let source = r#"
            import React from 'react';
            import { useState } from 'react';
            import * as utils from './utils';
            import './styles.css';
        "#;

        let imports = parser.extract_imports(source).unwrap();
        assert!(imports.len() >= 3);

        let paths: Vec<_> = imports.iter().map(|i| i.module_path.as_str()).collect();
        assert!(paths.contains(&"react"));
        assert!(paths.contains(&"./utils"));
        assert!(paths.contains(&"./styles.css"));
    }

    #[test]
    fn test_extract_require() {
        let parser = TypeScriptParser::new();
        let source = r#"
            const fs = require('fs');
            const path = require('path');
        "#;

        let imports = parser.extract_imports(source).unwrap();
        assert!(imports.len() >= 2);

        let paths: Vec<_> = imports.iter().map(|i| i.module_path.as_str()).collect();
        assert!(paths.contains(&"fs"));
        assert!(paths.contains(&"path"));
    }

    #[test]
    fn test_extract_export_from() {
        let parser = TypeScriptParser::new();
        let source = r#"
            export { foo, bar } from './module';
            export * from './other';
        "#;

        let imports = parser.extract_imports(source).unwrap();
        let reexports: Vec<_> = imports.iter().filter(|i| i.is_reexport).collect();
        assert!(reexports.len() >= 2);
    }

    #[test]
    fn test_extract_tsx() {
        let parser = TypeScriptParser::new();
        let source = r#"
            import React from 'react';
            import { Button } from './components/Button';

            export const App = () => <Button>Click me</Button>;
        "#;

        let imports = parser.extract_imports_for_extension(source, "tsx").unwrap();
        assert!(imports.len() >= 2);

        let paths: Vec<_> = imports.iter().map(|i| i.module_path.as_str()).collect();
        assert!(paths.contains(&"react"));
        assert!(paths.contains(&"./components/Button"));
    }
}
