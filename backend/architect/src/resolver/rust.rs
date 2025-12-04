//! Rust module path resolution

use super::ImportResolver;
use crate::parser::ImportInfo;
use std::path::{Path, PathBuf};

/// Rust import resolver
pub struct RustResolver;

impl RustResolver {
    pub fn new() -> Self {
        Self
    }

    /// Resolve a mod declaration (mod foo;) to a file path
    fn resolve_mod_decl(
        &self,
        current_file: &Path,
        current_dir: &Path,
        mod_name: &str,
    ) -> Option<PathBuf> {
        let current_file_name = current_file.file_name()?.to_str()?;

        let search_dir = if current_file_name == "mod.rs"
            || current_file_name == "lib.rs"
            || current_file_name == "main.rs"
        {
            current_dir.to_path_buf()
        } else {
            let stem = current_file.file_stem()?.to_str()?;
            current_dir.join(stem)
        };

        // Try foo.rs
        let candidate1 = search_dir.join(format!("{}.rs", mod_name));
        if candidate1.exists() {
            return Some(candidate1);
        }

        // Try foo/mod.rs
        let candidate2 = search_dir.join(mod_name).join("mod.rs");
        if candidate2.exists() {
            return Some(candidate2);
        }

        None
    }

    /// Resolve a use path (use crate::foo::bar) to a file path
    fn resolve_use_path(
        &self,
        project_root: &Path,
        current_dir: &Path,
        use_path: &str,
    ) -> Option<PathBuf> {
        let parts: Vec<&str> = use_path.split("::").collect();

        if parts.is_empty() {
            return None;
        }

        match parts[0] {
            "crate" => self.resolve_crate_path(project_root, current_dir, &parts[1..]),
            "self" => self.resolve_module_chain(current_dir, &parts[1..]),
            "super" => {
                let parent = current_dir.parent()?;
                self.resolve_module_chain(parent, &parts[1..])
            }
            _ => None,
        }
    }

    /// Resolve a crate-relative path
    fn resolve_crate_path(
        &self,
        project_root: &Path,
        current_dir: &Path,
        parts: &[&str],
    ) -> Option<PathBuf> {
        if parts.is_empty() {
            return None;
        }

        let crate_root = self.find_crate_root(project_root, current_dir)?;

        // First, try to resolve as a module chain
        if let Some(path) = self.resolve_module_chain(&crate_root, parts) {
            return Some(path);
        }

        // Fallback: item might be defined in lib.rs or main.rs
        let lib_rs = crate_root.join("lib.rs");
        if lib_rs.exists() {
            return Some(lib_rs);
        }

        let main_rs = crate_root.join("main.rs");
        if main_rs.exists() {
            return Some(main_rs);
        }

        None
    }

    /// Resolve a chain of module names to a file path
    fn resolve_module_chain(&self, start_dir: &Path, parts: &[&str]) -> Option<PathBuf> {
        if parts.is_empty() {
            return None;
        }

        // Try progressively shorter prefixes (item name vs module name heuristic)
        for i in (1..=parts.len()).rev() {
            let module_parts = &parts[..i];
            let mut current_dir = start_dir.to_path_buf();

            for (j, &part) in module_parts.iter().enumerate() {
                let is_last = j == module_parts.len() - 1;

                if is_last {
                    // Try as file: part.rs
                    let candidate1 = current_dir.join(format!("{}.rs", part));
                    if candidate1.exists() {
                        return Some(candidate1);
                    }

                    // Try as directory: part/mod.rs
                    let candidate2 = current_dir.join(part).join("mod.rs");
                    if candidate2.exists() {
                        return Some(candidate2);
                    }
                } else {
                    current_dir = current_dir.join(part);
                    if !current_dir.exists() {
                        break;
                    }
                }
            }
        }

        None
    }

    /// Find the crate root directory (containing lib.rs or main.rs)
    fn find_crate_root(&self, project_root: &Path, current_dir: &Path) -> Option<PathBuf> {
        let mut dir = current_dir.to_path_buf();

        loop {
            if dir.join("lib.rs").exists() || dir.join("main.rs").exists() {
                return Some(dir);
            }

            if dir == project_root || !dir.starts_with(project_root) {
                break;
            }

            dir = dir.parent()?.to_path_buf();
        }

        let src_dir = project_root.join("src");
        if src_dir.exists() {
            return Some(src_dir);
        }

        None
    }
}

impl Default for RustResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl ImportResolver for RustResolver {
    fn resolve(
        &self,
        project_root: &Path,
        current_file: &Path,
        import: &ImportInfo,
    ) -> Option<String> {
        let current_dir = current_file
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| project_root.to_path_buf());

        let resolved_path = if import.is_mod_decl {
            self.resolve_mod_decl(current_file, &current_dir, &import.module_path)
        } else {
            self.resolve_use_path(project_root, &current_dir, &import.module_path)
        };

        resolved_path.and_then(|path| {
            path.strip_prefix(project_root)
                .ok()
                .map(|p| p.to_string_lossy().to_string())
        })
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
    use std::fs;
    use tempfile::TempDir;

    fn setup_test_project() -> TempDir {
        let temp = TempDir::new().unwrap();
        let src = temp.path().join("src");

        fs::create_dir_all(&src).unwrap();
        fs::create_dir_all(src.join("foo")).unwrap();
        fs::create_dir_all(src.join("bar")).unwrap();

        fs::write(src.join("lib.rs"), "mod foo;\nmod bar;").unwrap();
        fs::write(src.join("foo.rs"), "// foo module").unwrap();
        fs::write(src.join("bar.rs"), "mod baz;").unwrap();
        fs::write(src.join("foo").join("mod.rs"), "// foo/mod.rs").unwrap();
        fs::write(src.join("bar").join("baz.rs"), "// baz").unwrap();

        temp
    }

    #[test]
    fn test_resolve_mod_decl() {
        let temp = setup_test_project();
        let project_root = temp.path();
        let current_file = project_root.join("src").join("lib.rs");
        let resolver = RustResolver::new();

        let import = ImportInfo {
            module_path: "foo".to_string(),
            is_mod_decl: true,
            is_reexport: false,
        };

        let resolved = resolver.resolve(project_root, &current_file, &import);
        assert!(resolved.is_some());
        assert!(resolved.unwrap().contains("foo"));
    }

    #[test]
    fn test_resolve_crate_path() {
        let temp = setup_test_project();
        let project_root = temp.path();
        let current_file = project_root.join("src").join("bar.rs");
        let resolver = RustResolver::new();

        let import = ImportInfo {
            module_path: "crate::foo".to_string(),
            is_mod_decl: false,
            is_reexport: false,
        };

        let resolved = resolver.resolve(project_root, &current_file, &import);
        assert!(resolved.is_some());
    }
}
