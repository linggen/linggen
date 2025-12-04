//! Python import resolution

use super::ImportResolver;
use crate::parser::ImportInfo;
use std::path::Path;

/// Python import resolver
pub struct PythonResolver;

impl PythonResolver {
    pub fn new() -> Self {
        Self
    }

    /// Resolve a relative import (starting with .)
    fn resolve_relative_import(
        &self,
        project_root: &Path,
        current_file: &Path,
        import_path: &str,
    ) -> Option<String> {
        let current_dir = current_file.parent()?;

        // Count leading dots to determine how many levels to go up
        let dots = import_path.chars().take_while(|&c| c == '.').count();
        let remaining = &import_path[dots..];

        // Navigate up directories based on dot count
        // . = current package, .. = parent package, etc.
        let mut target_dir = current_dir.to_path_buf();
        for _ in 1..dots {
            target_dir = target_dir.parent()?.to_path_buf();
        }

        // Convert remaining module path to file path
        if remaining.is_empty() {
            // Just dots, referring to package __init__.py
            let init_file = target_dir.join("__init__.py");
            if init_file.exists() {
                return init_file
                    .strip_prefix(project_root)
                    .ok()
                    .map(|p| p.to_string_lossy().to_string());
            }
        } else {
            let module_path = remaining.replace('.', "/");
            let target = target_dir.join(&module_path);

            // Try as .py file
            let py_file = format!("{}.py", target.display());
            let py_path = Path::new(&py_file);
            if py_path.exists() {
                return py_path
                    .strip_prefix(project_root)
                    .ok()
                    .map(|p| p.to_string_lossy().to_string());
            }

            // Try as package (__init__.py)
            let init_file = target.join("__init__.py");
            if init_file.exists() {
                return init_file
                    .strip_prefix(project_root)
                    .ok()
                    .map(|p| p.to_string_lossy().to_string());
            }

            // Try as .pyi stub file
            let pyi_file = format!("{}.pyi", target.display());
            let pyi_path = Path::new(&pyi_file);
            if pyi_path.exists() {
                return pyi_path
                    .strip_prefix(project_root)
                    .ok()
                    .map(|p| p.to_string_lossy().to_string());
            }
        }

        None
    }

    /// Resolve an absolute import (local module)
    fn resolve_absolute_import(
        &self,
        project_root: &Path,
        import_path: &str,
    ) -> Option<String> {
        let module_path = import_path.replace('.', "/");
        let target = project_root.join(&module_path);

        // Try as .py file
        let py_file = format!("{}.py", target.display());
        let py_path = Path::new(&py_file);
        if py_path.exists() {
            return py_path
                .strip_prefix(project_root)
                .ok()
                .map(|p| p.to_string_lossy().to_string());
        }

        // Try as package (__init__.py)
        let init_file = target.join("__init__.py");
        if init_file.exists() {
            return init_file
                .strip_prefix(project_root)
                .ok()
                .map(|p| p.to_string_lossy().to_string());
        }

        // Try in src/ directory (common Python project structure)
        let src_target = project_root.join("src").join(&module_path);
        let src_py_file = format!("{}.py", src_target.display());
        let src_py_path = Path::new(&src_py_file);
        if src_py_path.exists() {
            return src_py_path
                .strip_prefix(project_root)
                .ok()
                .map(|p| p.to_string_lossy().to_string());
        }

        let src_init_file = src_target.join("__init__.py");
        if src_init_file.exists() {
            return src_init_file
                .strip_prefix(project_root)
                .ok()
                .map(|p| p.to_string_lossy().to_string());
        }

        None
    }
}

impl Default for PythonResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl ImportResolver for PythonResolver {
    fn resolve(
        &self,
        project_root: &Path,
        current_file: &Path,
        import: &ImportInfo,
    ) -> Option<String> {
        let import_path = &import.module_path;

        if import_path.starts_with('.') {
            // Relative import
            self.resolve_relative_import(project_root, current_file, import_path)
        } else {
            // Absolute import - try to resolve locally
            self.resolve_absolute_import(project_root, import_path)
        }
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
    use std::fs;
    use tempfile::TempDir;

    fn setup_test_project() -> TempDir {
        let temp = TempDir::new().unwrap();
        let mypackage = temp.path().join("mypackage");
        let submodule = mypackage.join("submodule");

        fs::create_dir_all(&submodule).unwrap();

        fs::write(mypackage.join("__init__.py"), "").unwrap();
        fs::write(mypackage.join("utils.py"), "def helper(): pass").unwrap();
        fs::write(submodule.join("__init__.py"), "").unwrap();
        fs::write(submodule.join("handler.py"), "class Handler: pass").unwrap();

        temp
    }

    #[test]
    fn test_resolve_relative_import() {
        let temp = setup_test_project();
        let project_root = temp.path();
        let current_file = project_root.join("mypackage").join("submodule").join("handler.py");
        let resolver = PythonResolver::new();

        let import = ImportInfo {
            module_path: "..utils".to_string(),
            is_mod_decl: false,
            is_reexport: false,
        };

        let resolved = resolver.resolve(project_root, &current_file, &import);
        assert!(resolved.is_some());
        assert!(resolved.unwrap().contains("utils.py"));
    }

    #[test]
    fn test_resolve_absolute_import() {
        let temp = setup_test_project();
        let project_root = temp.path();
        let current_file = project_root.join("main.py");
        let resolver = PythonResolver::new();

        // Create main.py
        fs::write(&current_file, "from mypackage import utils").unwrap();

        let import = ImportInfo {
            module_path: "mypackage.utils".to_string(),
            is_mod_decl: false,
            is_reexport: false,
        };

        let resolved = resolver.resolve(project_root, &current_file, &import);
        assert!(resolved.is_some());
        assert!(resolved.unwrap().contains("utils.py"));
    }

    #[test]
    fn test_resolve_package_init() {
        let temp = setup_test_project();
        let project_root = temp.path();
        let current_file = project_root.join("main.py");
        let resolver = PythonResolver::new();

        fs::write(&current_file, "import mypackage").unwrap();

        let import = ImportInfo {
            module_path: "mypackage".to_string(),
            is_mod_decl: false,
            is_reexport: false,
        };

        let resolved = resolver.resolve(project_root, &current_file, &import);
        assert!(resolved.is_some());
        assert!(resolved.unwrap().contains("__init__.py"));
    }

    #[test]
    fn test_skip_stdlib() {
        let temp = setup_test_project();
        let project_root = temp.path();
        let current_file = project_root.join("main.py");
        let resolver = PythonResolver::new();

        let import = ImportInfo {
            module_path: "os".to_string(),
            is_mod_decl: false,
            is_reexport: false,
        };

        let resolved = resolver.resolve(project_root, &current_file, &import);
        // Should be None because 'os' doesn't exist locally
        assert!(resolved.is_none());
    }
}
