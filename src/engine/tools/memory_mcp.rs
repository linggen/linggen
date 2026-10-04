//! What the engine adds to a memory call a session makes.
//!
//! Memory is an MCP server now, so a model's call goes out over the wire much
//! as it wrote it. Four things about that call are the *host's* to know, and
//! none of them survive a raw pass-through:
//!
//! - **which host is speaking** (`host`) — the caller naming itself, which is
//!   why this belongs on the client and was deliberately not ported into
//!   ling-mem: a daemon that also serves Claude Code and Cursor would
//!   mislabel their rows.
//! - **which session authored a row** (`source_session`) — what makes the
//!   scan pass's skip-by-session idempotency real.
//! - **where the session stands** (`cwd`, `root`, `cwd_scope`) — the paths
//!   the daemon turns into a row's scope and a search's recall scope
//!   (`linggen-memory/doc/scope-index-spec.md`). A session bound to a skill
//!   stands in that skill's dir (`~/.linggen/skills/<name>`), which is what
//!   keeps a focused app like CFO to its own rows: app isolation is keyed by
//!   path now, not by a forced `contexts` tag.
//! - **whether a write may overwrite the user's own words.** The daemon
//!   enforces the same floor for every frontend, but it cannot see the one
//!   case the merge law prescribes — that the user just answered an AskUser.
//!
//! This is argument shaping on the client side, not a second memory tool.
//! Nothing here republishes ling-mem's surface: that was the proxy idea the
//! MCP arc considered and dropped, because the plugin ships both servers and
//! two identical `memory_search` tools leave the model picking arbitrarily
//! (`doc/mcp-client-spec.md`).
//!
//! **Which fields get filled is read from the server's own advertised
//! schema**, never from a list of tool names kept here. `memory_add` declares
//! `host` and `source_session`; `memory_update` declares neither, because an
//! edit does not re-author a row. Both facts live in ling-mem, where they
//! belong, and this file follows them.

use super::Tools;
use crate::mcp_client::{qualify, registry, AdvertisedTool, BUILTIN_MEMORY};
use anyhow::Result;
use serde_json::Value;
use std::time::Duration;

/// How long an answered AskUser unlocks user-voice writes. Generous for the
/// ask→resolve flow (always seconds apart), tight enough that a stale ask
/// from earlier work doesn't legitimize an unrelated replace.
const ASK_USER_UNLOCK_WINDOW: Duration = Duration::from_secs(600);

/// The host name stamped on rows this engine writes. Not configurable: it is
/// this program saying which program it is.
const HOST: &str = "linggen";

/// Is this qualified name served by the memory server?
///
/// By the name the server is listed under, not by the tool's own name — a
/// third-party server that happens to advertise a `host` field must never
/// have this engine's session state filled into it. A user entry named
/// `memory` wins over the built-in one (see `mcp_client::config`), and it is
/// still the memory server, so it is still scoped.
/// The advertised tool, if this is one of the memory server's.
///
/// Returns the row rather than a bool because every caller wants it: asking
/// twice meant two lock-and-scan passes and two deep clones of a schema, per
/// memory call the model makes.
fn memory_tool(qualified: &str) -> Option<AdvertisedTool> {
    registry()
        .advertised_tool(qualified)
        .filter(|t| t.server == BUILTIN_MEMORY)
}

/// Fill in what the model could not know, and refuse what it may not do.
///
/// Returns the arguments to put on the wire. Non-memory tools pass through
/// untouched — this is the only place the engine treats one server specially,
/// and it does so for the session state it alone holds.
pub(crate) async fn augment(tools: &Tools, qualified: &str, mut args: Value) -> Result<Value> {
    let Some(tool) = memory_tool(qualified) else {
        return Ok(args);
    };

    // The writing session, so a later scan knows this content is already
    // stored. Only when the model didn't supply one — the dream's promote path
    // carries the ORIGINAL row's session forward, and that must win.
    if declares(&tool.input_schema, "source_session") && !has(&args, "source_session") {
        if let Some(sid) = tools.session_id.clone() {
            set(&mut args, "source_session", Value::String(sid));
        }
    }

    // The caller naming itself.
    if declares(&tool.input_schema, "host") && !has(&args, "host") {
        set(&mut args, "host", Value::String(HOST.to_string()));
    }

    // Where the work is happening: the session cwd (`cwd`, a row's default
    // scope), its root (`root`, what a model's `scope` resolves against) and,
    // on a search, the recall scope (`cwd_scope` = root). The daemon owns the
    // rules — which dirs can be a scope, what a root sees — so this only
    // hands it the paths.
    //
    // The model is never asked for these. They are facts about the session,
    // not judgments, and a field the model has to copy by hand is a field that
    // ends up empty — which is the whole store bleeding into every question.
    //
    // Two refusals guard the stamp. A call that names ANOTHER session's row
    // (`source_session` ≠ this session — checked after the fill above, which
    // only ever inserts our own id) is the dream's promote or the scan's
    // backfill carrying the original row's origin: its cwd, when it had one,
    // rides in the same call, and this session's paths stamped over the gap
    // would rescope someone else's memory to wherever the dream happened to
    // run. And a session that stands in no project (see [`memory_place`]) is
    // stamped nothing — the dream mission runs at `~/.linggen`, and its own
    // searches must see the whole store.
    let foreign_row = args
        .get("source_session")
        .and_then(Value::as_str)
        .is_some_and(|s| tools.session_id.as_deref() != Some(s));
    if !foreign_row {
        if let Some(place) = memory_place(tools).await {
            let stamps = [
                ("cwd", &place.cwd),
                ("root", &place.root),
                ("cwd_scope", &place.root),
            ];
            for (field, value) in stamps {
                if declares(&tool.input_schema, field) && wants_project(&args, field) {
                    set(&mut args, field, Value::String(value.clone()));
                }
            }
        }
    }

    guard_user_voice(tools, &tool.server, &mut args).await?;
    Ok(args)
}

/// Where a session's memory stands: its cwd (a row's default scope) and its
/// root (git root, a skill's dir, or the cwd itself).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MemoryPlace {
    pub cwd: String,
    pub root: String,
}

/// This session's place: a skill-bound session stands in its skill's dir
/// (`~/.linggen/skills/<name>`) wherever its process cwd is; any other
/// session at its cwd when that is a project. `None` when it stands in no
/// project (`$HOME`, `~/.linggen`, temp).
pub(crate) async fn memory_place(tools: &Tools) -> Option<MemoryPlace> {
    if let Some(dir) = bound_skill_dir(tools).await {
        return place_of(&dir);
    }
    place_of(&tools.cwd())
}

/// The place for a cwd, if it is a project.
pub(crate) fn place_of(cwd: &std::path::Path) -> Option<MemoryPlace> {
    let root = memory_root(cwd)?;
    Some(MemoryPlace {
        cwd: cwd.to_string_lossy().to_string(),
        root: root.to_string_lossy().to_string(),
    })
}

/// A project's root for memory: a skill's own dir for anything inside it,
/// else the git root, else the dir itself. `None` when `cwd` is no project.
pub(crate) fn memory_root(cwd: &std::path::Path) -> Option<std::path::PathBuf> {
    if !is_project_dir(cwd) {
        return None;
    }
    if let Some(skill) = skill_dir_of(cwd) {
        return Some(skill);
    }
    Some(super::search_exec::find_git_root(cwd).unwrap_or_else(|| cwd.to_path_buf()))
}

/// `~/.linggen/skills/<name>` when `p` is that dir or inside it.
fn skill_dir_of(p: &std::path::Path) -> Option<std::path::PathBuf> {
    let skills = crate::paths::global_skills_dir();
    let first = p.strip_prefix(&skills).ok()?.components().next()?;
    Some(skills.join(first.as_os_str()))
}

/// Is this path a project, as far as memory scoping is concerned? Not the
/// home dir ("no particular work" — a scope there would claim every repo
/// underneath), not the engine's own `~/.linggen` (where every mission runs),
/// not a temp dir. A skill's own dir (`~/.linggen/skills/<name>`) is one: an
/// app's rows live there. Same rule the daemon applies
/// (`memory::scope::is_scope_dir`) and the plugin's hooks rely on.
pub(crate) fn is_project_dir(cwd: &std::path::Path) -> bool {
    if skill_dir_of(cwd).is_some() {
        return true;
    }
    if let Some(home) = dirs::home_dir() {
        if cwd == home || cwd.starts_with(home.join(".linggen")) {
            return false;
        }
    }
    !(cwd.starts_with(std::env::temp_dir())
        || cwd.starts_with("/tmp")
        || cwd.starts_with("/private/tmp")
        || cwd.starts_with("/private/var/folders"))
}

/// Does this call get the session's project stamped into `field`? Never over
/// a value the caller set, and never `cwd` on a row the model marked
/// `global: true` — a row about the person, not this project (the daemon
/// drops any cwd sent with it anyway).
fn wants_project(args: &Value, field: &str) -> bool {
    if has(args, field) {
        return false;
    }
    !(field == "cwd" && args.get("global").and_then(Value::as_bool) == Some(true))
}

/// Does the server accept this field for this tool?
fn declares(schema: &Value, field: &str) -> bool {
    schema
        .get("properties")
        .and_then(Value::as_object)
        .is_some_and(|props| props.contains_key(field))
}

fn has(args: &Value, field: &str) -> bool {
    args.get(field).is_some_and(|v| !v.is_null())
}

fn set(args: &mut Value, field: &str, value: Value) {
    if let Some(obj) = args.as_object_mut() {
        obj.insert(field.to_string(), value);
    }
}

/// The dir of the skill this session is bound to, when that skill uses
/// memory (declares `memory-context`). Skills that don't (e.g. Pulse) stand
/// at their session cwd like any other session.
async fn bound_skill_dir(tools: &Tools) -> Option<std::path::PathBuf> {
    let manager = tools.get_manager()?;
    let sid = tools.session_id.clone()?;
    let meta = manager
        .global_sessions
        .get_session_meta(&sid)
        .ok()
        .flatten()?;
    let skill_name = meta.skill?;
    let skill = manager.skills.reload_one(&skill_name).await?;
    skill
        .memory_context
        .filter(|c| !c.trim().is_empty())
        .map(|_| crate::paths::global_skills_dir().join(&skill_name))
}

/// Mechanical enforcement of the merge law's user-voice floor: a write that
/// replaces or rewrites `from=user` rows goes through only when (a) the model
/// asserts `user_directed` — the user's current message commanded the change —
/// or (b) an AskUser was answered within [`ASK_USER_UNLOCK_WINDOW`].
/// Everything else is the silent drifted overwrite the protocol forbids.
/// Derived rows (the agent's own notes) pass free.
///
/// The daemon enforces the same floor for every frontend and honours a
/// wire-level `user_directed`. The engine is the authority on its own path:
/// the model's raw assertion is stripped, and the flag is re-asserted on the
/// wire only when this guard authorizes the write — including the AskUser
/// case the daemon cannot see.
///
/// Which writes are guarded is read off the **shape of the request**, not the
/// tool's name: `replace_ids` retires rows, and new `content` on an existing
/// `id` rewrites one. A metadata-only edit (tier, hook, indexed) changes
/// nothing the row says and stays unguarded.
async fn guard_user_voice(tools: &Tools, server: &str, args: &mut Value) -> Result<()> {
    let Some(obj) = args.as_object_mut() else {
        return Ok(());
    };
    // Strip the model's raw assertion — the engine decides what goes on the
    // wire, not the model.
    let asserted = obj
        .remove("user_directed")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let mut targets: Vec<String> = obj
        .get("replace_ids")
        .and_then(Value::as_array)
        .map(|ids| {
            ids.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    if obj.contains_key("content") {
        if let Some(id) = obj.get("id").and_then(Value::as_str) {
            targets.push(id.to_string());
        }
    }
    if targets.is_empty() {
        return Ok(());
    }
    if asserted || tools.ask_user_recently(ASK_USER_UNLOCK_WINDOW) {
        // Authorized — assert the flag on the wire so the daemon's own floor
        // lets the write through.
        obj.insert("user_directed".to_string(), Value::Bool(true));
        return Ok(());
    }

    // Resolve each target's voice. A fetch miss is not the guard's problem —
    // the daemon reports a missing id on the real call.
    let get = qualify(server, "memory_get");
    let mut offending: Vec<String> = Vec::new();
    for id in &targets {
        let Ok(text) = registry().call(&get, serde_json::json!({"id": id})).await else {
            continue;
        };
        let Ok(row) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        if row.get("from").and_then(Value::as_str) != Some("user") {
            continue;
        }
        let gist: String = row
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or("")
            .chars()
            .take(60)
            .collect();
        offending.push(format!("{id} \"{gist}\""));
    }
    if offending.is_empty() {
        return Ok(());
    }

    anyhow::bail!(
        "BLOCKED by the user-voice merge guard: this write replaces/rewrites rows in the USER'S VOICE (from=user): {}. The user's voice changes only with the user (merge law). Recovery — pick ONE: (1) if the user's CURRENT message states this change as SETTLED — a command (\"update X to Y\", \"forget X\"), a declaration (\"my X is now Y\"), or a commitment (\"from now on, X\") — retry the exact same call with \"user_directed\": true; a hedged reflection (\"X feels about right to me\") does NOT qualify; (2) otherwise call AskUser to let the user choose, then retry after their answer (an answered AskUser unlocks this guard). Do NOT work around it by writing a new row without replace_ids — that creates the drift this guard exists to prevent.",
        offending.join(", ")
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The rule that keeps this file from carrying a list of tool names: a
    /// field is filled only where the server said it takes one.
    #[test]
    fn a_field_the_server_never_declared_is_not_invented() {
        let add = json!({"properties": {"content": {}, "host": {}, "source_session": {}}});
        assert!(declares(&add, "host"));
        assert!(declares(&add, "source_session"));

        // memory_update declares neither — an edit does not re-author a row.
        let update = json!({"properties": {"id": {}, "content": {}, "hook": {}}});
        assert!(!declares(&update, "host"));
        assert!(!declares(&update, "source_session"));
        assert!(declares(&update, "hook"));

        // A tool with no properties at all, and a malformed schema, both say no
        // rather than panicking — a third-party server's schema is not ours.
        assert!(!declares(&json!({"type": "object"}), "host"));
        assert!(!declares(&json!("nonsense"), "host"));
    }

    /// An explicit value the model passed is never overwritten by a default —
    /// the dream's promote path carries the original row's session forward.
    #[test]
    fn an_explicit_value_wins_over_the_default() {
        assert!(has(&json!({"source_session": "abc"}), "source_session"));
        // Null is absence, not a value: models fill optional fields with it.
        assert!(!has(&json!({"source_session": null}), "source_session"));
        assert!(!has(&json!({}), "source_session"));
    }

    /// A row the model marked global is about the person: no project stamp,
    /// though a search from the same session is still scoped.
    #[test]
    fn a_global_row_gets_no_project_stamp() {
        assert!(wants_project(&json!({"content": "x"}), "cwd"));
        assert!(!wants_project(
            &json!({"content": "x", "global": true}),
            "cwd"
        ));
        assert!(wants_project(
            &json!({"content": "x", "global": false}),
            "cwd"
        ));
        assert!(!wants_project(&json!({"cwd": "/elsewhere"}), "cwd"));
        assert!(wants_project(
            &json!({"query": "q", "global": true}),
            "cwd_scope"
        ));
    }

    /// Guarded writes are recognised by shape, so a renamed or added tool
    /// cannot slip past by not being on a list.
    #[tokio::test]
    async fn a_metadata_only_edit_is_not_a_rewrite() {
        let tools = Tools::new(std::env::temp_dir()).unwrap();

        // Retiring rows and rewriting content are both guarded…
        let mut retire = json!({"content": "x", "replace_ids": ["a"]});
        assert!(guard_user_voice(&tools, "memory", &mut retire)
            .await
            .is_ok());
        let mut rewrite = json!({"id": "a", "content": "new words"});
        assert!(guard_user_voice(&tools, "memory", &mut rewrite)
            .await
            .is_ok());

        // …while moving a row's tier changes nothing it says. The guard must
        // not even reach for the store here, which is why this passes with no
        // server connected.
        let mut retier = json!({"id": "a", "tier": "core", "user_directed": true});
        guard_user_voice(&tools, "memory", &mut retier)
            .await
            .unwrap();
        assert!(
            retier.get("user_directed").is_none(),
            "an unguarded write must not carry the flag onto the wire"
        );
    }

    /// The dirs a session sits in when it is not in any project must never
    /// become a scope — a row stamped with one is hidden from every
    /// project-scoped search (the dream mission runs at `~/.linggen`).
    #[tokio::test]
    async fn a_cwd_that_is_not_a_project_never_becomes_a_scope() {
        for non_project in [
            std::env::temp_dir(),
            dirs::home_dir().unwrap(),
            dirs::home_dir().unwrap().join(".linggen"),
            dirs::home_dir().unwrap().join(".linggen/missions"),
            crate::paths::global_skills_dir(),
        ] {
            assert!(
                place_of(&non_project).is_none(),
                "{non_project:?} is not a project"
            );
        }

        // A real repo checkout is one, rooted at its git root.
        let here = std::env::current_dir().unwrap();
        let tools = Tools::new(here.join("src")).unwrap();
        let place = memory_place(&tools).await.unwrap();
        assert!(place.cwd.ends_with("/src"));
        assert!(here.starts_with(&place.root));

        // A skill's own dir is one, and is its own root.
        let cfo = crate::paths::global_skills_dir().join("cfo");
        let place = place_of(&cfo.join("data")).unwrap();
        assert_eq!(place.root, cfo.to_string_lossy());
    }

    /// The model's own assertion never reaches the daemon unexamined.
    #[tokio::test]
    async fn the_flag_is_stripped_then_re_asserted_only_when_earned() {
        let tools = Tools::new(std::env::temp_dir()).unwrap();

        let mut claimed = json!({"content": "x", "replace_ids": ["a"], "user_directed": true});
        guard_user_voice(&tools, "memory", &mut claimed)
            .await
            .unwrap();
        assert_eq!(claimed["user_directed"], json!(true));

        // Without the assertion the guard has to resolve each target's voice.
        // With no memory server connected every fetch misses, which is not the
        // guard's problem — the daemon reports a missing id on the real call.
        let mut unclaimed = json!({"content": "x", "replace_ids": ["a"]});
        guard_user_voice(&tools, "memory", &mut unclaimed)
            .await
            .unwrap();
        assert!(unclaimed.get("user_directed").is_none());
    }
}
