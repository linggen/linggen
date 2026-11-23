//! The lock: an engine's fields are written by the engine. A session's
//! settings go through the session spec's writer (`SessionSpec::apply`,
//! `AgentEngine::respec`) or the engine's own methods; the thread's cache
//! through the chat's thread module (`server::chat::thread`). A new write
//! of any `AgentEngine` field from outside `engine/` fails here until it is
//! looked at and listed. The listed counts only ever fall.

use std::path::{Path, PathBuf};

/// Calls that change a field in place.
const MUTATORS: &[&str] = &[
    "clear()",
    "extend(",
    "insert(",
    "push(",
    "push_back(",
    "push_str(",
    "pop(",
    "pop_front(",
    "take()",
    "replace(",
    "retain(",
    "remove(",
    "drain(",
    "truncate(",
    "sort",
    "dedup",
    "append(",
    "entry(",
    "get_or_insert",
    "as_mut()",
    "iter_mut()",
    "get_mut(",
    "first_mut()",
    "last_mut()",
];

/// The writers of an engine's field outside `engine/`, by file: how many,
/// and why. Most are another struct's field of the same name.
const LISTED: &[(&str, usize, &str)] = &[
    // A session's settings written by a door, not their owner.
    (
        "src/server/chat/handler.rs",
        5,
        "ChatRequest's session and agent",
    ),
    (
        "crates/linggen-engine/src/extensions/missions/enter.rs",
        1,
        "SessionSpec::tools",
    ),
    (
        "src/server/resident/turn.rs",
        2,
        "tests",
    ),
    // The thread's cache.
    ("src/server/chat/thread.rs", 9, "the thread module itself"),
    (
        "src/server/mcp_agent.rs",
        2,
        "SessionSpec::tools; a run record (a test)",
    ),
    // Not the engine's: another struct's field of the same name.
    (
        "crates/linggen-engine/src/config.rs",
        20,
        "RoutingConfig::default_models (`make_default` and its tests included)",
    ),
    (
        "crates/linggen-engine/src/extensions/missions/mod.rs",
        1,
        "Mission::agent_id",
    ),
    (
        "crates/linggen-engine/src/state_fs/sessions.rs",
        2,
        "SessionMeta's compaction settings",
    ),
    (
        "src/server/api/pair/manage.rs",
        2,
        "RoutingConfig::default_models (tests)",
    ),
    ("src/server/day_session.rs", 1, "SessionMeta::withheld_tools"),
    ("src/server/rtc/page_state.rs", 3, "the page state's fields"),
    ("src/server/rtc/peer/control.rs", 1, "a view's session"),
];

/// The engine's fields, read from its struct.
fn engine_fields() -> Vec<String> {
    let types = include_str!("types.rs");
    let start = types.find("pub struct AgentEngine {").unwrap();
    let body = &types[start..];
    let body = &body[..body.find("\n}\n").unwrap()];
    body.lines()
        .filter_map(|line| {
            let line = line.trim_start();
            let rest = line
                .strip_prefix("pub(crate) ")
                .or_else(|| line.strip_prefix("pub "))
                .unwrap_or(line);
            let (name, _) = rest.split_once(':')?;
            (!name.is_empty() && name.chars().all(is_ident)).then(|| name.to_string())
        })
        .collect()
}

fn is_ident(c: char) -> bool {
    c == '_' || c.is_ascii_alphanumeric()
}

/// Whether the match of `.field` at `at` is borrowed mutably: `&mut x.field`,
/// or bound by `ref mut` (`if let Some(ref mut s) = x.field`).
fn borrowed_mut(text: &str, at: usize) -> bool {
    let head = text[..at].trim_end_matches(|c: char| is_ident(c) || c == '.');
    if head.ends_with("&mut ") {
        return true;
    }
    let line = &text[text[..at].rfind('\n').map_or(0, |n| n + 1)..at];
    line.rsplit_once("ref mut ")
        .is_some_and(|(_, after)| after.contains('='))
}

/// Whether what follows a field (past any `.sub` fields) writes it.
fn writes_after(rest: &str) -> bool {
    let mut rest = rest;
    loop {
        let trimmed = rest.trim_start();
        if let Some(op) = trimmed.strip_prefix('=') {
            return !op.starts_with('=') && !op.starts_with('>');
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
/// changes it in place, or a mutable borrow of it.
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

/// Every Rust file of the binary and the engine crate outside `engine/`,
/// by its path from the root, with its text.
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
        .filter(|(rel, _)| !rel.starts_with("crates/linggen-engine/src/engine/"))
        .collect()
}

#[test]
fn an_engines_fields_are_written_by_the_engine() {
    let fields = engine_fields();
    let sources = sources();
    let mut wrong: Vec<(String, usize, usize)> = sources
        .iter()
        .filter_map(|(rel, text)| {
            let found: usize = fields.iter().map(|f| writes(text, f)).sum();
            let want = LISTED.iter().find(|(f, _, _)| *f == rel).map_or(0, |e| e.1);
            (found != want).then(|| (rel.clone(), found, want))
        })
        .collect();
    for (file, want, _) in LISTED {
        if !sources.iter().any(|(rel, _)| rel == file) {
            wrong.push((file.to_string(), 0, *want));
        }
    }
    wrong.sort();
    assert!(
        wrong.is_empty(),
        "an engine's field written outside the engine — go through the session \
         spec's writer, the thread module or an engine method, or list the file \
         (file, found, listed):\n{wrong:#?}"
    );
}

/// The doors to a session's settings and the thread's cache, called from
/// outside the engine: the spec's and the engine's writers, and the chat's
/// thread module.
const SETTERS: &[&str] = &[
    ".take_up_mission(",
    ".tune_recall(",
    ".set_compaction(",
    ".read_session_grants(",
    ".hold_plan(",
    ".forget_plan(",
    ".drop_planned_plan(",
    ".read_live_state(",
    ".take_last_call(",
    "thread::add(",
    "thread::forget(",
    "thread::reload_next(",
    "thread::trim(",
    "thread::seed_task(",
];

/// Who calls them from outside the engine, by file: how many, and why. A
/// new door that changes a session goes here once it is looked at.
const SETTER_CALLERS: &[(&str, usize, &str)] = &[
    (
        "crates/linggen-engine/src/extensions/missions/enter.rs",
        1,
        "a mission taken up",
    ),
    ("src/server/resident/turn.rs", 2, "the companion's recall; live state"),
    ("src/server/chat/admin.rs", 3, "compaction settings; Clear, /compact"),
    ("src/server/chat/turn_place.rs", 1, "a skill's place reads the grants"),
    ("src/server/chat/skill_dispatch.rs", 1, "a slash command reads the grants"),
    ("src/server/chat/structured.rs", 2, "a stale plan dropped; an approved one held"),
    ("src/server/chat/plan_flow.rs", 2, "a new plan run; an approved plan's run"),
    ("src/server/chat/handler.rs", 2, "live state; a capped thread trimmed"),
    ("src/server/chat/light.rs", 1, "live state"),
    ("src/server/chat/runtime.rs", 4, "a turn's last call; what a turn adds"),
    ("src/server/chat/clear_place.rs", 1, "a cleared place reloads"),
    ("src/server/resident/session_roll.rs", 1, "a fresh thread's last words"),
    ("src/server/mcp_agent.rs", 1, "a delegate's task kept first"),
];

#[test]
fn a_sessions_settings_are_changed_from_the_listed_places() {
    let sources = sources();
    let mut wrong: Vec<(String, usize, usize)> = sources
        .iter()
        .filter_map(|(rel, text)| {
            let found: usize = SETTERS.iter().map(|s| text.matches(s).count()).sum();
            let want = SETTER_CALLERS
                .iter()
                .find(|(f, _, _)| *f == rel)
                .map_or(0, |e| e.1);
            (found != want).then(|| (rel.clone(), found, want))
        })
        .collect();
    for (file, want, _) in SETTER_CALLERS {
        if !sources.iter().any(|(rel, _)| rel == file) {
            wrong.push((file.to_string(), 0, *want));
        }
    }
    wrong.sort();
    assert!(
        wrong.is_empty(),
        "a new caller of a session's setter — look at whether this door should \
         change the session, then list it (file, found, listed):\n{wrong:#?}"
    );
}

/// A session's settings the binary cannot write at all: each is set
/// through the spec's writer or an engine method, and read through one.
/// The thread's cache (`chat_history`, `thread_mark`, `reload_thread`)
/// stays open: its owner, the chat's thread module, is the binary's.
const PRIVATE: &[&str] = &[
    "active_mission",
    "cached_system_prompt",
    "compact_threshold",
    "compact_focus",
    "last_call",
    "plan",
    "session_dir",
    "session_permissions",
];

#[test]
fn a_sessions_settings_are_private_to_the_engine() {
    let types = include_str!("types.rs");
    let open: Vec<&str> = PRIVATE
        .iter()
        .copied()
        .filter(|f| !types.contains(&format!("pub(crate) {f}:")))
        .collect();
    assert!(open.is_empty(), "a session's setting the binary may write: {open:?}");
}

#[test]
fn the_engines_fields_are_read_from_its_struct() {
    let fields = engine_fields();
    for field in ["cfg", "chat_history", "session_permissions", "native_tool_mode"] {
        assert!(fields.iter().any(|f| f == field), "{field} not found");
    }
    assert!(!fields.iter().any(|f| f == "AgentEngine"));
}

#[test]
fn a_write_is_told_from_a_read() {
    assert_eq!(writes("engine.plan = None;", "plan"), 1);
    assert_eq!(writes("if engine.plan == None {}", "plan"), 0);
    assert_eq!(writes("let p = engine.plan.clone();", "plan"), 0);
    assert_eq!(writes("engine.planner = true;", "plan"), 0);
    assert_eq!(writes("engine.cfg.memory_recall_count = 1;", "cfg"), 1);
    assert_eq!(writes("let n = engine.cfg.memory_recall_count;", "cfg"), 0);
    assert_eq!(writes("engine.chat_history.clear();", "chat_history"), 1);
    assert_eq!(writes("engine.chat_history.len()", "chat_history"), 0);
    assert_eq!(writes("trim(&mut engine.chat_history, 3);", "chat_history"), 1);
    assert_eq!(writes("look(&engine.chat_history);", "chat_history"), 0);
    assert_eq!(
        writes(
            "if let Some(ref mut set) = engine.cfg.mission_allowed_tools {",
            "cfg"
        ),
        1
    );
    assert_eq!(
        writes("if let Some(ref p) = engine.plan {", "plan"),
        0
    );
    assert_eq!(writes("engine.plan.as_mut().map(|p| p);", "plan"), 1);
    assert_eq!(writes("engine.plan.as_ref().map(|p| p);", "plan"), 0);
    assert_eq!(writes("let c = engine.last_call.take();", "last_call"), 1);
    assert_eq!(writes("match x { a => engine.plan }", "plan"), 0);
    assert_eq!(writes("engine.cfg.mission_allowed_tools.get_or_insert_with(f);", "cfg"), 1);
}
