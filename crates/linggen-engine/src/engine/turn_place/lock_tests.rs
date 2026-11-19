//! The lock: a turn's settings are set when the turn starts and put back
//! when it ends, in one place (`turn_place::turn`), and the agent loop is
//! started from the listed places only. A new writer of a turn's setting,
//! or a new caller of the loop, fails here until it is looked at and
//! listed. The listed counts only ever fall.

use std::path::{Path, PathBuf};

/// The engine's fields a turn sets for itself, and what it leaves for its
/// caller to read (its run, its last words, what it spent).
const TURN_FIELDS: &[&str] = &[
    "task",
    "observations",
    "pending_images",
    "silence_ok",
    "thinking_tx",
    "inbox",
    "plan_mode",
    "turn_language",
    "turn_fault",
    "turn_tool_allowlist",
    "withheld_tools",
    "suggest_followups",
    "kickoff_queue",
    "kickoff_stop",
    "kickoff_fresh",
    "kickoff_then",
    "last_run_id",
    "last_assistant_text",
    "session_lead",
    "session_tools",
    "run_usage",
    "run_faults",
];

/// The turn's fields only the engine may touch: the binary cannot write
/// them at all. What a turn leaves for its caller (`last_run_id`,
/// `last_assistant_text`) stays readable; the scan above holds its writers.
const PRIVATE: &[&str] = &[
    "task",
    "observations",
    "pending_images",
    "silence_ok",
    "thinking_tx",
    "inbox",
    "plan_mode",
    "turn_language",
    "turn_fault",
    "turn_tool_allowlist",
    "withheld_tools",
    "suggest_followups",
    "kickoff_queue",
    "kickoff_stop",
    "kickoff_fresh",
    "kickoff_then",
    "session_lead",
    "session_tools",
    "run_usage",
    "run_faults",
];

/// Calls that change a field in place.
const MUTATORS: &[&str] = &[
    "clear()",
    "extend(",
    "insert(",
    "push(",
    "push_back(",
    "pop_front(",
    "take()",
    "retain(",
    "remove(",
    "drain(",
    "truncate(",
];

/// The writers of a turn's settings outside `turn_place/`, by file: how
/// many, and why.
const LISTED: &[(&str, usize, &str)] = &[
    // The loop itself: what it observes, its last words, the kickoff it drains.
    (
        "crates/linggen-engine/src/engine/mod.rs",
        22,
        "the loop drains a mission's kickoff and keeps its last words; tests",
    ),
    (
        "crates/linggen-engine/src/engine/context.rs",
        2,
        "the loop's observations",
    ),
    (
        "crates/linggen-engine/src/engine/plan.rs",
        1,
        "a plan leaves plan mode",
    ),
    (
        "crates/linggen-engine/src/engine/light_turn.rs",
        1,
        "a light turn streams nothing",
    ),
    (
        "crates/linggen-engine/src/engine/prompt/mod.rs",
        1,
        "the first message takes the pending images",
    ),
    (
        "crates/linggen-engine/src/engine/types.rs",
        2,
        "a delegated child's task",
    ),
    (
        "crates/linggen-engine/src/engine/tools/delegation.rs",
        1,
        "a delegated child's task (its own fresh engine)",
    ),
    // Bypasses: a turn's settings written by its caller.
    // Not the engine's: another struct's field of the same name.
    (
        "crates/linggen-engine/src/engine/background.rs",
        1,
        "a background task's entry",
    ),
    (
        "crates/linggen-engine/src/engine/dispatch.rs",
        1,
        "a Task call's arguments",
    ),
    (
        "src/server/api/sessions.rs",
        1,
        "SessionMeta::withheld_tools",
    ),
    (
        "src/server/day_session.rs",
        1,
        "SessionMeta::withheld_tools",
    ),
    // Tests.
    (
        "crates/linggen-engine/src/engine/suggestion/mod.rs",
        4,
        "tests",
    ),
    (
        "crates/linggen-engine/src/engine/prompt/notes.rs",
        2,
        "tests",
    ),
    (
        "crates/linggen-engine/src/engine/prompt/persona_tests.rs",
        6,
        "tests",
    ),
    (
        "crates/linggen-engine/src/engine/prompt/world/tests.rs",
        1,
        "a test",
    ),
    (
        "crates/linggen-engine/src/engine/session_spec/tests.rs",
        1,
        "a test (SessionMeta)",
    ),
    (
        "crates/linggen-engine/src/engine/steer_loop_tests.rs",
        1,
        "a test",
    ),
];

/// Callers of the agent loop (`run_agent_loop`, and the chat's wrapper of
/// it, `run_loop_with_tracking`), by file: how many, and why. Each runs
/// inside a turn begun by `begin_turn`.
const LOOP_CALLERS: &[(&str, usize, &str)] = &[
    (
        "src/server/chat/cloud_gate.rs",
        2,
        "every chat run, inside the chat's wrapper",
    ),
    ("src/server/chat/runtime.rs", 1, "the chat's wrapper itself"),
    (
        "src/server/chat/skill_dispatch.rs",
        2,
        "a slash command's and a trigger's run, in run_session_turn",
    ),
    (
        "src/server/chat/structured.rs",
        1,
        "a reply's run, in run_session_turn",
    ),
    (
        "src/server/chat/plan_flow.rs",
        2,
        "a plan's run in run_session_turn; an approved plan's, in its own turn",
    ),
    (
        "crates/linggen-engine/src/extensions/missions/scheduler.rs",
        1,
        "a mission run",
    ),
    ("src/server/mcp_agent.rs", 1, "an agent_run"),
    (
        "crates/linggen-engine/src/engine/tools/delegation.rs",
        1,
        "a Task child, on a fresh engine dropped after it",
    ),
    (
        "crates/linggen-engine/src/engine/steer_loop_tests.rs",
        1,
        "a test",
    ),
    (
        "crates/linggen-engine/src/engine/cut_loop_tests.rs",
        1,
        "a test",
    ),
];

fn is_ident(c: char) -> bool {
    c == '_' || c.is_ascii_alphanumeric()
}

/// Whether the match of `.field` at `at` is borrowed mutably: `&mut x.field`.
fn borrowed_mut(text: &str, at: usize) -> bool {
    let head = text[..at].trim_end_matches(|c: char| is_ident(c) || c == '.');
    head.ends_with("&mut ")
}

/// Whether what follows a field (past any `.sub` fields) writes it.
fn writes_after(rest: &str) -> bool {
    let mut rest = rest;
    loop {
        let trimmed = rest.trim_start();
        if let Some(op) = trimmed.strip_prefix('=') {
            return !op.starts_with('=');
        }
        let Some(after_dot) = trimmed.strip_prefix('.') else {
            return false;
        };
        if MUTATORS.iter().any(|m| after_dot.starts_with(m)) {
            return true;
        }
        let ident_end = after_dot
            .find(|c: char| !is_ident(c))
            .unwrap_or(after_dot.len());
        if ident_end == 0 {
            return false;
        }
        rest = &after_dot[ident_end..];
    }
}

/// Writes of `.field` in `text`: an assignment (not `==`), a call that
/// changes it in place, or a `&mut` borrow of it.
fn writes(text: &str, field: &str) -> usize {
    let needle = format!(".{field}");
    text.match_indices(&needle)
        .filter(|(at, _)| {
            let rest = &text[at + needle.len()..];
            if rest.starts_with(is_ident) {
                return false;
            }
            writes_after(rest) || borrowed_mut(text, *at)
        })
        .count()
}

fn turn_writes(text: &str) -> usize {
    TURN_FIELDS.iter().map(|f| writes(text, f)).sum()
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
/// with its text; `turn_place/` itself left out.
fn sources() -> Vec<(String, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    for dir in ["src", "crates/linggen-engine/src"] {
        rust_files(&root.join(dir), &mut files);
    }
    files
        .into_iter()
        .map(|file| {
            let rel = file
                .strip_prefix(&root)
                .unwrap()
                .to_string_lossy()
                .to_string();
            let text = std::fs::read_to_string(&file).unwrap();
            (rel, text)
        })
        .filter(|(rel, _)| !rel.starts_with("crates/linggen-engine/src/engine/turn_place/"))
        .collect()
}

/// Each file whose count is not the listed one, as `(file, found, listed)`;
/// and each listed file that no longer exists.
fn mismatches(
    count: impl Fn(&str) -> usize,
    listed: &[(&str, usize, &str)],
) -> Vec<(String, usize, usize)> {
    let sources = sources();
    let mut wrong: Vec<(String, usize, usize)> = sources
        .iter()
        .filter_map(|(rel, text)| {
            let found = count(text);
            let want = listed.iter().find(|(f, _, _)| *f == rel).map_or(0, |e| e.1);
            (found != want).then(|| (rel.clone(), found, want))
        })
        .collect();
    for (file, want, _) in listed {
        if !sources.iter().any(|(rel, _)| rel == file) {
            wrong.push((file.to_string(), 0, *want));
        }
    }
    wrong.sort();
    wrong
}

#[test]
fn a_turns_settings_are_written_in_one_place() {
    let wrong = mismatches(turn_writes, LISTED);
    assert!(
        wrong.is_empty(),
        "a turn's settings written outside turn_place::turn — begin the turn \
         with them, or list the file (file, found, listed):\n{wrong:#?}"
    );
}

#[test]
fn the_agent_loop_is_started_from_the_listed_places() {
    let calls = |text: &str| {
        text.matches(".run_agent_loop(").count() + text.matches("run_loop_with_tracking(").count()
    };
    let wrong = mismatches(calls, LOOP_CALLERS);
    assert!(
        wrong.is_empty(),
        "a new caller of the agent loop — start a turn through run_session_turn \
         or begin_turn, or list it (file, found, listed):\n{wrong:#?}"
    );
}

#[test]
fn a_turns_fields_are_private_to_the_engine() {
    let types = include_str!("../types.rs");
    let open: Vec<&str> = PRIVATE
        .iter()
        .copied()
        .filter(|f| !types.contains(&format!("pub(crate) {f}:")))
        .collect();
    assert!(
        open.is_empty(),
        "a turn's field the binary may write: {open:?}"
    );
}

#[test]
fn a_write_is_told_from_a_read() {
    assert_eq!(writes("engine.task = Some(t);", "task"), 1);
    assert_eq!(writes("if engine.task == None {}", "task"), 0);
    assert_eq!(writes("let t = engine.task.clone();", "task"), 0);
    assert_eq!(writes("engine.task_bootstrap = true;", "task"), 0);
    assert_eq!(writes("engine.observations.clear();", "observations"), 1);
    assert_eq!(
        writes("engine.withheld_tools.extend(x);", "withheld_tools"),
        1
    );
    assert_eq!(
        writes(
            "let u = std::mem::take(&mut engine.run_usage);",
            "run_usage"
        ),
        1
    );
    assert_eq!(writes("engine.kickoff_queue.len()", "kickoff_queue"), 0);
    assert_eq!(writes("let x = engine.last_call.take();", "last_run_id"), 0);
    assert_eq!(writes("e.plan.as_ref().map(|p| p.x = 1)", "plan_mode"), 0);
}
