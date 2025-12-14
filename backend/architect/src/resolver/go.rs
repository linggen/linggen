//! Go import resolution

use super::{ImportResolver, ResolvedImport};
use crate::parser::ImportInfo;
use crate::EdgeKind;
use std::path::Path;

/// Go import resolver
pub struct GoResolver;

impl GoResolver {
    pub fn new() -> Self {
        Self
    }

    /// Try to resolve a relative Go import to a file
    fn resolve_relative_import(
        &self,
        project_root: &Path,
        current_dir: &Path,
        import_path: &str,
    ) -> Option<String> {
        let target_dir = current_dir.join(import_path);

        // Go imports reference packages (directories), find .go files
        if target_dir.is_dir() {
            // Return the first non-test .go file in the directory
            for entry in std::fs::read_dir(&target_dir).ok()? {
                let entry = entry.ok()?;
                let path = entry.path();

                if let Some(ext) = path.extension() {
                    if ext == "go" {
                        // Skip test files
                        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                            if name.ends_with("_test.go") {
                                continue;
                            }
                        }

                        return path
                            .strip_prefix(project_root)
                            .ok()
                            .map(|p| p.to_string_lossy().to_string());
                    }
                }
            }
        }

        None
    }

    /// Try to resolve a module path (e.g., github.com/user/project/pkg/utils)
    fn resolve_module_path(&self, project_root: &Path, import_path: &str) -> Option<String> {
        // Check if this looks like a local module path
        // Try to find go.mod to determine module name
        let go_mod = project_root.join("go.mod");
        if !go_mod.exists() {
            return None;
        }

        // Read go.mod to get module name
        let content = std::fs::read_to_string(&go_mod).ok()?;
        let module_name = content
            .lines()
            .find(|line| line.starts_with("module "))?
            .strip_prefix("module ")?
            .trim();

        // Check if import is within our module
        if !import_path.starts_with(module_name) {
            return None;
        }

        // Convert module path to relative path
        let relative_path = import_path
            .strip_prefix(module_name)?
            .trim_start_matches('/');
        let target_dir = project_root.join(relative_path);

        if target_dir.is_dir() {
            // Find first .go file
            for entry in std::fs::read_dir(&target_dir).ok()? {
                let entry = entry.ok()?;
                let path = entry.path();

                if let Some(ext) = path.extension() {
                    if ext == "go" {
                        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                            if name.ends_with("_test.go") {
                                continue;
                            }
                        }

                        return path
                            .strip_prefix(project_root)
                            .ok()
                            .map(|p| p.to_string_lossy().to_string());
                    }
                }
            }
        }

        None
    }
}

impl Default for GoResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl ImportResolver for GoResolver {
    fn resolve(
        &self,
        project_root: &Path,
        current_file: &Path,
        import: &ImportInfo,
    ) -> Option<ResolvedImport> {
        let import_path = &import.module_path;

        // Handle relative imports (./pkg or ../pkg)
        if import_path.starts_with("./") || import_path.starts_with("../") {
            let current_dir = current_file.parent()?;
            return self
                .resolve_relative_import(project_root, current_dir, import_path)
                .map(|target| ResolvedImport {
                    target,
                    kind: EdgeKind::Import,
                });
        }

        // Try to resolve as module path (e.g., github.com/user/project/pkg)
        self.resolve_module_path(project_root, import_path)
            .map(|target| ResolvedImport {
                target,
                kind: EdgeKind::Import,
            })
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
    use std::fs;
    use tempfile::TempDir;

    fn setup_test_project() -> TempDir {
        let temp = TempDir::new().unwrap();
        let pkg = temp.path().join("pkg");
        let internal = temp.path().join("internal");

        fs::create_dir_all(&pkg).unwrap();
        fs::create_dir_all(&internal).unwrap();

        fs::write(
            temp.path().join("go.mod"),
            "module github.com/test/project\n\ngo 1.21",
        )
        .unwrap();
        fs::write(temp.path().join("main.go"), "package main").unwrap();
        fs::write(pkg.join("utils.go"), "package pkg").unwrap();
        fs::write(internal.join("handler.go"), "package internal").unwrap();

        temp
    }

    #[test]
    fn test_resolve_relative_import() {
        let temp = setup_test_project();
        let project_root = temp.path();
        let current_file = project_root.join("main.go");
        let resolver = GoResolver::new();

        let import = ImportInfo {
            module_path: "./pkg".to_string(),
            is_mod_decl: false,
            is_reexport: false,
        };

        let resolved = resolver
            .resolve(project_root, &current_file, &import)
            .unwrap();
        assert!(resolved.target.contains("utils.go"));
    }

    #[test]
    fn test_resolve_module_path() {
        let temp = setup_test_project();
        let project_root = temp.path();
        let current_file = project_root.join("main.go");
        let resolver = GoResolver::new();

        let import = ImportInfo {
            module_path: "github.com/test/project/pkg".to_string(),
            is_mod_decl: false,
            is_reexport: false,
        };

        let resolved = resolver.resolve(project_root, &current_file, &import);
        assert!(resolved.is_some());
    }

    #[test]
    fn test_skip_external_packages() {
        let temp = setup_test_project();
        let project_root = temp.path();
        let current_file = project_root.join("main.go");
        let resolver = GoResolver::new();

        let import = ImportInfo {
            module_path: "fmt".to_string(),
            is_mod_decl: false,
            is_reexport: false,
        };

        let resolved = resolver.resolve(project_root, &current_file, &import);
        assert!(resolved.is_none());
    }
}
