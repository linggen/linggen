//! The engine restarting itself on a new build (doc/background-tasks-spec.md
//! § Engine restart). It waits a while for other sessions' turns to finish,
//! remembers every session whose turn the restart interrupts, and hands the
//! swap to a detached helper (`ling engine-restart`). The next engine tells
//! each of those sessions what happened, in a turn of its own.
use super::ServerState;
use crate::engine::agent::AgentRunStatus;
use crate::engine::events::QueuedChatItem;
use crate::state_fs::sessions::SessionMeta;
use axum::{extract::State, http::StatusCode, response::IntoResponse, response::Response, Json};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

static RESTARTING: AtomicBool = AtomicBool::new(false);

struct RestartGuard;

impl RestartGuard {
    fn acquire() -> Option<Self> {
        RESTARTING.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).ok().map(|_| Self)
    }
}

impl Drop for RestartGuard {
    fn drop(&mut self) {
        RESTARTING.store(false, Ordering::Release);
    }
}
use std::time::{Duration, Instant};

/// Longest a restart waits for other turns, whatever it was asked.
const MAX_WAIT: Duration = Duration::from_secs(600);
/// A plan older than this is stale: no restart is still settling.
const STALE_SECS: u64 = 15 * 60;

fn plan_file() -> PathBuf {
    crate::paths::linggen_home().join("restart/resume.json")
}

#[derive(serde::Deserialize)]
pub(crate) struct RestartRequest {
    binary: String,
    session_id: Option<String>,
    wait_secs: Option<u64>,
}

/// Top-level turns running now, one per session, missions left out (their
/// own catch-up covers them).
fn running_turns(state: &ServerState) -> Vec<(String, String)> {
    let mut turns: Vec<(String, String)> = Vec::new();
    for run in state.manager.run_store.list_runs(None) {
        if run.status != AgentRunStatus::Running || run.parent_run_id.is_some() {
            continue;
        }
        let mission = state
            .manager
            .global_sessions
            .get_session_meta(&run.session_id)
            .ok()
            .flatten()
            .is_some_and(|meta| meta.creator == "mission");
        if mission || turns.iter().any(|(sid, _)| *sid == run.session_id) {
            continue;
        }
        turns.push((run.session_id, run.agent_id));
    }
    turns
}

/// Where a queued message stands in the order messages were sent: its id
/// ends with the engine's queue sequence (`chat::enqueue_if_busy`).
fn sent_order(id: &str) -> u64 {
    id.rsplit_once('-').and_then(|(_, seq)| seq.parse().ok()).unwrap_or(u64::MAX)
}

/// The messages queued now, by session, each session's in the order they
/// were sent. Written out by hand: a queued item never serializes its text
/// or pictures.
fn queued_messages(all: &HashMap<String, Vec<QueuedChatItem>>) -> BTreeMap<String, Vec<Value>> {
    let mut items: Vec<&QueuedChatItem> = all.values().flatten().collect();
    items.sort_by_key(|item| sent_order(&item.id));
    let mut queued: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for item in items {
        queued.entry(item.session_id.clone()).or_default().push(json!({
            "text": item.text,
            "images": item.images,
            "agent_id": item.agent_id,
        }));
    }
    queued
}

/// The new build as the continue message names it: its version, and when it
/// was built (two builds of one version are otherwise the same words).
fn built_label(binary: &std::path::Path, version: &str) -> String {
    let built = std::fs::metadata(binary)
        .and_then(|m| m.modified())
        .ok()
        .map(|t| chrono::DateTime::<chrono::Local>::from(t).format("%Y-%m-%d %H:%M").to_string());
    let version = version.trim_start_matches("ling ").to_string();
    match built {
        Some(at) => format!("{version} built {at}"),
        None => version,
    }
}

/// POST /api/engine/restart
pub(crate) async fn restart(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<RestartRequest>,
) -> Response {
    let Some(guard) = RestartGuard::acquire() else {
        return (StatusCode::CONFLICT, "A restart is already in progress").into_response();
    };
    perform_restart(state, Some(req), guard).await
}

/// The owner's typed /restart: preserve running turns and queues before
/// scheduling the current executable's relaunch.
pub(crate) async fn restart_current(State(state): State<Arc<ServerState>>) -> Response {
    let Some(guard) = RestartGuard::acquire() else {
        return (StatusCode::CONFLICT, "A restart is already in progress").into_response();
    };
    perform_restart(state, None, guard).await
}

async fn perform_restart(state: Arc<ServerState>, req: Option<RestartRequest>, guard: RestartGuard) -> Response {
    // Never overwrite an unfinished handoff, even if the helper timed out
    // and the in-memory guard was released.
    if plan_file().exists() {
        return (StatusCode::CONFLICT, "An earlier restart has not finished recovery").into_response();
    }
    let current = req.is_none();
    let req = req.unwrap_or(RestartRequest { binary: String::new(), session_id: None, wait_secs: Some(0) });
    let binary = if current {
        match std::env::current_exe() {
            Ok(path) => path,
            Err(error) => return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
        }
    } else {
        PathBuf::from(&req.binary)
    };
    let probed = binary.clone();
    let version = if current {
        Ok(Ok(format!("ling {}", env!("CARGO_PKG_VERSION"))))
    } else {
        tokio::task::spawn_blocking(move || {
            crate::cli::self_update::probe_version(&probed, "ling", Duration::from_secs(10))
        }).await
    };
    let version = match version {
        Ok(Ok(version)) => version,
        Ok(Err(error)) => return (StatusCode::BAD_REQUEST, format!("{error:#}")).into_response(),
        Err(error) => return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
    };
    let caller = req.session_id.clone();
    let wait = Duration::from_secs(req.wait_secs.unwrap_or(120)).min(MAX_WAIT);
    let started = Instant::now();
    while started.elapsed() < wait
        && running_turns(&state).iter().any(|(sid, _)| Some(sid) != caller.as_ref())
    {
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    // Freeze every queue mutation before taking the snapshot and keep it frozen
    // until the helper has stopped this process (or the takeover times out).
    // Otherwise a later enqueue is lost, or a removed/dequeued item reappears.
    let queue_guard = state.queued_chats.clone().lock_owned().await;
    let mut turns = running_turns(&state);
    if let Some(sid) = &caller {
        if !turns.iter().any(|(s, _)| s == sid) {
            let agent = state
                .manager
                .global_sessions
                .get_session_meta(sid)
                .ok()
                .flatten()
                .map_or("ling", |meta| if meta.runtime.is_linggen() { "ling" } else { "claude" })
                .to_string();
            turns.push((sid.clone(), agent));
        }
    }
    let mut queued = queued_messages(&queue_guard);
    let mut sessions: Vec<Value> = turns
        .iter()
        .map(|(sid, agent)| {
            let tasks: Vec<String> = state
                .tasks
                .list(sid)
                .into_iter()
                .filter(|t| t.is_running())
                .map(|t| format!("{} ({})", t.title, t.id))
                .collect();
            let waiting = queued.remove(sid).unwrap_or_default();
            json!({"session_id": sid, "agent_id": agent, "caller": Some(sid) == caller.as_ref(), "tasks": tasks, "queued": waiting})
        })
        .collect();
    // A session with messages queued and no turn to continue (a mission's,
    // one between turns) is recorded without an agent: only its queue returns.
    sessions.extend(queued.into_iter().map(|(sid, waiting)| json!({"session_id": sid, "queued": waiting})));
    let plan = json!({
        "requested_at": crate::util::now_ts_secs(),
        "from_version": env!("CARGO_PKG_VERSION"),
        "to_version": built_label(&binary, version.trim()),
        "current": current,
        "sessions": sessions,
    });
    let path = plan_file();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(error) = std::fs::write(&path, plan.to_string()) {
        return (StatusCode::INTERNAL_SERVER_ERROR, format!("Could not keep the plan: {error}")).into_response();
    }
    // A previous status can have the same second-level timestamp. Never let
    // it masquerade as this helper's outcome when the new engine starts.
    match std::fs::remove_file(crate::cli::restart::status_file()) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            let _ = std::fs::remove_file(&path);
            return (StatusCode::INTERNAL_SERVER_ERROR, format!("Could not clear the previous restart status: {error}")).into_response();
        }
    }
    let config = state.manager.get_config_snapshot().await;
    let spawned = if current {
        crate::cli::restart::spawn_current_helper(&state.bound_host, state.port)
    } else {
        crate::cli::restart::spawn_helper(&binary, &config.server.host(), config.server.port())
    };
    if let Err(error) = spawned {
        let _ = std::fs::remove_file(&path);
        return (StatusCode::INTERNAL_SERVER_ERROR, format!("{error:#}")).into_response();
    }
    // Keep the lock while the helper takes over. If the helper fails before
    // stopping this process, release it so the owner can retry.
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(180)).await;
        drop(queue_guard);
        drop(guard);
    });
    tracing::info!("[restart] restarting on {} ({}); {} session(s) continue after", binary.display(), version.trim(), sessions.len());
    Json(json!({"status": "restarting", "waited_secs": started.elapsed().as_secs(), "sessions": sessions.len()}))
        .into_response()
}

/// What an interrupted session is told when it continues.
pub(crate) fn continue_message(plan: &Value, session: &Value, outcome: Option<&Value>) -> String {
    let from = plan["from_version"].as_str().unwrap_or("?");
    let to = plan["to_version"].as_str().unwrap_or("a new build");
    let how = match outcome.and_then(|o| o["state"].as_str()) {
        Some("ok") if plan["current"] == true => "the current engine is up and healthy".to_string(),
        Some("ok") => "the new build is up and healthy".to_string(),
        Some("rolled_back") => format!(
            "the new build did not come up, so the previous one was restored: {}",
            outcome.and_then(|o| o["detail"].as_str()).unwrap_or("")
        ),
        Some(_) => format!(
            "the swap failed: {}",
            outcome.and_then(|o| o["detail"].as_str()).unwrap_or("")
        ),
        None => "how the swap went is not known".to_string(),
    };
    let who = if session["caller"] == true {
        "You asked for this restart, and your turn ended with it."
    } else {
        "Your turn was running and the restart interrupted it."
    };
    let tasks: Vec<&str> = session["tasks"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    let tasks = if tasks.is_empty() {
        String::new()
    } else {
        format!(" Background tasks that ended with the restart: {}.", tasks.join("; "))
    };
    let queued = session["queued"].as_array().filter(|q| !q.is_empty()).map(|q| {
        format!(" Queued messages recorded at restart, in order (JSON; text, images and intended agent): {}. \
                 Whether each was already processed is unknown. Check the conversation and current state before deciding \
                 whether any work needs retrying; do not replay them blindly or wait for the user to decide.",
                Value::Array(q.clone()))
    }).unwrap_or_default();
    format!(
        "The engine restarted (from {from} to {to}; {how}). {who} Its last step may not have finished: \
         check the current state before redoing anything that changes things.{tasks}{queued} Then continue where you left off."
    )
}

/// Reject malformed plans rather than silently dropping queued content.
fn safe_to_resume(plan: &Value) -> bool {
    plan["sessions"].as_array().is_some_and(|sessions| sessions.iter().all(|session| {
        session["session_id"].as_str().is_some_and(|id| !id.is_empty())
            && (session.get("queued").is_none() || session["queued"].as_array().is_some_and(|items| {
                items.iter().all(|item| item["text"].is_string()
                    && item.get("images").is_none_or(|images| images.as_array().is_some_and(|a| a.iter().all(Value::is_string))))
            }))
    }))
}

/// After a restart: once the helper has said how it went, every session in
/// the plan continues with a turn of its own. At most once per plan.
pub(crate) fn spawn_resume(state: Arc<ServerState>) {
    // Acquire before spawning: a restart request must not overwrite the plan
    // while recovery is waiting for the helper or replaying its sessions.
    let Some(guard) = RestartGuard::acquire() else {
        tracing::error!("[restart] could not reserve recovery guard");
        return;
    };
    tokio::spawn(async move {
        let _guard = guard;
        let path = plan_file();
        let Some(plan) = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()) else {
            return;
        };
        let requested = plan["requested_at"].as_u64().unwrap_or(0);
        if crate::util::now_ts_secs().saturating_sub(requested) > STALE_SECS {
            tracing::error!("[restart] recovery plan is stale; preserving it for manual recovery");
            return;
        }
        if !safe_to_resume(&plan) {
            tracing::error!("[restart] recovery plan contains invalid sessions or queued messages; preserving it for manual recovery");
            return;
        }
        for session in plan["sessions"].as_array().unwrap() {
            let sid = session["session_id"].as_str().unwrap();
            if !matches!(state.manager.global_sessions.get_session_meta(sid), Ok(Some(_))) {
                tracing::error!("[restart] session {sid} is unavailable; preserving recovery plan");
                return;
            }
        }
        // The helper writes how it went once this engine answers health. An
        // engine it then rolls back never gets here, and leaves the plan for
        // the one that replaces it.
        let mut outcome = None;
        for _ in 0..180 {
            let status = std::fs::read(crate::cli::restart::status_file())
                .ok()
                .and_then(|b| serde_json::from_slice::<Value>(&b).ok());
            if status.as_ref().is_some_and(|s| s["at"].as_u64().unwrap_or(0) >= requested) {
                outcome = status;
                break;
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        let _ = std::fs::remove_file(&path);
        for session in plan["sessions"].as_array().into_iter().flatten() {
            let Some(sid) = session["session_id"].as_str() else {
                continue;
            };
            let Some(meta) = state.manager.global_sessions.get_session_meta(sid).ok().flatten() else {
                continue;
            };
            let agent = session["agent_id"].as_str().or_else(|| {
                session["queued"].as_array().and_then(|items| items.first())
                    .and_then(|item| item["agent_id"].as_str())
            }).unwrap_or(if meta.runtime.is_linggen() { "ling" } else { "claude" });
            tracing::info!("[restart] {sid} continues");
            let message = continue_message(&plan, session, outcome.as_ref());
            continue_session(&state, &meta, agent, message).await;
        }
    });
}

/// Start a session's continue turn. On return it holds its place in the
/// session's turn line, so what is sent next waits behind it.
async fn continue_session(state: &Arc<ServerState>, meta: &SessionMeta, agent: &str, message: String) {
    if meta.runtime.is_linggen() {
        tokio::spawn(super::tasks::tell(state.clone(), meta.id.clone(), agent.into(), message));
        return;
    }
    super::claude::start_task(state.clone(), meta.id.clone(), message, "system".into()).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_accepts_readable_queues_but_refuses_invalid_plans() {
        assert!(safe_to_resume(&json!({"sessions": [{"session_id": "s", "queued": []}]})));
        assert!(safe_to_resume(&json!({"sessions": [{"session_id": "s"}]})));
        assert!(safe_to_resume(&json!({"sessions": [{"session_id": "s", "queued": [{"text": "hello"}]}]})));
        assert!(!safe_to_resume(&json!({"sessions": [{"session_id": "s", "queued": [{}]}]})));
        assert!(!safe_to_resume(&json!({"sessions": [{"session_id": "s", "queued": null}]})));
        assert!(!safe_to_resume(&json!({"sessions": [{}]})));
        assert!(!safe_to_resume(&json!({})));
    }

    #[test]
    fn restart_guard_rejects_concurrent_requests_and_releases_on_failure() {
        let guard = RestartGuard::acquire().unwrap();
        assert!(RestartGuard::acquire().is_none());
        drop(guard);
        assert!(RestartGuard::acquire().is_some());
    }

    #[tokio::test]
    async fn snapshot_guard_blocks_queue_mutations_until_takeover_is_released() {
        let queue = Arc::new(tokio::sync::Mutex::new(HashMap::from([(
            "s|ling".to_string(), vec![item(1, "s", "ling", "first")],
        )])));
        let guard = queue.clone().lock_owned().await;
        let snapshot = queued_messages(&guard);
        let other = queue.clone();
        let mutation = tokio::spawn(async move {
            let mut queue = other.lock().await;
            queue.clear();
            queue.insert("s|ling".into(), vec![item(2, "s", "ling", "second")]);
        });
        tokio::task::yield_now().await;
        assert!(!mutation.is_finished());
        assert_eq!(snapshot["s"][0]["text"], "first");
        drop(guard);
        mutation.await.unwrap();
        assert_eq!(queued_messages(&*queue.lock().await)["s"][0]["text"], "second");
    }

    #[test]
    fn current_restart_outcome_does_not_claim_a_new_build() {
        let plan = json!({"from_version": "1", "to_version": "1", "current": true});
        let said = continue_message(&plan, &json!({}), Some(&json!({"state": "ok"})));
        assert!(said.contains("current engine is up and healthy"));
        assert!(!said.contains("new build"));
    }

    #[test]
    fn the_caller_and_the_interrupted_are_told_what_happened() {
        let plan = json!({"from_version": "1.10.0", "to_version": "ling 1.10.1"});
        let caller = json!({"caller": true, "tasks": []});
        let other = json!({"caller": false, "tasks": ["npm run dev (sh-1)"]});
        let ok = json!({"state": "ok"});
        let said = continue_message(&plan, &caller, Some(&ok));
        assert!(said.contains("from 1.10.0 to ling 1.10.1; the new build is up and healthy"), "{said}");
        assert!(said.contains("You asked for this restart"));
        assert!(said.contains("check the current state before redoing"));
        let said = continue_message(&plan, &other, Some(&json!({"state": "rolled_back", "detail": "no health"})));
        assert!(said.contains("previous one was restored: no health"), "{said}");
        assert!(said.contains("restart interrupted it"));
        assert!(said.contains("npm run dev (sh-1)"));
        assert!(continue_message(&plan, &caller, None).contains("not known"));
    }

    fn item(seq: u64, session: &str, agent: &str, text: &str) -> QueuedChatItem {
        QueuedChatItem {
            id: format!("1700000000000-{seq}"),
            agent_id: agent.into(),
            session_id: session.into(),
            preview: text.into(),
            timestamp: 0,
            steerable: false,
            text: text.into(),
            images: vec![format!("{text}.png")],
        }
    }

    /// Each session's queue is kept whole — text, pictures and whom it was
    /// for — in the order the messages were sent,
    /// across the members of a shared session.
    #[test]
    fn the_plan_keeps_each_sessions_queue_in_the_order_it_was_sent() {
        let mut all = HashMap::new();
        all.insert(
            "s1|ling".to_string(),
            vec![item(10, "s1", "ling", "first"), item(12, "s1", "ling", "third")],
        );
        all.insert("s1|yinyue".to_string(), vec![item(11, "s1", "yinyue", "second")]);
        all.insert("s2|claude".to_string(), vec![item(9, "s2", "claude", "alone")]);
        let queued = queued_messages(&all);
        let s1: Vec<&str> = queued["s1"].iter().filter_map(|m| m["text"].as_str()).collect();
        assert_eq!(s1, ["first", "second", "third"]);
        assert_eq!(queued["s1"][1]["agent_id"], "yinyue");
        assert_eq!(queued["s1"][1]["images"], json!(["second.png"]));
        assert_eq!(queued["s2"][0]["text"], "alone");
    }
}
