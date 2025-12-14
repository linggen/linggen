//! TypeScript/JavaScript import resolution

use super::{ImportResolver, ResolvedImport};
use crate::parser::ImportInfo;
use crate::EdgeKind;
use std::path::{Component, Path, PathBuf};

/// TypeScript/JavaScript import resolver
pub struct TypeScriptResolver;

impl TypeScriptResolver {
    pub fn new() -> Self {
        Self
    }

    /// Try to resolve a relative import path to a file
    fn resolve_relative_import(
        &self,
        project_root: &Path,
        current_dir: &Path,
        import_path: &str,
    ) -> Option<String> {
        let target = current_dir.join(import_path);

        // Extensions to try (in order of preference)
        let extensions = [
            "",          // exact match (e.g., ./styles.css)
            ".ts",       // TypeScript
            ".tsx",      // TSX
            ".js",       // JavaScript
            ".jsx",      // JSX
            ".mjs",      // ES modules
            ".cjs",      // CommonJS
            ".json",     // JSON
            "/index.ts", // Directory index
            "/index.tsx",
            "/index.js",
            "/index.jsx",
        ];

        for ext in &extensions {
            let candidate = if ext.is_empty() {
                target.clone()
            } else if ext.starts_with('/') {
                // Directory index file
                target.join(&ext[1..])
            } else {
                // File extension
                let mut path = target.clone();
                let current_ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                if current_ext.is_empty() {
                    // No extension, add one
                    path.set_extension(&ext[1..]);
                    path
                } else {
                    // Already has extension, try as-is first
                    continue;
                }
            };

            if candidate.exists() && candidate.is_file() {
                return candidate
                    .strip_prefix(project_root)
                    .ok()
                    .map(|p| normalize_relative_path(p).to_string_lossy().to_string());
            }
        }

        // Try exact match with original extension
        if target.exists() && target.is_file() {
            return target
                .strip_prefix(project_root)
                .ok()
                .map(|p| normalize_relative_path(p).to_string_lossy().to_string());
        }

        None
    }
}

fn normalize_relative_path(path: &Path) -> PathBuf {
    // Ensure we don't return paths like `src/./foo.ts` which won't match
    // the node ids produced by the walker (which are typically normalized).
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            Component::Normal(p) => out.push(p),
            // Ignore prefixes/root components in this context (we only call this with a relative path)
            _ => {}
        }
    }
    out
}

impl Default for TypeScriptResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl ImportResolver for TypeScriptResolver {
    fn resolve(
        &self,
        project_root: &Path,
        current_file: &Path,
        import: &ImportInfo,
    ) -> Option<ResolvedImport> {
        let import_path = &import.module_path;

        // Only resolve relative imports (starting with . or ..)
        if !import_path.starts_with('.') {
            return None;
        }

        let current_dir = current_file.parent()?;
        self.resolve_relative_import(project_root, current_dir, import_path)
            .map(|target| ResolvedImport {
                target,
                kind: if import.is_reexport {
                    EdgeKind::ReExport
                } else {
                    EdgeKind::Import
                },
            })
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
    use std::fs;
    use tempfile::TempDir;

    fn setup_test_project() -> TempDir {
        let temp = TempDir::new().unwrap();
        let src = temp.path().join("src");
        let components = src.join("components");

        fs::create_dir_all(&components).unwrap();

        fs::write(src.join("index.ts"), "import { App } from './App';").unwrap();
        fs::write(src.join("App.tsx"), "export const App = () => {};").unwrap();
        fs::write(
            components.join("Button.tsx"),
            "export const Button = () => {};",
        )
        .unwrap();
        fs::write(components.join("index.ts"), "export * from './Button';").unwrap();

        temp
    }

    #[test]
    fn test_resolve_relative_import() {
        let temp = setup_test_project();
        let project_root = temp.path();
        let current_file = project_root.join("src").join("index.ts");
        let resolver = TypeScriptResolver::new();

        let import = ImportInfo {
            module_path: "./App".to_string(),
            is_mod_decl: false,
            is_reexport: false,
        };

        let resolved = resolver
            .resolve(project_root, &current_file, &import)
            .unwrap();
        assert!(resolved.target.contains("App.tsx"));
        assert_eq!(resolved.kind, EdgeKind::Import);
    }

    #[test]
    fn test_resolve_directory_index() {
        let temp = setup_test_project();
        let project_root = temp.path();
        let current_file = project_root.join("src").join("index.ts");
        let resolver = TypeScriptResolver::new();

        let import = ImportInfo {
            module_path: "./components".to_string(),
            is_mod_decl: false,
            is_reexport: false,
        };

        let resolved = resolver
            .resolve(project_root, &current_file, &import)
            .unwrap();
        assert!(resolved.target.contains("index.ts"));
        assert_eq!(resolved.kind, EdgeKind::Import);
    }

    #[test]
    fn test_skip_npm_packages() {
        let temp = setup_test_project();
        let project_root = temp.path();
        let current_file = project_root.join("src").join("index.ts");
        let resolver = TypeScriptResolver::new();

        let import = ImportInfo {
            module_path: "react".to_string(),
            is_mod_decl: false,
            is_reexport: false,
        };

        let resolved = resolver.resolve(project_root, &current_file, &import);
        assert!(resolved.is_none()); // npm packages should not resolve
    }
}
