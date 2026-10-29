//! The engine restarting itself on a new build (doc/background-tasks-spec.md
//! § Engine restart). It waits a while for other sessions' turns to finish,
//! remembers every session whose turn the restart interrupts, and hands the
//! swap to a detached helper (`ling engine-restart`). The next engine tells
//! each of those sessions what happened, in a turn of its own.
use super::chat::{types::ChatRequest, TurnOrigin};
use super::ServerState;
use crate::engine::agent::AgentRunStatus;
use crate::engine::events::QueuedChatItem;
use crate::state_fs::sessions::SessionMeta;
use axum::{extract::State, http::StatusCode, response::IntoResponse, response::Response, Json};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;
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
    let mut items: Vec<(&str, &QueuedChatItem)> = all
        .iter()
        .flat_map(|(key, items)| items.iter().map(move |item| (key.as_str(), item)))
        .collect();
    items.sort_by_key(|(_, item)| sent_order(&item.id));
    let mut queued: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for (key, item) in items {
        let tail = format!("|{}|{}", item.session_id, item.agent_id);
        let project_root = key.strip_suffix(&tail).unwrap_or_default();
        queued.entry(item.session_id.clone()).or_default().push(json!({
            "text": item.text,
            "images": item.images,
            "agent_id": item.agent_id,
            "project_root": project_root,
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
    let binary = PathBuf::from(&req.binary);
    let probed = binary.clone();
    let version = tokio::task::spawn_blocking(move || {
        crate::cli::self_update::probe_version(&probed, "ling", Duration::from_secs(10))
    })
    .await;
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
    let mut queued = queued_messages(&*state.queued_chats.lock().await);
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
        "sessions": sessions,
    });
    let path = plan_file();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(error) = std::fs::write(&path, plan.to_string()) {
        return (StatusCode::INTERNAL_SERVER_ERROR, format!("Could not keep the plan: {error}")).into_response();
    }
    let config = state.manager.get_config_snapshot().await;
    if let Err(error) = crate::cli::restart::spawn_helper(&binary, &config.server.host(), config.server.port()) {
        let _ = std::fs::remove_file(&path);
        return (StatusCode::INTERNAL_SERVER_ERROR, format!("{error:#}")).into_response();
    }
    tracing::info!("[restart] restarting on {} ({}); {} session(s) continue after", binary.display(), version.trim(), sessions.len());
    Json(json!({"status": "restarting", "waited_secs": started.elapsed().as_secs(), "sessions": sessions.len()}))
        .into_response()
}

/// What an interrupted session is told when it continues.
pub(crate) fn continue_message(plan: &Value, session: &Value, outcome: Option<&Value>) -> String {
    let from = plan["from_version"].as_str().unwrap_or("?");
    let to = plan["to_version"].as_str().unwrap_or("a new build");
    let how = match outcome.and_then(|o| o["state"].as_str()) {
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
    format!(
        "The engine restarted (from {from} to {to}; {how}). {who} Its last step may not have finished: \
         check the current state before redoing anything that changes things.{tasks} Then continue where you left off."
    )
}

/// After a restart: once the helper has said how it went, every session in
/// the plan continues with a turn of its own. At most once per plan.
pub(crate) fn spawn_resume(state: Arc<ServerState>) {
    tokio::spawn(async move {
        let path = plan_file();
        let Some(plan) = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()) else {
            return;
        };
        let requested = plan["requested_at"].as_u64().unwrap_or(0);
        if crate::util::now_ts_secs().saturating_sub(requested) > STALE_SECS {
            let _ = std::fs::remove_file(&path);
            return;
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
            if let Some(agent) = session["agent_id"].as_str() {
                tracing::info!("[restart] {sid} continues");
                let message = continue_message(&plan, session, outcome.as_ref());
                continue_session(&state, &meta, agent, message).await;
            }
            resend_queued(&state, sid, session).await;
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

/// Send again, in their order, the messages the person had queued in a
/// session when the engine restarted. Each goes the way a typed one does
/// and takes its place in the turn line as it is sent, so they queue behind
/// the continue turn in the order they were typed.
async fn resend_queued(state: &Arc<ServerState>, sid: &str, session: &Value) {
    for item in session["queued"].as_array().into_iter().flatten() {
        let req = serde_json::from_value::<ChatRequest>(json!({
            "project_root": item["project_root"],
            "session_id": sid,
            "agent_id": item["agent_id"],
            "message": item["text"],
            "images": item["images"],
        }));
        let Ok(req) = req else {
            tracing::warn!("[restart] a queued message in {sid} could not be read back");
            continue;
        };
        super::chat::start_turn(state.clone(), req, TurnOrigin::Resumed).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            text: text.into(),
            images: vec![format!("{text}.png")],
        }
    }

    /// Each session's queue is kept whole — text, pictures, whom it was
    /// for and the queue's root — in the order the messages were sent,
    /// across the members of a shared session.
    #[test]
    fn the_plan_keeps_each_sessions_queue_in_the_order_it_was_sent() {
        let mut all = HashMap::new();
        all.insert(
            "/work|s1|ling".to_string(),
            vec![item(10, "s1", "ling", "first"), item(12, "s1", "ling", "third")],
        );
        all.insert("/work|s1|yinyue".to_string(), vec![item(11, "s1", "yinyue", "second")]);
        all.insert("/other|s2|claude".to_string(), vec![item(9, "s2", "claude", "alone")]);
        let queued = queued_messages(&all);
        let s1: Vec<&str> = queued["s1"].iter().filter_map(|m| m["text"].as_str()).collect();
        assert_eq!(s1, ["first", "second", "third"]);
        assert_eq!(queued["s1"][1]["agent_id"], "yinyue");
        assert_eq!(queued["s1"][1]["project_root"], "/work");
        assert_eq!(queued["s1"][1]["images"], json!(["second.png"]));
        assert_eq!(queued["s2"][0]["project_root"], "/other");
    }
}
