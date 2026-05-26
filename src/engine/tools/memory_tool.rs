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
//! - On `ConnectionRefused` or timeout, autostart `ling-mem start` and
//!   retry once. The daemon is idempotent — re-running `start` while up
//!   exits 0.
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

const AUTOSTART_CMD: &[&str] = &["ling-mem", "start"];

pub struct MemoryQueryTool;

#[async_trait]
impl Tool for MemoryQueryTool {
    fn name(&self) -> &'static str {
        "Memory_query"
    }
    fn description(&self) -> &'static str {
        "Read memory. Verb-dispatched: `get` (fetch one row by id), `search` (semantic search; ranked by relevance), `list` (filter-only browse, no semantic ranking — for audits or exact enumeration). Memory is the user's biography across sessions — durable identity, cross-project preferences, decisions with their reasoning, life context. Project-internal facts (code architecture, repo conventions) are NOT in memory — the agent reads the project's own files (source, the user's `AGENTS.md` / `CLAUDE.md` if any) directly when it needs that content.\n\n**All filters are optional and AND-combined; omit anything you aren't intentionally narrowing on.** Speculatively passing `from`, `outcome`, or a specific `type` is the #1 cause of empty results — most rows don't carry an `outcome`, and the user's actual data may not match the value you guessed. When unsure, start with just `verb` (+ `query` for search) and add filters only after you see what's there."
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
                "verb":     {"type": "string", "enum": ["get", "search", "list"], "description": "Read operation."},
                "id":       {"type": "string", "description": "Required for verb=get. Fact UUID."},
                "query":    {"type": "string", "description": "Required for verb=search. Natural-language description of what you're looking for."},
                "contexts": {"type": "array", "items": {"type": "string"}, "description": "Filter to these scope tags (AND semantics). For verb=search, narrows ranked results; for verb=list, primary filter. Omit to skip."},
                "type":     {"type": "string", "enum": ["fact", "preference", "decision", "tried", "fixed", "learned", "built"], "description": "Filter by fact type. Omit to return all types."},
                "tier":     {"type": "string", "enum": ["core", "semantic", "episodic"], "description": "Memory category. `core` = always-on identity/style universals; `semantic` = curated retrieval pool; `episodic` = staging table (encoder writes here pre-promotion). **Omit to span all three.**"},
                "from":     {"type": "string", "enum": ["user", "agent", "derived"], "description": "**DEFAULT: do not pass.** Filter by origin. Pass only when the user explicitly asked to see rows from a specific origin (rare)."},
                "outcome":  {"type": "string", "enum": ["positive", "negative", "neutral"], "description": "**DEFAULT: do not pass.** Filter by outcome. Almost no rows have `outcome=neutral`; passing it returns 0 rows even when the store has data. Pass only when the user explicitly asked to see only positive / negative outcomes."},
                "since":    {"type": "string", "description": "RFC-3339 lower bound on effective timestamp. Omit to skip."},
                "until":    {"type": "string", "description": "RFC-3339 upper bound (verb=list only). Omit to skip."},
                "past_ttl": {"type": "boolean", "description": "verb=list only. When true, ask the daemon for rows that are past its configured episodic TTL (resolves the cutoff server-side using `episodic_ttl_days`). Used by the dream consolidator so the mission body doesn't have to know the TTL value. An explicit `until` wins."},
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
        "Modify memory. Verb-dispatched: `add` (insert a new row), `update` (edit fields of an existing row by id), `delete` (hard-delete a single row by id). **Follow `[memory_protocol]` in your system prompt** — every `add` MUST be preceded by a `Memory_query`, dups are skipped, contradictions go through `AskUser`. Memory should grow with genuinely durable signal: cross-project user identity / goals (`fact`), commitment-language behavioral rules (`preference`), decisions whose reasoning is the retrieval value (`decision`), cross-project tech gotchas (`learned`). Don't store project-internal architecture, conventions, or implementation detail — drop those candidates entirely. Memory does NOT write to project files (`<project>/AGENTS.md`, `CLAUDE.md`, source, docs); those are user-curated, and the agent reads them directly when needed. Reserve `verb=update` for mechanical rephrasing of the same fact and `verb=delete` for explicit user requests to forget (or for the post-AskUser resolution of a contradiction). Bulk forget is not on this tool surface — handle it via the dashboard or by iterating verb=delete after explicit user confirmation."
    }
    fn tier(&self) -> PermissionMode {
        PermissionMode::Chat
    }
    fn args_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "verb":          {"type": "string", "enum": ["add", "update", "delete"], "description": "Write operation."},
                "id":            {"type": "string", "description": "Required for verb=update / verb=delete. The row UUID."},
                "content":       {"type": "string", "description": "verb=add/update. The fact text the model will see when the row is recalled."},
                "type":          {"type": "string", "enum": ["fact", "preference", "decision", "tried", "fixed", "learned", "built"], "description": "verb=add/update. Fact category. Default `fact`."},
                "from":          {"type": "string", "enum": ["user", "agent", "derived"], "description": "verb=add/update. Origin of the fact. Default `derived`."},
                "tier":          {"type": "string", "enum": ["core", "semantic", "episodic"], "description": "verb=add/update. Destination memory category. `core` = always-injected identity/style universals (keep tiny); `semantic` (default) = curated retrieval pool; `episodic` = staging table (per-session encoder + dream consolidator write here; live agent saves stay on `semantic`)."},
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

async fn dispatch_memory(
    tools: &Tools,
    tool_name: &'static str,
    mut args: Value,
) -> Result<ToolResult> {
    // Resolve ling_mem_url from the live config snapshot — config edits
    // (Settings → General → Ling-mem URL) reach the next dispatch.
    let manager = tools.get_manager().ok_or_else(|| {
        anyhow!(
            "{tool_name} requires a running AgentManager context — tool context not set"
        )
    })?;
    let ling_mem_url = manager.get_config_snapshot().await.agent.ling_mem_url;
    let ling_mem_url = ling_mem_url.trim_end_matches('/').to_string();

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
    let endpoint = format!("/api/memory/{verb}");
    let url = format!("{ling_mem_url}{endpoint}");

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

    let args_preview = serde_json::to_string(&args).unwrap_or_else(|_| "<unserializable>".into());
    let args_preview = if args_preview.len() > 200 {
        format!("{}…", &args_preview[..199])
    } else {
        args_preview
    };
    tracing::info!("memory_tool dispatch → POST {url} body={args_preview}");

    let value = match post_once(&url, &args).await {
        Ok(v) => v,
        Err(DispatchError::NoDaemon) => {
            autostart()
                .await
                .with_context(|| format!("autostarting ling-mem after first attempt to {url} failed"))?;
            post_once(&url, &args)
                .await
                .map_err(anyhow::Error::from)?
        }
        Err(DispatchError::Other(e)) => return Err(e),
    };

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

    Ok(ToolResult::Success(value.to_string()))
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

/// Spawn `ling-mem start` and wait. The subprocess is idempotent —
/// running it when the daemon is already up exits 0.
async fn autostart() -> Result<()> {
    let binary = std::path::PathBuf::from(AUTOSTART_CMD[0]);
    let args: Vec<&str> = AUTOSTART_CMD[1..].to_vec();
    let output = tokio::time::timeout(
        AUTOSTART_TIMEOUT,
        tokio::process::Command::new(&binary)
            .args(&args)
            .env("LINGGEN_DATA_DIR", crate::paths::linggen_home())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| {
        anyhow!(
            "`{} {}` did not complete within {}s",
            binary.display(),
            args.join(" "),
            AUTOSTART_TIMEOUT.as_secs()
        )
    })?
    .with_context(|| format!("spawning `{} {}`", binary.display(), args.join(" ")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let trimmed = stderr.trim();
        return Err(anyhow!(
            "`{} {}` exited with status {}{}",
            binary.display(),
            args.join(" "),
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
