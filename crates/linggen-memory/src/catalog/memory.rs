//! Memory's branches: the seventeen verbs a host reaches over MCP, each one
//! entry that is its `op` of one of the five memory tools (`memory_search`
//! op `list`), the names those verbs had before the tools were folded, and
//! the adapters every caller of a memory branch shares — the arguments
//! repaired the way models get them wrong, the store's answer read back,
//! and the answers a model reliably misreads said plainly.
//!
//! Every branch here runs in the store alone: none needs an account or a
//! generative model, and all are visible to an approved computer as well as
//! this one. The engine adds the upkeep branches only it can run
//! (`dream_status`, `dream_run`) to `memory_upkeep` when it composes its
//! catalogue.

use super::{Branch, Catalog, Retired, ToolDoc, Views};
use crate::call::Who;
use crate::http::envelope::ApiError;
use crate::http::state::SharedState;
use axum::response::Response;
use serde_json::{json, Map, Value};
use std::sync::LazyLock;
use std::time::Duration;

/// How long a tool call waits on its verb. A write can pay
/// for a LanceDB auto-cleanup inside its commit (9s once, 2026-09-15), so
/// 10s cut off adds that then landed anyway. Kept under the 30s the engine
/// and the CLI wait, so the caller hears this daemon's own timeout message.
pub const CALL_TIMEOUT_MS: u64 = 25_000;

/// The memory tools, in the order a host lists them.
pub const TOOLS: &[ToolDoc] = &[
    ToolDoc {
        name: "memory_search",
        description: "Read memory. With only a query (op omitted, or op=semantic): rows ranked by relevance — who the user is, their preferences and decisions, not codebase facts. op=get fetches one row by id; op=list browses by filters without ranking (audits, a day's worklist). Pass the fewest filters that answer the question; scope_root is the host's to fill.",
        implicit_op: Some("semantic"),
    },
    ToolDoc {
        name: "memory_add",
        description: "Save one durable row — identity, a preference, a decision with its why — not project internals or secrets. Default tier episodic, from derived; the user's own words are from=user. scope is the directory the row is about (one of the session's scope candidates), global:true when it is about the person. A standing rule gets indexed:true and a one-line summary. host, source_session, cwd and root are the host's to fill. Merge your own rows atomically with replace_ids, never add then delete; replacing a from=user row needs the user's answer and user_directed:true. occurred_at (RFC 3339) dates an imported or promoted row.",
        implicit_op: None,
    },
    ToolDoc {
        name: "memory_update",
        description: "Edit one row in place by id (content, type, tier, scope or global, summary, indexed); a tier change keeps the id. Rewriting a from=user row's content needs the user's answer and user_directed:true. To merge several rows use memory_add with replace_ids.",
        implicit_op: None,
    },
    ToolDoc {
        name: "memory_delete",
        description: "Hard-delete one row by id: an explicit forget, an exact duplicate, a secret saved by mistake. Never part of a merge — memory_add with replace_ids does that in one call.",
        implicit_op: None,
    },
    ToolDoc {
        name: "memory_upkeep",
        description: "Memory upkeep, one op per step of the scan, dream and solve runbooks (the Linggen plugin's skill references). Ops here: {ops}. session_start is the host's call at session start. Each op keeps its own checks: past local days only, the dream lease, the primary. issues lists the review queue; you solve each item from evidence and close it with issue_resolve.",
        implicit_op: None,
    },
];

/// The names memory's verbs were published under before the tools were
/// folded. A host calling one has an old plugin: it is told to update.
pub const RETIRED: &[Retired] = &[
    Retired {
        name: "memory_list",
        now: "memory_search op=list",
    },
    Retired {
        name: "memory_get",
        now: "memory_search op=get",
    },
    Retired {
        name: "memory_session_start",
        now: "memory_upkeep op=session_start",
    },
    Retired {
        name: "memory_harvest_day",
        now: "memory_upkeep op=harvest_day",
    },
    Retired {
        name: "memory_days",
        now: "memory_upkeep op=days",
    },
    Retired {
        name: "memory_remember_day",
        now: "memory_upkeep op=remember_day",
    },
    Retired {
        name: "memory_dream_lease",
        now: "memory_upkeep op=dream_lease",
    },
    Retired {
        name: "memory_role",
        now: "memory_upkeep op=role",
    },
    Retired {
        name: "memory_sweep",
        now: "memory_upkeep op=sweep",
    },
    Retired {
        name: "memory_issues",
        now: "memory_upkeep op=issues",
    },
    Retired {
        name: "memory_issue_add",
        now: "memory_upkeep op=issue_add",
    },
    Retired {
        name: "memory_issue_resolve",
        now: "memory_upkeep op=issue_resolve",
    },
    Retired {
        name: "memory_chains",
        now: "memory_upkeep op=chains",
    },
];

/// Memory's branches.
pub const BRANCHES: &[Branch<&str>] = &[
    Branch {
        tool: "memory_search",
        op: "semantic",
        note: "",
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "query":    {"type": "string", "description": "Natural-language description of what to find."},
                    "tier":     {"type": "string", "enum": ["core", "semantic", "episodic"], "description": "Restrict to one tier. Omit to span all."},
                    "scope_root": {"type": "string", "description": "HOST-FILLED — leave it out; the host stamps the session root, which weighs each row by how near its stored scope is to the root (a skill's dir sees only its own rows)."},
                    "limit":    {"type": "integer", "description": "Max rows. Default 10."}
                },
                "required": ["query"]
            })
        },
        // `min_score` is deliberately absent. The REST handler accepts it
        // and the tool call passes it through, so a *programmatic* caller
        // (Linggen's auto-recall) can set a floor — but advertising a
        // relevance threshold to a model invites exactly the over-fill
        // failure these narrow schemas exist to prevent: a guessed value
        // silently narrows recall to zero rows. Omitted here, the daemon's
        // store-wide `recall_min_score` applies. `type` is the same: an
        // app's auto-recall searches one row type at a time; a model is
        // told never to pass it.
        extra: None,
        hidden: &["min_score", "from", "outcome", "type"],
        fixed: &[],
        check: None,
        timeout_ms: CALL_TIMEOUT_MS,
        views: Views::ALL,
        dispatch: "search",
    },
    Branch {
        tool: "memory_search",
        op: "list",
        note: "",
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "tier":     {"type": "string", "enum": ["core", "semantic", "episodic"]},
                    "types":    {"type": "array", "items": {"type": "string", "enum": ["fact", "preference", "decision", "tried", "fixed", "learned", "built"]}, "description": "Only rows of these types."},
                    "scope_root": {"type": "string", "description": "HOST-FILLED unless `day` is given — leave it out; the host stamps the session root: rows whose stored scope is under it or at its parents, and rows about the person (no scope). A skill's dir sees only its own rows. A day's worklist is never narrowed."},
                    "indexed":  {"type": "boolean", "description": "Only indexed rows (true) or only the rest (false)."},
                    "past_ttl": {"type": "boolean", "description": "Return only rows past the configured episodic TTL. Implies tier=episodic."},
                    "day":      {"type": "string", "description": "One local calendar day, YYYY-MM-DD — the remember stage lists a single day's worklist with this."},
                    "unjudged": {"type": "boolean", "description": "With day: only the rows no remember pass has judged yet — a re-opened day's new rows, not the whole day."},
                    "sort":     {"type": "string", "enum": ["newest", "oldest"]},
                    "limit":    {"type": "integer"},
                    "offset":   {"type": "integer"}
                }
            })
        },
        extra: None,
        hidden: &["from", "outcome"],
        fixed: &[],
        check: Some(list_rule),
        timeout_ms: CALL_TIMEOUT_MS,
        views: Views::ALL,
        dispatch: "list",
    },
    Branch {
        tool: "memory_upkeep",
        op: "session_start",
        note: "",
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "cwd":  {"type": "string", "description": "HOST-FILLED — the session's working directory."},
                    "root": {"type": "string", "description": "HOST-FILLED — the session's root (git root, or where it started)."}
                }
            })
        },
        extra: None,
        hidden: &[],
        fixed: &[],
        check: None,
        timeout_ms: CALL_TIMEOUT_MS,
        views: Views::ALL,
        dispatch: "session_start",
    },
    Branch {
        tool: "memory_search",
        op: "get",
        note: "",
        schema: || {
            json!({
                "type": "object",
                "properties": { "id": {"type": "string"} },
                "required": ["id"]
            })
        },
        extra: None,
        hidden: &[],
        fixed: &[],
        check: None,
        timeout_ms: CALL_TIMEOUT_MS,
        views: Views::ALL,
        dispatch: "get",
    },
    Branch {
        tool: "memory_add",
        op: "add",
        note: "",
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "content":  {"type": "string", "description": "The fact text the model will see when this row is recalled."},
                    "type":     {"type": "string", "enum": ["fact", "preference", "decision", "tried", "fixed", "learned", "built"]},
                    "from":     {"type": "string", "enum": ["user", "agent", "derived"], "description": "Whose words: user = the user stated it (a preference, a decision, a fact about them); agent = you did or observed it; derived (default) = your inference."},
                    "tier":     {"type": "string", "enum": ["core", "semantic", "episodic"], "description": "Destination tier: episodic = per-turn staging (the default when omitted, no search-first); semantic = curated durable facts (search-first); core = tiny always-injected universals (search-first). Doctrine in the server instructions."},
                    "scope":    {"type": "string", "description": "The directory this row is about, when it is not where you stand: one of the session's \"Memory scopes here\" candidates (e.g. skills/lingjing, ~/workspace). Omit for the session's own directory. Stored as the row's `scope` (absolute path; none = about the person)."},
                    "summary":  {"type": "string", "description": "One line, ≤ 80 chars: what this row is for — what the index shows in its place. Matters on indexed rows; without one the index shows the row's opening."},
                    "indexed":  {"type": "boolean", "description": "true = put the row in its scope's index: its summary loads at every session start in that directory or below. For standing rules the user states (\"always…\", \"from now on…\")."},
                    "host":     {"type": "string", "description": "HOST-FILLED — leave it out; the host stamps the committing runtime."},
                    "source_session": {"type": "string", "description": "HOST-FILLED — leave it out; the host stamps this session. Only a promote pass carrying an original row's session forward passes it."},
                    "cwd":      {"type": "string", "description": "HOST-FILLED, request only — leave it out; the host stamps the session's working directory: the stored scope when you name none, and the base a named one is checked against. Never stored under this name."},
                    "root":     {"type": "string", "description": "HOST-FILLED — leave it out; the host stamps the session root that `scope` resolves against."},
                    "global":   {"type": "boolean", "description": "true = the row is about the person, not a directory: stored with no scope, recalled everywhere."},
                    "replace_ids": {"type": "array", "items": {"type": "string"}, "description": "Row ids this row replaces — inserted and deleted atomically. For merges and resolved conflicts; never separate add + delete calls."},
                    "user_directed": {"type": "boolean", "description": "Assert the user directed this change (a settled command/declaration, or they just answered your ask). Required when replace_ids targets from=user rows — memory BLOCKS such writes otherwise. Hedged reflections don't qualify; see the server instructions."}
                },
                "required": ["content"]
            })
        },
        extra: Some(add_extra),
        hidden: &["outcome", "skip_dedup", "force"],
        fixed: &[],
        check: None,
        timeout_ms: CALL_TIMEOUT_MS,
        views: Views::ALL,
        dispatch: "add",
    },
    Branch {
        tool: "memory_delete",
        op: "delete",
        note: "",
        schema: || {
            json!({
                "type": "object",
                "properties": { "id": {"type": "string"} },
                "required": ["id"]
            })
        },
        extra: None,
        hidden: &[],
        fixed: &[],
        check: None,
        timeout_ms: CALL_TIMEOUT_MS,
        views: Views::ALL,
        dispatch: "delete",
    },
    Branch {
        tool: "memory_update",
        op: "update",
        note: "",
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "id":       {"type": "string"},
                    "content":  {"type": "string", "description": "Replacement fact text."},
                    "type":     {"type": "string", "enum": ["fact", "preference", "decision", "tried", "fixed", "learned", "built"]},
                    "tier":     {"type": "string", "enum": ["core", "semantic", "episodic"], "description": "Moving tier relocates the row across tables, id preserved."},
                    "scope":    {"type": "string", "description": "Move the row: its stored scope, the absolute directory it is about (~/… allowed)."},
                    "summary":  {"type": "string", "description": "New one-line summary (≤ 80 chars) — what the index shows for an indexed row."},
                    "indexed":  {"type": "boolean", "description": "Put the row in (true) or take it out of (false) its scope's index."},
                    "global":   {"type": "boolean", "description": "true = clear the row's stored scope so it applies everywhere."},
                    "user_directed": {"type": "boolean", "description": "Required when rewriting content on a from=user row — see memory_add.user_directed."}
                },
                "required": ["id"]
            })
        },
        extra: None,
        hidden: &[],
        fixed: &[],
        check: None,
        timeout_ms: CALL_TIMEOUT_MS,
        views: Views::ALL,
        dispatch: "update",
    },
    Branch {
        tool: "memory_upkeep",
        op: "harvest_day",
        note: "",
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "date": {"type": "string", "description": "Local calendar day, YYYY-MM-DD."}
                },
                "required": ["date"]
            })
        },
        extra: None,
        hidden: &[],
        fixed: &[],
        check: None,
        timeout_ms: CALL_TIMEOUT_MS,
        views: Views::ALL,
        dispatch: "harvest_day",
    },
    Branch {
        tool: "memory_upkeep",
        op: "days",
        note: "",
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "undreamed_only": {"type": "boolean", "description": "Only days awaiting a dream pass (the worklist)."},
                    "from": {"type": "string", "description": "Inclusive YYYY-MM-DD lower bound."},
                    "to":   {"type": "string", "description": "Inclusive YYYY-MM-DD upper bound."}
                }
            })
        },
        extra: None,
        hidden: &[],
        fixed: &[],
        check: None,
        timeout_ms: CALL_TIMEOUT_MS,
        views: Views::ALL,
        dispatch: "days",
    },
    Branch {
        tool: "memory_upkeep",
        op: "remember_day",
        note: "",
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "date":     {"type": "string", "description": "Local calendar day, YYYY-MM-DD."},
                    "judged":   {"type": "integer", "description": "Rows judged in this pass."},
                    "promoted": {"type": "integer", "description": "Rows promoted to semantic in this pass."},
                    "holder":   {"type": "string", "description": "The dream lease holder, when one was taken."}
                },
                "required": ["date"]
            })
        },
        extra: None,
        hidden: &[],
        fixed: &[],
        check: Some(counts_rule),
        timeout_ms: CALL_TIMEOUT_MS,
        views: Views::ALL,
        dispatch: "remember_day",
    },
    Branch {
        tool: "memory_upkeep",
        op: "dream_lease",
        note: "",
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "day":    {"type": "string", "description": "Local calendar day, YYYY-MM-DD."},
                    "holder": {"type": "string", "description": "Who is dreaming, e.g. claude-code@<host>."},
                    "action": {"type": "string", "enum": ["acquire", "renew", "release"]}
                },
                "required": ["day", "holder"]
            })
        },
        extra: None,
        hidden: &[],
        fixed: &[],
        check: None,
        timeout_ms: CALL_TIMEOUT_MS,
        views: Views::ALL,
        dispatch: "dream_lease",
    },
    Branch {
        tool: "memory_upkeep",
        op: "role",
        note: "",
        schema: || json!({ "type": "object", "properties": {} }),
        extra: None,
        hidden: &[],
        fixed: &[],
        check: None,
        timeout_ms: CALL_TIMEOUT_MS,
        views: Views::ALL,
        dispatch: "role",
    },
    Branch {
        tool: "memory_upkeep",
        op: "sweep",
        note: "",
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "dry_run": {"type": "boolean", "description": "Report what would be evicted without deleting."}
                }
            })
        },
        extra: None,
        hidden: &[],
        fixed: &[],
        check: None,
        timeout_ms: CALL_TIMEOUT_MS,
        views: Views::ALL,
        dispatch: "sweep",
    },
    Branch {
        tool: "memory_upkeep",
        op: "issues",
        note: "",
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "status": {"type": "string", "enum": ["open", "resolved", "dismissed", "all"], "description": "Which items to list (default open)."},
                    "limit":  {"type": "integer", "description": "Max items (default 50)."}
                }
            })
        },
        extra: None,
        hidden: &[],
        fixed: &[],
        check: Some(page_rule),
        timeout_ms: CALL_TIMEOUT_MS,
        views: Views::ALL,
        dispatch: "issues",
    },
    Branch {
        tool: "memory_upkeep",
        op: "issue_add",
        note: "",
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "kind":    {"type": "string", "enum": ["chain", "stale-status", "contradiction", "subject"], "description": "What you saw: `chain` = uncertain merge candidate; `stale-status` = a status claim likely overtaken by the world (verify against git/files at solve time); `contradiction` = conflicting rows needing the user's pick; `subject` = digest cluster of doubtful subject coherence (list ALL member ids). Scope, index and summary fixes are not queued — the dream applies them itself."},
                    "row_ids": {"type": "array", "items": {"type": "string"}, "description": "The memory row ids this item is about."},
                    "note":    {"type": "string", "description": "What you saw and what a solver should check — the item's whole context, since the solver starts from this line alone. Write it in plain words; it may become the user's question."}
                },
                "required": ["kind", "note"]
            })
        },
        extra: None,
        hidden: &[],
        fixed: &[],
        check: None,
        timeout_ms: CALL_TIMEOUT_MS,
        views: Views::ALL,
        dispatch: "issue_add",
    },
    Branch {
        tool: "memory_upkeep",
        op: "issue_resolve",
        note: "",
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "id":      {"type": "string", "description": "The issue id from op=issues."},
                    "outcome": {"type": "string", "enum": ["resolved", "dismissed"]},
                    "note":    {"type": "string", "description": "One-line record of what was done."}
                },
                "required": ["id", "outcome"]
            })
        },
        extra: None,
        hidden: &[],
        fixed: &[],
        check: None,
        timeout_ms: CALL_TIMEOUT_MS,
        views: Views::ALL,
        dispatch: "issue_resolve",
    },
    Branch {
        tool: "memory_upkeep",
        op: "chains",
        note: "",
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "kind":   {"type": "string", "enum": ["cited", "marker", "subject"], "description": "cited (default) = id-citation chains; marker = provisional-state candidates; subject = same-subject clusters for digests."},
                    "limit":  {"type": "integer", "description": "Clusters per page (default 10)."},
                    "offset": {"type": "integer", "description": "Pagination offset over the cluster list."},
                    "derived_only": {"type": "boolean", "description": "Only clusters mergeable unattended (every row from=derived, tier=semantic). Unattended condense passes MUST set true."}
                }
            })
        },
        extra: None,
        hidden: &[],
        fixed: &[],
        check: Some(page_rule),
        timeout_ms: CALL_TIMEOUT_MS,
        views: Views::ALL,
        dispatch: "chains",
    },
];

/// Fields only the folded `memory_add` advertises.
fn add_extra() -> Value {
    json!({
        "occurred_at": {"type": "string", "description": "RFC 3339 time the row is about — for an imported or promoted row; dates its day and TTL, never its rank."}
    })
}

fn non_negative(obj: &Map<String, Value>, keys: &[&str]) -> Result<(), String> {
    for key in keys {
        if obj.get(*key).and_then(Value::as_i64).is_some_and(|n| n < 0) {
            return Err(format!("`{key}` must not be negative"));
        }
    }
    Ok(())
}

/// `unjudged` reads one day; a TTL sweep reads the staging table; pages
/// count from zero.
fn list_rule(obj: &Map<String, Value>) -> Result<(), String> {
    non_negative(obj, &["limit", "offset"])?;
    if obj.get("unjudged") == Some(&Value::Bool(true)) && !obj.contains_key("day") {
        return Err("`unjudged` needs `day` (YYYY-MM-DD)".into());
    }
    let tier = obj.get("tier").and_then(Value::as_str);
    if obj.get("past_ttl") == Some(&Value::Bool(true)) && tier.is_some_and(|t| t != "episodic") {
        return Err("`past_ttl` reads episodic rows; drop `tier` or pass tier=episodic".into());
    }
    Ok(())
}

fn counts_rule(obj: &Map<String, Value>) -> Result<(), String> {
    non_negative(obj, &["judged", "promoted"])
}

fn page_rule(obj: &Map<String, Value>) -> Result<(), String> {
    non_negative(obj, &["limit", "offset"])
}

/// Memory's own catalogue: what its MCP server publishes, and the folded
/// memory tools a host's catalogue composes.
pub fn catalog() -> &'static Catalog<&'static str> {
    static CATALOG: LazyLock<Catalog<&'static str>> = LazyLock::new(|| {
        Catalog::new(TOOLS.to_vec(), BRANCHES.to_vec(), RETIRED.to_vec())
            .expect("memory's catalogue is consistent")
    });
    &CATALOG
}

/// Dispatch-boundary fixes ported from `linggen/src/engine/tools/memory_tool.rs`.
/// These compensate for shapes models commonly produce that the daemon
/// can't or shouldn't interpret as-is.
pub fn prepare(args: &mut Value) {
    let Some(obj) = args.as_object_mut() else {
        return;
    };

    // 1. Drop soft-empty fields. `until: ""` would crash the RFC-3339
    //    parser; empty arrays narrow unintentionally; nulls are noise.
    obj.retain(|_, v| match v {
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Null => false,
        _ => true,
    });

    // 2. `tier=episodic` → `episodic=true` on the wire (the daemon splits
    //    the episodic store into a separate table). Other tier values stay
    //    as filters within the semantic store.
    if let Some(tier) = obj.get("tier").and_then(|v| v.as_str()) {
        if tier == "episodic" {
            obj.insert("episodic".to_string(), Value::Bool(true));
            obj.remove("tier");
        }
    }

    // 3. `past_ttl=true` is a bulk-eviction sweep — it wants every past-TTL
    //    episodic row regardless of `type`/`from`/`outcome`. gpt-5.5 has
    //    been observed to over-fill these with hallucinated defaults
    //    (`type=fact, from=user, outcome=positive`) which narrows the
    //    sweep to zero rows. Strip them at the dispatch boundary; the
    //    schema doesn't expose them on list, but defend defensively.
    //    Not gated on `verb == "list"`: `past_ttl` is only meaningful on a
    //    sweep, so stripping it wherever it appears costs nothing and covers
    //    a model that reaches for the sweep shape on the wrong verb.
    let is_ttl_sweep = obj
        .get("past_ttl")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if is_ttl_sweep {
        for k in ["type", "from", "outcome"] {
            let _ = obj.remove(k);
        }
    }
}

/// The mirror of [`prepare`] on the way back out: shapes the
/// daemon returns that a *model* reliably misreads.
///
/// This layer, not the REST handler, is the right home for them for the same
/// reason the request-side fixes live here — CLI arguments come from clap,
/// typed, from a human, and REST callers are programs. Only the MCP surface
/// is talking to something that infers intent from JSON it did not write.
pub fn finish(verb: &str, value: &mut Value) {
    if verb != "delete" {
        return;
    }
    let Some(obj) = value.as_object_mut() else {
        return;
    };
    // Deleting an already-absent row is success, not an anomaly — the row is
    // gone either way (commonly this daemon's own cross-tier dedup removed
    // the episodic copy during a promote add). A bare `removed:false` reads
    // as an error signal to LLM callers: three dream runs were observed
    // aborting over it, claiming "store inconsistency". Say what it means.
    if obj.get("removed").and_then(|v| v.as_bool()) == Some(false) {
        obj.insert("already_gone".to_string(), Value::Bool(true));
        obj.insert(
            "note".to_string(),
            Value::String(
                "row was already absent — treat as success; do not retry or verify".to_string(),
            ),
        );
    }
}

/// Wrap a JSON value into MCP's `content` array. We always return a
/// single text item containing the serialized JSON — the model sees the
/// raw structured data and can reason about it directly.
pub fn text_content(data: &Value) -> Value {
    let text = serde_json::to_string(data).unwrap_or_else(|_| "null".into());
    json!({
        "content": [
            { "type": "text", "text": text }
        ]
    })
}

/// Run a memory branch on `state`: [`prepare`], the verb as its route
/// answers it (`http::verb::answer`, on this runtime, no request made), its
/// `{ok, data}` envelope unwrapped, then [`finish`]. The verb runs as its
/// own task: one that outlives the branch's timeout still finishes, which
/// is what the timeout's message tells the caller.
///
/// The call is the owner's: a memory branch is reached only by callers the
/// host admitted as this machine or a machine it approved (a phone never
/// reaches it — the engine's `memory_host::routes`).
pub async fn run(
    state: &SharedState,
    branch: &Branch<&'static str>,
    mut args: Value,
) -> Result<Value, ApiError> {
    prepare(&mut args);
    let verb = branch.dispatch;
    let (state, owned) = (state.clone(), verb.to_string());
    let task =
        tokio::spawn(
            async move { crate::http::verb::answer(state, Who::Owner, &owned, args).await },
        );
    let wait = Duration::from_millis(branch.timeout_ms);
    let mut data = match tokio::time::timeout(wait, task).await {
        Err(_) => return Err(timed_out(verb, wait)),
        Ok(Err(e)) => return Err(ApiError::internal(anyhow::Error::from(e))),
        Ok(Ok(response)) => read_answer(verb, response).await?,
    };
    finish(verb, &mut data);
    Ok(data)
}

/// A timeout says what it means for a write: the store may have finished the
/// work after we stopped waiting, so look before doing it again.
pub(crate) fn timed_out(verb: &str, wait: Duration) -> ApiError {
    ApiError::internal(anyhow::anyhow!(timed_out_line(verb, wait)))
}

/// What a memory branch that got no answer in time means, in the words
/// every door says it with — memory's own runner and the engine's.
pub fn timed_out_line(verb: &str, wait: Duration) -> String {
    format!(
        "{verb} timed out after {}s — a write may still have landed; search before retrying",
        wait.as_secs()
    )
}

/// The route's answer, read as the HTTP caller read it.
pub(crate) async fn read_answer(verb: &str, response: Response) -> Result<Value, ApiError> {
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .map_err(|e| ApiError::internal(anyhow::Error::from(e)))?;
    let body = String::from_utf8_lossy(&bytes);
    // A body the route refused before its verb ran (axum's 422 for a field
    // of the wrong type) is plain text. Say what it said — "error decoding
    // response body" told nobody which field was wrong.
    let value: Value = serde_json::from_str(&body).map_err(|_| {
        ApiError::internal(anyhow::anyhow!(
            "{verb}: HTTP {}: {}",
            status.as_u16(),
            body.trim()
        ))
    })?;
    let code = crate::ErrorCode::from_wire(value["code"].as_str(), status.as_u16());
    read_envelope(value).map_err(|message| ApiError { code, message })
}

/// Memory's `{ok, data}` envelope: the data, or the error it carries.
pub fn read_envelope(value: Value) -> Result<Value, String> {
    if value.get("ok").and_then(|v| v.as_bool()) == Some(true) {
        Ok(value.get("data").cloned().unwrap_or(Value::Null))
    } else {
        Err(value
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("upstream error")
            .to_string())
    }
}
