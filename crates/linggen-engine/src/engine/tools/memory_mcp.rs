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
//! - **where the session stands** (`cwd`, `root`, `scope_root`) — request-only
//!   paths the daemon turns into a row's stored `scope` and a search's recall
//!   scope
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
//! schema, for the operation the call will run**, never from a list of tool
//! names kept here. A folded tool's schema is the union of its operations'
//! fields, so the call's branch is read off memory's catalogue the way the
//! server resolves it (`Catalog::branch_of`, the implicit `op` included):
//! `memory_search` op=semantic takes `scope_root` and op=get does not;
//! `memory_upkeep` op=session_start takes `cwd` and `root`, no other op
//! does. `memory_add` declares `host` and `source_session`; `memory_update`
//! declares neither, because an edit does not re-author a row. These facts
//! live in memory's catalogue, where they belong, and this file follows
//! them.

use super::Tools;
use crate::mcp_client::{qualify, registry, AdvertisedTool, McpRegistry, BUILTIN_MEMORY};
use ling_mem::catalog::{memory::catalog, Branch, View};
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

/// Whether `qualified` is one of the memory server's tools.
pub(crate) fn is_memory_tool(qualified: &str) -> bool {
    crate::mcp_client::is_mcp_tool(qualified) && memory_tool(qualified).is_some()
}

/// Whether a private app's memory write can affect rows recalled outside it.
/// Missing or unreadable scope information is not proof a write stays here.
pub(super) async fn write_leaves_app(tools: &Tools, app: &str, qualified: &str, args: &Value) -> bool {
    let Some(tool) = memory_tool(qualified) else { return true };
    let Some(branch) = catalog().branch_of(View::Local, &tool.original, args) else { return true };
    let verb = branch.dispatch;
    if widens_memory(verb, tools.session_id.as_deref(), args) {
        return true;
    }
    let app_dir = crate::paths::global_skills_dir().join(app);
    if verb == "add" {
        let Some(place) = memory_place(tools).await else { return true };
        if !Takes::of(&tool, args).stamps("cwd", args) || !scope_inside_app(&place.cwd, &app_dir) {
            return true;
        }
    }
    let ids: Vec<&str> = match verb {
        "update" | "delete" => match args.get("id").and_then(Value::as_str) {
            Some(id) => vec![id],
            None => return true,
        },
        "add" => match args.get("replace_ids") {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::Array(ids)) => {
                let Some(ids) = ids.iter().map(Value::as_str).collect::<Option<Vec<_>>>() else { return true };
                ids
            }
            _ => return true,
        },
        _ => return true,
    };
    let search = qualify(&tool.server, "memory_search");
    for id in ids {
        let mut get = serde_json::json!({"op": "get", "id": id});
        if verb != "add" {
            for field in ["episodic", "account", "all_accounts"] {
                if let Some(value) = args.get(field) {
                    get[field] = value.clone();
                }
            }
        }
        let Ok(text) = registry().call(&search, get).await else { return true };
        let Ok(row) = serde_json::from_str::<Value>(&text) else { return true };
        if row.get("id").and_then(Value::as_str) != Some(id)
            || !matches!(row.get("tier").and_then(Value::as_str), Some("semantic" | "episodic"))
            || !row.get("scope").and_then(Value::as_str).is_some_and(|scope| scope_inside_app(scope, &app_dir))
        {
            return true;
        }
    }
    false
}

pub(super) fn widens_memory(verb: &str, session_id: Option<&str>, args: &Value) -> bool {
    if verb == "delete" {
        return false;
    }
    args.get("tier").and_then(Value::as_str) == Some("core")
        || ["global", "clear_scope", "clear_cwd"].iter().any(|key| args.get(key).and_then(Value::as_bool) == Some(true))
        || ["cwd", "scope", "root", "scope_root", "cwd_scope"].iter().any(|key| has(args, key))
        || (verb == "add" && foreign_source(session_id, args))
}

fn scope_inside_app(scope: &str, app_dir: &std::path::Path) -> bool {
    use std::path::{Component, Path};
    let scope = Path::new(scope);
    // Require both lexical and resolved membership: the former controls
    // recall, and the latter excludes paths pointing outside the app.
    scope.starts_with(app_dir)
        && !scope.components().any(|part| part == Component::ParentDir)
        && app_dir.canonicalize().ok().zip(scope.canonicalize().ok())
            .is_some_and(|(app, scope)| scope.starts_with(app))
}

fn foreign_source(session_id: Option<&str>, args: &Value) -> bool {
    args.get("source_session").and_then(Value::as_str).is_some_and(|sid| session_id != Some(sid))
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
    let takes = Takes::of(&tool, &args);

    // The writing session, so a later scan knows this content is already
    // stored. Only when the model didn't supply one — the dream's promote path
    // carries the ORIGINAL row's session forward, and that must win.
    if takes.stamps("source_session", &args) && !has(&args, "source_session") {
        if let Some(sid) = tools.session_id.clone() {
            set(&mut args, "source_session", Value::String(sid));
        }
    }

    // The caller naming itself.
    if takes.stamps("host", &args) && !has(&args, "host") {
        set(&mut args, "host", Value::String(HOST.to_string()));
    }

    // Where the work is happening: the session cwd (`cwd`, a row's default
    // scope), its root (`root`, what a model's `scope` resolves against) and,
    // on a search, the recall scope (`scope_root` = root; `cwd_scope` on a
    // pre-v2 daemon — only the field the server declares is filled). The
    // daemon owns the rules — which dirs can be a scope, what a root sees —
    // so this only hands it the paths.
    //
    // The model is never asked for these. They are facts about the session,
    // not judgments, and a field the model has to copy by hand is a field that
    // ends up empty — which is the whole store bleeding into every question.
    //
    // Two refusals guard the stamp. A call that names ANOTHER session's row
    // (`source_session` ≠ this session — checked after the fill above, which
    // only ever inserts our own id) is the dream's promote or the scan's
    // backfill carrying the original row's origin: its scope, when it had one,
    // rides in the same call, and this session's paths stamped over the gap
    // would rescope someone else's memory to wherever the dream happened to
    // run. And a session that stands in no project (see [`memory_place`]) is
    // stamped nothing — the dream mission runs at `~/.linggen`, and its own
    // searches must see the whole store.
    let foreign_row = foreign_source(tools.session_id.as_deref(), &args);
    if !foreign_row {
        if let Some(place) = memory_place(tools).await {
            let stamps = [
                ("cwd", &place.cwd),
                ("root", &place.root),
                ("scope_root", &place.root),
                ("cwd_scope", &place.root),
            ];
            for (field, value) in stamps {
                if takes.stamps(field, &args) && wants_project(&args, field) {
                    set(&mut args, field, Value::String(value.clone()));
                }
            }
        }
    }

    attribute_row(tools, &takes, &mut args);
    guard_user_voice(tools, &tool.server, &mut args).await?;
    Ok(args)
}

/// The fields the call that will run takes, as its server advertised them.
enum Takes<'a> {
    /// A folded memory tool: its schema, and the branch the call names.
    Branch(&'a Value, &'static Branch<&'static str>),
    /// A folded memory tool whose call names no branch: the server refuses
    /// it with the form that works, so nothing is filled in.
    Nothing,
    /// A tool memory's catalogue does not fold (an older server's): its own
    /// advertised schema.
    Schema(&'a Value),
}

impl<'a> Takes<'a> {
    fn of(tool: &'a AdvertisedTool, args: &Value) -> Self {
        let cat = catalog();
        if let Some(branch) = cat.branch_of(View::Local, &tool.original, args) {
            return Self::Branch(&tool.input_schema, branch);
        }
        if cat.tools().iter().any(|t| t.name == tool.original) {
            return Self::Nothing;
        }
        Self::Schema(&tool.input_schema)
    }

    /// Whether `field` is one the server shows for this call — never a
    /// field only another operation of the same tool takes, and never a
    /// hidden filter (`from` on a search).
    fn field(&self, field: &str) -> bool {
        match self {
            Self::Branch(schema, branch) => declares(schema, field) && branch.advertises(field),
            Self::Nothing => false,
            Self::Schema(schema) => declares(schema, field),
        }
    }

    /// Whether the engine stamps `field` on this call: for a folded tool,
    /// what the catalogue says the host fills (`Branch::host_fills` — the
    /// same reading the plugin's hook is checked against); for an older
    /// server's tool, any field it declares.
    fn stamps(&self, field: &str, args: &Value) -> bool {
        match self {
            Self::Branch(schema, branch) => {
                declares(schema, field) && branch.host_fills(field, args)
            }
            Self::Nothing => false,
            Self::Schema(schema) => declares(schema, field),
        }
    }
}

/// A delegated brief is not the person's words. File such a claim as a
/// derived row, and leave authorization of changes to user rows to the guard.
fn attribute_row(tools: &Tools, takes: &Takes, args: &mut Value) {
    if !tools.delegated_session || tools.person_spoke {
        return;
    }
    if takes.field("from")
        && args.get("from").and_then(Value::as_str).is_some_and(|s| s.eq_ignore_ascii_case("user"))
    {
        set(args, "from", Value::String("derived".into()));
    }
    if let Some(obj) = args.as_object_mut() {
        obj.remove("user_directed");
    }
}

/// Where a session's memory stands: its cwd (a row's default scope) and its
/// root (git root, a skill's dir, or the cwd itself).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryPlace {
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
pub fn place_of(cwd: &std::path::Path) -> Option<MemoryPlace> {
    let root = memory_root(cwd)?;
    Some(MemoryPlace {
        cwd: cwd.to_string_lossy().to_string(),
        root: root.to_string_lossy().to_string(),
    })
}

/// A project's root for memory: a skill's own dir for anything inside it,
/// else the git root, else the dir itself. `None` when `cwd` is no project.
pub fn memory_root(cwd: &std::path::Path) -> Option<std::path::PathBuf> {
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

/// The dir of the skill the call's place stands in (`Tools::place`), when
/// that skill uses memory (declares `memory-context`). Skills that don't
/// (e.g. Pulse) stand at their session cwd like any other session.
async fn bound_skill_dir(tools: &Tools) -> Option<std::path::PathBuf> {
    let manager = tools.get_manager()?;
    let skill_name = tools.place().skill?;
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
/// `id` rewrites one. A metadata-only edit (tier, scope, summary, indexed) changes
/// nothing the row says and stays unguarded.
async fn guard_user_voice(tools: &Tools, server: &str, args: &mut Value) -> Result<()> {
    guard_user_voice_in(registry(), tools, server, args).await
}

/// [`guard_user_voice`], each target's voice read through `reg`.
async fn guard_user_voice_in(
    reg: &McpRegistry,
    tools: &Tools,
    server: &str,
    args: &mut Value,
) -> Result<()> {
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

    // Resolve each target's voice: `memory_search` op=get, one row by id.
    // A fetch miss is not the guard's problem — the daemon reports a missing
    // id on the real call, and its own floor still refuses a user-voice
    // write the engine did not authorize.
    let search = qualify(server, "memory_search");
    let mut offending: Vec<String> = Vec::new();
    for id in &targets {
        let get = serde_json::json!({"op": "get", "id": id});
        let Ok(text) = reg.call(&search, get).await else {
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

    #[test]
    fn a_delegates_brief_is_not_user_voice() {
        let mut tools = Tools::new(std::path::PathBuf::from("/workspace")).unwrap();
        let schema = json!({"properties": {"from": {}}});
        let takes = Takes::Schema(&schema);
        let mut args = json!({"content": "A caller's finding", "from": "user", "user_directed": true});
        let original = args.clone();
        attribute_row(&tools, &takes, &mut args);
        assert_eq!(args, original);

        tools.delegated_session = true;
        attribute_row(&tools, &takes, &mut args);
        assert_eq!(args["from"], "derived");
        assert!(!has(&args, "user_directed"));

        tools.person_spoke = true;
        let mut typed = original.clone();
        attribute_row(&tools, &takes, &mut typed);
        assert_eq!(typed, original);
    }

    /// The folded tools as memory's server publishes them to this engine.
    fn folded(name: &str) -> AdvertisedTool {
        let def = catalog()
            .tools_list(View::Local)
            .into_iter()
            .find(|t| t["name"] == name)
            .unwrap_or_else(|| panic!("{name} is not folded"));
        AdvertisedTool {
            qualified: qualify(BUILTIN_MEMORY, name),
            original: name.to_string(),
            server: BUILTIN_MEMORY.to_string(),
            description: String::new(),
            input_schema: def["inputSchema"].clone(),
        }
    }

    fn stamped(tool: &str, args: Value) -> Vec<&'static str> {
        let tool = folded(tool);
        let takes = Takes::of(&tool, &args);
        ["host", "source_session", "cwd", "root", "scope_root", "cwd_scope"]
            .into_iter()
            .filter(|f| takes.stamps(f, &args))
            .collect()
    }

    /// A folded tool's schema is the union of its operations' fields; what
    /// is filled in is the fields of the operation the call runs.
    #[test]
    fn only_the_running_operations_fields_are_filled() {
        assert_eq!(stamped("memory_search", json!({"query": "q"})), ["scope_root"], "an op-less query is a search");
        assert_eq!(stamped("memory_search", json!({"op": "semantic", "query": "q"})), ["scope_root"]);
        assert_eq!(stamped("memory_search", json!({"op": "list"})), ["scope_root"]);
        assert!(
            stamped("memory_search", json!({"op": "list", "day": "2026-11-01"})).is_empty(),
            "a day's worklist is never narrowed to a place"
        );
        assert!(stamped("memory_search", json!({"op": "get", "id": "a"})).is_empty(), "get takes no place");
        assert!(stamped("memory_search", json!({"id": "a"})).is_empty(), "no branch, nothing filled");
        assert_eq!(
            stamped("memory_add", json!({"content": "x"})),
            ["host", "source_session", "cwd", "root"]
        );
        assert!(stamped("memory_update", json!({"id": "a"})).is_empty(), "an edit re-authors nothing");
        assert_eq!(stamped("memory_upkeep", json!({"op": "session_start"})), ["cwd", "root"]);
        for op in ["days", "harvest_day", "sweep", "chains", "issues", "role"] {
            assert!(stamped("memory_upkeep", json!({"op": op})).is_empty(), "{op}");
        }
        // A delegate's search filter `from` is hidden on a search: never
        // rewritten as though it were a row's voice.
        let search = folded("memory_search");
        assert!(!Takes::of(&search, &json!({"query": "q", "from": "user"})).field("from"));
        assert!(Takes::of(&folded("memory_add"), &json!({"content": "x"})).field("from"));
    }

    /// The rule that keeps this file from carrying a list of tool names: a
    /// field is filled only where the server said it takes one.
    #[test]
    fn a_field_the_server_never_declared_is_not_invented() {
        let add = json!({"properties": {"content": {}, "host": {}, "source_session": {}}});
        assert!(declares(&add, "host"));
        assert!(declares(&add, "source_session"));

        // memory_update declares neither — an edit does not re-author a row.
        let update = json!({"properties": {"id": {}, "content": {}, "summary": {}}});
        assert!(!declares(&update, "host"));
        assert!(!declares(&update, "source_session"));
        assert!(declares(&update, "summary"));

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
            "scope_root"
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

    /// The guard reads each target's voice from a real store, the way the
    /// engine's agents reach it (`memory_search` op=get): a rewrite of the
    /// user's own row is refused by the guard itself, before the store's
    /// floor is ever asked; the agent's own rows pass.
    #[tokio::test]
    async fn the_guard_reads_the_rows_voice_through_the_folded_search() {
        let dir = tempfile::tempdir().unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let state = ling_mem::hosted::open_quiet_fake(dir.path(), base.clone())
            .await
            .unwrap();
        let app = ling_mem::hosted::router(state)
            .into_make_service_with_connect_info::<std::net::SocketAddr>();
        tokio::spawn(async move { axum::serve(listener, app).await });
        let reg = McpRegistry::empty();
        let mut cfgs = std::collections::BTreeMap::new();
        cfgs.insert(
            BUILTIN_MEMORY.to_string(),
            crate::mcp_client::McpServerConfig {
                url: Some(format!("{base}/mcp")),
                gated: false,
                ..Default::default()
            },
        );
        reg.connect_all(&cfgs).await;
        let add = |content: &str, from: &str| {
            let args = json!({"content": content, "from": from, "tier": "semantic"});
            let reg = &reg;
            async move {
                let text = reg.call(&qualify(BUILTIN_MEMORY, "memory_add"), args).await.unwrap();
                let added: Value = serde_json::from_str(&text).unwrap_or_else(|_| panic!("{text}"));
                added["fact"]["id"].as_str().unwrap().to_string()
            }
        };
        let theirs = add("Alex takes green tea", "user").await;
        let mine = add("Alex's kettle is in the left cupboard", "derived").await;
        let tools = Tools::new(std::env::temp_dir()).unwrap();

        let mut replace = json!({"content": "Alex takes black tea", "replace_ids": [theirs]});
        let why = guard_user_voice_in(&reg, &tools, BUILTIN_MEMORY, &mut replace)
            .await
            .unwrap_err()
            .to_string();
        assert!(why.starts_with("BLOCKED by the user-voice merge guard"), "{why}");
        assert!(why.contains("Alex takes green tea"), "{why}");
        let mut rewrite = json!({"id": theirs, "content": "Alex takes black tea"});
        let why = guard_user_voice_in(&reg, &tools, BUILTIN_MEMORY, &mut rewrite)
            .await
            .unwrap_err()
            .to_string();
        assert!(why.starts_with("BLOCKED by the user-voice merge guard"), "{why}");

        let mut own = json!({"content": "The kettle moved", "replace_ids": [mine]});
        guard_user_voice_in(&reg, &tools, BUILTIN_MEMORY, &mut own)
            .await
            .unwrap();
        assert!(own.get("user_directed").is_none(), "{own}");
    }

    /// The dream's promote and the scan's backfill as `agents/memory.md`
    /// writes them run as written: the folded `memory_add` takes every field
    /// (the original row's session, its scope as `cwd`, its time), and a
    /// field the call carries is never filled over.
    #[test]
    fn the_promote_and_backfill_shapes_run_as_written() {
        let promote = json!({"content": "Alex keeps bees", "type": "fact", "from": "user", "tier": "semantic",
            "occurred_at": "2026-11-10T12:00:00Z", "source_session": "sess-original", "cwd": "/work/harbor", "summary": "bees"});
        let backfill = json!({"content": "Alex keeps bees", "type": "fact", "from": "user", "tier": "episodic",
            "occurred_at": "2026-11-10T12:00:00Z", "source_session": "cc-session", "cwd": "/work/harbor"});
        for sent in [promote, backfill] {
            let ran = match catalog().resolve(View::Local, "memory_add", sent.clone()) {
                ling_mem::catalog::Resolved::Call { branch, args } => {
                    assert_eq!(branch.op, "add");
                    args
                }
                ling_mem::catalog::Resolved::Refused(why) => panic!("{sent}: {why}"),
                ling_mem::catalog::Resolved::Unknown => panic!("memory_add is folded"),
            };
            for field in ["source_session", "cwd", "occurred_at", "from", "tier"] {
                assert_eq!(ran[field], sent[field], "{field}");
            }
            assert!(!wants_project(&sent, "cwd"), "the row's own scope is not stamped over");
            assert!(has(&sent, "source_session"), "the original session is not filled over");
        }
    }

    /// The engine's own agents, missions and prompts call memory by its
    /// folded tools: none of memory's retired tool names (a legacy name that
    /// is no folded tool) is left in them for a model to call into nothing.
    /// The names the engine's side of the door retired (the dream's two
    /// among them) are the door's catalogue's, checked over the same files
    /// by the door's repo check (`repo_check::
    /// no_live_file_names_a_retired_tool_address_or_namespace`) — no list of
    /// them is kept here.
    #[test]
    fn the_engines_own_prompts_name_only_the_folded_tools() {
        let cat = catalog();
        let retired: Vec<&str> = cat.retired().iter().map(|r| r.name).collect();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut files = Vec::new();
        for dir in ["agents", "missions", "prompts"] {
            collect_text(&root.join(dir), &mut files);
        }
        assert!(files.len() > 5, "found {files:?}");
        let mut stale = Vec::new();
        for path in &files {
            let text = std::fs::read_to_string(path).unwrap_or_default();
            for name in &retired {
                let named = text.match_indices(name).any(|(at, _)| {
                    let next = text[at + name.len()..].chars().next();
                    !next.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
                });
                if named {
                    stale.push(format!("{}: {name}", path.display()));
                }
            }
        }
        assert!(stale.is_empty(), "retired memory tools named: {stale:#?}");
    }

    fn collect_text(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_text(&path, out);
            } else if matches!(path.extension().and_then(|e| e.to_str()), Some("md" | "toml")) {
                out.push(path);
            }
        }
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
