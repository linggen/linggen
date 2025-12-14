//! Rust module path resolution

use super::{ImportResolver, ResolvedImport};
use crate::parser::ImportInfo;
use crate::EdgeKind;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// Rust import resolver
pub struct RustResolver;

/// Cache: project_root -> (crate_name -> crate_root_file_abs)
static WORKSPACE_CRATE_CACHE: OnceLock<Mutex<HashMap<PathBuf, HashMap<String, PathBuf>>>> =
    OnceLock::new();

impl RustResolver {
    pub fn new() -> Self {
        Self
    }

    fn workspace_cache() -> &'static Mutex<HashMap<PathBuf, HashMap<String, PathBuf>>> {
        WORKSPACE_CRATE_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
    }

    fn crate_name_key(name: &str) -> String {
        // Cargo package names may contain '-', but Rust crate identifiers use '_'.
        name.trim().replace('-', "_")
    }

    /// Find the nearest Cargo workspace root (directory containing Cargo.toml with `[workspace]`)
    /// by walking up from `start_dir` until `project_root`.
    fn find_workspace_root(project_root: &Path, start_dir: &Path) -> Option<PathBuf> {
        let mut dir = start_dir.to_path_buf();
        loop {
            let manifest = dir.join("Cargo.toml");
            if manifest.exists() {
                if let Ok(s) = std::fs::read_to_string(&manifest) {
                    // Fast check first to avoid parsing in most dirs
                    if s.contains("[workspace]") {
                        // Ensure it's valid TOML workspace (best-effort)
                        if let Ok(v) = s.parse::<toml::Value>() {
                            if v.get("workspace").is_some() {
                                return Some(dir);
                            }
                        }
                    }
                }
            }

            if dir == project_root || !dir.starts_with(project_root) {
                break;
            }
            dir = dir.parent()?.to_path_buf();
        }
        None
    }

    /// Best-effort: get workspace member crates for this `project_root`.
    ///
    /// This is intentionally heuristic (no `cargo metadata`) and is meant to catch
    /// the common case where `use some_workspace_crate::...` should create a file edge.
    ///
    /// Note: the workspace root may be a subdirectory of the indexed source root.
    fn workspace_crate_roots(workspace_root: &Path) -> HashMap<String, PathBuf> {
        let root_key = workspace_root
            .canonicalize()
            .unwrap_or_else(|_| workspace_root.to_path_buf());

        // Fast path: cached
        if let Ok(cache) = Self::workspace_cache().lock() {
            if let Some(m) = cache.get(&root_key) {
                return m.clone();
            }
        }

        let mut result: HashMap<String, PathBuf> = HashMap::new();

        let manifest = workspace_root.join("Cargo.toml");
        let manifest_str = match std::fs::read_to_string(&manifest) {
            Ok(s) => s,
            Err(_) => {
                // Cache empty map
                if let Ok(mut cache) = Self::workspace_cache().lock() {
                    cache.insert(root_key, HashMap::new());
                }
                return HashMap::new();
            }
        };

        let doc: toml::Value = match manifest_str.parse::<toml::Value>() {
            Ok(v) => v,
            Err(_) => {
                if let Ok(mut cache) = Self::workspace_cache().lock() {
                    cache.insert(root_key, HashMap::new());
                }
                return HashMap::new();
            }
        };

        // Helper to add a crate mapping from a member dir's Cargo.toml
        let mut add_member = |member_dir: &Path| {
            let cargo_toml = member_dir.join("Cargo.toml");
            let cargo_str = std::fs::read_to_string(&cargo_toml).ok()?;
            let cargo_doc: toml::Value = cargo_str.parse::<toml::Value>().ok()?;

            let pkg_name = cargo_doc
                .get("package")
                .and_then(|p| p.get("name"))
                .and_then(|n| n.as_str())
                .map(|s| s.to_string())?;

            // Determine crate root file:
            // - prefer [lib].path if set
            // - otherwise prefer src/lib.rs, then src/main.rs
            let lib_path = cargo_doc
                .get("lib")
                .and_then(|l| l.get("path"))
                .and_then(|p| p.as_str())
                .map(|p| member_dir.join(p));

            let root_file = lib_path
                .filter(|p| p.exists())
                .or_else(|| {
                    let p = member_dir.join("src").join("lib.rs");
                    p.exists().then_some(p)
                })
                .or_else(|| {
                    let p = member_dir.join("src").join("main.rs");
                    p.exists().then_some(p)
                })?;

            result.insert(Self::crate_name_key(&pkg_name), root_file);
            Some(())
        };

        // If this is a workspace, scan members; otherwise treat as single crate.
        if let Some(members) = doc
            .get("workspace")
            .and_then(|w| w.get("members"))
            .and_then(|m| m.as_array())
        {
            for m in members {
                let Some(pattern) = m.as_str() else { continue };
                let pattern_abs = workspace_root.join(pattern);

                // Expand glob patterns when present; otherwise treat as a single path.
                let pattern_str = pattern_abs.to_string_lossy().to_string();
                if pattern.contains('*') || pattern.contains('?') || pattern.contains('[') {
                    if let Ok(paths) = glob::glob(&pattern_str) {
                        for p in paths.flatten() {
                            if p.is_dir() {
                                let _ = add_member(&p);
                            }
                        }
                    }
                } else if pattern_abs.is_dir() {
                    let _ = add_member(&pattern_abs);
                }
            }
        } else {
            // Single crate (non-workspace)
            let pkg_name = doc
                .get("package")
                .and_then(|p| p.get("name"))
                .and_then(|n| n.as_str())
                .map(|s| s.to_string());

            if let Some(pkg_name) = pkg_name {
                let root_file = workspace_root
                    .join("src")
                    .join("lib.rs")
                    .exists()
                    .then_some(workspace_root.join("src").join("lib.rs"))
                    .or_else(|| {
                        workspace_root
                            .join("src")
                            .join("main.rs")
                            .exists()
                            .then_some(workspace_root.join("src").join("main.rs"))
                    });

                if let Some(root_file) = root_file {
                    result.insert(Self::crate_name_key(&pkg_name), root_file);
                }
            }
        }

        if let Ok(mut cache) = Self::workspace_cache().lock() {
            cache.insert(root_key, result.clone());
        }

        result
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
            // Heuristic: treat `use workspace_crate::...` as a dependency on that crate root file
            // when `workspace_crate` is a workspace member under `project_root`.
            other => {
                let workspace_root = Self::find_workspace_root(project_root, current_dir)?;
                let map = Self::workspace_crate_roots(&workspace_root);
                map.get(&Self::crate_name_key(other)).cloned()
            }
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
    ) -> Option<ResolvedImport> {
        let current_dir = current_file
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| project_root.to_path_buf());

        // Determine the edge kind, with an override for workspace crate imports.
        let mut kind = if import.is_mod_decl {
            EdgeKind::ModuleDecl
        } else if import.is_reexport {
            EdgeKind::ReExport
        } else {
            EdgeKind::Import
        };

        // If the import starts with an identifier and resolves to a workspace member crate root,
        // mark it specially so the UI can distinguish crate deps from module deps.
        if !import.is_mod_decl {
            let parts: Vec<&str> = import.module_path.split("::").collect();
            if let Some(first) = parts.first().copied() {
                if first != "crate" && first != "self" && first != "super" {
                    if let Some(workspace_root) =
                        Self::find_workspace_root(project_root, &current_dir)
                    {
                        let map = Self::workspace_crate_roots(&workspace_root);
                        if map.contains_key(&Self::crate_name_key(first)) {
                            kind = EdgeKind::WorkspaceCrateImport;
                        }
                    }
                }
            }
        }

        let resolved_path = if import.is_mod_decl {
            self.resolve_mod_decl(current_file, &current_dir, &import.module_path)
        } else {
            self.resolve_use_path(project_root, &current_dir, &import.module_path)
        }?;

        let target = resolved_path
            .strip_prefix(project_root)
            .ok()
            .map(|p| p.to_string_lossy().to_string())?;

        Some(ResolvedImport { target, kind })
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
    fn test_resolve_workspace_crate_use_path() {
        let temp = TempDir::new().unwrap();

        // Root workspace Cargo.toml
        fs::write(
            temp.path().join("Cargo.toml"),
            r#"
[workspace]
members = ["linggen_core", "app"]
"#,
        )
        .unwrap();

        // Member: linggen_core
        fs::create_dir_all(temp.path().join("linggen_core").join("src")).unwrap();
        fs::write(
            temp.path().join("linggen_core").join("Cargo.toml"),
            r#"
[package]
name = "linggen_core"
version = "0.1.0"
edition = "2021"
"#,
        )
        .unwrap();
        fs::write(
            temp.path().join("linggen_core").join("src").join("lib.rs"),
            "pub fn x() {}",
        )
        .unwrap();

        // Member: app
        fs::create_dir_all(temp.path().join("app").join("src")).unwrap();
        fs::write(
            temp.path().join("app").join("Cargo.toml"),
            r#"
[package]
name = "app"
version = "0.1.0"
edition = "2021"
"#,
        )
        .unwrap();
        fs::write(
            temp.path().join("app").join("src").join("main.rs"),
            "fn main() {}",
        )
        .unwrap();
        fs::write(
            temp.path().join("app").join("src").join("lib.rs"),
            r#"use linggen_core::{x};"#,
        )
        .unwrap();

        let resolver = RustResolver::new();
        let project_root = temp.path();
        let current_file = project_root.join("app").join("src").join("lib.rs");

        let import = ImportInfo {
            module_path: "linggen_core::x".to_string(),
            is_mod_decl: false,
            is_reexport: false,
        };

        let resolved = resolver
            .resolve(project_root, &current_file, &import)
            .unwrap();
        assert_eq!(resolved.target, "linggen_core/src/lib.rs".to_string());
        assert_eq!(resolved.kind, EdgeKind::WorkspaceCrateImport);
    }

    #[test]
    fn test_resolve_workspace_in_subdir_of_project_root() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();

        // Project root has no Cargo.toml. Workspace is in a subdir.
        let backend = root.join("backend");
        fs::create_dir_all(&backend).unwrap();
        fs::write(
            backend.join("Cargo.toml"),
            r#"
[workspace]
members = ["core", "api"]
"#,
        )
        .unwrap();

        // backend/core
        fs::create_dir_all(backend.join("core").join("src")).unwrap();
        fs::write(
            backend.join("core").join("Cargo.toml"),
            r#"
[package]
name = "linggen_core"
version = "0.1.0"
edition = "2021"
"#,
        )
        .unwrap();
        fs::write(
            backend.join("core").join("src").join("lib.rs"),
            "pub fn x() {}",
        )
        .unwrap();

        // backend/api
        fs::create_dir_all(backend.join("api").join("src")).unwrap();
        fs::write(
            backend.join("api").join("Cargo.toml"),
            r#"
[package]
name = "api"
version = "0.1.0"
edition = "2021"
"#,
        )
        .unwrap();
        let api_lib = backend.join("api").join("src").join("lib.rs");
        fs::write(&api_lib, "use linggen_core::x;").unwrap();

        let resolver = RustResolver::new();
        let import = ImportInfo {
            module_path: "linggen_core::x".to_string(),
            is_mod_decl: false,
            is_reexport: false,
        };

        let resolved = resolver.resolve(root, &api_lib, &import).unwrap();
        assert_eq!(resolved.target, "backend/core/src/lib.rs".to_string());
        assert_eq!(resolved.kind, EdgeKind::WorkspaceCrateImport);
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
        let resolved = resolved.unwrap();
        assert!(resolved.target.contains("foo"));
        assert_eq!(resolved.kind, EdgeKind::ModuleDecl);
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
