//! The engine's own REST client for `ling-mem`, and the daemon it depends on.
//!
//! **No model reaches memory through this file.** A model's memory tools are
//! ling-mem's own, discovered over MCP (`mcp_client`, and `memory_mcp` for
//! what the engine adds to such a call). What is left here is the engine
//! talking to memory as a *program*:
//!
//! - `/apps/<skill>/capability/*`, the door a skill's webpage uses
//! - the dream rollup and the mission scheduler's stats
//! - the core-block reconcile fetch
//! - `api::memory`, the phone's passthrough: Yinyue runs on the phone, so her
//!   memory calls arrive over the WebRTC channel and terminate here rather
//!   than on memory's own loopback routes
//!
//! Memory is hosted in this process (`memory_host`); the address these calls
//! go to is its loopback prefix on the engine's own port. Nothing here starts,
//! installs or looks for a separate `ling-mem` daemon any more.
//!
//! Wire behaviour that comes with this path: verb-dispatched endpoints,
//! soft-empty cleanup, `tier: "episodic"` → `episodic: true`. The same fixes
//! exist in ling-mem's MCP layer for model callers, which is where they belong
//! — models fill arguments sloppily, programs don't. Naming the calling host
//! is each caller's own job, not this function's; see `memory_mcp::HOST`.

use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::time::Duration;

/// Per-call HTTP timeout. Matches the old `capability_tools.rs` budget.
const DISPATCH_TIMEOUT: Duration = Duration::from_secs(5);

/// POST `args` to `<ling_mem_url>/api/memory/<verb>` (verb taken from
/// `args["verb"]` and stripped) and return the daemon's `data` payload.
/// Handles soft-empty cleanup, the `tier=episodic` → `episodic=true` wire
/// translation.
///
/// `caller` names who is asking, for the error and the log line only — it
/// changes nothing on the wire. Stamping `host` on a row is the caller's own
/// business, and each one does it: the `/mcp` group says `mcp`, a skill's
/// webpage door says `linggen`. A default buried here would have named this
/// engine on behalf of a caller that isn't it.
pub async fn call_memory_http(ling_mem_url: &str, caller: &str, mut args: Value) -> Result<Value> {
    let ling_mem_url = ling_mem_url.trim_end_matches('/');

    // Verb → endpoint. Strip the verb from the body so the daemon sees
    // the same shape it accepts from the CLI.
    let verb = args
        .get("verb")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("{caller}: `verb` is required"))?
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
    }

    // TTL-sweep guard: when the caller is doing a `past_ttl: true` list
    // (the dream consolidator's worklist query), strip `type`, `from`,
    // and `outcome` filters. Observed failure mode: `gpt-5.5` ignores
    // the schema's "DEFAULT: do not pass" hints and fills these in with
    // arbitrary defaults (`type=fact, from=derived, outcome=neutral`),
    // which over-constrains the query to 0 rows. The dream is a
    // bulk-eviction sweep — it wants every past-TTL episodic row
    // regardless of type/origin/outcome. Scope fields (`scope_root`) stay
    // intact for callers that legitimately scope by path.
    if let Some(obj) = args.as_object_mut() {
        let is_ttl_sweep = obj
            .get("past_ttl")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if is_ttl_sweep {
            for k in ["type", "from", "outcome"] {
                if obj.remove(k).is_some() {
                    tracing::debug!(
                        "ling-mem client: dropped over-constraining `{k}` on past_ttl sweep"
                    );
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
    tracing::info!("ling-mem client → POST {url} body={args_preview}");

    // The status is the passthrough's business, not a program's — an error is
    // an error here, and `parse_envelope` already carries ling-mem's own code.
    let (_status, envelope) = post_memory_verb(ling_mem_url, &verb, &args).await?;
    let mut value = parse_envelope(envelope)?;

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
    tracing::info!("ling-mem client ← {caller}: {summary}");

    Ok(value)
}

#[derive(Debug)]
enum DispatchError {
    /// Memory isn't reachable at that address.
    NoDaemon,
    /// Anything else — surface to the model.
    Other(anyhow::Error),
}

impl From<DispatchError> for anyhow::Error {
    fn from(e: DispatchError) -> Self {
        match e {
            DispatchError::NoDaemon => {
                anyhow!("memory is not reachable — check `ling status`")
            }
            DispatchError::Other(e) => e,
        }
    }
}

/// The one way anything in this engine reaches memory's REST surface.
///
/// Returns the daemon's envelope **verbatim** — `{ok, data}` or
/// `{ok:false, error, code}` — because its two callers want different halves
/// of it. `call_memory_http` unwraps to `data` for programs inside the engine;
/// `api::memory`'s passthrough hands the whole thing to the phone, which reads
/// the same documented shape a loopback caller would. Neither reimplements the
/// request or the error classification.
///
/// The daemon's HTTP status rides along with it. A non-2xx is not automatically
/// a failure of this hop — a 404 is ling-mem saying that verb doesn't exist and
/// a 400 is it rejecting the arguments, both of which the caller should see as
/// themselves rather than as "memory is down". Only a daemon that never
/// answered is our error.
pub async fn post_memory_verb(
    ling_mem_url: &str,
    verb: &str,
    body: &Value,
) -> Result<(u16, Value)> {
    let url = format!("{}/api/memory/{verb}", ling_mem_url.trim_end_matches('/'));
    post_raw(&url, body).await.map_err(anyhow::Error::from)
}

/// One request. Returns `(status, envelope)` whenever the daemon answered at
/// all — including a non-2xx, whose body is normally the error envelope and
/// carries a `code` a bare status would throw away. A reply that isn't JSON
/// (an empty 404 body from an unrouted path, say) is given the envelope shape
/// here, so everything downstream parses exactly one thing.
async fn post_raw(url: &str, args: &Value) -> Result<(u16, Value), DispatchError> {
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
    let body = response.text().await.map_err(|e| {
        DispatchError::Other(anyhow::Error::from(e).context(format!("reading {url} response")))
    })?;

    match serde_json::from_str::<Value>(&body) {
        Ok(envelope) if envelope.is_object() => Ok((status.as_u16(), envelope)),
        _ => {
            let trimmed = body.trim();
            let message = format!(
                "ling-mem error [{}]: {}",
                status.as_u16(),
                if trimmed.is_empty() {
                    "<empty body>"
                } else {
                    trimmed
                }
            );
            if status.is_success() {
                // A 2xx that isn't the envelope is the daemon misbehaving,
                // not a routine rejection — surface it.
                return Err(DispatchError::Other(anyhow!(message)));
            }
            Ok((
                status.as_u16(),
                json!({ "ok": false, "error": message, "code": format!("HTTP_{}", status.as_u16()) }),
            ))
        }
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
