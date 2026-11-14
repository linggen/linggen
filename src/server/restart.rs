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
        RESTARTING
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| Self)
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

/// Keep the plan whole: a torn plan would not parse, and its sessions and
/// queues would be lost with it.
pub(super) fn write_plan(path: &std::path::Path, plan: &Value) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let staged = path.with_extension("json.new");
    std::fs::write(&staged, plan.to_string())?;
    std::fs::rename(&staged, path)
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
    id.rsplit_once('-')
        .and_then(|(_, seq)| seq.parse().ok())
        .unwrap_or(u64::MAX)
}

/// The messages queued now, by session, each session's in the order they
/// were sent. Written out by hand: a queued item never serializes its text
/// or pictures.
fn queued_messages(all: &HashMap<String, Vec<QueuedChatItem>>) -> BTreeMap<String, Vec<Value>> {
    let mut items: Vec<&QueuedChatItem> = all.values().flatten().collect();
    items.sort_by_key(|item| sent_order(&item.id));
    let mut queued: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for item in items {
        queued
            .entry(item.session_id.clone())
            .or_default()
            .push(json!({
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
        .map(|t| {
            chrono::DateTime::<chrono::Local>::from(t)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        });
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

async fn perform_restart(
    state: Arc<ServerState>,
    req: Option<RestartRequest>,
    guard: RestartGuard,
) -> Response {
    // Never overwrite an unfinished handoff, even if the helper timed out
    // and the in-memory guard was released.
    if plan_file().exists() {
        return (
            StatusCode::CONFLICT,
            "An earlier restart has not finished recovery",
        )
            .into_response();
    }
    let current = req.is_none();
    let req = req.unwrap_or(RestartRequest {
        binary: String::new(),
        session_id: None,
        wait_secs: Some(0),
    });
    let binary = if current {
        match std::env::current_exe() {
            Ok(path) => path,
            Err(error) => {
                return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response()
            }
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
        })
        .await
    };
    let version = match version {
        Ok(Ok(version)) => version,
        Ok(Err(error)) => return (StatusCode::BAD_REQUEST, format!("{error:#}")).into_response(),
        Err(error) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response()
        }
    };
    let caller = req.session_id.clone();
    let wait = Duration::from_secs(req.wait_secs.unwrap_or(120)).min(MAX_WAIT);
    let started = Instant::now();
    while started.elapsed() < wait
        && running_turns(&state)
            .iter()
            .any(|(sid, _)| Some(sid) != caller.as_ref())
    {
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    // Freeze every queue mutation before taking the snapshot and keep it frozen
    // until the helper has stopped this process (or the takeover times out).
    // Otherwise a later enqueue is lost, or a removed/dequeued item reappears.
    let mut admission = state.admission.clone().lock_owned().await;
    let queue_guard = state.queued_chats.clone().lock_owned().await;
    let mut turns = running_turns(&state);
    // A message admitted just before holds its session's turn place, its run
    // not on record yet: that turn is interrupted too.
    for sid in state.manager.sessions_with_pending_turn() {
        if turns.iter().any(|(s, _)| *s == sid) {
            continue;
        }
        let Some(meta) = state
            .manager
            .global_sessions
            .get_session_meta(&sid)
            .ok()
            .flatten()
        else {
            continue;
        };
        if meta.creator == "mission" {
            continue;
        }
        let agent = if meta.runtime.is_linggen() {
            super::chat::members::default_responder(
                &super::chat::members::of_session(&state.manager, &sid).await,
            )
        } else {
            "claude".to_string()
        };
        turns.push((sid, agent));
    }
    if let Some(sid) = &caller {
        if !turns.iter().any(|(s, _)| s == sid) {
            let agent = state
                .manager
                .global_sessions
                .get_session_meta(sid)
                .ok()
                .flatten()
                .map_or("ling", |meta| {
                    if meta.runtime.is_linggen() {
                        "ling"
                    } else {
                        "claude"
                    }
                })
                .to_string();
            turns.push((sid.clone(), agent));
        }
    }
    // A session can be idle while its background work is still running.
    for sid in state.tasks.sessions_running() {
        if turns.iter().any(|(s, _)| s == &sid) {
            continue;
        }
        let Some(meta) = state
            .manager
            .global_sessions
            .get_session_meta(&sid)
            .ok()
            .flatten()
        else {
            continue;
        };
        if meta.creator == "mission" {
            continue;
        }
        let agent = state
            .tasks
            .list(&sid)
            .into_iter()
            .find(|task| task.is_running() && !task.child_owner && task.owner_agent.is_some())
            .and_then(|task| task.owner_agent);
        if let Some(agent) = agent {
            turns.push((sid, agent));
        }
    }
    let mut running: Vec<String> = running_turns(&state)
        .into_iter()
        .map(|(sid, _)| sid)
        .collect();
    running.extend(state.manager.sessions_with_pending_turn());
    let mut queued = queued_messages(&queue_guard);
    let mut sessions: Vec<Value> = turns
        .iter()
        .map(|(sid, agent)| {
            let tasks: Vec<_> = state.tasks.list(sid).into_iter().filter(|t| t.is_running()).collect();
            let waiting = queued.remove(sid).unwrap_or_default();
            json!({"session_id": sid, "agent_id": agent, "caller": Some(sid) == caller.as_ref(), "turn_interrupted": running.contains(sid), "tasks": tasks, "queued": waiting})
        })
        .collect();
    // A session with messages queued and no turn to continue (a mission's,
    // one between turns) is recorded without an agent: only its queue returns.
    sessions.extend(
        queued
            .into_iter()
            .map(|(sid, waiting)| json!({"session_id": sid, "queued": waiting})),
    );
    let plan = json!({
        "requested_at": crate::util::now_ts_secs(),
        "from_version": env!("CARGO_PKG_VERSION"),
        "to_version": built_label(&binary, version.trim()),
        "current": current,
        "sessions": sessions,
    });
    let path = plan_file();
    if let Err(error) = write_plan(&path, &plan) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Could not keep the plan: {error}"),
        )
            .into_response();
    }
    // A previous status can have the same second-level timestamp. Never let
    // it masquerade as this helper's outcome when the new engine starts.
    match std::fs::remove_file(crate::cli::restart::status_file()) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            let _ = std::fs::remove_file(&path);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Could not clear the previous restart status: {error}"),
            )
                .into_response();
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
    // From here every new message joins the plan's queue (`admission`).
    super::admission::freeze(&mut admission, path);
    drop(admission);
    // Keep the queue frozen while the helper takes over. If it never stops
    // this process, this engine goes on as it was.
    let outlived = (state.clone(), plan.clone(), caller.clone());
    tokio::spawn(async move {
        let requested = outlived.1["requested_at"].as_u64().unwrap_or(0);
        let outcome =
            helper_outcome(&crate::cli::restart::status_file(), requested, TAKEOVER).await;
        go_on(outlived, outcome, queue_guard).await;
        drop(guard);
    });
    tracing::info!(
        "[restart] restarting on {} ({}); {} session(s) continue after",
        binary.display(),
        version.trim(),
        sessions.len()
    );
    Json(json!({"status": "restarting", "waited_secs": started.elapsed().as_secs(), "sessions": sessions.len()}))
        .into_response()
}

/// Longest the helper may take to stop this engine.
const TAKEOVER: Duration = Duration::from_secs(180);

/// How the helper said the restart asked for at `requested` went, once it
/// has: `None` after `wait`. Status from before the request is not its.
async fn helper_outcome(status: &std::path::Path, requested: u64, wait: Duration) -> Option<Value> {
    let deadline = Instant::now() + wait;
    loop {
        let said = std::fs::read(status)
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .filter(|s| s["at"].as_u64().unwrap_or(0) >= requested);
        if said.is_some() || Instant::now() >= deadline {
            return said;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// The helper is done, or gave up, and this engine is still running: it
/// never stopped it (the swap failed before that, or the helper died). The
/// plan goes, what waited in it is sent as it came, and the caller hears.
async fn go_on(
    (state, plan, caller): (Arc<ServerState>, Value, Option<String>),
    outcome: Option<Value>,
    queue_guard: tokio::sync::OwnedMutexGuard<HashMap<String, Vec<QueuedChatItem>>>,
) {
    tracing::error!("[restart] the helper did not stop this engine ({outcome:?}); going on");
    let late = super::admission::thaw(&state.admission).await;
    drop(queue_guard);
    for resend in late {
        resend().await;
    }
    let Some(sid) = caller else {
        return;
    };
    let Some(meta) = state
        .manager
        .global_sessions
        .get_session_meta(&sid)
        .ok()
        .flatten()
    else {
        return;
    };
    let message = not_restarted_message(&plan, outcome.as_ref());
    if meta.runtime.is_linggen() {
        let agent = plan["sessions"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|s| s["session_id"] == sid.as_str())
            .and_then(|s| s["agent_id"].as_str())
            .unwrap_or("ling")
            .to_string();
        tokio::spawn(super::tasks::tell(state.clone(), sid, agent, message));
    } else {
        super::claude::start_task(state.clone(), sid, message, "system".into()).await;
    }
}

/// What the caller is told when the engine it asked to restart never stopped.
fn not_restarted_message(plan: &Value, outcome: Option<&Value>) -> String {
    let from = plan["from_version"].as_str().unwrap_or("?");
    let how = match outcome {
        Some(_) => how_it_went(plan, outcome),
        None => "the helper did not take over in time".to_string(),
    };
    format!(
        "The restart you asked for did not happen ({how}). This engine kept running {from} and \
         nothing was interrupted; messages sent meanwhile were delivered now. Check the build \
         before trying again."
    )
}

/// How the helper said the swap went, in the words a continue turn uses.
fn how_it_went(plan: &Value, outcome: Option<&Value>) -> String {
    let detail = || outcome.and_then(|o| o["detail"].as_str()).unwrap_or("");
    match outcome.and_then(|o| o["state"].as_str()) {
        Some("ok") if plan["current"] == true => "the current engine is up and healthy".to_string(),
        Some("ok") => "the new build is up and healthy".to_string(),
        Some("rolled_back") => format!(
            "the new build did not come up, so the previous one was restored: {}",
            detail()
        ),
        Some(_) => format!("the swap failed: {}", detail()),
        None => "how the swap went is not known".to_string(),
    }
}

/// What an interrupted session is told when it continues.
pub(crate) fn continue_message(plan: &Value, session: &Value, outcome: Option<&Value>) -> String {
    let from = plan["from_version"].as_str().unwrap_or("?");
    let to = plan["to_version"].as_str().unwrap_or("a new build");
    let how = how_it_went(plan, outcome);
    let who = if session["caller"] == true {
        "You asked for this restart, and your turn ended with it."
    } else if session["turn_interrupted"] == false {
        "Your background work was running and the restart interrupted it."
    } else if session["agent_id"].is_string() {
        "Your turn was running and the restart interrupted it."
    } else {
        "This session had messages queued at the restart."
    };
    let tasks: Vec<String> = session["tasks"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|task| {
            task.as_str().map(str::to_owned).or_else(|| {
                Some(format!(
                    "{}: {} ({})",
                    task["kind"].as_str().unwrap_or("task"),
                    task["title"].as_str()?,
                    task["id"].as_str()?
                ))
            })
        })
        .collect();
    let tasks = if tasks.is_empty() {
        String::new()
    } else {
        format!(" Background tasks that ended with the restart: {}. Check their state and decide whether to relaunch them; do not relaunch blindly or wait for the user to decide.", tasks.join("; "))
    };
    let queued = session["queued"].as_array().filter(|q| !q.is_empty()).map(|q| {
        format!(" Queued messages recorded at restart, in order (JSON; text, image file paths and intended agent): {}. \
                 Use Read to inspect an image file when needed. Whether each was already processed is unknown. Check the conversation and current state before deciding \
                 whether any work needs retrying; do not replay them blindly or wait for the user to decide.",
                Value::Array(q.clone()))
    }).unwrap_or_default();
    format!(
        "The engine restarted (from {from} to {to}; {how}). {who} Its last step may not have finished: \
         check the current state before redoing anything that changes things.{tasks}{queued} Then continue where you left off."
    )
}

/// Whether a session's entry can be continued: it names its session, and
/// its queue (if any) is readable text and picture paths.
fn session_readable(session: &Value) -> bool {
    let item_readable = |item: &Value| {
        item["text"].is_string()
            && item.get("images").is_none_or(|images| {
                images
                    .as_array()
                    .is_some_and(|a| a.iter().all(Value::is_string))
            })
    };
    session["session_id"]
        .as_str()
        .is_some_and(|id| !id.is_empty())
        && (session.get("queued").is_none()
            || session["queued"]
                .as_array()
                .is_some_and(|items| items.iter().all(item_readable)))
}

/// How many times a continue turn that could not be kept is tried.
const ATTEMPTS: usize = 3;
/// The pause between those tries.
const RETRY_GAP: Duration = Duration::from_secs(2);
/// Longest a continue turn waits for its place before recovery ends.
const DELIVERY_WAIT: Duration = Duration::from_secs(30 * 60);

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
        resume(&state, &plan_file()).await;
    });
}

/// Run the plan at `path`. Every way out ends with the plan gone: each
/// session continued, or kept in the record of what was not (`retire`), so
/// a stuck plan never refuses later restarts.
async fn resume(state: &Arc<ServerState>, path: &std::path::Path) {
    let Ok(bytes) = std::fs::read(path) else {
        return; // no restart to recover from
    };
    let Some(plan) = serde_json::from_slice::<Value>(&bytes)
        .ok()
        .filter(|p| p["sessions"].is_array())
    else {
        retire_unreadable(path, &bytes);
        return;
    };
    let requested = plan["requested_at"].as_u64().unwrap_or(0);
    let sessions = plan["sessions"].as_array().cloned().unwrap_or_default();
    if crate::util::now_ts_secs().saturating_sub(requested) > STALE_SECS {
        let left = sessions
            .iter()
            .map(|s| not_delivered(s, "the plan was stale"))
            .collect();
        retire(state, path, &plan, left);
        return;
    }
    let (ready, mut left) = sort_sessions(state, sessions);
    // The helper writes how it went once this engine answers health — a
    // rolled-back engine too: the old build, started again, reads
    // `rolled_back` here and continues every session at once. An engine the
    // helper then stops never gets past this, and leaves the plan whole.
    let outcome = helper_outcome(&crate::cli::restart::status_file(), requested, TAKEOVER).await;
    let (readable, unkept) = keep_pictures(state, ready);
    left.extend(unkept);
    left.extend(deliver(state, &plan, readable, outcome.as_ref()).await);
    retire(state, path, &plan, left);
}

/// A session's entry, kept for the record with why it was not continued.
fn not_delivered(session: &Value, why: &str) -> Value {
    let mut session = session.clone();
    session["not_delivered"] = json!(why);
    session
}

/// The sessions that can continue, and those that cannot (unreadable, or
/// deleted since).
fn sort_sessions(state: &ServerState, sessions: Vec<Value>) -> (Vec<Value>, Vec<Value>) {
    let mut ready = Vec::new();
    let mut left = Vec::new();
    for session in sessions {
        if !session_readable(&session) {
            left.push(not_delivered(&session, "its entry was unreadable"));
            continue;
        }
        let sid = session["session_id"].as_str().unwrap_or_default();
        if !matches!(
            state.manager.global_sessions.get_session_meta(sid),
            Ok(Some(_))
        ) {
            left.push(not_delivered(&session, "the session was deleted"));
            continue;
        }
        ready.push(session);
    }
    (ready, left)
}

/// Keep each queued picture as a file, its path in place of what was sent:
/// both runtimes can inspect the same paths with their ordinary Read tool.
/// A session whose picture cannot be kept is not continued.
fn keep_pictures(state: &ServerState, sessions: Vec<Value>) -> (Vec<Value>, Vec<Value>) {
    let mut readable = Vec::new();
    let mut left = Vec::new();
    for session in sessions {
        match with_kept_pictures(state, &session) {
            Some(kept) => readable.push(kept),
            None => left.push(not_delivered(
                &session,
                "a queued picture could not be kept",
            )),
        }
    }
    (readable, left)
}

fn with_kept_pictures(state: &ServerState, session: &Value) -> Option<Value> {
    let mut session = session.clone();
    let sid = session["session_id"].as_str()?.to_owned();
    let store = &state.manager.global_sessions;
    let images = session["queued"]
        .as_array_mut()
        .into_iter()
        .flatten()
        .filter_map(|item| item["images"].as_array_mut());
    for image in images.flatten() {
        let name = store.keep_images(&sid, &[image.as_str()?.to_owned()]).pop();
        let file = name.and_then(|n| store.image_path(&sid, &n))?;
        *image = Value::String(file.to_string_lossy().into_owned());
    }
    Some(session)
}

/// One continue turn to give: a session's, for one agent.
#[derive(Clone)]
struct Delivery {
    meta: SessionMeta,
    agent: String,
    message: String,
    mark: String,
    /// The session's entry as this agent is told it, for the record.
    scoped: Value,
}

/// Give every continue turn, trying a native one again when it could not
/// be kept (it never ran).
/// Returns the entries of those never kept.
async fn deliver(
    state: &Arc<ServerState>,
    plan: &Value,
    sessions: Vec<Value>,
    outcome: Option<&Value>,
) -> Vec<Value> {
    let requested = plan["requested_at"].as_u64().unwrap_or(0);
    let mut pending: Vec<Delivery> = Vec::new();
    for session in &sessions {
        pending.extend(deliveries_for(state, plan, session, outcome, requested));
    }
    let mut given_up = Vec::new();
    for attempt in 1..=ATTEMPTS {
        let tries = pending
            .iter()
            .cloned()
            .map(|d| attempt_delivery(state.clone(), d));
        let kept = futures_util::future::join_all(tries).await;
        pending = pending
            .into_iter()
            .zip(kept)
            .filter(|(_, kept)| !kept)
            .map(|(d, _)| d)
            .collect();
        // A Claude session's turn runs whether or not its row was kept:
        // it is never tried twice. A native one that was not kept never ran.
        let (native, claude): (Vec<_>, Vec<_>) = pending
            .into_iter()
            .partition(|d| d.meta.runtime.is_linggen());
        given_up.extend(claude);
        pending = native;
        if pending.is_empty() {
            break;
        }
        tracing::warn!(
            "[restart] {} continue turn(s) not kept (try {attempt} of {ATTEMPTS})",
            pending.len()
        );
        if attempt < ATTEMPTS {
            tokio::time::sleep(RETRY_GAP).await;
        }
    }
    given_up.extend(pending);
    given_up
        .iter()
        .map(|d| not_delivered(&d.scoped, "its continue turn could not be kept"))
        .collect()
}

/// One try at a continue turn; true once it is in the transcript (or was
/// already, or its session is gone), or it still waits for its place when
/// recovery's patience ends — it is in the turn line then, and comes.
async fn attempt_delivery(state: Arc<ServerState>, d: Delivery) -> bool {
    if told(&state.manager.global_sessions, &d.meta.id, &d.mark) {
        return true;
    }
    let sid = d.meta.id.clone();
    let turn = continue_session(state, d.meta, d.agent.clone(), d.message, d.mark);
    match tokio::time::timeout(DELIVERY_WAIT, turn).await {
        Ok(kept) => kept,
        Err(_) => {
            tracing::warn!("[restart] {sid}/{} still waits for its turn", d.agent);
            true
        }
    }
}

/// The continue turns a session's entry asks for: its agent's, and one for
/// each other agent that owned a task the restart ended. Those already told
/// (a replayed plan) are left out; the session's tasks are listed as
/// interrupted.
fn deliveries_for(
    state: &Arc<ServerState>,
    plan: &Value,
    session: &Value,
    outcome: Option<&Value>,
    requested: u64,
) -> Vec<Delivery> {
    let Some(sid) = session["session_id"].as_str() else {
        return Vec::new();
    };
    let Some(meta) = state
        .manager
        .global_sessions
        .get_session_meta(sid)
        .ok()
        .flatten()
    else {
        return Vec::new();
    };
    let agent = session["agent_id"]
        .as_str()
        .or_else(|| {
            session["queued"]
                .as_array()
                .and_then(|items| items.first())
                .and_then(|item| item["agent_id"].as_str())
        })
        .unwrap_or(if meta.runtime.is_linggen() {
            "ling"
        } else {
            "claude"
        });
    let tasks: Vec<crate::engine::events::BackgroundTask> = session["tasks"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|value| serde_json::from_value(value.clone()).ok())
        .collect();
    restore_interrupted_tasks(&state.tasks, sid, &tasks);
    if !tasks.is_empty() {
        super::tasks::emit(state, sid);
    }
    let mut owners = std::collections::BTreeSet::new();
    owners.insert(agent.to_owned());
    for task in tasks.iter().filter(|task| !task.child_owner) {
        if let Some(owner) = &task.owner_agent {
            owners.insert(owner.clone());
        }
    }
    let mut deliveries = Vec::new();
    for owner in owners {
        let scoped = scoped_to(session, &owner, agent);
        let mark = delivery_mark(requested, &owner, scoped["turn_interrupted"] == true);
        if told(&state.manager.global_sessions, sid, &mark) {
            tracing::info!("[restart] {sid}/{owner} was already told");
            continue;
        }
        tracing::info!("[restart] {sid}/{owner} continues");
        let message = continue_message(plan, &scoped, outcome);
        deliveries.push(Delivery {
            meta: meta.clone(),
            agent: owner,
            message,
            mark,
            scoped,
        });
    }
    deliveries
}

/// A session's entry as `owner` is told it: its own tasks only, and — when
/// it is not the session's agent — no turn, queue or call of its own.
fn scoped_to(session: &Value, owner: &str, agent: &str) -> Value {
    let mut scoped = session.clone();
    scoped["tasks"] = Value::Array(
        session["tasks"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|task| {
                task.is_string() && owner == agent
                    || task["owner_agent"].as_str() == Some(owner) && task["child_owner"] != true
            })
            .cloned()
            .collect(),
    );
    if owner != agent {
        scoped["caller"] = json!(false);
        scoped["turn_interrupted"] = json!(false);
        scoped["queued"] = json!([]);
    }
    scoped
}

/// Where the sessions a plan could not continue are kept, by its request.
fn unrecovered_file(requested_at: u64) -> PathBuf {
    crate::paths::linggen_home().join(format!("restart/unrecovered-{requested_at}.json"))
}

/// The plan is done: gone when every session continued; else what was not
/// delivered — the person's queued messages with it — is kept in the record
/// first, and the person is told it is there.
fn retire(state: &Arc<ServerState>, path: &std::path::Path, plan: &Value, left: Vec<Value>) {
    if !left.is_empty() {
        let requested = plan["requested_at"].as_u64().unwrap_or(0);
        let mut record = plan.clone();
        record["sessions"] = Value::Array(left);
        let file = unrecovered_file(requested);
        if let Err(error) = write_plan(&file, &record) {
            tracing::error!(
                "[restart] could not keep what was not delivered: {error}; preserving the plan"
            );
            return;
        }
        tracing::error!(
            "[restart] sessions not continued after the restart are kept in {}",
            file.display()
        );
        tokio::spawn(tell_the_person(state.clone(), file, record));
    }
    if let Err(error) = std::fs::remove_file(path) {
        tracing::error!("[restart] could not remove the recovery plan: {error}");
    }
}

/// A plan that does not parse: kept as it was, beside the plans retired.
fn retire_unreadable(path: &std::path::Path, bytes: &[u8]) {
    let file = unrecovered_file(crate::util::now_ts_secs());
    if let Err(error) = std::fs::rename(path, &file) {
        tracing::error!("[restart] could not set the unreadable plan aside: {error}");
        return;
    }
    tracing::error!(
        "[restart] the recovery plan did not parse ({} bytes); kept in {}",
        bytes.len(),
        file.display()
    );
}

/// The facts for Yinyue to tell the person, in her words: chats that could
/// not be picked up again after the restart, and where what they held is.
/// A deleted chat is nobody's to tell about.
async fn tell_the_person(state: Arc<ServerState>, file: PathBuf, record: Value) {
    let left: Vec<&Value> = record["sessions"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|s| s["not_delivered"] != "the session was deleted")
        .collect();
    if left.is_empty() {
        return;
    }
    let messages: usize = left
        .iter()
        .filter_map(|s| s["queued"].as_array())
        .map(Vec::len)
        .sum();
    let kickoff = format!(
        "After the engine restarted, {} chat(s) could not be picked up again ({messages} queued message(s) \
         of the user's among them). Nothing was lost: it is all kept in {}. Tell the user in one brief line.",
        left.len(),
        file.display()
    );
    crate::server::resident::wake_herald(state, kickoff, "neutral").await;
}

/// The mark a continue turn is kept under: one per plan and agent. One
/// that only reports background work, no turn having been cut, is marked
/// apart (see [`continues_a_turn`]).
fn delivery_mark(requested_at: u64, agent: &str, turn_interrupted: bool) -> String {
    if turn_interrupted {
        format!("restart-{requested_at}-{agent}")
    } else {
        format!("{TASKS_ONLY_MARK}{requested_at}-{agent}")
    }
}

const TASKS_ONLY_MARK: &str = "restart-tasks-";

/// Whether a row kept under `client_id` is a continue turn that goes on
/// with the turn a restart cut — it answers what that turn answered.
pub(crate) fn continues_a_turn(client_id: &str) -> bool {
    client_id.starts_with("restart-") && !client_id.starts_with(TASKS_ONLY_MARK)
}

/// Whether the session's transcript holds the continue turn marked `mark`.
fn told(store: &crate::state_fs::SessionStore, session: &str, mark: &str) -> bool {
    store.get_chat_history(session).is_ok_and(|rows| {
        rows.iter()
            .any(|row| row.client_id.as_deref() == Some(mark))
    })
}

fn restore_interrupted_tasks(
    store: &super::tasks::Tasks,
    session: &str,
    tasks: &[crate::engine::events::BackgroundTask],
) {
    for mut task in tasks.iter().cloned() {
        task.end("interrupted");
        store.upsert(session, task);
    }
}

/// Start a session's continue turn, marked `mark`; true once it is kept in
/// the transcript. It holds its place in the session's turn line from the
/// call on, so what is sent next waits behind it.
fn continue_session(
    state: Arc<ServerState>,
    meta: SessionMeta,
    agent: String,
    message: String,
    mark: String,
) -> impl std::future::Future<Output = bool> + Send + 'static {
    let place = meta.runtime.is_linggen().then(|| {
        let (kept, was_kept) = tokio::sync::oneshot::channel();
        tokio::spawn(super::tasks::tell_marked(
            state.clone(),
            meta.id.clone(),
            agent.clone(),
            message.clone(),
            (mark.clone(), kept),
        ));
        was_kept
    });
    async move {
        let kept = match place {
            Some(was_kept) => was_kept.await.is_ok(),
            None => {
                super::claude::start_marked_task(
                    state.clone(),
                    meta.id.clone(),
                    message.clone(),
                    mark.clone(),
                )
                .await;
                wait_until_shown(&state, &meta.id, &message).await;
                told(&state.manager.global_sessions, &meta.id, &mark)
            }
        };
        // A session deleted meanwhile has no one left to tell.
        kept || !matches!(
            state.manager.global_sessions.get_session_meta(&meta.id),
            Ok(Some(_))
        )
    }
}

/// A queued continue turn waits behind the running one: it is shown (kept)
/// as it leaves the queue, never after.
async fn wait_until_shown(state: &ServerState, session: &str, message: &str) {
    loop {
        let queued = state
            .queued_chats
            .lock()
            .await
            .values()
            .flatten()
            .any(|item| item.session_id == session && item.text == message);
        if !queued {
            return;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovered_tasks_are_interrupted_and_keep_their_owner() {
        let store = super::super::tasks::Tasks::default();
        let mut task = crate::engine::events::BackgroundTask::running("sh-1", "shell", "make");
        task.owner_agent = Some("yinyue".into());
        restore_interrupted_tasks(&store, "s", &[task]);
        let recovered = store.get("s", "sh-1").unwrap();
        assert_eq!(recovered.status, "interrupted");
        assert_eq!(recovered.owner_agent.as_deref(), Some("yinyue"));
        assert!(recovered.ended_at.is_some());
    }

    #[test]
    fn recovery_accepts_readable_queues_but_refuses_invalid_entries() {
        assert!(session_readable(&json!({"session_id": "s", "queued": []})));
        assert!(session_readable(&json!({"session_id": "s"})));
        assert!(session_readable(
            &json!({"session_id": "s", "queued": [{"text": "hello", "sender": "system"}]})
        ));
        assert!(!session_readable(
            &json!({"session_id": "s", "queued": [{}]})
        ));
        assert!(!session_readable(
            &json!({"session_id": "s", "queued": null})
        ));
        assert!(!session_readable(&json!({})));
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
            "s|ling".to_string(),
            vec![item(1, "s", "ling", "first")],
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
        assert_eq!(
            queued_messages(&*queue.lock().await)["s"][0]["text"],
            "second"
        );
    }

    /// The engine that asked for a restart watches for the helper's word:
    /// an older status is not this restart's, and with none it goes on
    /// after its patience — never stays frozen.
    #[tokio::test]
    async fn the_asking_engine_hears_when_the_helper_gave_up_on_it() {
        let dir = tempfile::tempdir().unwrap();
        let status = dir.path().join("status.json");
        std::fs::write(&status, json!({"state": "failed", "at": 99}).to_string()).unwrap();
        let wait = Duration::from_millis(600);
        assert!(
            helper_outcome(&status, 100, wait).await.is_none(),
            "an earlier status is not this restart's"
        );
        let writer = {
            let status = status.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(200)).await;
                std::fs::write(
                    &status,
                    json!({"state": "failed", "detail": "no probe", "at": 100}).to_string(),
                )
                .unwrap();
            })
        };
        let said = helper_outcome(&status, 100, Duration::from_secs(5))
            .await
            .unwrap();
        writer.await.unwrap();
        assert_eq!(said["state"], "failed");
        let told = not_restarted_message(&json!({"from_version": "2.0.0"}), Some(&said));
        assert!(
            told.contains("did not happen (the swap failed: no probe)"),
            "{told}"
        );
        assert!(told.contains("kept running 2.0.0"), "{told}");
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
        let other = json!({"caller": false, "agent_id": "ling", "tasks": ["npm run dev (sh-1)"]});
        let ok = json!({"state": "ok"});
        let said = continue_message(&plan, &caller, Some(&ok));
        assert!(
            said.contains("from 1.10.0 to ling 1.10.1; the new build is up and healthy"),
            "{said}"
        );
        assert!(said.contains("You asked for this restart"));
        assert!(said.contains("check the current state before redoing"));
        let said = continue_message(
            &plan,
            &other,
            Some(&json!({"state": "rolled_back", "detail": "no health"})),
        );
        assert!(
            said.contains("previous one was restored: no health"),
            "{said}"
        );
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
            vec![
                item(10, "s1", "ling", "first"),
                item(12, "s1", "ling", "third"),
            ],
        );
        all.insert(
            "s1|yinyue".to_string(),
            vec![item(11, "s1", "yinyue", "second")],
        );
        all.insert(
            "s2|claude".to_string(),
            vec![item(9, "s2", "claude", "alone")],
        );
        let queued = queued_messages(&all);
        let s1: Vec<&str> = queued["s1"]
            .iter()
            .filter_map(|m| m["text"].as_str())
            .collect();
        assert_eq!(s1, ["first", "second", "third"]);
        assert_eq!(queued["s1"][1]["agent_id"], "yinyue");
        assert_eq!(queued["s1"][1]["images"], json!(["second.png"]));
        assert_eq!(queued["s2"][0]["text"], "alone");
    }
}
