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
//! silently write files. Widening to write is a future opt-in.

use std::path::PathBuf;
use std::sync::Arc;

use crate::engine::permission::PermissionMode;
use crate::server::ServerState;

/// The delegate's tool allowlist. Read / search / memory / browser only —
/// no Bash, Write, Edit, or Task, so the read-only boundary can't be worked
/// around (e.g. `echo > file` via Bash). Browser mutations still pass the
/// extension's own permission prompt. Widening to write is a future opt-in.
const AGENT_RUN_TOOLS: &[&str] = &[
    "Read", "Grep", "Glob",
    "WebSearch", "WebFetch",
    "Memory_query", "Memory_write",
    "Skill",
    "Browser_navigate", "Browser_readPage", "Browser_screenshot", "Browser_click",
    "Browser_type", "Browser_key", "Browser_scroll", "Browser_wait",
    "Browser_readConsole", "Browser_tabs",
];

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

/// Read the final assistant reply from a finished session — the last
/// non-observation message not authored by "user".
fn final_reply(session_id: &str) -> Option<String> {
    let store = crate::state_fs::SessionStore::with_sessions_dir(
        crate::paths::global_sessions_dir(),
    );
    let msgs = store.get_chat_history(session_id).ok()?;
    msgs.into_iter()
        .rev()
        .find(|m| !m.is_observation && m.from_id != "user" && !m.content.trim().is_empty())
        .map(|m| m.content)
}

/// Run one delegated agent turn. `Ok(text)` is the agent's final reply;
/// `Err(msg)` is a model-readable failure (unknown agent, run error).
pub async fn run(state: &Arc<ServerState>, agent: Option<&str>, prompt: &str) -> Result<String, String> {
    let prompt = prompt.trim();
    if prompt.is_empty() {
        return Err("agent_run requires a non-empty prompt".to_string());
    }

    let root = workspace_root();
    let agent_id = agent.map(|a| a.trim().to_lowercase()).unwrap_or_else(|| "ling".to_string());

    // Validate the agent up front so an unknown name is a clean error, not a
    // half-created session — and list the real options for the caller.
    if !state.manager.agent_exists(&root, &agent_id).await {
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
            if names.is_empty() { "(none)".into() } else { names.join(", ") }
        ));
    }

    // Fresh, visible session (creator "agent" → shows in the session list with
    // an agent badge). Titled from the prompt.
    let session_id = format!(
        "sess-{}-{}",
        crate::util::now_ts_secs(),
        &uuid::Uuid::new_v4().to_string()[..8]
    );
    let title = title_from_prompt(prompt);
    let root_str = root.to_string_lossy().to_string();
    let store = crate::state_fs::SessionStore::with_sessions_dir(
        crate::paths::global_sessions_dir(),
    );
    let meta = crate::state_fs::sessions::SessionMeta {
        id: session_id.clone(),
        title: title.clone(),
        created_at: crate::util::now_ts_secs(),
        skill: None,
        creator: "agent".into(),
        cwd: Some(root_str.clone()),
        project: Some(root_str.clone()),
        project_name: root.file_name().map(|n| n.to_string_lossy().to_string()),
        mission_id: None,
        agent_id: Some(agent_id.clone()),
        model_id: None,
        user_id: None,
        compact_threshold: None,
        compact_focus: None,
        title_locked: true,
    };
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
        let mut perms = crate::engine::permission::SessionPermissions::default();
        perms.interactive = false;
        perms.set_path_mode(&root_str, PermissionMode::Read);
        perms.save(&crate::paths::global_sessions_dir().join(&session_id));
    }
    let _ = state.events_tx.send(crate::server::ServerEvent::SessionCreated {
        session_id: session_id.clone(),
        title,
        creator: "agent".into(),
        project: Some(root_str.clone()),
        project_name: root.file_name().map(|n| n.to_string_lossy().to_string()),
        skill: None,
        mission_id: None,
    });

    // Persist the prompt as the first user turn so the session reads naturally.
    let _ = store.add_chat_message(
        &session_id,
        &crate::state_fs::sessions::ChatMsg {
            agent_id: agent_id.clone(),
            from_id: "user".to_string(),
            to_id: agent_id.clone(),
            content: prompt.to_string(),
            timestamp: crate::util::now_ts_secs(),
            is_observation: false,
        },
    );

    let agent = state
        .manager
        .get_or_create_session_agent(&session_id, &root, &agent_id)
        .await
        .map_err(|e| format!("failed to create agent '{agent_id}': {e}"))?;
    let mut engine = agent.lock().await;

    let run_id = state
        .manager
        .begin_agent_run(&root, Some(&session_id), &agent_id, None, Some("agent_run".into()))
        .await
        .map_err(|e| format!("failed to begin run: {e}"))?;

    // Browser + memory bridges (no AskUser — a headless MCP delegate has no
    // Linggen-side user to answer a prompt; a blocked run would hang).
    engine.tools.set_browser_bridge(state.bridge.clone());

    engine.observations.clear();
    engine.task = Some(prompt.to_string());
    engine.set_parent_agent(None);
    engine.set_run_id(Some(run_id.clone()));

    // Restrict the toolset to read / memory / browser. This is the real
    // boundary — the permission.json Read grant stops Write/Edit, and dropping
    // Bash stops the `echo > file` end-run around it. cfg isn't reloaded by
    // initialize_loop, so this sticks. (It also flips the session
    // non-interactive on load, matching the permission.json.)
    engine.cfg.mission_allowed_tools =
        Some(AGENT_RUN_TOOLS.iter().map(|s| s.to_string()).collect());

    // Permissions come from the permission.json written above — initialize_loop
    // reloads them at the top of the run.
    let result = engine.run_agent_loop(Some(&session_id)).await;
    engine.set_run_id(None);
    drop(engine);

    match result {
        Ok(_) => {
            let _ = state
                .manager
                .finish_agent_run(&run_id, crate::engine::agent::AgentRunStatus::Completed, None)
                .await;
            let _ = state.events_tx.send(crate::server::ServerEvent::StateUpdated);
            Ok(final_reply(&session_id)
                .unwrap_or_else(|| "(the agent finished without a textual reply)".to_string()))
        }
        Err(e) => {
            let _ = state
                .manager
                .finish_agent_run(&run_id, crate::engine::agent::AgentRunStatus::Failed, Some(e.to_string()))
                .await;
            Err(format!("agent run failed: {e}"))
        }
    }
}
