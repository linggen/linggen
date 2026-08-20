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
//! - **which slice of the store this session may see** (`contexts`), when it
//!   is bound to a skill that declares `memory_context`. This one is not a
//!   nicety: without it a focused app like CFO reads and writes the whole
//!   cross-app store.
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

    // Per-skill memory isolation. FORCED, not defaulted: a session bound to a
    // skill that declares `memory_context` only ever sees and writes its own
    // namespace, whatever `contexts` the model passed.
    if declares(&tool.input_schema, "contexts") {
        if let Some(scope) = skill_scope(tools).await {
            set(
                &mut args,
                "contexts",
                Value::Array(vec![Value::String(scope)]),
            );
        }
    }

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

    // Where the work is happening — stamped on the way in (`cwd`) and asked
    // for on the way out (`cwd_scope`), so a row can only be found from the
    // project it was written in, or from a parent of it.
    //
    // The model is never asked for either. It is a fact about the session, not
    // a judgment, and a field the model has to copy by hand is a field that
    // ends up empty — which is the whole store bleeding into every question.
    //
    // Two refusals guard the stamp. A call that names ANOTHER session's row
    // (`source_session` ≠ this session — checked after the fill above, which
    // only ever inserts our own id) is the dream's promote or the scan's
    // backfill carrying the original row's origin: its cwd, when it had one,
    // rides in the same call, and this session's cwd stamped over the gap
    // would rescope someone else's memory to wherever the dream happened to
    // run. And a cwd that is not a project (see [`project_cwd`]) must never
    // become one — the dream mission runs at `~/.linggen`, and a row stamped
    // there is hidden from every project-scoped search.
    let foreign_row = args
        .get("source_session")
        .and_then(Value::as_str)
        .is_some_and(|s| tools.session_id.as_deref() != Some(s));
    if !foreign_row {
        if let Some(project) = project_cwd(tools) {
            for field in ["cwd", "cwd_scope"] {
                if declares(&tool.input_schema, field) && !has(&args, field) {
                    set(&mut args, field, Value::String(project.clone()));
                }
            }
        }
    }

    guard_user_voice(tools, &tool.server, &mut args).await?;
    Ok(args)
}

/// This session's cwd, if it is a place recall scoping should know about.
///
/// `None` for the dirs a session sits in when it is not in any project: the
/// home dir ("no particular work" — a scope there would claim every repo
/// underneath), the engine's own `~/.linggen` (where every mission runs),
/// and temp dirs. Stamping one of those onto a write hides the row from
/// every project search, and scoping a read to one hides every project row
/// from the reader — a scope that is not a project is worse than no scope.
/// Same rule the plugin's stamp-cwd.sh / recall.sh apply for Claude Code.
pub(crate) fn project_cwd(tools: &Tools) -> Option<String> {
    let cwd = tools.cwd();
    is_project_dir(&cwd).then(|| cwd.to_string_lossy().to_string())
}

/// Is this path a project, as far as memory scoping is concerned?
/// Also used by the chat runtime's per-turn auto-recall on the session's
/// own cwd — one predicate, both directions.
pub(crate) fn is_project_dir(cwd: &std::path::Path) -> bool {
    if let Some(home) = dirs::home_dir() {
        if cwd == home || cwd.starts_with(home.join(".linggen")) {
            return false;
        }
    }
    !(cwd.starts_with(std::env::temp_dir())
        || cwd.starts_with("/tmp")
        || cwd.starts_with("/private/tmp"))
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

/// The `memory_context` of the skill this session is bound to, if any.
/// Skills without one (e.g. Pulse) keep full-store access.
async fn skill_scope(tools: &Tools) -> Option<String> {
    let manager = tools.get_manager()?;
    let sid = tools.session_id.clone()?;
    let meta = manager
        .global_sessions
        .get_session_meta(&sid)
        .ok()
        .flatten()?;
    let skill_name = meta.skill?;
    let skill = manager.skills.reload_one(&skill_name).await?;
    skill.memory_context.filter(|c| !c.trim().is_empty())
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
/// `id` rewrites one. A metadata-only edit (tier, contexts, tags) changes
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
        let update = json!({"properties": {"id": {}, "content": {}, "contexts": {}}});
        assert!(!declares(&update, "host"));
        assert!(!declares(&update, "source_session"));
        assert!(declares(&update, "contexts"));

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
    #[test]
    fn a_cwd_that_is_not_a_project_never_becomes_a_scope() {
        for non_project in [
            std::env::temp_dir(),
            dirs::home_dir().unwrap(),
            dirs::home_dir().unwrap().join(".linggen"),
            dirs::home_dir().unwrap().join(".linggen/missions"),
        ] {
            let tools = Tools::new(non_project.clone()).unwrap();
            assert!(
                project_cwd(&tools).is_none(),
                "{non_project:?} is not a project"
            );
        }

        // A real repo checkout is one.
        let tools = Tools::new(std::env::current_dir().unwrap()).unwrap();
        assert!(project_cwd(&tools).is_some());
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
