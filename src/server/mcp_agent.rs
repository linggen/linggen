//! `agent_run` — the MCP front door's delegation tool (mcp-spec.md).
//!
//! Lets any outside agent (Claude Code, Cursor, Codex via `/mcp`) hand a task
//! to a **local Linggen agent** — one that has this machine's skills, memory,
//! and configured models — and get its final answer back. The capability no
//! generic tool server can copy: it runs *your* agent, not a sandbox.
//!
//! One-shot and headless: a fresh session, the agent loop runs to completion,
//! the final assistant message is returned. Safe by default — the run is
//! non-interactive (no Linggen-side prompt can block an MCP caller who can't
//! see it) and granted Read on the workspace, so the delegate can read,
//! search, use memory, and drive the (separately-gated) browser, but can't
//! silently write files.
//!
//! With `write_dir` the caller opts in to writing: that one folder gets admin
//! (Bash, Write, Edit), every other path stays Read, and the run still never
//! prompts. The caller's own approval of the tool call is the grant — a coding
//! host hands its delegate a worktree and reviews the diff after.
//!
//! An action beyond the grants (never the hardcoded deny floor) is asked of
//! the CALLER: it waits as a permission request on the wait stream and in
//! `agent_status.pending` until `agent_permit` answers it, or counts as
//! denied after [`CALLER_ASK_WAIT`](crate::engine::tool_exec::CALLER_ASK_WAIT).

use std::path::PathBuf;
use std::sync::Arc;

use crate::engine::permission::PermissionMode;
use crate::server::ServerState;

/// The delegate's tool allowlist. Read / search / memory / browser only —
/// no Bash, Write, Edit, or Task, so the read-only boundary can't be worked
/// around (e.g. `echo > file` via Bash). Browser mutations still pass the
/// extension's own permission prompt. Widening to write is a future opt-in.
const AGENT_RUN_TOOLS: &[&str] = &[
    "Read",
    "Grep",
    "Glob",
    "WebSearch",
    "ImageSearch",
    "VideoSearch",
    "WebFetch",
    // The memory server by name, expanded to the tools it advertises — so
    // this list doesn't have to be re-edited when ling-mem gains a verb.
    "mcp__memory",
    "Skill",
    // Its checklist shows in the session, as in any chat (it writes nothing).
    "UpdatePlan",
    // Pictures, as in any chat; ChatGPT's lane when the caller asks for it.
    "GenerateImage",
    "Browser_navigate",
    "Browser_readPage",
    "Browser_screenshot",
    "Browser_click",
    "Browser_type",
    "Browser_key",
    "Browser_scroll",
    "Browser_wait",
    "Browser_readConsole",
    "Browser_tabs",
];

/// What a write run adds to the read-only toolset.
const WRITE_TOOLS: &[&str] = &["Bash", "Write", "Edit"];

/// The folder a write run may change: an existing directory, absolute, and
/// never `/` or the home folder itself — a task gets a checkout, not a disk.
fn write_root(dir: &str) -> Result<PathBuf, String> {
    let path = crate::util::resolve_path(std::path::Path::new(dir.trim()));
    if !path.is_absolute() || !path.is_dir() {
        return Err(format!(
            "write_dir must be an existing absolute folder: {dir}"
        ));
    }
    let home = dirs::home_dir().unwrap_or_default();
    if path.parent().is_none() || path == home {
        return Err(format!(
            "write_dir is too wide: {dir} — pass the task's own checkout"
        ));
    }
    Ok(path)
}

/// Truncate a prompt to a session title.
fn title_from_prompt(prompt: &str) -> String {
    let line = prompt.trim().lines().next().unwrap_or("").trim();
    let mut t: String = line.chars().take(60).collect();
    if line.chars().count() > 60 {
        t.push('…');
    }
    if t.is_empty() {
        "agent_run".to_string()
    } else {
        t
    }
}

/// The workspace root a delegated run operates in — the daemon's launch dir,
/// same convention `list_agents_api` uses as its fallback.
fn workspace_root() -> PathBuf {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    crate::util::resolve_path(&cwd)
}

/// The reply to the caller's latest prompt (see [`turn_replies`]).
fn final_reply(session_id: &str) -> Option<String> {
    session_replies(session_id).reply
}

fn session_replies(session_id: &str) -> TurnReplies {
    let store =
        crate::state_fs::SessionStore::with_sessions_dir(crate::paths::global_sessions_dir());
    store
        .get_chat_history(session_id)
        .map(turn_replies)
        .unwrap_or_default()
}

/// What a session's agent last said, split by whose turn it answered.
#[derive(Default, Debug, PartialEq)]
struct TurnReplies {
    /// The last agent message of the turn the latest caller prompt opened.
    reply: Option<String>,
    /// The last agent message of a later turn the engine told on its own
    /// (a background task's end), when there was one.
    followup: Option<String>,
}

/// A turn opens on a prompt (`user`, `claude-code`) or on the engine's own
/// notice (`system`, not an observation). Tool results are observations.
fn turn_replies(msgs: Vec<crate::state_fs::sessions::ChatMsg>) -> TurnReplies {
    let mut out = TurnReplies::default();
    let mut callers_turn = true;
    for m in msgs {
        if m.is_observation {
            continue;
        }
        match m.from_id.as_str() {
            "user" | "claude-code" => {
                out = TurnReplies::default();
                callers_turn = true;
            }
            "system" => callers_turn = false,
            _ if m.content.trim().is_empty() => {}
            _ if callers_turn => out.reply = Some(m.content),
            _ => out.followup = Some(m.content),
        }
    }
    out
}

/// Run one delegated agent turn. `Ok(text)` is the agent's final reply;
/// `Err(msg)` is a model-readable failure (unknown agent, run error).
pub async fn run(
    state: &Arc<ServerState>,
    agent: Option<&str>,
    prompt: &str,
    write_dir: Option<&str>,
    session: Option<&str>,
    model: Option<&str>,
    permission_mode: Option<&str>,
    host: Option<&str>,
    model_fallback: Option<bool>,
) -> Result<String, String> {
    let prompt = prompt.trim();
    if prompt.is_empty() {
        return Err("agent_run requires a non-empty prompt".to_string());
    }

    validate_options(session, write_dir, model)?;
    if session.is_some() && permission_mode.is_some() {
        return Err("permission_mode cannot be used with session_id".into());
    }
    let mode = permission_mapping(permission_mode, write_dir.is_some())?;
    if let Some(session) = session {
        let meta = session_meta(state, session)?;
        if !meta.runtime.is_linggen() {
            return Err("session is not a Linggen session".into());
        }
        let agent_id = agent
            .map(str::to_string)
            .or_else(|| meta.agents.first().map(|m| m.id.clone()))
            .unwrap_or_else(|| "ling".into());
        clear_denials(session);
        keep_delegate_tools(state, session, &meta, &agent_id).await;
        if let Some(allowed) = model_fallback {
            state
                .manager
                .global_sessions
                .set_model_fallback(session, allowed)
                .map_err(|e| format!("cannot set model_fallback: {e}"))?;
        }
        let turn = super::tasks::tell_from(
            state.clone(),
            session.into(),
            agent_id,
            prompt.into(),
            "claude-code".into(),
        );
        let sid = session.to_string();
        let manager = state.manager.clone();
        let task = tokio::spawn(async move {
            turn.await;
            // A turn that failed (its model above all) is an error to the
            // caller, named as the engine named it — not "no textual reply".
            if let Some(error) = failed_error(&manager, &sid) {
                return Err(format!("agent run failed: {error}"));
            }
            Ok(with_denials(
                &sid,
                final_reply(&sid).unwrap_or_else(|| NO_REPLY.into()),
            ))
        });
        return await_run(state, session, task).await;
    }
    let model = if let Some(model) = model {
        let models = state.manager.models.read().await;
        let resolved = models.resolve_id(model);
        drop(models);
        let config = state.manager.get_config_snapshot().await;
        Some(validate_model(
            model,
            resolved,
            &config
                .models
                .iter()
                .map(|m| m.id.clone())
                .collect::<Vec<_>>(),
        )?)
    } else {
        None
    };

    let root = workspace_root();
    let write = write_dir.map(write_root).transpose()?;
    let agent_id = agent
        .map(|a| a.trim().to_lowercase())
        .unwrap_or_else(|| "ling".to_string());

    // Validate the agent up front so an unknown name is a clean error, not a
    // half-created session — and list the real options for the caller.
    let here = state.manager.agent_present_now(&agent_id).await;
    if !here || !state.manager.agent_exists(&root, &agent_id).await {
        let names: Vec<String> = state
            .manager
            .list_agents(&root)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|s| s.name)
            .collect();
        return Err(format!(
            "unknown agent '{agent_id}'. Available: {}",
            if names.is_empty() {
                "(none)".into()
            } else {
                names.join(", ")
            }
        ));
    }

    // A session that may not switch models runs on the model its agent is
    // meant to have — not the next one down the chain when that one's sign-in
    // is missing — so the run fails on it, named, and the pin keeps the
    // session's later turns there.
    let model = match model {
        None if !new_session_fallback(model_fallback) => {
            state.manager.intended_model_id(&root, &agent_id).await
        }
        chosen => chosen,
    };

    // Fresh, visible session (creator "agent" → shows in the session list with
    // an agent badge). Titled from the prompt.
    let session_id = format!(
        "sess-{}-{}",
        crate::util::now_ts_secs(),
        &uuid::Uuid::new_v4().to_string()[..8]
    );
    let title = title_from_prompt(prompt);
    // A write run lives in its folder; a read run in the workspace.
    let root_str = write
        .as_ref()
        .unwrap_or(&root)
        .to_string_lossy()
        .to_string();
    let store =
        crate::state_fs::SessionStore::with_sessions_dir(crate::paths::global_sessions_dir());
    let mut meta = delegate_meta(
        &session_id,
        &title,
        &agent_id,
        model.clone(),
        host.unwrap_or("claude-code"),
        mode.is_some(),
        model_fallback,
    );
    meta.set_cwd(&root_str);
    if let Err(e) = store.add_session(&meta) {
        return Err(format!("failed to create session: {e}"));
    }

    // Persist the permission posture to the session's permission.json BEFORE
    // the run: the engine's `initialize_loop` reloads permissions from disk at
    // the top of `run_agent_loop`, so any in-memory mutation would be clobbered.
    // Safe-by-default — non-interactive (a headless MCP delegate has no
    // Linggen-side user to answer a prompt, so a permission-needed action
    // silently denies and the agent continues) and Read on the workspace.
    {
        let mut perms = crate::engine::permission::SessionPermissions {
            interactive: false,
            // Beyond the grants, ask the caller (agent_permit) — not refuse.
            ask_caller: true,
            ..Default::default()
        };
        if permission_mode.is_none() && write.is_none() {
            perms.set_path_mode(&root_str, PermissionMode::Read);
        } else {
            perms.set_path_mode("*", PermissionMode::Read);
            if let Some(grant) = mode {
                perms.set_path_mode(&root_str, grant);
            }
        }
        perms.save(&crate::paths::global_sessions_dir().join(&session_id));
    }
    let _ = state
        .events_tx
        .send(crate::server::ServerEvent::SessionCreated {
            session_id: session_id.clone(),
            title,
            creator: "external".into(),
            project: meta.project.clone(),
            project_name: meta.project_name.clone(),
            skill: None,
            mission_id: None,
        });

    // Persist the prompt as the first user turn so the session reads naturally.
    let _ = store.add_chat_message(
        &session_id,
        &crate::state_fs::sessions::ChatMsg {
            run_id: None,
            via: None,
            via_note: None,
            agent_id: agent_id.clone(),
            from_id: "user".to_string(),
            to_id: agent_id.clone(),
            content: prompt.to_string(),
            timestamp: crate::util::now_ts_secs(),
            is_observation: false,
            client_id: None,
            failed: false,
            images: Vec::new(),
        },
    );

    // The run outlives the call: a caller's tool timeout must not drop it
    // halfway (2026-11-06: a write run died at Claude Code's ~1 min limit
    // and left its session spinning). Wait a while, else hand back where
    // the answer will be.
    clear_denials(&session_id);
    // Reserve the session turn synchronously, before a watcher can observe idle.
    // Keep it through drive, including failures, so no startup gap looks finished.
    let (_, place) = super::chat::helpers::take_turn_place(state.manager.session_turn(&session_id));
    let run_state = state.clone();
    let run_session = session_id.clone();
    let prompt = prompt.to_string();
    let run = tokio::spawn(async move {
        let _turn = place.await;
        drive(
            run_state,
            run_session,
            // The agent stands where it may write: a relative path then lands in
            // the grant (2026-11-06: Write review.json resolved against the
            // daemon's launch dir and was refused).
            write.unwrap_or(root),
            agent_id,
            prompt.to_string(),
            mode.is_some(),
            model,
        )
        .await
    });
    await_run(state, &session_id, run).await
}

/// How long a call waits for the run before handing back its session.
const WAIT_SECS: u64 = 45;

const NO_REPLY: &str = "(the agent finished without a textual reply)";

/// The session param that keeps a delegate's toolset: `read` or `write`.
const TOOLS_PARAM: &str = "agent_run_tools";

/// Wait a while for the run; hand back its session sooner when it asks the
/// caller for a permission (the caller can't answer while blocked here).
async fn await_run(
    state: &Arc<ServerState>,
    session: &str,
    run: tokio::task::JoinHandle<Result<String, String>>,
) -> Result<String, String> {
    let deadline = tokio::time::sleep(std::time::Duration::from_secs(WAIT_SECS));
    tokio::pin!(deadline);
    let mut events = state.events_tx.subscribe();
    tokio::pin!(run);
    loop {
        if !pending_requests(state, session).await.is_empty() {
            return Ok(still_running(state, session).await);
        }
        tokio::select! {
            done = &mut run => return done.unwrap_or_else(|e| Err(format!("agent run failed: {e}"))),
            _ = &mut deadline => return Ok(still_running(state, session).await),
            event = events.recv() => if matches!(event, Err(tokio::sync::broadcast::error::RecvError::Closed)) {
                return Ok(still_running(state, session).await);
            },
        }
    }
}

/// The delegate's toolset: read / search / memory / browser, plus Bash,
/// Write and Edit for a write run.
fn delegate_tool_scope(write: bool) -> Option<std::collections::HashSet<String>> {
    let extra: &[&str] = if write { WRITE_TOOLS } else { &[] };
    let declared: Vec<String> = AGENT_RUN_TOOLS
        .iter()
        .chain(extra)
        .map(|s| s.to_string())
        .collect();
    crate::engine::tool_scope::compute_tool_scope(&declared)
}

/// A continued session keeps the toolset it was started with — its grants
/// already live in its permission.json. A cached engine still holds it; one
/// rebuilt since (a restart) gets it back here. A busy engine is mid-turn
/// with its toolset in place.
async fn keep_delegate_tools(
    state: &Arc<ServerState>,
    session: &str,
    meta: &crate::state_fs::sessions::SessionMeta,
    agent_id: &str,
) {
    let Some(kind) = meta.params.get(TOOLS_PARAM) else {
        return;
    };
    let root = PathBuf::from(meta.project.clone().unwrap_or_else(|| meta.cwd.clone()));
    let Ok(agent) = state
        .manager
        .get_or_create_session_agent(session, &root, agent_id)
        .await
    else {
        return;
    };
    let Ok(mut engine) = agent.try_lock() else {
        return;
    };
    engine.cfg.mission_allowed_tools = delegate_tool_scope(kind == "write");
}

/// One headless run in its own task, to completion whether or not anyone
/// still waits for it.
async fn drive(
    state: Arc<ServerState>,
    session_id: String,
    root: PathBuf,
    agent_id: String,
    prompt: String,
    write: bool,
    model: Option<String>,
) -> Result<String, String> {
    let agent = state
        .manager
        .get_or_create_session_agent(&session_id, &root, &agent_id)
        .await
        .map_err(|e| format!("failed to create agent '{agent_id}': {e}"))?;
    let mut engine = agent.lock().await;
    if let Some(model) = model {
        engine.model_id = model;
    }

    let run_id = state
        .manager
        .begin_agent_run(
            &root,
            Some(&session_id),
            &agent_id,
            None,
            Some("agent_run".into()),
        )
        .await
        .map_err(|e| format!("failed to begin run: {e}"))?;
    let _life = state.manager.run_life(&run_id);

    // Browser bridge, and the AskUser bridge its permission requests wait
    // on (the AskUser TOOL stays out of its toolset: no person is there).
    engine.tools.set_browser_bridge(state.bridge.clone());
    engine
        .tools
        .set_ask_user_bridge(Arc::new(crate::engine::tools::AskUserBridge {
            events_tx: state.events_tx.clone(),
            pending: state.pending_ask_user.clone(),
            session_id: Some(session_id.clone()),
        }));

    engine.observations.clear();
    engine.task = Some(prompt.to_string());
    engine.set_parent_agent(None);
    engine.set_run_id(Some(run_id.clone()));

    // Restrict the toolset to read / memory / browser. This is the real
    // boundary — the permission.json Read grant stops Write/Edit, and dropping
    // Bash stops the `echo > file` end-run around it. cfg isn't reloaded by
    // initialize_loop, so this sticks. (It also flips the session
    // non-interactive on load, matching the permission.json.)
    engine.cfg.mission_allowed_tools = delegate_tool_scope(write);

    // Register the turn just as typed chat does, including its terminal event.
    let model_label = engine.model_id.clone();
    let result = with_turn_status(
        async {
            state
                .send_agent_status(
                    agent_id.clone(),
                    crate::server::AgentStatusKind::ModelLoading,
                    Some(format!("Loading model: {model_label}")),
                    None,
                    Some(session_id.clone()),
                )
                .await;
            let _ = state
                .events_tx
                .send(crate::server::ServerEvent::StateUpdated);
        },
        async {
            let result = engine.run_agent_loop(Some(&session_id)).await;
            let (status, error) = match &result {
                Ok(_) => (crate::engine::agent::AgentRunStatus::Completed, None),
                Err(e) => (
                    crate::engine::agent::AgentRunStatus::Failed,
                    Some(e.to_string()),
                ),
            };
            // The session tells a failure as a chat turn's does, before the
            // turn closes.
            if let Err(e) = &result {
                crate::server::chat::helpers::persist_and_emit_message(
                    &state.manager,
                    &state.events_tx,
                    &root,
                    &agent_id,
                    &agent_id,
                    "user",
                    &crate::server::chat::helpers::format_turn_error(&e.to_string()),
                    Some(&session_id),
                    false,
                    Some(&run_id),
                )
                .await;
            }
            let _ = state.manager.finish_agent_run(&run_id, status, error).await;
            result
        },
        || {
            crate::server::chat::close_turn(
                &state,
                agent_id.clone(),
                Some(session_id.clone()),
                Some(run_id.clone()),
            )
        },
    )
    .await;
    engine.set_run_id(None);
    seed_first_prompt(&mut engine.chat_history, &prompt);
    drop(engine);

    let _ = state
        .events_tx
        .send(crate::server::ServerEvent::StateUpdated);
    match result {
        Ok(_) => Ok(with_denials(
            &session_id,
            final_reply(&session_id).unwrap_or_else(|| NO_REPLY.to_string()),
        )),
        Err(e) => {
            let model = state
                .manager
                .run_store
                .outcome(&session_id)
                .and_then(|o| o.model);
            Err(format!(
                "agent run failed: {}",
                naming_model(e.to_string(), model.as_deref())
            ))
        }
    }
}

/// The session record of a new delegated run: titled from the prompt, made by
/// an `external` host, its model pinned, its toolset kept for follow-ups, and
/// its model-fallback switch off unless the caller asked for it.
fn delegate_meta(
    id: &str,
    title: &str,
    agent_id: &str,
    model: Option<String>,
    host: &str,
    write: bool,
    model_fallback: Option<bool>,
) -> crate::state_fs::sessions::SessionMeta {
    let mut meta = crate::state_fs::sessions::SessionMeta {
        runtime: Default::default(),
        claude_permission_mode: Default::default(),
        parent_session_id: None,
        id: id.to_string(),
        title: title.to_string(),
        created_at: crate::util::now_ts_secs(),
        updated_at: 0,
        skill: None,
        creator: "external".into(),
        cwd: String::new(),
        project: None,
        project_name: None,
        mission_id: None,
        agents: vec![crate::state_fs::sessions::SessionMember::new(agent_id)],
        legacy: Default::default(),
        user_id: None,
        compact_threshold: None,
        compact_focus: None,
        title_locked: true,
        withheld_tools: Vec::new(),
        // A delegated run that cannot use its model ends there, named, for the
        // calling agent to decide — unless the caller opts back in.
        model_fallback: Some(new_session_fallback(model_fallback)),
        params: Default::default(),
    };
    meta.params.insert("host".into(), host.into());
    // The delegate's toolset, so a continued session keeps it (see
    // `keep_delegate_tools`).
    meta.params.insert(
        TOOLS_PARAM.into(),
        if write { "write" } else { "read" }.into(),
    );
    meta.set_member_model(agent_id, model);
    meta
}

/// A new delegated session's model-fallback switch: off unless the caller
/// asked for it.
fn new_session_fallback(requested: Option<bool>) -> bool {
    requested.unwrap_or(false)
}

/// An error as the caller reads it, with the model that failed named: a
/// switching-off failure already names it; a sign-in refusal or a bad request
/// may not. The chat's own markers (`AUTH_REQUIRED:` …) stay in the text.
fn naming_model(error: String, model: Option<&str>) -> String {
    match model {
        Some(m) if !error.contains(m) => format!("model '{m}' failed: {error}"),
        _ => error,
    }
}

/// The error that ended the session's latest run, when it failed.
fn failed_error(manager: &crate::engine::agent::AgentManager, session: &str) -> Option<String> {
    let outcome = manager.run_store.outcome(session)?;
    (outcome.status == crate::engine::agent::AgentRunStatus::Failed).then(|| {
        naming_model(
            outcome.error.unwrap_or_else(|| "the run failed".into()),
            outcome.model.as_deref(),
        )
    })
}

/// Open and close the UI turn around every outcome of the driven loop.
async fn with_turn_status<T, F: std::future::Future<Output = ()>>(
    start: impl std::future::Future<Output = ()>,
    run: impl std::future::Future<Output = T>,
    close: impl FnOnce() -> F,
) -> T {
    start.await;
    let result = run.await;
    close().await;
    result
}

/// Keep the bootstrap task in the cached thread, before the agent's replies.
fn seed_first_prompt(history: &mut Vec<crate::message::ChatMessage>, prompt: &str) {
    history.insert(
        0,
        crate::message::ChatMessage::new("user", prompt.to_string()),
    );
}

/// What a caller gets while the run goes on: where to watch, and any
/// permission requests already waiting on it.
async fn still_running(state: &Arc<ServerState>, session: &str) -> String {
    let mut text = still_running_text(state.port, session);
    let pending = pending_requests(state, session).await;
    if !pending.is_empty() {
        text.push_str(&format!(
            "\nWaiting for permission: {}",
            serde_json::Value::Array(pending)
        ));
    }
    text
}

fn still_running_text(port: u16, session: &str) -> String {
    format!(
        "Still running in the background in Linggen session {session}. Don't poll: \
        watch ws://127.0.0.1:{port}/api/agent/wait?session_id={session} with a background \
        watcher. It sends {{\"type\":\"permission_request\"}} (answer with agent_permit) and \
        {{\"type\":\"finished\"}} (then read agent_status). Start a new watch for every agent_run, including continued sessions. If it closes or expires without finished, call agent_status once; never poll."
    )
}

fn validate_options(
    session: Option<&str>,
    write: Option<&str>,
    model: Option<&str>,
) -> Result<(), String> {
    if session.is_some() && write.is_some() {
        return Err("write_dir cannot be used with session_id".into());
    }
    if session.is_some() && model.is_some() {
        return Err("model cannot be used with session_id".into());
    }
    Ok(())
}

fn validate_model(model: &str, resolved: Option<String>, ids: &[String]) -> Result<String, String> {
    resolved.ok_or_else(|| format!("unknown model '{model}'. Available: {}", ids.join(", ")))
}

fn session_meta(
    state: &ServerState,
    session: &str,
) -> Result<crate::state_fs::sessions::SessionMeta, String> {
    let meta = state
        .manager
        .global_sessions
        .get_session_meta(session)
        .map_err(|e| format!("cannot read session: {e}"))?;
    require_session(session, meta)
}

fn require_session<T>(session: &str, meta: Option<T>) -> Result<T, String> {
    meta.ok_or_else(|| format!("unknown session '{session}'"))
}

pub async fn status(state: &Arc<ServerState>, session: &str) -> Result<String, String> {
    session_meta(state, session)?;
    let work = super::api::sessions::session_running_work(state, session).await;
    let last = last_run(&state.manager, session);
    let replies = if work.turn {
        TurnReplies::default()
    } else {
        session_replies(session)
    };
    Ok(
        serde_json::json!({"running": work.turn, "tasks": work.tasks,
        "subagents": work.subagents, "pending": pending_requests(state, session).await,
        "denied": denials(session), "reply": replies.reply, "followup": replies.followup,
        "status": last.status, "error": last.error, "model": last.model})
        .to_string(),
    )
}

/// The permission requests `session` is waiting on, for its caller.
async fn pending_requests(state: &ServerState, session: &str) -> Vec<serde_json::Value> {
    caller_requests(&*state.pending_ask_user.lock().await, session)
}

fn caller_requests(
    pending: &std::collections::HashMap<String, crate::engine::tools::PendingAskUser>,
    session: &str,
) -> Vec<serde_json::Value> {
    let mut rows: Vec<_> = pending
        .iter()
        .filter(|(_, entry)| entry.session_id.as_deref() == Some(session))
        .filter_map(|(id, entry)| Some(request_json(id, entry.caller_request.as_ref()?)))
        .collect();
    rows.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
    rows
}

fn request_json(
    id: &str,
    request: &crate::engine::tools::CallerPermissionRequest,
) -> serde_json::Value {
    let mut row = serde_json::to_value(request).unwrap_or_default();
    row["id"] = id.into();
    row
}

/// `agent_permit`: answer one permission request of an `agent_run`
/// session. Only that session's own caller requests are answerable here —
/// never a person's prompt in another chat.
pub async fn permit(
    state: &Arc<ServerState>,
    session: &str,
    request_id: &str,
    decision: &str,
    scope: Option<&str>,
) -> Result<String, String> {
    session_meta(state, session)?;
    let session_dir = crate::paths::global_sessions_dir().join(session);
    if !crate::engine::permission::SessionPermissions::load(&session_dir).ask_caller {
        return Err(format!(
            "session '{session}' does not ask its caller for permissions"
        ));
    }
    let label = answer_request(
        &state.pending_ask_user,
        session,
        request_id,
        decision,
        scope,
    )
    .await?;
    let _ = state
        .events_tx
        .send(crate::server::ServerEvent::WidgetResolved {
            widget_id: request_id.to_string(),
            session_id: Some(session.to_string()),
        });
    Ok(
        serde_json::json!({"request_id": request_id, "decision": decision, "answer": label})
            .to_string(),
    )
}

/// Take the request and send the decision's answer to the waiting run.
async fn answer_request(
    pending: &tokio::sync::Mutex<
        std::collections::HashMap<String, crate::engine::tools::PendingAskUser>,
    >,
    session: &str,
    request_id: &str,
    decision: &str,
    scope: Option<&str>,
) -> Result<String, String> {
    let mut pending = pending.lock().await;
    let entry = pending
        .get(request_id)
        .ok_or_else(|| unknown_request(request_id))?;
    let label = decision_label(entry, session, decision, scope)?;
    let entry = pending
        .remove(request_id)
        .ok_or_else(|| unknown_request(request_id))?;
    let answer = crate::engine::tools::AskUserAnswer {
        question_index: 0,
        selected: vec![label.clone()],
        custom_text: None,
    };
    entry
        .sender
        .send(vec![answer])
        .map_err(|_| format!("request '{request_id}' already expired"))?;
    Ok(label)
}

fn unknown_request(request_id: &str) -> String {
    format!("unknown permission request '{request_id}' (answered, expired, or not this session's)")
}

/// The offered option a decision selects. `allow_session` grants the target
/// (`scope: path`) or its folder (`scope: dir`, the default when grantable)
/// at the requested level — never a session-wide grant.
fn decision_label(
    entry: &crate::engine::tools::PendingAskUser,
    session: &str,
    decision: &str,
    scope: Option<&str>,
) -> Result<String, String> {
    use crate::engine::permission::{grant_at_label, ALLOW_ONCE};
    let request = entry
        .caller_request
        .as_ref()
        .filter(|_| entry.session_id.as_deref() == Some(session))
        .ok_or_else(|| "this request is not this session's to answer".to_string())?;
    let label = match (decision, scope) {
        ("allow_once", _) => ALLOW_ONCE.to_string(),
        ("deny", _) => "Deny".to_string(),
        ("allow_session", Some("path")) => grant_at_label(&request.level, &request.path),
        ("allow_session", Some("dir")) => grant_at_label(
            &request.level,
            request
                .dir
                .as_deref()
                .ok_or("this target's folder cannot be granted; use scope \"path\"")?,
        ),
        ("allow_session", None) => grant_at_label(
            &request.level,
            request.dir.as_deref().unwrap_or(&request.path),
        ),
        ("allow_session", Some(other)) => {
            return Err(format!("unknown scope '{other}': use path or dir"))
        }
        (other, _) => {
            return Err(format!(
                "unknown decision '{other}': use allow_once, allow_session or deny"
            ))
        }
    };
    let offered = entry
        .questions
        .first()
        .is_some_and(|q| q.options.iter().any(|o| o.label == label));
    if !offered {
        return Err(format!("'{label}' was not offered for this request"));
    }
    Ok(label)
}

fn denials(session: &str) -> Vec<crate::engine::tool_exec::DeniedAction> {
    crate::engine::tool_exec::DENIALS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(session)
}
fn clear_denials(session: &str) {
    crate::engine::tool_exec::DENIALS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clear(session);
}
fn with_denials(session: &str, mut reply: String) -> String {
    let rows = denials(session);
    if !rows.is_empty() {
        reply.push_str("\n\nDenied:");
        for row in rows {
            reply.push_str(&format!(
                "\n- {} {} ({}): {}",
                row.tool, row.summary, row.mode, row.reason
            ));
        }
    }
    reply
}

fn wait_ready(work: &super::api::sessions::RunningWork, pending_turn: bool) -> bool {
    !work.turn && work.tasks == 0 && work.subagents == 0 && !pending_turn
}

#[derive(serde::Deserialize)]
pub(crate) struct WaitQuery {
    session_id: String,
}

pub(crate) async fn wait_socket(
    axum::extract::State(state): axum::extract::State<Arc<ServerState>>,
    axum::extract::Query(query): axum::extract::Query<WaitQuery>,
    headers: axum::http::HeaderMap,
    ws: axum::extract::ws::WebSocketUpgrade,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    if headers.get(axum::http::header::ORIGIN).is_some_and(|v| {
        !v.to_str()
            .map(super::loopback_guard::origin_is_loopback_page)
            .unwrap_or(false)
    }) {
        return (axum::http::StatusCode::FORBIDDEN, "origin not allowed").into_response();
    }
    if session_meta(&state, &query.session_id).is_err() {
        return (axum::http::StatusCode::NOT_FOUND, "unknown session").into_response();
    }
    let events = state.events_tx.subscribe();
    ws.on_upgrade(move |mut socket| async move {
        let session = query.session_id.as_str();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let watch = watch_session(session, events, tx, WaitTiming {
            tick: std::time::Duration::from_secs(2),
            grace: std::time::Duration::from_secs(2),
            resend: std::time::Duration::from_secs(60),
        }, || async {
            let pending = pending_requests(&state, session).await;
            let work = super::api::sessions::session_running_work(&state, session).await;
            (pending, wait_ready(&work, state.manager.has_pending_turn(session)))
        });
        tokio::pin!(watch);
        let mut watching = true;
        let mut ping = tokio::time::interval_at(tokio::time::Instant::now() + std::time::Duration::from_secs(20), std::time::Duration::from_secs(20));
        loop {
            tokio::select! {
                biased;
                frame = rx.recv() => match frame {
                    Some(WatchFrame::Permission(frame)) => if socket.send(axum::extract::ws::Message::Text(frame.to_string().into())).await.is_err() { break; },
                    Some(WatchFrame::Finished) => {
                        let frame = finished_frame(session, last_run(&state.manager, session), final_reply(session), denials(session));
                        let _ = socket.send(axum::extract::ws::Message::Text(frame.to_string().into())).await;
                        let _ = socket.send(axum::extract::ws::Message::Close(None)).await;
                        break;
                    }
                    None => break,
                },
                _ = &mut watch, if watching => watching = false,
                _ = ping.tick() => if socket.send(axum::extract::ws::Message::Ping(Default::default())).await.is_err() { break; },
                incoming = socket.recv() => if incoming.is_none() || matches!(incoming, Some(Err(_)) | Some(Ok(axum::extract::ws::Message::Close(_)))) { break; },
            }
        }
    })
}

struct WaitTiming {
    tick: std::time::Duration,
    grace: std::time::Duration,
    resend: std::time::Duration,
}

enum WatchFrame {
    Permission(serde_json::Value),
    Finished,
}

/// Probe on broadcasts and on a timer: completion requires continuous observed
/// idleness, so a short gap before a background follow-up is not terminal.
async fn watch_session<E, P, F>(
    session: &str,
    mut events: tokio::sync::broadcast::Receiver<E>,
    sink: tokio::sync::mpsc::UnboundedSender<WatchFrame>,
    timing: WaitTiming,
    mut probe: P,
) where
    E: Clone,
    P: FnMut() -> F,
    F: std::future::Future<Output = (Vec<serde_json::Value>, bool)>,
{
    let mut told = std::collections::HashMap::new();
    let mut ready_since = None;
    let mut tick = tokio::time::interval_at(tokio::time::Instant::now() + timing.tick, timing.tick);
    loop {
        let (pending, ready) = probe().await;
        let now = tokio::time::Instant::now();
        for frame in new_requests(&pending, &mut told, session, now, timing.resend) {
            if sink.send(WatchFrame::Permission(frame)).is_err() {
                return;
            }
        }
        if ready {
            let since = ready_since.get_or_insert(now);
            if now.duration_since(*since) >= timing.grace {
                let _ = sink.send(WatchFrame::Finished);
                return;
            }
        } else {
            ready_since = None;
        }
        tokio::select! {
            _ = tick.tick() => {},
            event = events.recv() => if matches!(event, Err(tokio::sync::broadcast::error::RecvError::Closed)) { return; },
            _ = sink.closed() => return,
        }
    }
}

/// Stream event types a watcher matches on.
const PERMISSION_REQUEST_EVENT: &str = "permission_request";
const FINISHED_EVENT: &str = "finished";

/// `permission_request` frames for requests not told yet.
fn new_requests(
    pending: &[serde_json::Value],
    told: &mut std::collections::HashMap<String, tokio::time::Instant>,
    session: &str,
    now: tokio::time::Instant,
    interval: std::time::Duration,
) -> Vec<serde_json::Value> {
    told.retain(|id, _| pending.iter().any(|row| row["id"].as_str() == Some(id)));
    pending
        .iter()
        .filter(|row| row["id"].as_str().is_some_and(|id| {
            if told.get(id).is_some_and(|last| now.duration_since(*last) < interval) { return false; }
            told.insert(id.to_string(), now);
            true
        }))
        .map(|row| serde_json::json!({"type": PERMISSION_REQUEST_EVENT, "session_id": session, "request": row}))
        .collect()
}

/// The stream's last frame: how the session's work ended, its reply, what
/// was denied, and — for a failed run — `error` (the model named, with the
/// provider's reason) — plus `model`, the one that answered its last call.
/// Earlier keys (`session_id`, `status`, `reply`, `denied`) stay.
fn finished_frame(
    session: &str,
    last: LastRun,
    reply: Option<String>,
    denied: Vec<crate::engine::tool_exec::DeniedAction>,
) -> serde_json::Value {
    serde_json::json!({"type": FINISHED_EVENT, "session_id": session, "status": last.status,
        "reply": reply, "denied": denied, "error": last.error, "model": last.model})
}

/// How the session's latest top-level run ended or stands.
#[derive(Debug, PartialEq, Eq)]
struct LastRun {
    /// `running`, `completed`, `failed` or `cancelled` — `completed` when no
    /// run is on record (an idle session, or one from before a restart).
    status: &'static str,
    /// Why a failed run failed.
    error: Option<String>,
    /// The model that answered the run's last model call (or failed it) —
    /// not the one asked for, when a fallback was allowed and happened.
    model: Option<String>,
}

fn last_run(manager: &crate::engine::agent::AgentManager, session: &str) -> LastRun {
    use crate::engine::agent::AgentRunStatus;
    let store = &manager.run_store;
    let runs = store.list_runs(Some(session));
    let running = runs
        .iter()
        .filter(|r| r.parent_run_id.is_none() && r.status == AgentRunStatus::Running)
        .max_by_key(|r| r.started_at);
    if let Some(run) = running {
        return LastRun {
            status: "running",
            error: None,
            model: run.model.clone(),
        };
    }
    match store.outcome(session) {
        Some(o) => LastRun {
            status: match o.status {
                AgentRunStatus::Failed => "failed",
                AgentRunStatus::Cancelled => "cancelled",
                AgentRunStatus::Running => "running",
                AgentRunStatus::Completed => "completed",
            },
            error: o.error.map(|e| naming_model(e, o.model.as_deref())),
            model: o.model,
        },
        None => LastRun {
            status: "completed",
            error: None,
            model: None,
        },
    }
}

#[cfg(test)]
mod wait_ready_tests {
    use super::*;
    use crate::server::api::sessions::RunningWork;
    #[test]
    fn wait_ready_requires_all_work_to_finish() {
        assert!(wait_ready(&RunningWork::default(), false));
        assert!(!wait_ready(&RunningWork::default(), true));
        assert!(!wait_ready(
            &RunningWork {
                turn: true,
                ..Default::default()
            },
            false
        ));
        assert!(!wait_ready(
            &RunningWork {
                tasks: 1,
                ..Default::default()
            },
            false
        ));
        assert!(!wait_ready(
            &RunningWork {
                subagents: 1,
                ..Default::default()
            },
            false
        ));
    }

    #[tokio::test]
    async fn reserved_turn_stays_busy_until_released() {
        let manager = super::permit_tests::manager();
        let (_, place) =
            crate::server::chat::helpers::take_turn_place(manager.session_turn("reserved"));
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        assert!(!wait_ready(
            &RunningWork::default(),
            manager.has_pending_turn("reserved")
        ));
        drop(place);
        assert!(wait_ready(
            &RunningWork::default(),
            manager.has_pending_turn("reserved")
        ));
    }

    fn watcher(
        ready: Arc<std::sync::atomic::AtomicBool>,
        pending: Arc<std::sync::Mutex<Vec<serde_json::Value>>>,
        manager: Option<Arc<crate::engine::agent::AgentManager>>,
    ) -> (
        tokio::sync::broadcast::Sender<()>,
        tokio::sync::mpsc::UnboundedReceiver<WatchFrame>,
        tokio::task::JoinHandle<()>,
    ) {
        let (events, rx) = tokio::sync::broadcast::channel(8);
        let (tx, frames) = tokio::sync::mpsc::unbounded_channel();
        let task = tokio::spawn(async move {
            watch_session(
                "reserved",
                rx,
                tx,
                WaitTiming {
                    tick: std::time::Duration::from_millis(20),
                    grace: std::time::Duration::from_millis(40),
                    resend: std::time::Duration::from_millis(60),
                },
                || {
                    let rows = pending.lock().unwrap().clone();
                    let idle = ready.load(std::sync::atomic::Ordering::SeqCst)
                        && !manager
                            .as_ref()
                            .is_some_and(|m| m.has_pending_turn("reserved"));
                    async move { (rows, idle) }
                },
            )
            .await;
        });
        (events, frames, task)
    }

    #[tokio::test]
    async fn idle_without_broadcast_finishes_once() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let ready = Arc::new(AtomicBool::new(false));
        let (_events, mut frames, task) = watcher(ready.clone(), Default::default(), None);
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        ready.store(true, Ordering::SeqCst);
        assert!(matches!(
            tokio::time::timeout(std::time::Duration::from_millis(180), frames.recv())
                .await
                .unwrap(),
            Some(WatchFrame::Finished)
        ));
        task.await.unwrap();
        assert!(frames.recv().await.is_none());
    }

    #[tokio::test]
    async fn transient_idle_resets_grace() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let ready = Arc::new(AtomicBool::new(true));
        let (events, mut frames, task) = watcher(ready.clone(), Default::default(), None);
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        ready.store(false, Ordering::SeqCst);
        events.send(()).unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(90)).await;
        assert!(frames.try_recv().is_err());
        task.abort();
    }

    #[tokio::test]
    async fn pending_before_watch_repeats_until_answered() {
        let rows = Arc::new(std::sync::Mutex::new(vec![serde_json::json!({"id":"r1"})]));
        let (events, mut frames, task) = watcher(
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
            rows.clone(),
            None,
        );
        for _ in 0..2 {
            let frame = tokio::time::timeout(std::time::Duration::from_millis(200), frames.recv())
                .await
                .unwrap();
            assert!(
                matches!(frame, Some(WatchFrame::Permission(value)) if value["request"]["id"] == "r1")
            );
        }
        rows.lock().unwrap().clear();
        events.send(()).unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert!(frames.try_recv().is_err());
        task.abort();
    }

    #[tokio::test]
    async fn registered_unstarted_turn_never_finishes() {
        let manager = super::permit_tests::manager();
        let (_, place) =
            crate::server::chat::helpers::take_turn_place(manager.session_turn("reserved"));
        let (_events, mut frames, task) = watcher(
            Arc::new(std::sync::atomic::AtomicBool::new(true)),
            Default::default(),
            Some(manager),
        );
        tokio::time::sleep(std::time::Duration::from_millis(120)).await;
        assert!(frames.try_recv().is_err());
        drop(place);
        task.abort();
    }

    fn row(from: &str, content: &str, observation: bool) -> crate::state_fs::sessions::ChatMsg {
        crate::state_fs::sessions::ChatMsg {
            agent_id: "ling".into(),
            from_id: from.into(),
            to_id: "ling".into(),
            content: content.into(),
            timestamp: 0,
            is_observation: observation,
            client_id: None,
            failed: false,
            via: None,
            via_note: None,
            images: vec![],
            run_id: None,
        }
    }

    #[test]
    fn final_reply_skips_system_notice() {
        let messages = vec![
            row("ling", "agent answer", false),
            row("system", "Background task sh-1 ended: failed", false),
        ];
        assert_eq!(
            turn_replies(messages).reply.as_deref(),
            Some("agent answer")
        );
    }

    #[test]
    fn the_reply_answers_the_callers_prompt_not_a_later_notice_turn() {
        let messages = vec![
            row("user", "first task", false),
            row("ling", "old answer", false),
            row("claude-code", "fix the bug", false),
            row("ling", "looking", false),
            row("system", "Tool Bash: ok", true),
            row("ling", "Fixed in abc123; tests pass.", false),
            row("system", "Background task sh-1 ended: completed", false),
            row("ling", "The watcher ended.", false),
            row("system", "Background task sh-2 ended: completed", false),
            row("ling", "", false),
        ];
        assert_eq!(
            turn_replies(messages),
            TurnReplies {
                reply: Some("Fixed in abc123; tests pass.".into()),
                followup: Some("The watcher ended.".into()),
            }
        );
        // A new prompt starts over: nothing from before answers it.
        let messages = vec![
            row("ling", "old answer", false),
            row("system", "notice", false),
            row("ling", "notice answer", false),
            row("claude-code", "next", false),
        ];
        assert_eq!(turn_replies(messages), TurnReplies::default());
    }
}

pub async fn stop(state: &Arc<ServerState>, session: &str) -> Result<String, String> {
    session_meta(state, session)?;
    let runs = state
        .manager
        .list_agent_runs(&PathBuf::new(), Some(session))
        .await
        .map_err(|e| e.to_string())?;
    let mut stopped = Vec::new();
    for run in runs.iter().filter(|r| {
        r.status == crate::engine::agent::AgentRunStatus::Running && r.parent_run_id.is_none()
    }) {
        let cancelled = state
            .manager
            .cancel_run_tree(&run.run_id)
            .await
            .map_err(|e| e.to_string())?;
        // A run waiting on a permission request stops now, not when it expires.
        super::api::agents::drop_pending_asks(state, &cancelled).await;
        stopped.extend(cancelled.into_iter().map(|r| r.run_id));
    }
    Ok(serde_json::json!({"stopped": stopped}).to_string())
}

fn permission_mapping(mode: Option<&str>, has_dir: bool) -> Result<Option<PermissionMode>, String> {
    let grant = match mode {
        None | Some("auto") | Some("bypassPermissions") => PermissionMode::Admin,
        Some("default") | Some("acceptEdits") => PermissionMode::Edit,
        Some("plan") => return Ok(None),
        Some(other) => return Err(format!("unknown permission_mode: {other}")),
    };
    Ok(has_dir.then_some(grant))
}

fn format_models(models: &crate::provider::models::ModelManager, default: Option<&str>) -> String {
    let default = default.and_then(|id| models.resolve_id(id));
    serde_json::json!({"models": models.list_models().iter().map(|m| serde_json::json!({
        "id": m.id, "provider": m.provider, "default": Some(m.id.as_str()) == default.as_deref()
    })).collect::<Vec<_>>()})
    .to_string()
}

pub async fn models(state: &Arc<ServerState>) -> String {
    let config = state.manager.get_config_snapshot().await;
    let models = state.manager.models.read().await;
    format_models(
        &models,
        config.routing.default_models.first().map(String::as_str),
    )
}

fn worktree_decision(linked: bool, clean: bool, merged: bool) -> Result<(), String> {
    if !linked {
        return Err("worktree check failed: not a linked worktree root".into());
    }
    if !clean {
        return Err("clean check failed: worktree has changes".into());
    }
    if !merged {
        return Err("merged check failed: HEAD is not contained in a remote branch".into());
    }
    Ok(())
}

fn git(cwd: &std::path::Path, args: &[&str]) -> Result<String, String> {
    let output = std::process::Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .map_err(|e| format!("git failed: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub async fn close(
    state: &Arc<ServerState>,
    session: &str,
    remove: bool,
) -> Result<String, String> {
    let meta = session_meta(state, session)?;
    let stopped = stop(state, session).await?;
    if !remove {
        return Ok(stopped);
    }
    let work = super::api::sessions::session_running_work(state, session).await;
    if work.turn || work.tasks > 0 || work.subagents > 0 {
        return Err("running check failed: work is still stopping; retry".into());
    }
    let cwd = std::path::Path::new(&meta.cwd);
    let root = git(cwd, &["rev-parse", "--show-toplevel"])?;
    let root_path = std::fs::canonicalize(&root).map_err(|e| e.to_string())?;
    let cwd_path = std::fs::canonicalize(cwd).map_err(|e| e.to_string())?;
    let dotgit = root_path.join(".git");
    let linked = root_path == cwd_path && dotgit.is_file();
    let clean = git(cwd, &["status", "--porcelain", "--untracked-files=all"])?.is_empty();
    let merged = !git(cwd, &["branch", "-r", "--contains", "HEAD"])?.is_empty();
    worktree_decision(linked, clean, merged)?;
    // git refuses to remove the worktree it runs in: run from the main checkout.
    let common = git(
        cwd,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?;
    let main = std::path::Path::new(&common)
        .parent()
        .ok_or("no main checkout")?;
    git(main, &["worktree", "remove", "--", &root])?;
    Ok(serde_json::json!({"stopped": stopped, "removed_worktree": root}).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn mcp_driven_turn_status_clears_on_success_and_failure() {
        use std::cell::Cell;
        for fail in [false, true] {
            let busy = Cell::new(false);
            let result: Result<(), &str> = with_turn_status(
                async { busy.set(true) },
                async {
                    assert!(busy.get());
                    if fail {
                        Err("failed")
                    } else {
                        Ok(())
                    }
                },
                || async { busy.set(false) },
            )
            .await;
            assert_eq!(result.is_err(), fail);
            assert!(!busy.get());
        }
    }
    #[test]
    fn mcp_agent_permission_mapping() {
        for (mode, expected) in [
            ("plan", None),
            ("default", Some(PermissionMode::Edit)),
            ("acceptEdits", Some(PermissionMode::Edit)),
            ("auto", Some(PermissionMode::Admin)),
            ("bypassPermissions", Some(PermissionMode::Admin)),
        ] {
            assert_eq!(permission_mapping(Some(mode), true).unwrap(), expected);
            assert_eq!(permission_mapping(Some(mode), false).unwrap(), None);
        }
        assert!(permission_mapping(Some("bad"), true).is_err());
        assert_eq!(
            permission_mapping(None, true).unwrap(),
            Some(PermissionMode::Admin)
        );
    }
    #[test]
    fn mcp_agent_worktree_safety() {
        assert!(worktree_decision(true, true, true).is_ok());
        assert!(worktree_decision(false, true, true)
            .unwrap_err()
            .contains("worktree"));
        assert!(worktree_decision(true, false, true)
            .unwrap_err()
            .contains("clean"));
        assert!(worktree_decision(true, true, false)
            .unwrap_err()
            .contains("merged"));
    }
    #[test]
    fn mcp_agent_models_format() {
        let manager = crate::provider::models::ModelManager::new_with_credentials(
            vec![],
            &crate::credentials::Credentials::default(),
        );
        let builtin = manager.list_models().first().unwrap().id.clone();
        let result: serde_json::Value =
            serde_json::from_str(&format_models(&manager, Some(&builtin))).unwrap();
        let rows = result["models"].as_array().unwrap();
        assert_eq!(rows.len(), manager.list_models().len());
        let selected = rows.iter().find(|m| m["id"] == builtin).unwrap();
        assert_eq!(selected["default"], true);
        assert_eq!(
            selected["provider"],
            manager.provider_kind(&builtin).unwrap()
        );
        assert_eq!(rows.iter().filter(|m| m["default"] == true).count(), 1);
        assert!(rows.iter().any(|m| m["id"] == "deepseek-flash"));
    }
    #[test]
    fn mcp_agent_first_prompt_enters_cached_thread() {
        let mut history = vec![crate::message::ChatMessage::new("assistant", "first reply")];
        seed_first_prompt(&mut history, "first prompt");
        assert_eq!(
            (history[0].role.as_str(), history[0].content.as_str()),
            ("user", "first prompt")
        );
        assert_eq!(history[1].content, "first reply");
    }
    #[test]
    fn mcp_agent_rejects_incompatible_options() {
        assert!(validate_options(Some("s"), Some("/tmp"), None).is_err());
        assert!(validate_options(Some("s"), None, Some("m")).is_err());
    }
    #[test]
    fn mcp_agent_rejects_unknown_session() {
        assert!(require_session::<()>("missing", None)
            .unwrap_err()
            .contains("unknown session"));
    }
    #[test]
    fn mcp_agent_rejects_unknown_model() {
        assert!(validate_model("bad", None, &["good".into()])
            .unwrap_err()
            .contains("good"));
    }
}

#[cfg(test)]
mod permit_tests {
    use super::*;
    use crate::engine::permission::{
        build_caller_question, build_exceeds_ceiling_question, check_permission,
        parse_exceeds_ceiling_answer, PermissionAction, PermissionCheckResult, SessionPermissions,
    };
    use crate::engine::tools::{AskUserBridge, CallerPermissionRequest, PendingAskUser};
    use std::collections::HashMap;
    use std::time::Duration;
    use tokio::sync::Mutex;

    type Pending = Arc<Mutex<HashMap<String, PendingAskUser>>>;

    fn request(dir: Option<&str>) -> CallerPermissionRequest {
        CallerPermissionRequest {
            tool: "Bash".into(),
            summary: "touch ~/out/file".into(),
            level: PermissionMode::Edit,
            path: "~/out/file".into(),
            dir: dir.map(str::to_string),
            reason: Some("make the marker".into()),
        }
    }

    fn question(dir: Option<&str>) -> crate::engine::tools::AskUserQuestion {
        let dirs: Vec<String> = dir.into_iter().map(str::to_string).collect();
        build_caller_question(
            "touch ~/out/file",
            &PermissionMode::Edit,
            "~/out/file",
            &dirs,
        )
    }

    fn bridge(pending: &Pending, session: &str) -> AskUserBridge {
        AskUserBridge {
            events_tx: tokio::sync::broadcast::channel(16).0,
            pending: pending.clone(),
            session_id: Some(session.into()),
        }
    }

    /// Raise a caller request in `session`, answer it with `decision`, and
    /// return what the waiting gate would act on.
    async fn ask_and_answer(
        decision: &str,
        scope: Option<&str>,
        dir: Option<&str>,
    ) -> (Result<String, String>, Option<PermissionAction>) {
        let pending: Pending = Default::default();
        let question = question(dir);
        let offered: Vec<String> = question.options.iter().map(|o| o.label.clone()).collect();
        let b = bridge(&pending, "s1");
        let req = request(dir);
        let ask = tokio::spawn(async move {
            crate::engine::tool_exec::ask_on_bridge(
                &b,
                "ling".into(),
                question,
                Some(req),
                Duration::from_secs(5),
            )
            .await
        });
        let id = loop {
            if let Some(id) = pending.lock().await.keys().next().cloned() {
                break id;
            }
            tokio::task::yield_now().await;
        };
        let answered = answer_request(&pending, "s1", &id, decision, scope).await;
        if answered.is_err() {
            ask.abort();
            return (answered, None);
        }
        let answers = ask.await.unwrap().flatten().expect("answered");
        let action = parse_exceeds_ceiling_answer(&answers, &PermissionMode::Edit, &offered);
        (answered, Some(action))
    }

    #[tokio::test]
    async fn permit_allow_once_runs_that_action() {
        let (_, action) = ask_and_answer("allow_once", None, Some("~/out")).await;
        assert_eq!(action, Some(PermissionAction::AllowOnce));
    }

    #[tokio::test]
    async fn permit_allow_session_grants_folder_or_path_only() {
        let (_, dir) = ask_and_answer("allow_session", Some("dir"), Some("~/out")).await;
        assert_eq!(
            dir,
            Some(PermissionAction::AllowDirectory {
                all: false,
                mode: PermissionMode::Edit,
                path: "~/out".into()
            })
        );
        let (_, default) = ask_and_answer("allow_session", None, Some("~/out")).await;
        assert_eq!(default, dir);
        let (_, path) = ask_and_answer("allow_session", Some("path"), Some("~/out")).await;
        assert_eq!(
            path,
            Some(PermissionAction::AllowDirectory {
                all: false,
                mode: PermissionMode::Edit,
                path: "~/out/file".into()
            })
        );
        // An ungrantable folder (home itself) offers the path, not the folder.
        let (err, _) = ask_and_answer("allow_session", Some("dir"), None).await;
        assert!(err.unwrap_err().contains("scope"));
        let (_, fallback) = ask_and_answer("allow_session", None, None).await;
        assert_eq!(fallback, path);
    }

    #[tokio::test]
    async fn permit_deny_refuses() {
        let (_, action) = ask_and_answer("deny", None, Some("~/out")).await;
        assert_eq!(action, Some(PermissionAction::Deny));
        let (err, _) = ask_and_answer("maybe", None, Some("~/out")).await;
        assert!(err.unwrap_err().contains("unknown decision"));
    }

    #[tokio::test]
    async fn unanswered_request_times_out_and_is_gone() {
        let pending: Pending = Default::default();
        let b = bridge(&pending, "s1");
        let answer = crate::engine::tool_exec::ask_on_bridge(
            &b,
            "ling".into(),
            question(None),
            Some(request(None)),
            Duration::from_millis(30),
        )
        .await;
        assert!(answer.is_none(), "a timeout is no answer");
        assert!(pending.lock().await.is_empty());
    }

    #[tokio::test]
    async fn permit_answers_only_its_own_sessions_caller_requests() {
        let pending: Pending = Default::default();
        let (tx, _rx) = tokio::sync::oneshot::channel();
        let (tx2, _rx2) = tokio::sync::oneshot::channel();
        {
            let mut map = pending.lock().await;
            map.insert(
                "other".into(),
                PendingAskUser {
                    agent_id: "ling".into(),
                    questions: vec![question(None)],
                    sender: tx,
                    session_id: Some("s2".into()),
                    caller_request: Some(request(None)),
                },
            );
            // A person's prompt in the same session: not the caller's to answer.
            let card =
                build_exceeds_ceiling_question("x", &PermissionMode::Edit, "~/out/file", &[]);
            map.insert(
                "person".into(),
                PendingAskUser {
                    agent_id: "ling".into(),
                    questions: vec![card],
                    sender: tx2,
                    session_id: Some("s1".into()),
                    caller_request: None,
                },
            );
        }
        assert!(answer_request(&pending, "s1", "other", "allow_once", None)
            .await
            .is_err());
        assert!(answer_request(&pending, "s1", "person", "allow_once", None)
            .await
            .is_err());
        assert!(answer_request(&pending, "s1", "missing", "deny", None)
            .await
            .is_err());
        assert_eq!(
            pending.lock().await.len(),
            2,
            "refusals leave requests pending"
        );
        assert!(caller_requests(&*pending.lock().await, "s1").is_empty());
        let rows = caller_requests(&*pending.lock().await, "s2");
        assert_eq!(
            (rows[0]["id"].as_str(), rows[0]["level"].as_str()),
            (Some("other"), Some("edit"))
        );
    }

    #[test]
    fn deny_floor_is_never_a_request() {
        let mut perms = SessionPermissions {
            interactive: false,
            ask_caller: true,
            ..Default::default()
        };
        perms.set_path_mode("/tmp/wd", PermissionMode::Admin);
        for cmd in ["sudo -n true", "rm -rf /", "ls && sudo rm /etc/foo"] {
            let result = check_permission(
                "Bash",
                Some(cmd),
                None,
                std::path::Path::new("/tmp/wd"),
                &perms,
                None,
            );
            assert!(
                matches!(result, PermissionCheckResult::Blocked(_)),
                "{cmd}: {result:?}"
            );
        }
    }

    #[test]
    fn caller_question_offers_no_session_wide_grant() {
        let q = build_caller_question("touch", &PermissionMode::Edit, "~/a/f", &["~/a".into()]);
        let labels: Vec<&str> = q.options.iter().map(|o| o.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "grant:at:edit:~/a",
                "grant:at:edit:~/a/f",
                "Allow once",
                "Deny"
            ]
        );
        // The person's card offers no "Allow once": that label denies there.
        let card = build_exceeds_ceiling_question(
            "touch",
            &PermissionMode::Edit,
            "~/a/f",
            &["~/a".into()],
        );
        let offered: Vec<String> = card.options.iter().map(|o| o.label.clone()).collect();
        let once = crate::engine::tools::AskUserAnswer {
            question_index: 0,
            selected: vec!["Allow once".into()],
            custom_text: None,
        };
        assert_eq!(
            parse_exceeds_ceiling_answer(&[once], &PermissionMode::Edit, &offered),
            PermissionAction::Deny
        );
    }

    #[test]
    fn ask_caller_persists_only_when_set() {
        let dir = tempfile::tempdir().unwrap();
        SessionPermissions::default().save(dir.path());
        assert!(!std::fs::read_to_string(dir.path().join("permission.json"))
            .unwrap()
            .contains("ask_caller"));
        SessionPermissions {
            interactive: false,
            ask_caller: true,
            ..Default::default()
        }
        .save(dir.path());
        assert!(SessionPermissions::load(dir.path()).ask_caller);
    }

    #[test]
    fn stream_tells_each_request_once_then_finished() {
        let mut told = std::collections::HashMap::new();
        let now = tokio::time::Instant::now();
        let interval = Duration::from_millis(20);
        let rows = vec![serde_json::json!({"id": "r1", "tool": "Bash"})];
        let first = new_requests(&rows, &mut told, "s1", now, interval);
        assert_eq!(first[0]["type"], "permission_request");
        assert_eq!(first[0]["request"]["id"], "r1");
        assert!(new_requests(&rows, &mut told, "s1", now, interval).is_empty());
        assert_eq!(
            new_requests(&rows, &mut told, "s1", now + interval, interval)[0]["request"]["id"],
            "r1"
        );
        let done = finished_frame(
            "s1",
            LastRun {
                status: "completed",
                error: None,
                model: None,
            },
            Some("ok".into()),
            vec![],
        );
        assert_eq!(
            (
                done["type"].as_str(),
                done["status"].as_str(),
                done["reply"].as_str()
            ),
            (Some("finished"), Some("completed"), Some("ok"))
        );
        assert!(done["denied"].is_array() && done["session_id"] == "s1");
    }

    #[test]
    fn still_running_names_the_stream_events() {
        let text = still_running_text(9527, "s1");
        assert!(text.contains("ws://127.0.0.1:9527/api/agent/wait?session_id=s1"));
        assert!(
            text.contains("permission_request")
                && text.contains("finished")
                && text.contains("agent_permit")
        );
    }

    pub(super) fn manager() -> Arc<crate::engine::agent::AgentManager> {
        crate::engine::agent::AgentManager::new(
            crate::config::Config::default(),
            None,
            Arc::new(crate::engine::test_registries::Empty),
            Arc::new(crate::engine::test_registries::Empty),
            Arc::new(crate::engine::test_registries::Empty),
            crate::engine::InterfaceMode::Web,
        )
        .0
    }

    fn record(
        id: &str,
        status: crate::engine::agent::AgentRunStatus,
    ) -> crate::engine::agent::AgentRunRecord {
        crate::engine::agent::AgentRunRecord {
            run_id: id.into(),
            repo_path: String::new(),
            session_id: "s1".into(),
            agent_id: "ling".into(),
            agent_kind: None,
            parent_run_id: None,
            status,
            detail: None,
            started_at: 1,
            ended_at: None,
            model: None,
        }
    }

    #[test]
    fn a_delegated_session_does_not_switch_models_unless_asked() {
        assert!(!new_session_fallback(None));
        assert!(!new_session_fallback(Some(false)));
        assert!(new_session_fallback(Some(true)));
        // The meta `run` stores for the new session carries it.
        let meta = |asked| delegate_meta("s", "t", "ling", None, "claude-code", false, asked);
        assert_eq!(meta(None).model_fallback, Some(false));
        assert_eq!(meta(Some(true)).model_fallback, Some(true));
        assert!(!meta(None).allows_model_fallback());
        assert!(meta(Some(true)).allows_model_fallback());
    }

    #[test]
    fn the_tool_offers_an_optional_model_fallback() {
        let schema = crate::server::mcp::tool_schema("agent_run").unwrap();
        assert_eq!(schema["properties"]["model_fallback"]["type"], "boolean");
        assert!(schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .all(|k| k != "model_fallback"));
    }

    #[test]
    fn a_failure_names_the_model_when_the_provider_did_not() {
        let auth = "AUTH_REQUIRED: ChatGPT session expired. Sign in with ChatGPT to continue.";
        let said = naming_model(auth.into(), Some("gpt-6.1-sol"));
        assert!(
            said.starts_with("model 'gpt-6.1-sol' failed: AUTH_REQUIRED:"),
            "{said}"
        );
        // Already named (a switching-off failure): left as it is.
        let named = "MODEL_FAILED: model 'gpt-6.1-sol' failed (rate limit): x".to_string();
        assert_eq!(naming_model(named.clone(), Some("gpt-6.1-sol")), named);
        assert_eq!(naming_model(auth.into(), None), auth);
    }

    #[test]
    fn the_last_run_names_its_model_and_why_it_failed() {
        use crate::engine::agent::AgentRunStatus::{Completed, Failed, Running};
        let manager = manager();
        // Nothing on record: an idle session reads as completed.
        let idle = last_run(&manager, "s1");
        assert_eq!(
            (idle.status, idle.error, idle.model),
            ("completed", None, None)
        );
        // While it runs: running, and the model answering so far.
        manager.run_store.add_run(&record("r1", Running));
        manager.run_store.set_model("r1", "gpt-6.1-sol");
        let running = last_run(&manager, "s1");
        assert_eq!(
            (running.status, running.model.as_deref()),
            ("running", Some("gpt-6.1-sol"))
        );
        // Failed: the run leaves the store; the session keeps how it ended.
        let mut done = manager.run_store.get_run("r1").unwrap();
        done.status = Failed;
        done.detail = Some("MODEL_FAILED: model 'gpt-6.1-sol' failed (rate limit): 429".into());
        manager.run_store.record_outcome(&done);
        manager.run_store.remove_run("r1");
        let failed = last_run(&manager, "s1");
        assert_eq!(failed.status, "failed");
        assert!(failed.error.as_deref().unwrap().contains("gpt-6.1-sol"));
        assert_eq!(failed.model.as_deref(), Some("gpt-6.1-sol"));
        assert_eq!(failed_error(&manager, "s1"), failed.error);
        // A later completed run replaces it: no stale error.
        let mut ok = record("r2", Completed);
        ok.model = Some("deepseek-flash".into());
        manager.run_store.record_outcome(&ok);
        let after = last_run(&manager, "s1");
        assert_eq!(
            (after.status, after.error, after.model.as_deref()),
            ("completed", None, Some("deepseek-flash"))
        );
        assert_eq!(failed_error(&manager, "s1"), None);
    }

    #[test]
    fn the_finished_frame_carries_error_and_model() {
        let frame = finished_frame(
            "s1",
            LastRun {
                status: "failed",
                error: Some("MODEL_FAILED: model 'm' failed".into()),
                model: Some("m".into()),
            },
            None,
            vec![],
        );
        assert_eq!(frame["status"], "failed");
        assert_eq!(frame["error"], "MODEL_FAILED: model 'm' failed");
        assert_eq!(frame["model"], "m");
    }

    #[tokio::test]
    async fn a_new_run_forgets_how_the_last_one_ended() {
        use crate::engine::agent::AgentRunStatus::Failed;
        let manager = manager();
        let mut old = record("r0", Failed);
        old.detail = Some("the last turn's model failed".into());
        manager.run_store.record_outcome(&old);
        assert!(failed_error(&manager, "s1").is_some());
        let dir = tempfile::tempdir().unwrap();
        manager
            .begin_agent_run(dir.path(), Some("s1"), "ling", None, None)
            .await
            .unwrap();
        // Running now, and nothing of the turn before is read as its result.
        assert_eq!(failed_error(&manager, "s1"), None);
        assert_eq!(last_run(&manager, "s1").status, "running");
    }

    #[test]
    fn a_subagents_run_is_not_the_sessions_outcome() {
        let manager = manager();
        let mut child = record("c1", crate::engine::agent::AgentRunStatus::Failed);
        child.parent_run_id = Some("r1".into());
        manager.run_store.record_outcome(&child);
        assert!(manager.run_store.outcome("s1").is_none());
    }

    #[test]
    fn delegate_toolset_widens_only_for_write() {
        let read = delegate_tool_scope(false).unwrap();
        let write = delegate_tool_scope(true).unwrap();
        assert!(!read.contains("Bash") && write.contains("Bash") && write.contains("Edit"));
        assert!(!write.contains("Task") && !write.contains("AskUser"));
    }
}
