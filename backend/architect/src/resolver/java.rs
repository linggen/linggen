//! Java import resolution

use super::{ImportResolver, ResolvedImport};
use crate::parser::ImportInfo;
use crate::EdgeKind;
use std::path::{Path, PathBuf};

/// Java import resolver.
///
/// Java imports are package-based (e.g. `com.foo.Bar`), so we map:
/// `com.foo.Bar` -> `com/foo/Bar.java` under common source roots.
pub struct JavaResolver;

impl JavaResolver {
    pub fn new() -> Self {
        Self
    }

    fn common_source_roots() -> &'static [&'static str] {
        &[
            "",                  // project root (rare, but cheap)
            "src",               // simple projects
            "src/main/java",     // Maven/Gradle
            "src/test/java",     // tests
            "app/src/main/java", // Android
        ]
    }

    fn is_external(import_path: &str) -> bool {
        // Treat JDK / Jakarta / common external namespaces as external.
        import_path.starts_with("java.")
            || import_path.starts_with("javax.")
            || import_path.starts_with("jakarta.")
            || import_path.starts_with("kotlin.")
            || import_path.starts_with("android.")
            || import_path.starts_with("org.junit.")
    }

    fn import_to_candidate_relpaths(import_path: &str) -> Vec<String> {
        // Strip members from static imports: `com.foo.Bar.baz` -> `com.foo.Bar`
        // If it ends with wildcard, skip (can't map to a single file).
        let s = import_path.trim();
        if s.ends_with(".*") {
            return Vec::new();
        }

        let mut base = s.to_string();
        // Heuristic: if last segment starts with lowercase, it's likely a member.
        // Example: com.foo.Bar.baz -> com.foo.Bar
        if let Some(last_dot) = base.rfind('.') {
            let last_seg = &base[last_dot + 1..];
            if last_seg
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_lowercase())
            {
                base.truncate(last_dot);
            }
        }

        // Map to path segments
        let rel = base.replace('.', "/") + ".java";
        vec![rel]
    }

    fn resolve_under_roots(&self, project_root: &Path, rel: &str) -> Option<String> {
        for root in Self::common_source_roots() {
            let candidate = project_root.join(root).join(rel);
            if candidate.exists() && candidate.is_file() {
                return candidate
                    .strip_prefix(project_root)
                    .ok()
                    .map(|p| p.to_string_lossy().to_string());
            }
        }
        None
    }

    fn normalize_relative(path: &Path) -> PathBuf {
        // Minimal normalization: collapse `.` and `..` for returned relative paths.
        use std::path::Component;
        let mut out = PathBuf::new();
        for c in path.components() {
            match c {
                Component::CurDir => {}
                Component::ParentDir => {
                    out.pop();
                }
                Component::Normal(p) => out.push(p),
                _ => {}
            }
        }
        out
    }
}

impl Default for JavaResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl ImportResolver for JavaResolver {
    fn resolve(
        &self,
        project_root: &Path,
        _current_file: &Path,
        import: &ImportInfo,
    ) -> Option<ResolvedImport> {
        let import_path = import.module_path.trim();
        if import_path.is_empty() {
            return None;
        }
        if Self::is_external(import_path) {
            return None;
        }

        // Only handle dotted package/class imports.
        if !import_path.contains('.') {
            return None;
        }

        for rel in Self::import_to_candidate_relpaths(import_path) {
            if let Some(found) = self.resolve_under_roots(project_root, &rel) {
                return Some(ResolvedImport {
                    target: Self::normalize_relative(Path::new(&found))
                        .to_string_lossy()
                        .to_string(),
                    kind: EdgeKind::Import,
                });
            }
        }

        None
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
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_resolve_maven_style_src_main_java() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        let src_main = root.join("src/main/java");
        let app_pkg = src_main.join("com/example/app");
        let util_pkg = src_main.join("com/example/util");
        fs::create_dir_all(&app_pkg).unwrap();
        fs::create_dir_all(&util_pkg).unwrap();

        fs::write(app_pkg.join("Main.java"), "package com.example.app;").unwrap();
        fs::write(util_pkg.join("Helper.java"), "package com.example.util;").unwrap();

        let resolver = JavaResolver::new();
        let import = ImportInfo {
            module_path: "com.example.util.Helper".to_string(),
            is_mod_decl: false,
            is_reexport: false,
        };
        let current_file = app_pkg.join("Main.java");
        let resolved = resolver.resolve(root, &current_file, &import).unwrap();
        assert_eq!(
            resolved.target,
            "src/main/java/com/example/util/Helper.java"
        );
    }

    #[test]
    fn test_skip_jdk_imports() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        let resolver = JavaResolver::new();
        let import = ImportInfo {
            module_path: "java.util.List".to_string(),
            is_mod_decl: false,
            is_reexport: false,
        };
        let resolved = resolver.resolve(root, root, &import);
        assert!(resolved.is_none());
    }

    #[test]
    fn test_static_member_import_resolves_to_class_file() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        let src = root.join("src");
        let pkg = src.join("com/foo");
        fs::create_dir_all(&pkg).unwrap();
        fs::write(pkg.join("Util.java"), "package com.foo;").unwrap();

        let resolver = JavaResolver::new();
        let import = ImportInfo {
            module_path: "com.foo.Util.baz".to_string(),
            is_mod_decl: false,
            is_reexport: false,
        };
        let resolved = resolver.resolve(root, root, &import).unwrap();
        assert_eq!(resolved.target, "src/com/foo/Util.java");
    }

    #[test]
    fn test_wildcard_import_is_skipped() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        let resolver = JavaResolver::new();
        let import = ImportInfo {
            module_path: "com.foo.*".to_string(),
            is_mod_decl: false,
            is_reexport: false,
        };
        let resolved = resolver.resolve(root, root, &import);
        assert!(resolved.is_none());
    }
}
