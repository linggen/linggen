//! Python import extraction using Tree-sitter

use super::{ImportExtractor, ImportInfo};
use anyhow::{Context, Result};
use std::sync::Mutex;
use tree_sitter::{Parser, Query, QueryCursor};

/// Python import extractor
pub struct PythonParser {
    parser: Mutex<Parser>,
    query: Query,
}

impl PythonParser {
    /// Create a new Python parser
    pub fn new() -> Self {
        let mut parser = Parser::new();
        let language = tree_sitter_python::LANGUAGE;
        parser
            .set_language(&language.into())
            .expect("Failed to set Python language");

        // Tree-sitter query for Python imports
        let query_source = r#"
            ; import foo
            (import_statement
                name: (dotted_name) @import_module)
            
            ; import foo as bar
            (import_statement
                name: (aliased_import
                    name: (dotted_name) @import_module))
            
            ; from foo import bar
            (import_from_statement
                module_name: (dotted_name) @from_module)
            
            ; from foo import bar (with relative import)
            (import_from_statement
                module_name: (relative_import
                    (dotted_name) @from_module))
            
            ; from . import bar (relative import without module name)
            (import_from_statement
                module_name: (relative_import) @relative_import)
        "#;

        let query =
            Query::new(&language.into(), query_source).expect("Failed to compile Python query");

        Self {
            parser: Mutex::new(parser),
            query,
        }
    }
}

impl Default for PythonParser {
    fn default() -> Self {
        Self::new()
    }
}

impl ImportExtractor for PythonParser {
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
            let text = capture
                .node
                .utf8_text(source.as_bytes())
                .unwrap_or_default()
                .to_string();

            // Avoid duplicates
            if seen.contains(&text) {
                continue;
            }
            seen.insert(text.clone());

            match capture_name.as_ref() {
                "import_module" | "from_module" => {
                    imports.push(ImportInfo {
                        module_path: text,
                        is_mod_decl: false,
                        is_reexport: false,
                    });
                }
                "relative_import" => {
                    // Handle relative imports like "." or ".."
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
        "python"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["py", "pyi"]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_simple_import() {
        let parser = PythonParser::new();
        let source = r#"
import os
import sys
import json
"#;

        let imports = parser.extract_imports(source).unwrap();
        assert_eq!(imports.len(), 3);

        let paths: Vec<_> = imports.iter().map(|i| i.module_path.as_str()).collect();
        assert!(paths.contains(&"os"));
        assert!(paths.contains(&"sys"));
        assert!(paths.contains(&"json"));
    }

    #[test]
    fn test_extract_from_import() {
        let parser = PythonParser::new();
        let source = r#"
from os import path
from collections import defaultdict, Counter
from typing import List, Dict, Optional
"#;

        let imports = parser.extract_imports(source).unwrap();
        assert!(imports.len() >= 3);

        let paths: Vec<_> = imports.iter().map(|i| i.module_path.as_str()).collect();
        assert!(paths.contains(&"os"));
        assert!(paths.contains(&"collections"));
        assert!(paths.contains(&"typing"));
    }

    #[test]
    fn test_extract_dotted_import() {
        let parser = PythonParser::new();
        let source = r#"
import os.path
import xml.etree.ElementTree
from urllib.parse import urljoin
"#;

        let imports = parser.extract_imports(source).unwrap();
        assert!(imports.len() >= 3);

        let paths: Vec<_> = imports.iter().map(|i| i.module_path.as_str()).collect();
        assert!(paths.contains(&"os.path"));
        assert!(paths.contains(&"xml.etree.ElementTree"));
        assert!(paths.contains(&"urllib.parse"));
    }

    #[test]
    fn test_extract_relative_import() {
        let parser = PythonParser::new();
        let source = r#"
from . import utils
from .. import config
from .models import User
from ..helpers import format_date
"#;

        let imports = parser.extract_imports(source).unwrap();
        // Should capture relative imports
        assert!(!imports.is_empty());
    }

    #[test]
    fn test_extract_aliased_import() {
        let parser = PythonParser::new();
        let source = r#"
import numpy as np
import pandas as pd
from datetime import datetime as dt
"#;

        let imports = parser.extract_imports(source).unwrap();
        assert!(imports.len() >= 3);

        let paths: Vec<_> = imports.iter().map(|i| i.module_path.as_str()).collect();
        assert!(paths.contains(&"numpy"));
        assert!(paths.contains(&"pandas"));
        assert!(paths.contains(&"datetime"));
    }
}
