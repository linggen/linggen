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
        "Read memory. Verb-dispatched: `get` (fetch one row by id), `search` (semantic search; ranked by relevance — takes `query`), `list` (filter-only browse, no ranking — for audits or sweeps), `days` (per-day dream-state rollup: each day's episodic counts + pipeline state today/staging/pending/remembered/forgotten; `pending_only` = the dream worklist, oldest first). Memory holds the user's biography across sessions — identity, cross-project preferences, decisions with their reasoning. For codebase facts, read project files directly.\n\n**All filters are optional and AND-combined.** Speculatively passing `type`, `from`, or `outcome` is the #1 cause of empty results — start with just `verb` (+ `query` for search) and add filters only after seeing what's there.\n\n**Example — the dream worklist (days awaiting a remember pass):**\n```\n{ \"verb\": \"days\", \"pending_only\": true }\n```\n**Example — one day's remember worklist:**\n```\n{ \"verb\": \"list\", \"tier\": \"episodic\", \"day\": \"2026-07-01\", \"limit\": 50 }\n```\nNo `type`/`from`/`outcome` — those would narrow the sweep to zero rows."
    }
    fn tier(&self) -> PermissionMode {
        // Memory ops are conversation primitives — the user's own data
        // being saved on their behalf, not workspace mutations. Pin at
        // Chat so every session tier can use them without a permission
        // prompt.
        PermissionMode::Chat
    }
    fn args_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "verb":     {"type": "string", "enum": ["get", "search", "list", "days"], "description": "Read operation."},
                "id":       {"type": "string", "description": "Required for verb=get. Fact UUID."},
                "query":    {"type": "string", "description": "Required for verb=search. Natural-language description of what you're looking for."},
                "contexts": {"type": "array", "items": {"type": "string"}, "description": "Filter to these scope tags (AND semantics). For verb=search, narrows ranked results; for verb=list, primary filter. Omit to skip."},
                "type":     {"type": "string", "enum": ["fact", "preference", "decision", "tried", "fixed", "learned", "built"], "description": "Filter by fact type. Omit to return all types."},
                "tier":     {"type": "string", "enum": ["core", "semantic", "episodic"], "description": "Memory category. `core` = always-on identity/style universals; `semantic` = curated retrieval pool; `episodic` = staging table (encoder writes here pre-promotion). **Omit to span all three.**"},
                "from":     {"type": "string", "enum": ["user", "agent", "derived"], "description": "**DEFAULT: do not pass.** Filter by origin. Pass only when the user explicitly asked to see rows from a specific origin (rare)."},
                "outcome":  {"type": "string", "enum": ["positive", "negative", "neutral"], "description": "**DEFAULT: do not pass.** Filter by outcome. Almost no rows have `outcome=neutral`; passing it returns 0 rows even when the store has data. Pass only when the user explicitly asked to see only positive / negative outcomes."},
                "since":    {"type": "string", "description": "RFC-3339 lower bound on effective timestamp. Omit to skip."},
                "until":    {"type": "string", "description": "RFC-3339 upper bound (verb=list only). Omit to skip."},
                "past_ttl": {"type": "boolean", "description": "verb=list only. When true, ask the daemon for rows that are past its configured episodic TTL (resolves the cutoff server-side using `episodic_ttl_days`). An explicit `until` wins."},
                "day":      {"type": "string", "description": "verb=list only. One local calendar day, YYYY-MM-DD — sugar over since/until covering exactly that day. The remember stage lists a single day's worklist with this."},
                "pending_only": {"type": "boolean", "description": "verb=days only. Return only days awaiting a remember pass, oldest first — the dream worklist."},
                "sort":     {"type": "string", "enum": ["newest", "oldest"], "description": "verb=list only. Defaults to newest."},
                "limit":    {"type": "integer", "description": "Max rows. Defaults to 10 for search, 50 for list."},
                "offset":   {"type": "integer", "description": "verb=list only. Skip this many rows in sort order."}
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
        dispatch_memory(tools, "Memory_query", call.args).await
    }
}

pub struct MemoryWriteTool;

#[async_trait]
impl Tool for MemoryWriteTool {
    fn name(&self) -> &'static str {
        "Memory_write"
    }
    fn description(&self) -> &'static str {
        "Modify memory. Verb-dispatched: `add` (insert a new row), `update` (edit fields of an existing row by id), `delete` (hard-delete a single row by id), `remember_day` (stamp a day judged after a remember pass — pass `date` + `judged`/`promoted` counts), `sweep` (the forget stage: mechanically evict episodic rows that are past TTL, on a remembered day, and were judged; never touches un-judged rows — safe anytime). **Follow `[memory_protocol]` in your system prompt** — every `add` MUST be preceded by a `Memory_query`, dups are skipped, contradictions go through `AskUser`. Memory should grow with genuinely durable signal: cross-project user identity / goals (`fact`), commitment-language behavioral rules (`preference`), decisions whose reasoning is the retrieval value (`decision`), cross-project tech gotchas (`learned`). Don't store project-internal architecture, conventions, or implementation detail — drop those candidates entirely. Memory does NOT write to project files (`<project>/AGENTS.md`, `CLAUDE.md`, source, docs); those are user-curated, and the agent reads them directly when needed. Reserve `verb=update` for mechanical rephrasing of the same fact and `verb=delete` for explicit user requests to forget (or for the post-AskUser resolution of a contradiction). Bulk forget is not on this tool surface — handle it via the dashboard or by iterating verb=delete after explicit user confirmation."
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
                "tier":          {"type": "string", "enum": ["core", "semantic", "episodic"], "description": "verb=add/update. Destination memory category. `episodic` = per-turn working capture (fast, append-only, no query-first; the dream consolidator promotes/evicts) — the default lane for uncertain-durability signal; capture here each turn. `semantic` = curated durable pool (query-first). `core` = tiny always-injected universals about the person (query-first)."},
                "contexts":      {"type": "array", "items": {"type": "string"}, "description": "verb=add/update. Scope tags (e.g. `cross-project`, `project/foo`)."},
                "outcome":       {"type": "string", "enum": ["positive", "negative", "neutral"], "description": "verb=add/update. Optional outcome marker."},
                "occurred_at":   {"type": "string", "description": "verb=add/update. RFC-3339 user-event timestamp; falls back to `created_at` if unset."},
                "source_session":{"type": "string", "description": "verb=add/update. Engine session id that authored this row. The engine fills this on each call; the model usually omits it."},
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
        dispatch_memory(tools, "Memory_write", call.args).await
    }
}

// ---------------------------------------------------------------------------
// HTTP dispatch — POST to <ling_mem_url>/api/memory/<verb>
// ---------------------------------------------------------------------------

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

    let ling_mem_url = manager.get_config_snapshot().await.agent.ling_mem_url;
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
