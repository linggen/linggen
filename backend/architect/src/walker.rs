//! Project directory walker with ignore support

use anyhow::Result;
use ignore::WalkBuilder;
use std::path::{Path, PathBuf};

/// Entry returned by the project walker
pub struct WalkerEntry {
    path: PathBuf,
}

impl WalkerEntry {
    /// Get the path of this entry
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Walks a project directory, respecting .gitignore and common ignore patterns
pub struct ProjectWalker {
    root: PathBuf,
    /// Additional patterns to ignore
    ignore_patterns: Vec<String>,
}

impl ProjectWalker {
    /// Create a new project walker
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
            ignore_patterns: vec![
                // Common directories to ignore
                "target".to_string(),
                "node_modules".to_string(),
                ".git".to_string(),
                "dist".to_string(),
                "build".to_string(),
                "__pycache__".to_string(),
                ".venv".to_string(),
                "venv".to_string(),
            ],
        }
    }

    /// Add additional patterns to ignore
    pub fn add_ignore_pattern(&mut self, pattern: &str) {
        self.ignore_patterns.push(pattern.to_string());
    }

    /// Walk the project and return an iterator of source files
    pub fn walk(&self) -> Result<Vec<WalkerEntry>> {
        let mut entries = Vec::new();

        let walker = WalkBuilder::new(&self.root)
            .hidden(true) // Skip hidden files by default
            .git_ignore(true) // Respect .gitignore
            .git_global(true)
            .git_exclude(true)
            .build();

        for result in walker {
            match result {
                Ok(entry) => {
                    let path = entry.path();

                    // Skip directories
                    if path.is_dir() {
                        continue;
                    }

                    // Skip files in ignored directories
                    let should_ignore = self.ignore_patterns.iter().any(|pattern| {
                        path.components().any(|c| {
                            c.as_os_str()
                                .to_str()
                                .map(|s| s == pattern)
                                .unwrap_or(false)
                        })
                    });

                    if should_ignore {
                        continue;
                    }

                    // Only include source files we can analyze
                    if is_analyzable_file(path) {
                        entries.push(WalkerEntry {
                            path: path.to_path_buf(),
                        });
                    }
                }
                Err(e) => {
                    tracing::warn!("Error walking directory: {}", e);
                }
            }
        }

        Ok(entries)
    }
}

/// Check if a file is analyzable (supported source file)
fn is_analyzable_file(path: &Path) -> bool {
    match path.extension().and_then(|e| e.to_str()) {
        Some("rs") => true,  // Rust
        Some("ts") => true,  // TypeScript
        Some("tsx") => true, // TypeScript JSX
        Some("js") => true,  // JavaScript
        Some("jsx") => true, // JavaScript JSX
        Some("mjs") => true, // ES modules
        Some("cjs") => true, // CommonJS
        Some("go") => true,  // Go
        Some("py") => true,  // Python
        Some("pyi") => true, // Python stubs
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_walk_project() {
        let temp = TempDir::new().unwrap();
        let src = temp.path().join("src");
        fs::create_dir_all(&src).unwrap();

        fs::write(src.join("main.rs"), "fn main() {}").unwrap();
        fs::write(src.join("lib.rs"), "mod foo;").unwrap();
        fs::write(temp.path().join("README.md"), "# Test").unwrap();

        let walker = ProjectWalker::new(temp.path());
        let entries = walker.walk().unwrap();

        // Should find main.rs and lib.rs, but not README.md
        assert_eq!(entries.len(), 2);
        let paths: Vec<_> = entries
            .iter()
            .map(|e| e.path().file_name().unwrap().to_str().unwrap())
            .collect();
        assert!(paths.contains(&"main.rs"));
        assert!(paths.contains(&"lib.rs"));
    }

    #[test]
    fn test_ignore_target_dir() {
        let temp = TempDir::new().unwrap();
        let src = temp.path().join("src");
        let target = temp.path().join("target");
        fs::create_dir_all(&src).unwrap();
        fs::create_dir_all(&target).unwrap();

        fs::write(src.join("main.rs"), "fn main() {}").unwrap();
        fs::write(target.join("debug.rs"), "// should be ignored").unwrap();

        let walker = ProjectWalker::new(temp.path());
        let entries = walker.walk().unwrap();

        assert_eq!(entries.len(), 1);
        assert!(entries[0].path().ends_with("main.rs"));
    }
}
