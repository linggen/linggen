//! The engine's own memory client, for the engine talking to memory as a
//! *program*:
//!
//! - `/apps/<skill>/capability/*`, the door a skill's webpage uses
//! - the dream rollup and the mission scheduler's stats
//! - the core block (`prompt::core_block`)
//!
//! **No model reaches memory through this file.** A model's memory tools are
//! memory's own, discovered over MCP (`mcp_client`, and `memory_mcp` for what
//! the engine adds to such a call). The phone's door (`api::memory`) calls
//! `memory_host` itself: it wants memory's whole envelope, not its `data`.
//!
//! Memory is hosted in this process (`memory_host`), and these calls reach it
//! in-process (`memory_host::post_verb`), on memory's own runtime.
//!
//! What this client adds on the way: the verb taken from `args["verb"]`,
//! soft-empty cleanup, `tier: "episodic"` → `episodic: true`. The same fixes
//! exist in memory's MCP layer for model callers, which is where they belong
//! — models fill arguments sloppily, programs don't. Naming the calling host
//! is each caller's own job, not this function's; see `memory_mcp::HOST`.

use anyhow::{anyhow, Result};
use ling_mem::Who;
use serde_json::Value;
use std::time::Duration;

/// How long a call waits for memory. Matches the old `capability_tools.rs`
/// budget; the phone's door waits as long.
pub const DISPATCH_TIMEOUT: Duration = Duration::from_secs(5);

/// Run the verb `args["verb"]` (stripped from the args) for `who` and return
/// memory's `data` payload, after the clean-ups above.
///
/// `caller` names who is asking, for the error and the log line only — it
/// changes nothing memory sees. Stamping `host` on a row is the caller's own
/// business, and each one does it: the `/mcp` group says `mcp`, a skill's
/// webpage door says `linggen`. A default buried here would have named this
/// engine on behalf of a caller that isn't it.
pub async fn call_memory(who: Who, caller: &str, args: Value) -> Result<Value> {
    let (verb, args) = prepare(caller, args)?;
    let answer = crate::memory_host::post_verb(who, &verb, args, DISPATCH_TIMEOUT).await?;
    finish(caller, &verb, answer)
}

/// [`call_memory`] for a caller that is not async, waiting up to `timeout`:
/// `None` when memory did not answer in time.
pub fn call_memory_blocking(
    who: Who,
    caller: &str,
    args: Value,
    timeout: Duration,
) -> Option<Result<Value>> {
    let (verb, args) = match prepare(caller, args) {
        Ok(prepared) => prepared,
        Err(e) => return Some(Err(e)),
    };
    let answer = crate::memory_host::post_verb_blocking(who, &verb, args, timeout)?;
    Some(answer.and_then(|answer| finish(caller, &verb, answer)))
}

/// The verb, and the args as memory should see them.
fn prepare(caller: &str, mut args: Value) -> Result<(String, Value)> {
    // Strip the verb from the body so memory sees the shape its route takes.
    let verb = args
        .get("verb")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("{caller}: `verb` is required"))?
        .to_string();
    if let Some(obj) = args.as_object_mut() {
        obj.remove("verb");
        drop_soft_empty(obj);
        episodic_as_flag(obj);
        unnarrow_ttl_sweep(obj);
    }
    log_call(&verb, &args);
    Ok((verb, args))
}

/// Drop soft-empty fields the model often fills in. `until: ""` would
/// otherwise crash memory's RFC-3339 parse; empty arrays narrow
/// unintentionally; nulls are noise.
fn drop_soft_empty(obj: &mut serde_json::Map<String, Value>) {
    obj.retain(|_, v| match v {
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Null => false,
        _ => true,
    });
}

/// tier="episodic" → episodic=true (memory splits the episodic store into a
/// separate table). Other tier values are kept as filters within the
/// semantic store.
fn episodic_as_flag(obj: &mut serde_json::Map<String, Value>) {
    if obj.get("tier").and_then(|v| v.as_str()) == Some("episodic") {
        obj.insert("episodic".to_string(), Value::Bool(true));
        obj.remove("tier");
    }
}

/// TTL-sweep guard: when the caller is doing a `past_ttl: true` list
/// (the dream consolidator's worklist query), strip `type`, `from`,
/// and `outcome` filters. Observed failure mode: `gpt-5.5` ignores
/// the schema's "DEFAULT: do not pass" hints and fills these in with
/// arbitrary defaults (`type=fact, from=derived, outcome=neutral`),
/// which over-constrains the query to 0 rows. The dream is a
/// bulk-eviction sweep — it wants every past-TTL episodic row
/// regardless of type/origin/outcome. Scope fields (`scope_root`) stay
/// intact for callers that legitimately scope by path.
fn unnarrow_ttl_sweep(obj: &mut serde_json::Map<String, Value>) {
    let is_ttl_sweep = obj
        .get("past_ttl")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !is_ttl_sweep {
        return;
    }
    for k in ["type", "from", "outcome"] {
        if obj.remove(k).is_some() {
            tracing::debug!("ling-mem client: dropped over-constraining `{k}` on past_ttl sweep");
        }
    }
}

fn log_call(verb: &str, args: &Value) {
    let preview = serde_json::to_string(args).unwrap_or_else(|_| "<unserializable>".into());
    let preview = if preview.len() > 200 {
        format!("{}…", &preview[..199])
    } else {
        preview
    };
    tracing::info!("ling-mem client → {verb} body={preview}");
}

/// Memory's answer, unwrapped to `data`. The status is the door's business,
/// not a program's — an error is an error here, and `parse_envelope`
/// already carries memory's own code.
fn finish(
    caller: &str,
    verb: &str,
    (_status, envelope): crate::memory_host::Answer,
) -> Result<Value> {
    let mut value = parse_envelope(envelope)?;
    if verb == "delete" {
        mark_already_gone(&mut value);
    }
    tracing::info!("ling-mem client ← {caller}: {}", summary(&value));
    Ok(value)
}

/// Deleting an already-absent row is success, not an anomaly — the row is
/// gone either way (commonly memory's cross-tier dedup removed the episodic
/// copy during a promote add). A bare `removed:false` reads as an error
/// signal to LLM callers (observed: three dream runs aborted claiming
/// "store inconsistency" over it), so say what it means.
fn mark_already_gone(value: &mut Value) {
    let Some(obj) = value.as_object_mut() else {
        return;
    };
    if obj.get("removed").and_then(|v| v.as_bool()) != Some(false) {
        return;
    }
    obj.insert("already_gone".to_string(), Value::Bool(true));
    obj.insert(
        "note".to_string(),
        Value::String(
            "row was already absent — treat as success; do not retry or verify".to_string(),
        ),
    );
}

fn summary(value: &Value) -> String {
    match value {
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
    }
}

fn parse_envelope(envelope: Value) -> Result<Value> {
    let obj = envelope
        .as_object()
        .ok_or_else(|| anyhow!("daemon response is not a JSON object: {envelope}"))?;
    match obj.get("ok").and_then(|v| v.as_bool()) {
        Some(true) => Ok(obj.get("data").cloned().unwrap_or(Value::Null)),
        Some(false) => {
            let msg = obj
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error");
            let code = obj.get("code").and_then(|v| v.as_str());
            match code {
                Some(c) => Err(anyhow!("ling-mem error [{c}]: {msg}")),
                None => Err(anyhow!("ling-mem error: {msg}")),
            }
        }
        None => Err(anyhow!("daemon response missing `ok` field: {envelope}")),
    }
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
    let dir = crate::paths::linggen_home().join("memory/backups");
    tokio::fs::create_dir_all(&dir).await?;
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let path = dir.join(format!("store-{today}.ndjson"));
    if tokio::fs::try_exists(&path).await.unwrap_or(false) {
        return Ok(()); // one snapshot per day covers every run that day
    }

    if let Err(e) = crate::memory_host::export_to(&path).await {
        let _ = tokio::fs::remove_file(&path).await;
        return Err(e.context("exporting the memory store"));
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
