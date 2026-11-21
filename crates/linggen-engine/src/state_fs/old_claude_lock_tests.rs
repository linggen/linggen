//! The lock: whether a session is an old Claude one is decided by
//! `SessionMeta::is_old_claude` (`sessions.rs`) alone. `SessionRuntime::is_linggen`
//! is private there, so no other file can ask it; and no other file names
//! `SessionRuntime` or `ClaudeAgentSdk`, which is how a runtime is compared.
//! A new door asks the method. The page's own copy of the rule is locked in
//! `ui/tests/oldClaude.test.mts`.

use std::path::{Path, PathBuf};

/// Files outside `sessions.rs` allowed to name the runtime, by file: how many
/// times, and why.
const ALLOWED: &[(&str, usize, &str)] = &[
    (
        "src/server/rtc/page_state.rs",
        1,
        "sends the stored runtime to the page, which tells old sessions apart itself",
    ),
    (
        "crates/linggen-engine/src/engine/agent/mod.rs",
        2,
        "a test builds a session as 2.5.1 stored it",
    ),
    (
        "crates/linggen-engine/src/engine/session_spec/tests.rs",
        2,
        "a test builds a session as 2.5.1 stored it",
    ),
];

/// Mentions of the runtime type or its old variant.
fn reads(text: &str) -> usize {
    text.matches("SessionRuntime").count() + text.matches("ClaudeAgentSdk").count()
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn one_place_decides_an_old_claude_session() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    for dir in ["src", "crates/linggen-engine/src"] {
        rust_files(&root.join(dir), &mut files);
    }
    for file in files {
        let rel = file
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .to_string();
        if rel.ends_with("state_fs/sessions.rs") || rel.ends_with("old_claude_lock_tests.rs") {
            continue;
        }
        let want = ALLOWED
            .iter()
            .find(|(f, _, _)| *f == rel)
            .map_or(0, |e| e.1);
        let got = reads(&std::fs::read_to_string(&file).unwrap());
        assert_eq!(
            got, want,
            "{rel} names the session runtime: ask SessionMeta::is_old_claude, or list it"
        );
    }
}

#[test]
fn the_scan_sees_a_mention() {
    assert_eq!(reads("SessionRuntime::ClaudeAgentSdk; x.runtime_dir"), 2);
}
