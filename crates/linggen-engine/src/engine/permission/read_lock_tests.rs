use std::path::Path;

fn policy_calls(source: &str) -> usize {
    source.split_whitespace().collect::<String>().matches("read_scope::decide(").count()
}

fn within_gate(source: &str, in_gate: bool) -> bool {
    policy_calls(source) == if in_gate { 2 } else { 0 }
}

fn check_tree(dir: &Path) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            check_tree(&path);
        } else if path.extension().is_some_and(|ext| ext == "rs")
            && path.file_name().unwrap() != "read_lock_tests.rs"
        {
            let source = std::fs::read_to_string(&path).unwrap();
            let in_gate = path.ends_with("engine/permission/model.rs");
            assert!(within_gate(&source, in_gate), "{}: route read decisions through the engine gate", path.display());
        }
    }
}

#[test]
fn read_policy_has_one_gate() {
    let engine = Path::new(env!("CARGO_MANIFEST_DIR"));
    check_tree(&engine.join("src"));
    check_tree(&engine.join("../../src"));
}

#[test]
fn read_policy_lock_detects_another_caller() {
    assert!(!within_gate("let decision = read_scope :: decide (target, grants);", false));
}
