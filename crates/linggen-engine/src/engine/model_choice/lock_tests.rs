//! The lock: [`super::resolve`] decides a turn's model and
//! [`AgentEngine::run_on`] writes it. A new writer of an engine's model id,
//! or a new check of whether a model can run, fails here until it is looked
//! at and listed.

use std::path::{Path, PathBuf};

/// Writers of an engine's model id outside this module, by file: how many,
/// and why.
const WRITERS: &[(&str, usize, &str)] = &[
    (
        "crates/linggen-engine/src/engine/streaming.rs",
        1,
        "a model that fails during the call moves to the next one down, told by ModelFallback",
    ),
    ("crates/linggen-engine/src/engine/context.rs", 2, "a test"),
    (
        "crates/linggen-engine/src/engine/session_spec/tests.rs",
        6,
        "tests",
    ),
];

/// Checks of whether a model can run, outside this module and the model
/// registry itself. None is listed: `live`, `is_here` and `next_down` are
/// the checks.
const CHECKS: &[&str] = &["resolve_id(", "has_model("];

/// Callers of the availability checks outside this module, by file: how
/// many, and why. A new ladder built on them, instead of on `resolve`,
/// fails here until it is looked at and listed.
const CHECK_CALLERS: &[(&str, usize, &str)] = &[
    (
        "crates/linggen-engine/src/engine/agent/mod.rs",
        3,
        "the agent's priority chain: how a standing model is found",
    ),
    (
        "crates/linggen-engine/src/engine/streaming.rs",
        2,
        "the stream refuses a guest's unshared model and a missing one with switching off",
    ),
    (
        "src/server/mcp_agent.rs",
        2,
        "agent_run refuses an unknown model; the model list marks the default",
    ),
    (
        "src/server/old_claude.rs",
        1,
        "continuing an old Claude session keeps its pinned model when it is here",
    ),
];

const CHECK_CALLS: &[&str] = &["model_choice::live(", "model_choice::is_here("];

fn check_calls(text: &str) -> usize {
    CHECK_CALLS.iter().map(|c| text.matches(c).count()).sum()
}

/// Assignments to `.model_id`: an `=` that is not `==`.
fn writes(text: &str) -> usize {
    text.match_indices(".model_id")
        .filter(|(at, field)| {
            let rest = &text[at + field.len()..];
            let rest = rest.trim_start();
            rest.starts_with('=') && !rest.starts_with("==")
        })
        .count()
}

fn checks(text: &str) -> usize {
    CHECKS.iter().map(|c| text.matches(c).count()).sum()
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

/// Every Rust file of the binary and the engine, by its path from the root,
/// leaving out this module and the model registry.
fn scanned() -> Vec<(String, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    for dir in ["src", "crates/linggen-engine/src"] {
        rust_files(&root.join(dir), &mut files);
    }
    files
        .into_iter()
        .map(|f| {
            let rel = f.strip_prefix(&root).unwrap().to_string_lossy().to_string();
            (rel, std::fs::read_to_string(&f).unwrap())
        })
        .filter(|(rel, _)| {
            !rel.starts_with("crates/linggen-engine/src/engine/model_choice/")
                && !rel.starts_with("crates/linggen-engine/src/provider/models/")
        })
        .collect()
}

#[test]
fn one_place_writes_a_turns_model() {
    for (rel, text) in scanned() {
        let want = WRITERS
            .iter()
            .find(|(f, _, _)| *f == rel)
            .map_or(0, |e| e.1);
        assert_eq!(
            writes(&text),
            want,
            "{rel} writes an engine's model id: go through model_choice::resolve and run_on, or list it"
        );
    }
}

#[test]
fn one_place_decides_whether_a_model_can_run() {
    for (rel, text) in scanned() {
        assert_eq!(
            checks(&text),
            0,
            "{rel} checks a model's availability: use model_choice::live, is_here or next_down"
        );
    }
}

#[test]
fn no_second_ladder_is_built_on_the_checks() {
    for (rel, text) in scanned() {
        let want = CHECK_CALLERS
            .iter()
            .find(|(f, _, _)| *f == rel)
            .map_or(0, |e| e.1);
        assert_eq!(
            check_calls(&text),
            want,
            "{rel} calls model_choice::live or is_here: decide through model_choice::resolve, or list it"
        );
    }
}

#[test]
fn the_scan_sees_a_write_and_a_check() {
    assert_eq!(
        writes("e.model_id = x; e.model_id == y; e.default_model_id = z;"),
        1
    );
    assert_eq!(checks("m.resolve_id(a); m.has_model(b)"), 2);
}

/// Reads of `name` as a field or a call: the text `name` where it is not a
/// declaration (`name:`), an initialiser, or an assignment (`name =`), and
/// not the start of a longer name.
fn reads(text: &str, name: &str) -> usize {
    text.match_indices(name)
        .filter(|(at, _)| {
            let after = &text[at + name.len()..];
            let ident = |c: char| c.is_alphanumeric() || c == '_';
            let longer = name.ends_with(ident) && after.starts_with(ident);
            let rest = after.trim_start();
            let written = rest.starts_with(':') || (rest.starts_with('=') && !rest.starts_with("=="));
            !longer && !written
        })
        .count()
}

/// The two switches that say whether a turn may move to another model, and
/// who may read each outside `model_choice`: how many reads, and why.
/// `AgentEngine::switching` is the one that reads them both; a door or the
/// mid-call fallback asks it.
const SWITCH_READS: &[(&str, &[(&str, usize, &str)])] = &[
    (
        // The session's switch, by the store: `model_fallback_allowed(` is the
        // store's reader of a stored session.
        ".model_fallback_allowed(",
        &[(
            "crates/linggen-engine/src/state_fs/sessions.rs",
            6,
            "the store's own test of it",
        )],
    ),
    (
        // The session's switch, on a meta.
        ".allows_model_fallback(",
        &[
            (
                "crates/linggen-engine/src/state_fs/sessions.rs",
                1,
                "the store's reader of a stored session",
            ),
            (
                "src/server/mcp_agent.rs",
                2,
                "a test of a delegated run's new session",
            ),
        ],
    ),
    (
        // The config's switch, as the engine holds it.
        ".allow_model_fallback",
        &[(
            "crates/linggen-engine/src/engine/types.rs",
            1,
            "a test of a new engine's default",
        )],
    ),
];

#[test]
fn the_two_switches_are_each_read_in_one_place() {
    for (needle, listed) in SWITCH_READS {
        for (rel, text) in scanned() {
            let want = listed
                .iter()
                .find(|(f, _, _)| *f == rel)
                .map_or(0, |e| e.1);
            assert_eq!(
                reads(&text, needle),
                want,
                "{rel} reads `{needle}`: ask AgentEngine::switching (model_choice::Switching), or list it"
            );
        }
    }
}

#[test]
fn the_switch_scan_sees_a_read_and_not_a_write() {
    assert_eq!(reads("a.allow_model_fallback; b.allow_model_fallback = c", ".allow_model_fallback"), 1);
    assert_eq!(reads("x.model_fallback_allowed(s)", ".model_fallback_allowed("), 1);
    assert_eq!(reads("pub allow_model_fallback: bool", "allow_model_fallback"), 0);
}
