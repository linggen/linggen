//! Go import extraction using Tree-sitter

use super::{ImportExtractor, ImportInfo};
use anyhow::{Context, Result};
use std::sync::Mutex;
use tree_sitter::{Parser, Query, QueryCursor};

/// Go import extractor
pub struct GoParser {
    parser: Mutex<Parser>,
    query: Query,
}

impl GoParser {
    /// Create a new Go parser
    pub fn new() -> Self {
        let mut parser = Parser::new();
        let language = tree_sitter_go::LANGUAGE;
        parser
            .set_language(&language.into())
            .expect("Failed to set Go language");

        // Tree-sitter query for Go imports
        let query_source = r#"
            ; Single import: import "fmt"
            (import_declaration
                (import_spec
                    path: (interpreted_string_literal) @import_path))
            
            ; Import with alias: import f "fmt"
            (import_declaration
                (import_spec
                    name: (package_identifier)?
                    path: (interpreted_string_literal) @import_path))
            
            ; Import block: import ( "fmt" "os" )
            (import_declaration
                (import_spec_list
                    (import_spec
                        path: (interpreted_string_literal) @import_path)))
        "#;

        let query = Query::new(&language.into(), query_source).expect("Failed to compile Go query");

        Self {
            parser: Mutex::new(parser),
            query,
        }
    }
}

impl Default for GoParser {
    fn default() -> Self {
        Self::new()
    }
}

impl ImportExtractor for GoParser {
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
        let mut seen = std::collections::HashSet::new();

        use streaming_iterator::StreamingIterator;
        while let Some((m, capture_index)) = captures.next() {
            let capture = &m.captures[*capture_index];
            let capture_name = &self.query.capture_names()[capture.index as usize];

            if *capture_name == "import_path" {
                // Get the raw text and strip quotes
                let raw_text = capture
                    .node
                    .utf8_text(source.as_bytes())
                    .unwrap_or_default();
                let text = raw_text.trim_matches('"');

                // Avoid duplicates from overlapping query matches
                if seen.insert(text.to_string()) {
                    imports.push(ImportInfo {
                        module_path: text.to_string(),
                        is_mod_decl: false,
                        is_reexport: false,
                    });
                }
            }
        }

        Ok(imports)
    }

    fn language(&self) -> &'static str {
        "go"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["go"]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_single_import() {
        let parser = GoParser::new();
        let source = r#"
            package main

            import "fmt"

            func main() {
                fmt.Println("Hello")
            }
        "#;

        let imports = parser.extract_imports(source).unwrap();
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].module_path, "fmt");
    }

    #[test]
    fn test_extract_import_block() {
        let parser = GoParser::new();
        let source = r#"
            package main

            import (
                "fmt"
                "os"
                "strings"
            )

            func main() {}
        "#;

        let imports = parser.extract_imports(source).unwrap();
        assert_eq!(imports.len(), 3);

        let paths: Vec<_> = imports.iter().map(|i| i.module_path.as_str()).collect();
        assert!(paths.contains(&"fmt"));
        assert!(paths.contains(&"os"));
        assert!(paths.contains(&"strings"));
    }

    #[test]
    fn test_extract_aliased_import() {
        let parser = GoParser::new();
        let source = r#"
            package main

            import (
                f "fmt"
                . "strings"
                _ "database/sql"
            )

            func main() {}
        "#;

        let imports = parser.extract_imports(source).unwrap();
        assert_eq!(imports.len(), 3);

        let paths: Vec<_> = imports.iter().map(|i| i.module_path.as_str()).collect();
        assert!(paths.contains(&"fmt"));
        assert!(paths.contains(&"strings"));
        assert!(paths.contains(&"database/sql"));
    }

    #[test]
    fn test_extract_local_import() {
        let parser = GoParser::new();
        let source = r#"
            package main

            import (
                "github.com/user/project/pkg/utils"
                "./internal/handler"
            )

            func main() {}
        "#;

        let imports = parser.extract_imports(source).unwrap();
        assert_eq!(imports.len(), 2);

        let paths: Vec<_> = imports.iter().map(|i| i.module_path.as_str()).collect();
        assert!(paths.contains(&"github.com/user/project/pkg/utils"));
        assert!(paths.contains(&"./internal/handler"));
    }
}
