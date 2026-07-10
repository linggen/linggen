//! `Memory_query` / `Memory_write` as built-in tools.
//!
//! These were previously routed through `engine::capability_tools::dispatch`
//! via the `memory` capability abstraction. The capability layer was useful
//! when `shared-memory` was a skill that declared `provides: [memory]`, but
//! today `ling-mem` ships with linggen itself — the dispatch path is just
//! "POST to `<ling_mem_url>/api/memory/<verb>`". So Memory_* are regular
//! built-in tools that happen to be HTTP-backed, the same way `Bash` is a
//! built-in tool that happens to be process-backed.
//!
//! Behavior preserved verbatim from the old capability path:
//! - Verb-dispatched: `verb` arg picks the endpoint, then is stripped.
//! - Soft-empty fields (empty string / empty array / null) are dropped so
//!   the daemon's serde parse doesn't reject `until: ""`.
//! - `tier: "episodic"` is translated to `episodic: true` on the wire.
//! - `host` defaults to `"linggen"` on `Memory_write` calls.
//! - On `ConnectionRefused` or timeout, ensure the binary is present
//!   (auto-install the pinned `ling-mem` if missing — a fresh Linggen
//!   marketplace install ships skill files but no binary), then autostart
//!   `ling-mem start` and retry once. The daemon is idempotent — re-running
//!   `start` while up exits 0. The engine owns this dependency: callers (the
//!   3am dream, auto-recall) reach memory over HTTP, never via the CLI, so
//!   nothing else would install/start the daemon for them.
//!
//! Schemas mirror what `engine::capabilities::CAPABILITIES` used to expose.

use super::builtin::Tool;
use super::{ToolCall, ToolResult, Tools};
use crate::engine::permission::PermissionMode;
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::time::Duration;

/// Per-call HTTP timeout. Matches the old `capability_tools.rs` budget.
const DISPATCH_TIMEOUT: Duration = Duration::from_secs(5);

/// Outer budget for `ling-mem start` autostart. The spawned command has
/// its own ~10s internal budget; this ceiling avoids cancelling a start
/// that's about to succeed.
const AUTOSTART_TIMEOUT: Duration = Duration::from_secs(15);

/// Outer bound on one full Memory_* dispatch — meta lookup, voice guard,
/// HTTP with one autostart retry (worst legitimate case ≈ 30s). The
/// 2026-07-10 dream run wedged on an await BEFORE the HTTP layer's own
/// 5s budget and froze the mission for 29 minutes; every path through
/// dispatch must surface as a tool error, never a silent hang.
const MEMORY_TOOL_TIMEOUT: Duration = Duration::from_secs(60);

/// Binary version the engine bootstraps when `ling-mem` is missing. A semver
/// range floor — `install-bin.sh` resolves it to the highest matching release.
/// Major-version range now that ling-mem has cut 1.0 — resolves to the highest
/// 1.x release, so minors/patches flow without a re-pin (store schema-version
/// guard keeps it data-safe). NOTE the form: `install-bin.sh`'s `~` needs
/// `X.Y`, so a major range is `^1` (or `1.x`), never `~1`. Override with
/// `$LING_MEM_VERSION`.
const LING_MEM_PIN: &str = "^1";

/// Canonical binary-only installer (SHA-256 verified inside). Fetched over
/// HTTPS and run via `bash -s` when the binary is absent.
const INSTALL_BIN_URL: &str =
    "https://raw.githubusercontent.com/linggen/linggen-memory/main/plugins/shared-memory/scripts/install-bin.sh";

pub struct MemoryQueryTool;

#[async_trait]
impl Tool for MemoryQueryTool {
    fn name(&self) -> &'static str {
        "Memory_query"
    }
    fn description(&self) -> &'static str {
        "Read memory. Verb-dispatched: `get` (fetch one row by id), `search` (semantic search; ranked by relevance — takes `query`), `list` (filter-only browse, no ranking — for audits or sweeps), `days` (per-day dream-state rollup: each day's episodic counts + pipeline state today/staging/pending/remembered/forgotten; `pending_only` = the dream worklist, oldest first), `chains` (condense scan — mechanical, read-only detection of stale same-subject chains in long-term memory; `kind=cited` groups rows citing another row's id verbatim, `kind=marker` lists provisional-state rows with nearest neighbors for confirmation; each cluster carries `derived_only` — merge unattended only when true). Memory holds the user's biography across sessions — identity, cross-project preferences, decisions with their reasoning. For codebase facts, read project files directly.\n\n**All filters are optional and AND-combined.** Speculatively passing `type`, `from`, or `outcome` is the #1 cause of empty results — start with just `verb` (+ `query` for search) and add filters only after seeing what's there.\n\n**Example — the dream worklist (days awaiting a remember pass):**\n```\n{ \"verb\": \"days\", \"pending_only\": true }\n```\n**Example — one day's remember worklist:**\n```\n{ \"verb\": \"list\", \"tier\": \"episodic\", \"day\": \"2026-07-01\", \"limit\": 50 }\n```\nNo `type`/`from`/`outcome` — those would narrow the sweep to zero rows."
    }
    fn tier(&self) -> PermissionMode {
        // Memory ops are conversation primitives — the user's own data
        // being saved on their behalf, not workspace mutations. Pin at
        // Chat so every session tier can use them without a permission
        // prompt.
        PermissionMode::Chat
    }
    fn cacheable(&self) -> bool {
        // The store is live mutable state shared across sessions and
        // hosts — an identical query can legitimately return new data
        // (the condense pass re-scans at offset 0 after every merge).
        false
    }
    fn args_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "verb":     {"type": "string", "enum": ["get", "search", "list", "days", "chains"], "description": "Read operation."},
                "id":       {"type": "string", "description": "Required for verb=get. Fact UUID."},
                "query":    {"type": "string", "description": "Required for verb=search. Natural-language description of what you're looking for."},
                "contexts": {"type": "array", "items": {"type": "string"}, "description": "Filter to these scope tags (AND semantics). For verb=search, narrows ranked results; for verb=list, primary filter. Omit to skip."},
                "type":     {"type": "string", "enum": ["fact", "preference", "decision", "tried", "fixed", "learned", "built"], "description": "Filter by fact type. Omit to return all types."},
                "tier":     {"type": "string", "enum": ["core", "semantic", "episodic"], "description": "Memory category. `core` = always-on identity/style universals; `semantic` = curated retrieval pool; `episodic` = per-turn capture staging (the dream pass judges it nightly). **Omit to span all three.**"},
                "from":     {"type": "string", "enum": ["user", "agent", "derived"], "description": "**DEFAULT: do not pass.** Filter by origin. Pass only when the user explicitly asked to see rows from a specific origin (rare)."},
                "outcome":  {"type": "string", "enum": ["positive", "negative", "neutral"], "description": "**DEFAULT: do not pass.** Filter by outcome. Almost no rows have `outcome=neutral`; passing it returns 0 rows even when the store has data. Pass only when the user explicitly asked to see only positive / negative outcomes."},
                "since":    {"type": "string", "description": "RFC-3339 lower bound on effective timestamp. Omit to skip."},
                "until":    {"type": "string", "description": "RFC-3339 upper bound (verb=list only). Omit to skip."},
                "past_ttl": {"type": "boolean", "description": "verb=list only. When true, ask the daemon for rows that are past its configured episodic TTL (resolves the cutoff server-side using `episodic_ttl_days`). An explicit `until` wins."},
                "day":      {"type": "string", "description": "verb=list only. One local calendar day, YYYY-MM-DD — sugar over since/until covering exactly that day. The remember stage lists a single day's worklist with this."},
                "pending_only": {"type": "boolean", "description": "verb=days only. Return only days awaiting a remember pass, oldest first — the dream worklist."},
                "kind":     {"type": "string", "enum": ["cited", "marker", "subject"], "description": "verb=chains only. `cited` (default) = id-citation chains, auto-accept quality; `marker` = provisional-state candidates, confirm before merging; `subject` = same-subject clusters (3+ rows) for the digest pass — one focused per-subject row, never a mega row."},
                "derived_only": {"type": "boolean", "description": "verb=chains only. Only clusters mergeable unattended (every row from=derived, tier=semantic). Unattended condense passes MUST set true."},
                "sort":     {"type": "string", "enum": ["newest", "oldest"], "description": "verb=list only. Defaults to newest."},
                "limit":    {"type": "integer", "description": "Max rows. Defaults to 10 for search, 50 for list. For verb=chains: clusters per page (default 10)."},
                "offset":   {"type": "integer", "description": "verb=list / verb=chains. Skip this many rows (or clusters) in sort order."}
            },
            "required": ["verb"]
        })
    }
    fn legacy_schema_entry(&self) -> Value {
        json!({
            "name": "Memory_query",
            "args": {"verb": "string", "query": "string?", "id": "string?", "tier": "string?", "limit": "integer?"},
            "returns": "array of rows | single row | error envelope",
            "notes": "Verb-dispatched read. See args_schema for the full filter list."
        })
    }

    async fn execute(&self, tools: &Tools, call: ToolCall) -> Result<ToolResult> {
        dispatch_memory_bounded(tools, "Memory_query", call.args).await
    }
}

pub struct MemoryWriteTool;

#[async_trait]
impl Tool for MemoryWriteTool {
    fn name(&self) -> &'static str {
        "Memory_write"
    }
    fn description(&self) -> &'static str {
        "Modify memory. Verb-dispatched: `add` (insert a new row), `update` (edit fields of an existing row by id), `delete` (hard-delete a single row by id), `remember_day` (stamp a day judged after a remember pass — pass `date` + `judged`/`promoted` counts), `sweep` (the forget stage: mechanically evict episodic rows that are past TTL, on a remembered day, and were judged; never touches un-judged rows — safe anytime). **Follow `[memory_protocol]` in your system prompt** — every `add` MUST be preceded by a `Memory_query`, dups are skipped, contradictions go through `AskUser`. Memory should grow with genuinely durable signal: cross-project user identity / goals (`fact`), commitment-language behavioral rules (`preference`), decisions whose reasoning is the retrieval value (`decision`), cross-project tech gotchas (`learned`). Project-internal architecture, conventions, and implementation detail stay OUT of core/semantic — stage them in `tier=episodic` instead (episodic is staging, not user-biography; the dream pass decides if any earn a curated row). Memory does NOT write to project files (`<project>/AGENTS.md`, `CLAUDE.md`, source, docs); those are user-curated, and the agent reads them directly when needed. Reserve `verb=update` for mechanical rephrasing of the same fact and `verb=delete` for explicit user requests to forget (or for the post-AskUser resolution of a contradiction). Bulk forget is not on this tool surface — handle it via the dashboard or by iterating verb=delete after explicit user confirmation."
    }
    fn tier(&self) -> PermissionMode {
        PermissionMode::Chat
    }
    fn args_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "verb":          {"type": "string", "enum": ["add", "update", "delete", "remember_day", "harvest_day", "sweep"], "description": "Write operation. `harvest_day` stamps a day scanned (a session backfill covered it) WITHOUT marking it remembered — its staged rows go pending for the next dream pass."},
                "id":            {"type": "string", "description": "Required for verb=update / verb=delete. The row UUID."},
                "date":          {"type": "string", "description": "Required for verb=remember_day / verb=harvest_day. The local calendar day, YYYY-MM-DD. Only past days are accepted."},
                "judged":        {"type": "integer", "description": "verb=remember_day. Rows judged in this pass (accumulates onto the day's total)."},
                "promoted":      {"type": "integer", "description": "verb=remember_day. Rows promoted to semantic in this pass (accumulates)."},
                "dry_run":       {"type": "boolean", "description": "verb=sweep. Report what would be evicted without deleting."},
                "content":       {"type": "string", "description": "verb=add/update. The fact text the model will see when the row is recalled."},
                "type":          {"type": "string", "enum": ["fact", "preference", "decision", "tried", "fixed", "learned", "built"], "description": "verb=add/update. Fact category. Default `fact`."},
                "from":          {"type": "string", "enum": ["user", "agent", "derived"], "description": "verb=add/update. Origin of the fact. Default `derived`."},
                "tier":          {"type": "string", "enum": ["core", "semantic", "episodic"], "description": "verb=add/update. Destination memory category. `episodic` = per-turn working capture (fast, append-only, no query-first; the nightly dream pass promotes what's durable and deletes nothing — the separate forget sweep evicts judged rows past TTL) — the default lane for uncertain-durability signal; capture here each turn. `semantic` = curated durable pool (query-first). `core` = tiny always-injected universals about the person (query-first)."},
                "contexts":      {"type": "array", "items": {"type": "string"}, "description": "verb=add/update. Scope tags (e.g. `cross-project`, `project/foo`)."},
                "outcome":       {"type": "string", "enum": ["positive", "negative", "neutral"], "description": "verb=add/update. Optional outcome marker."},
                "occurred_at":   {"type": "string", "description": "verb=add/update. RFC-3339 user-event timestamp; falls back to `created_at` if unset."},
                "source_session":{"type": "string", "description": "verb=add/update. Session id that authored this content. The engine fills the current session id when omitted — pass explicitly ONLY to carry forward an original row's session (the dream promote path)."},
                "user_directed": {"type": "boolean", "description": "verb=add/update. Assert that the user's CURRENT message states this change as SETTLED: a command (\"update X to Y\", \"forget X\"), a declaration (\"my X is now Y\"), or a commitment (\"from now on, X\"). A hedged reflection (\"X feels about right to me\", \"I think I prefer X\") does NOT qualify — that's a contradiction: AskUser first. Required to replace/rewrite from=user rows without an AskUser — the engine blocks such writes otherwise. Never set from your own inference."},
                "replace_ids":   {"type": "array", "items": {"type": "string"}, "description": "verb=add only. **Atomic contradiction resolution.** Pass the ids of every conflicting prior row the user picked against via AskUser. The daemon inserts the new row AND deletes every id in this list in the same call — both tables are searched, you don't need to know each loser's tier. Use this whenever you're resolving a same-subject conflict. Never call add then delete separately for resolution."}
            },
            "required": ["verb"]
        })
    }
    fn legacy_schema_entry(&self) -> Value {
        json!({
            "name": "Memory_write",
            "args": {"verb": "string", "content": "string?", "id": "string?", "tier": "string?"},
            "returns": "row | { ok: true } | error envelope",
            "notes": "Verb-dispatched write. Follow [memory_protocol]: query-first, AskUser on conflict, replace_ids for atomic resolution."
        })
    }

    async fn execute(&self, tools: &Tools, call: ToolCall) -> Result<ToolResult> {
        dispatch_memory_bounded(tools, "Memory_write", call.args).await
    }
}

/// How long an answered AskUser unlocks user-voice writes. Generous for
/// the ask→resolve flow (always seconds apart), tight enough that a
/// stale ask from earlier work doesn't legitimize an unrelated replace.
const ASK_USER_UNLOCK_WINDOW: Duration = Duration::from_secs(600);

/// Mechanical enforcement of the merge law's user-voice floor: a write
/// that replaces or rewrites `from=user` rows goes through only when
/// (a) the model asserts `user_directed: true` — the user's current
/// message explicitly commanded the change ("update X to Y",
/// "forget X", a stated new rule), or (b) an AskUser was answered
/// within [`ASK_USER_UNLOCK_WINDOW`]. Everything else is the silent
/// drifted overwrite the protocol forbids — blocked with a recovery
/// path in the error. Derived rows (the agent's own notes) pass free.
///
/// The daemon enforces the same floor for every frontend and honors a
/// wire-level `user_directed`. The engine is the authority on its own
/// path: the model's raw assertion is stripped, and the flag is
/// re-asserted on the wire only when this guard authorizes the write —
/// including the AskUser-unlock case the daemon cannot see.
async fn guard_user_voice_replace(
    tools: &Tools,
    ling_mem_url: &str,
    args: &mut Value,
) -> Result<()> {
    let Some(obj) = args.as_object_mut() else {
        return Ok(());
    };
    // Strip the model's raw assertion — the engine decides what goes on
    // the wire, not the model.
    let user_directed = obj
        .remove("user_directed")
        .and_then(|v| v.as_bool().into())
        .flatten()
        .unwrap_or(false);

    let verb = obj.get("verb").and_then(|v| v.as_str()).unwrap_or("");
    let mut target_ids: Vec<String> = Vec::new();
    match verb {
        "add" => {
            if let Some(ids) = obj.get("replace_ids").and_then(|v| v.as_array()) {
                target_ids.extend(ids.iter().filter_map(|v| v.as_str().map(String::from)));
            }
        }
        // A content rewrite of an existing row is the same violation
        // class as a replace. Metadata-only updates (tier, contexts,
        // tags) stay unguarded — they don't change what the row says.
        "update" if obj.contains_key("content") => {
            if let Some(id) = obj.get("id").and_then(|v| v.as_str()) {
                target_ids.push(id.to_string());
            }
        }
        _ => {}
    }
    if target_ids.is_empty() {
        return Ok(());
    }
    if user_directed || tools.ask_user_recently(ASK_USER_UNLOCK_WINDOW) {
        // Authorized — assert the flag on the wire so the daemon's own
        // floor lets the write through.
        obj.insert("user_directed".to_string(), Value::Bool(true));
        return Ok(());
    }

    // Resolve each target's voice. A fetch miss is not the guard's
    // problem — the daemon will report the missing id on the real call.
    let mut offending: Vec<String> = Vec::new();
    for id in &target_ids {
        let row = call_memory_http(
            ling_mem_url,
            "Memory_query",
            json!({"verb": "get", "id": id}),
        )
        .await;
        if let Ok(row) = row {
            if row.get("from").and_then(|v| v.as_str()) == Some("user") {
                let gist: String = row
                    .get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .chars()
                    .take(60)
                    .collect();
                offending.push(format!("{id} \"{gist}\""));
            }
        }
    }
    if offending.is_empty() {
        return Ok(());
    }

    anyhow::bail!(
        "BLOCKED by the user-voice merge guard: this write replaces/rewrites rows in the USER'S VOICE (from=user): {}. The user's voice changes only with the user (merge law). Recovery — pick ONE: (1) if the user's CURRENT message states this change as SETTLED — a command (\"update X to Y\", \"forget X\"), a declaration (\"my X is now Y\"), or a commitment (\"from now on, X\") — retry the exact same call with \"user_directed\": true; a hedged reflection (\"X feels about right to me\") does NOT qualify; (2) otherwise call AskUser to let the user choose, then retry after their answer (an answered AskUser unlocks this guard). Do NOT work around it by writing a new row without replace_ids — that creates the drift this guard exists to prevent.",
        offending.join(", ")
    );
}

// ---------------------------------------------------------------------------
// HTTP dispatch — POST to <ling_mem_url>/api/memory/<verb>
// ---------------------------------------------------------------------------

/// Timeout shell around [`dispatch_memory`] — the model-facing entry for
/// both Memory_* tools. A timed-out write's store outcome is unknown
/// (the daemon may have applied it after we stopped waiting), so the
/// error tells the model to re-query before retrying.
async fn dispatch_memory_bounded(
    tools: &Tools,
    tool_name: &'static str,
    args: Value,
) -> Result<ToolResult> {
    match tokio::time::timeout(MEMORY_TOOL_TIMEOUT, dispatch_memory(tools, tool_name, args)).await
    {
        Ok(result) => result,
        Err(_) => anyhow::bail!(
            "{tool_name} timed out after {}s and was abandoned. The store may or may not \
             have applied it — Memory_query first to check, then retry once if needed.",
            MEMORY_TOOL_TIMEOUT.as_secs()
        ),
    }
}

/// Tool-trait wrapper: read `ling_mem_url` from the engine config and
/// return the result as a `ToolResult::Success(json_string)`. The model
/// only sees what the daemon returned in the `data` field of its
/// `{ok, data}` envelope.
async fn dispatch_memory(
    tools: &Tools,
    tool_name: &'static str,
    mut args: Value,
) -> Result<ToolResult> {
    let manager = tools.get_manager().ok_or_else(|| {
        anyhow!("{tool_name} requires a running AgentManager context — tool context not set")
    })?;

    // Per-skill memory isolation: when this session is bound to a skill that
    // declares `memory_context`, FORCE that scope tag onto every read filter
    // and write — so a focused app (e.g. CFO ↔ "cfo") only ever sees/writes
    // its own namespace, never the shared cross-app store, regardless of what
    // `contexts` the model passed. Skills without `memory_context` (e.g. Pulse)
    // are unaffected and keep full-store access.
    if let Some(sid) = tools.session_id.clone() {
        if let Some(meta) = manager
            .global_sessions
            .get_session_meta(&sid)
            .ok()
            .flatten()
        {
            if let Some(skill_name) = meta.skill {
                if let Some(skill) = manager.skills.reload_one(&skill_name).await {
                    if let Some(ctx) = skill.memory_context.filter(|c| !c.trim().is_empty()) {
                        if let Some(obj) = args.as_object_mut() {
                            obj.insert(
                                "contexts".to_string(),
                                Value::Array(vec![Value::String(ctx)]),
                            );
                        }
                    }
                }
            }
        }
    }

    // Stamp the writing session on writes so scan's skip-by-session
    // idempotency is real (memory-spec Open#9). Only when the model didn't
    // supply one — the dream promote path carries the ORIGINAL row's
    // source_session forward, which must win. Session-scoped by design:
    // sessionless callers of call_memory_http (auto-recall, admin
    // dispatch) have no authoring session to stamp.
    if tool_name == "Memory_write" && !args.get("source_session").is_some_and(|v| !v.is_null()) {
        if let (Some(sid), Some(obj)) = (&tools.session_id, args.as_object_mut()) {
            obj.insert("source_session".to_string(), Value::String(sid.clone()));
        }
    }

    let ling_mem_url = manager.get_config_snapshot().await.agent.ling_mem_url;

    // User-voice merge guard — the merge law, enforced mechanically.
    // Prompt hardening alone does not stop models from silently replacing
    // `from=user` rows (measured by the write-side eval); a filter does.
    if tool_name == "Memory_write" {
        guard_user_voice_replace(tools, &ling_mem_url, &mut args).await?;
    }
    let value = call_memory_http(&ling_mem_url, tool_name, args).await?;
    Ok(ToolResult::Success(value.to_string()))
}

/// Public entry point — POSTs `args` to `<ling_mem_url>/api/memory/<verb>`
/// (verb taken from `args["verb"]` and stripped) and returns the daemon's
/// `data` payload on success. Handles soft-empty cleanup, the
/// `tier=episodic` → `episodic=true` wire translation, the default
/// `host=linggen` on writes, and one autostart retry on
/// `ConnectionRefused`/`Timeout`.
///
/// Used by the Tool impls above and by direct callers (auto-recall in
/// the chat runtime, the `/api/tool-dispatch` admin endpoint) that need
/// to talk to ling-mem outside an engine session.
pub async fn call_memory_http(
    ling_mem_url: &str,
    tool_name: &str,
    mut args: Value,
) -> Result<Value> {
    let ling_mem_url = ling_mem_url.trim_end_matches('/');

    // Verb → endpoint. Strip the verb from the body so the daemon sees
    // the same shape it accepts from the CLI.
    let verb = args
        .get("verb")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("{tool_name}: `verb` is required"))?
        .to_string();
    if let Some(obj) = args.as_object_mut() {
        obj.remove("verb");
    }
    let url = format!("{ling_mem_url}/api/memory/{verb}");

    // Drop soft-empty fields the model often fills in. `until: ""` would
    // otherwise crash the daemon's RFC-3339 parse; empty arrays narrow
    // unintentionally; nulls are noise.
    if let Some(obj) = args.as_object_mut() {
        obj.retain(|_, v| match v {
            Value::String(s) => !s.is_empty(),
            Value::Array(a) => !a.is_empty(),
            Value::Null => false,
            _ => true,
        });
    }

    // tier="episodic" → episodic=true on the wire (daemon splits the
    // episodic store into a separate table). Other tier values are kept
    // as filters within the semantic store.
    if let Some(obj) = args.as_object_mut() {
        if let Some(tier) = obj.get("tier").and_then(|v| v.as_str()) {
            if tier == "episodic" {
                obj.insert("episodic".to_string(), Value::Bool(true));
                obj.remove("tier");
            }
        }
        if tool_name == "Memory_write" && !obj.contains_key("host") {
            obj.insert("host".to_string(), Value::String("linggen".to_string()));
        }
    }

    // TTL-sweep guard: when the caller is doing a `past_ttl: true` list
    // (the dream consolidator's worklist query), strip `type`, `from`,
    // and `outcome` filters. Observed failure mode: `gpt-5.5` ignores
    // the schema's "DEFAULT: do not pass" hints and fills these in with
    // arbitrary defaults (`type=fact, from=derived, outcome=neutral`),
    // which over-constrains the query to 0 rows. The dream is a
    // bulk-eviction sweep — it wants every past-TTL episodic row
    // regardless of type/origin/outcome. Keep `contexts` intact for
    // callers that legitimately scope by tag.
    if let Some(obj) = args.as_object_mut() {
        let is_ttl_sweep = obj.get("past_ttl").and_then(|v| v.as_bool()).unwrap_or(false);
        if is_ttl_sweep {
            for k in ["type", "from", "outcome"] {
                if obj.remove(k).is_some() {
                    tracing::debug!("memory_tool: dropped over-constraining `{k}` on past_ttl sweep");
                }
            }
        }
    }

    let args_preview = serde_json::to_string(&args).unwrap_or_else(|_| "<unserializable>".into());
    let args_preview = if args_preview.len() > 200 {
        format!("{}…", &args_preview[..199])
    } else {
        args_preview
    };
    tracing::info!("memory_tool dispatch → POST {url} body={args_preview}");

    let mut value = match post_once(&url, &args).await {
        Ok(v) => v,
        Err(DispatchError::NoDaemon) => {
            autostart()
                .await
                .with_context(|| format!("autostarting ling-mem after first attempt to {url} failed"))?;
            post_once(&url, &args).await.map_err(anyhow::Error::from)?
        }
        Err(DispatchError::Other(e)) => return Err(e),
    };

    // Deleting an already-absent row is success, not an anomaly — the row is
    // gone either way (commonly the daemon's cross-tier dedup removed the
    // episodic copy during a promote add). A bare `removed:false` reads as
    // an error signal to LLM callers (observed: three dream runs aborted
    // claiming "store inconsistency" over it), so say what it means.
    if verb == "delete" {
        if let Some(obj) = value.as_object_mut() {
            if obj.get("removed").and_then(|v| v.as_bool()) == Some(false) {
                obj.insert("already_gone".to_string(), Value::Bool(true));
                obj.insert(
                    "note".to_string(),
                    Value::String(
                        "row was already absent — treat as success; do not retry or verify"
                            .to_string(),
                    ),
                );
            }
        }
    }

    let summary = match &value {
        Value::Array(a) => format!("array len={}", a.len()),
        Value::Object(o) => {
            let n = o.get("rows").and_then(|v| v.as_array()).map(|a| a.len());
            let err = o.get("error").and_then(|v| v.as_str());
            match (n, err) {
                (_, Some(e)) => format!("error={e}"),
                (Some(n), _) => format!("rows={n}"),
                _ => format!("object keys={:?}", o.keys().collect::<Vec<_>>()),
            }
        }
        Value::Null => "null".to_string(),
        _ => "scalar".to_string(),
    };
    tracing::info!("memory_tool dispatch ← {tool_name}: {summary}");

    Ok(value)
}

#[derive(Debug)]
enum DispatchError {
    /// Daemon isn't reachable — autostart + retry.
    NoDaemon,
    /// Anything else — surface to the model.
    Other(anyhow::Error),
}

impl From<DispatchError> for anyhow::Error {
    fn from(e: DispatchError) -> Self {
        match e {
            DispatchError::NoDaemon => anyhow!(
                "ling-mem daemon is not reachable after autostart — check `ling-mem status`"
            ),
            DispatchError::Other(e) => e,
        }
    }
}

async fn post_once(url: &str, args: &Value) -> Result<Value, DispatchError> {
    let client = reqwest::Client::builder()
        .timeout(DISPATCH_TIMEOUT)
        .build()
        .map_err(|e| DispatchError::Other(anyhow!(e)))?;

    let response = match client.post(url).json(args).send().await {
        Ok(r) => r,
        Err(e) if e.is_connect() || e.is_timeout() => return Err(DispatchError::NoDaemon),
        Err(e) => {
            return Err(DispatchError::Other(
                anyhow::Error::from(e).context(format!("POST {url} failed")),
            ));
        }
    };

    let status = response.status();
    if !status.is_success() {
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| "<could not read body>".to_string());
        let trimmed = body.trim();
        return Err(DispatchError::Other(anyhow!(
            "ling-mem error [{}]: {}",
            status.as_u16(),
            if trimmed.is_empty() { "<empty body>" } else { trimmed }
        )));
    }

    let envelope: Value = response
        .json()
        .await
        .map_err(|e| DispatchError::Other(anyhow::Error::from(e).context("parsing daemon response as JSON")))?;
    parse_envelope(envelope).map_err(DispatchError::Other)
}

fn parse_envelope(envelope: Value) -> Result<Value> {
    let obj = envelope
        .as_object()
        .ok_or_else(|| anyhow!("daemon response is not a JSON object: {envelope}"))?;
    match obj.get("ok").and_then(|v| v.as_bool()) {
        Some(true) => Ok(obj.get("data").cloned().unwrap_or(Value::Null)),
        Some(false) => {
            let msg = obj.get("error").and_then(|v| v.as_str()).unwrap_or("unknown error");
            let code = obj.get("code").and_then(|v| v.as_str());
            match code {
                Some(c) => Err(anyhow!("ling-mem error [{c}]: {msg}")),
                None => Err(anyhow!("ling-mem error: {msg}")),
            }
        }
        None => Err(anyhow!("daemon response missing `ok` field: {envelope}")),
    }
}

/// Parse `<bin> --version` ("ling-mem X.Y.Z") into a comparable tuple.
fn ling_mem_version(path: &std::path::Path) -> Option<(u32, u32, u32)> {
    let out = std::process::Command::new(path).arg("--version").output().ok()?;
    if !out.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v = stdout.split_whitespace().nth(1)?;
    let mut it = v.trim().split('.');
    let major = it.next()?.parse().ok()?;
    let minor = it.next()?.parse().ok()?;
    let patch = it.next().and_then(|p| p.parse().ok()).unwrap_or(0);
    Some((major, minor, patch))
}

/// Resolve the `ling-mem` binary to the **highest-version** copy among the
/// `$PATH` hit and the two installer dirs (`~/.local/bin`, `/usr/local/bin`).
/// Picking by version — not first-on-PATH — avoids starting a stale copy when
/// a default PATH happens to shadow a newer one with an older `/usr/local/bin`
/// (a real multi-host skew: different installers drop different versions).
/// `None` if no usable binary is found.
fn resolve_ling_mem() -> Option<std::path::PathBuf> {
    let mut candidates: Vec<std::path::PathBuf> = Vec::new();
    if let Ok(out) = std::process::Command::new("which").arg("ling-mem").output() {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !s.is_empty() {
                candidates.push(std::path::PathBuf::from(s));
            }
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        candidates.push(std::path::PathBuf::from(home).join(".local/bin/ling-mem"));
    }
    candidates.push(std::path::PathBuf::from("/usr/local/bin/ling-mem"));

    candidates.retain(|p| p.is_file());
    candidates.sort();
    candidates.dedup();
    candidates
        .into_iter()
        .filter_map(|p| ling_mem_version(&p).map(|v| (v, p)))
        .max_by_key(|(v, _)| *v)
        .map(|(_, p)| p)
}

/// Fetch the canonical binary-only installer and run it (`bash -s`), pinned to
/// [`LING_MEM_PIN`]. The engine owns the dependency — a fresh marketplace
/// install ships skill files but no binary, and nothing else would install it
/// for the HTTP memory path.
async fn install_ling_mem() -> Result<()> {
    let pin = std::env::var("LING_MEM_VERSION").unwrap_or_else(|_| LING_MEM_PIN.to_string());
    tracing::info!("ling-mem binary not found — installing {pin} via install-bin.sh");

    let script = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| anyhow!(e))?
        .get(INSTALL_BIN_URL)
        .send()
        .await
        .context("fetching install-bin.sh")?
        .error_for_status()
        .context("install-bin.sh fetch returned non-success")?
        .text()
        .await
        .context("reading install-bin.sh body")?;

    let mut child = tokio::process::Command::new("bash")
        .arg("-s")
        .arg("--")
        .arg("--version")
        .arg(&pin)
        .arg("--quiet")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .context("spawning bash to run install-bin.sh")?;

    {
        use tokio::io::AsyncWriteExt;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("could not open stdin to install-bin.sh"))?;
        stdin
            .write_all(script.as_bytes())
            .await
            .context("piping install-bin.sh to bash")?;
    } // drop stdin → EOF so bash runs

    let out = tokio::time::timeout(Duration::from_secs(120), child.wait_with_output())
        .await
        .map_err(|_| anyhow!("install-bin.sh did not complete within 120s"))?
        .context("waiting for install-bin.sh")?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(anyhow!("install-bin.sh failed: {}", err.trim()));
    }
    Ok(())
}

/// Snapshot the store before a memory-agent mission run. Condense
/// merges retire rows atomically (`replace_ids` has no undo), so every
/// run day gets one ndjson export under `~/.linggen/memory/backups/`,
/// pruned to the last 7. Best-effort by design: a failed backup logs
/// and the run proceeds — the unattended condense stage is already
/// capped to pre-confirmed cited chains.
pub(crate) async fn backup_store_best_effort() {
    if let Err(e) = try_backup_store().await {
        tracing::warn!("memory store backup failed (run proceeds): {e:#}");
    }
}

async fn try_backup_store() -> Result<()> {
    let bin =
        resolve_ling_mem().ok_or_else(|| anyhow!("ling-mem binary not found for backup"))?;
    let dir = crate::paths::linggen_home().join("memory/backups");
    tokio::fs::create_dir_all(&dir).await?;
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let path = dir.join(format!("store-{today}.ndjson"));
    if tokio::fs::try_exists(&path).await.unwrap_or(false) {
        return Ok(()); // one snapshot per day covers every run that day
    }

    let out = tokio::time::timeout(
        Duration::from_secs(60),
        tokio::process::Command::new(&bin)
            .arg("export")
            .arg(&path)
            .env("LINGGEN_DATA_DIR", crate::paths::linggen_home())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| anyhow!("`ling-mem export` did not complete within 60s"))?
    .context("spawning `ling-mem export`")?;

    if !out.status.success() {
        let _ = tokio::fs::remove_file(&path).await;
        anyhow::bail!(
            "`ling-mem export` failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    tracing::info!("memory store snapshot written: {}", path.display());
    prune_backups(&dir).await;
    Ok(())
}

/// Keep the newest 7 `store-*.ndjson` snapshots — date-named, so
/// lexicographic order is chronological.
async fn prune_backups(dir: &std::path::Path) {
    let Ok(mut rd) = tokio::fs::read_dir(dir).await else {
        return;
    };
    let mut files: Vec<std::path::PathBuf> = Vec::new();
    while let Ok(Some(entry)) = rd.next_entry().await {
        let p = entry.path();
        let is_snapshot = p
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("store-") && n.ends_with(".ndjson"));
        if is_snapshot {
            files.push(p);
        }
    }
    files.sort();
    while files.len() > 7 {
        let _ = tokio::fs::remove_file(files.remove(0)).await;
    }
}

/// Ensure the binary exists (installing it if missing) and run `<bin> start`.
/// Idempotent — `start` while the daemon is already up exits 0.
pub(crate) async fn autostart() -> Result<()> {
    let bin = match resolve_ling_mem() {
        Some(p) => p,
        None => {
            install_ling_mem()
                .await
                .context("ling-mem binary missing and auto-install failed")?;
            resolve_ling_mem()
                .ok_or_else(|| anyhow!("ling-mem still not found after auto-install"))?
        }
    };

    let output = tokio::time::timeout(
        AUTOSTART_TIMEOUT,
        tokio::process::Command::new(&bin)
            .arg("start")
            .env("LINGGEN_DATA_DIR", crate::paths::linggen_home())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| {
        anyhow!(
            "`{} start` did not complete within {}s",
            bin.display(),
            AUTOSTART_TIMEOUT.as_secs()
        )
    })?
    .with_context(|| format!("spawning `{} start`", bin.display()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let trimmed = stderr.trim();
        return Err(anyhow!(
            "`{} start` exited with status {}{}",
            bin.display(),
            output.status,
            if trimmed.is_empty() {
                String::new()
            } else {
                format!(": {trimmed}")
            }
        ));
    }
    Ok(())
}
