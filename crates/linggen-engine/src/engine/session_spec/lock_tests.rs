//! The lock: `SessionSpec::apply` is the one writer of a session's prompt
//! profile, tool restriction, interactivity, world and framing. A new
//! writer anywhere else fails here until it is looked at and listed.

use std::path::{Path, PathBuf};

/// What writes one of the spec's fields.
const WRITES: &[&str] = &[
    "prompt_profile",
    ".mission_allowed_tools",
    ".consumer_allowed_tools",
    ".consumer_allowed_skills",
    ".interactive",
    ".ask_caller",
    ".session_creator",
    ".session_policy",
];

/// Calls that narrow a profile in place.
const NARROWS: &[&str] = &["narrow_for_creator(", "scope_to_app("];

/// The writers allowed outside the spec, by file: how many, and why.
const LISTED: &[(&str, usize, &str)] = &[
    (
        "crates/linggen-engine/src/engine/turn_place/bind.rs",
        5,
        "a turn's place narrows the turn and puts it back",
    ),
    (
        "crates/linggen-engine/src/engine/skill_activation.rs",
        1,
        "a skill taken up scopes the skills it may name",
    ),
    (
        "crates/linggen-engine/src/engine/tools/delegation.rs",
        1,
        "a Task child takes its parent's grants (the spec set the same)",
    ),
    (
        "crates/linggen-engine/src/engine/prompt/profile.rs",
        5,
        "the narrowing rules themselves, and their test",
    ),
    (
        "crates/linggen-engine/src/engine/permission/mod.rs",
        3,
        "tests",
    ),
    ("crates/linggen-engine/src/engine/mod.rs", 2, "tests"),
    ("crates/linggen-engine/src/engine/types.rs", 1, "a test"),
    (
        "crates/linggen-engine/src/engine/tools/builtin/claude_session.rs",
        1,
        "a test",
    ),
    (
        "crates/linggen-engine/src/engine/prompt/notes.rs",
        4,
        "tests",
    ),
    (
        "crates/linggen-engine/src/engine/prompt/persona_tests.rs",
        2,
        "tests",
    ),
    (
        "crates/linggen-engine/src/engine/prompt/world/tests.rs",
        6,
        "tests",
    ),
];

/// Assignments to `field` (or to one of its fields, for the profile): an
/// `=` that is not `==`.
fn assignments(text: &str, field: &str) -> usize {
    text.match_indices(field)
        .filter(|(at, _)| {
            let rest = text[at + field.len()..]
                .trim_start_matches(|c: char| c == '.' || c == '_' || c.is_ascii_alphanumeric());
            let rest = rest.trim_start();
            rest.starts_with('=') && !rest.starts_with("==")
        })
        .count()
}

fn writers(text: &str) -> usize {
    let writes: usize = WRITES.iter().map(|f| assignments(text, f)).sum();
    let narrows: usize = NARROWS.iter().map(|n| text.matches(n).count()).sum();
    writes + narrows
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
fn the_spec_is_the_one_writer_of_a_sessions_behaviour() {
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
        if rel.starts_with("crates/linggen-engine/src/engine/session_spec/") {
            continue;
        }
        let found = writers(&std::fs::read_to_string(&file).unwrap());
        let want = LISTED.iter().find(|(f, _, _)| *f == rel).map_or(0, |e| e.1);
        assert_eq!(
            found, want,
            "{rel} writes a session's behaviour: go through SessionSpec::apply, or list it"
        );
    }
}

/// Calls that write a session's grant table.
const GRANTS: &[&str] = &[
    "set_path_mode(",
    "apply_grants(",
    ".granted_tools = ",
    ".granted_tools.push(",
    ".granted_tools.extend(",
];

/// The grant writers allowed outside the spec's preset grants (`grants.rs`),
/// by file: how many, and why. A skill's, a mission's and a delegate's
/// grants go through `stamp`; none of them is listed.
const GRANT_WRITERS: &[(&str, usize, &str)] = &[
    (
        "crates/linggen-engine/src/engine/permission/store.rs",
        1,
        "the writer itself",
    ),
    (
        "crates/linggen-engine/src/engine/permission/manifest.rs",
        2,
        "the frontmatter helper itself (a mission test uses it)",
    ),
    (
        "crates/linggen-engine/src/engine/tool_exec.rs",
        2,
        "a person's answer to a prompt; a test",
    ),
    (
        "src/server/api/permissions.rs",
        1,
        "a person's change from the permission badge",
    ),
    (
        "crates/linggen-engine/src/engine/agent/mod.rs",
        1,
        "that same change carried into the live engines",
    ),
    (
        "crates/linggen-engine/src/engine/mod.rs",
        4,
        "a new person's session starts at the configured level; tests",
    ),
    ("src/server/mcp_agent.rs", 1, "a test"),
    (
        "crates/linggen-engine/src/extensions/missions/mod.rs",
        1,
        "a test",
    ),
    (
        "crates/linggen-engine/src/engine/tools/delegation.rs",
        3,
        "a Task child takes its parent's granted tools; a test",
    ),
    (
        "crates/linggen-engine/src/engine/turn_place/bind/grants_tests.rs",
        1,
        "a test",
    ),
    (
        "crates/linggen-engine/src/engine/permission/mod.rs",
        31,
        "tests",
    ),
    (
        "crates/linggen-engine/src/engine/permission/granted_tools_tests.rs",
        2,
        "tests",
    ),
    (
        "crates/linggen-engine/src/engine/permission/gate_copy_tests.rs",
        2,
        "tests",
    ),
];

#[test]
fn preset_grants_are_written_in_one_place() {
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
        if rel.starts_with("crates/linggen-engine/src/engine/session_spec/") {
            continue;
        }
        let text = std::fs::read_to_string(&file).unwrap();
        let found: usize = GRANTS.iter().map(|g| text.matches(g).count()).sum();
        let want = GRANT_WRITERS
            .iter()
            .find(|(f, _, _)| *f == rel)
            .map_or(0, |e| e.1);
        assert_eq!(
            found, want,
            "{rel} writes a session's grants: go through PresetGrants, or list it"
        );
    }
}

#[test]
fn an_assignment_is_told_from_a_comparison() {
    assert_eq!(
        assignments("e.session_permissions.interactive = false;", ".interactive"),
        1
    );
    assert_eq!(assignments("if p.interactive == x {}", ".interactive"), 0);
    assert_eq!(
        assignments("e.prompt_profile.include_memory = false;", "prompt_profile"),
        1
    );
    assert_eq!(
        assignments("let p = &e.prompt_profile;", "prompt_profile"),
        0
    );
    assert_eq!(
        assignments("let session_creator = x;", ".session_creator"),
        0
    );
}
