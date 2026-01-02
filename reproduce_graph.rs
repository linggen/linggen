use architect::parser::ImportInfo;
use architect::resolver::rust::RustResolver;
use architect::resolver::ImportResolver;
use std::path::{Path, PathBuf};

fn main() {
    let resolver = RustResolver::new();
    let project_root = Path::new("/tmp/test_rust_proj");
    std::fs::create_dir_all(project_root.join("src")).unwrap();
    std::fs::write(
        project_root.join("Cargo.toml"),
        r#"
[package]
name = "my-crate"
version = "0.1.0"
"#,
    )
    .unwrap();
    std::fs::write(project_root.join("src/lib.rs"), "pub mod foo;").unwrap();
    std::fs::write(project_root.join("src/main.rs"), "use my_crate::foo;").unwrap();

    let current_file = project_root.join("src/main.rs");
    let import = ImportInfo {
        module_path: "my_crate".to_string(), // Simplified for the test
        is_mod_decl: false,
        is_reexport: false,
    };

    let resolved = resolver.resolve(project_root, &current_file, &import);
    println!("Resolved: {:?}", resolved);

    let import2 = ImportInfo {
        module_path: "my_crate::foo".to_string(),
        is_mod_decl: false,
        is_reexport: false,
    };
    let resolved2 = resolver.resolve(project_root, &current_file, &import2);
    println!("Resolved 2: {:?}", resolved2);
}
