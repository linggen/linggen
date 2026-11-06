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
        return Err(format!("write_dir must be an existing absolute folder: {dir}"));
    }
    let home = dirs::home_dir().unwrap_or_default();
    if path.parent().is_none() || path == home {
        return Err(format!("write_dir is too wide: {dir} — pass the task's own checkout"));
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

/// Read the final assistant reply from a finished session — the last
/// non-observation message not authored by "user".
fn final_reply(session_id: &str) -> Option<String> {
    let store =
        crate::state_fs::SessionStore::with_sessions_dir(crate::paths::global_sessions_dir());
    let msgs = store.get_chat_history(session_id).ok()?;
    msgs.into_iter()
        .rev()
        .find(|m| !m.is_observation && m.from_id != "user" && m.from_id != "claude-code" && !m.content.trim().is_empty())
        .map(|m| m.content)
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
) -> Result<String, String> {
    let prompt = prompt.trim();
    if prompt.is_empty() {
        return Err("agent_run requires a non-empty prompt".to_string());
    }

    validate_options(session, write_dir, model)?;
    if session.is_some() && permission_mode.is_some() { return Err("permission_mode cannot be used with session_id".into()); }
    let mode = permission_mapping(permission_mode, write_dir.is_some())?;
    if let Some(session) = session {
        let meta = session_meta(state, session)?;
        if !meta.runtime.is_linggen() {
            return Err("session is not a Linggen session".into());
        }
        let agent_id = agent.map(str::to_string)
            .or_else(|| meta.agents.first().map(|m| m.id.clone()))
            .unwrap_or_else(|| "ling".into());
        let turn = super::tasks::tell_from(state.clone(), session.into(), agent_id,
            prompt.into(), "claude-code".into());
        let task = tokio::spawn(turn);
        return match tokio::time::timeout(std::time::Duration::from_secs(WAIT_SECS), task).await {
            Ok(Ok(())) => {
                Ok(final_reply(session).unwrap_or_else(|| "(the agent finished without a textual reply)".into()))
            }
            Ok(Err(e)) => Err(format!("agent run failed: {e}")),
            Err(_) => Ok(still_running(session)),
        };
    }
    let model = if let Some(model) = model {
        let models = state.manager.models.read().await;
        let resolved = models.resolve_id(model);
        drop(models);
        let config = state.manager.get_config_snapshot().await;
        Some(validate_model(model, resolved, &config.models.iter().map(|m| m.id.clone()).collect::<Vec<_>>())?)
    } else { None };

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

    // Fresh, visible session (creator "agent" → shows in the session list with
    // an agent badge). Titled from the prompt.
    let session_id = format!(
        "sess-{}-{}",
        crate::util::now_ts_secs(),
        &uuid::Uuid::new_v4().to_string()[..8]
    );
    let title = title_from_prompt(prompt);
    // A write run lives in its folder; a read run in the workspace.
    let root_str = write.as_ref().unwrap_or(&root).to_string_lossy().to_string();
    let store =
        crate::state_fs::SessionStore::with_sessions_dir(crate::paths::global_sessions_dir());
    let mut meta = crate::state_fs::sessions::SessionMeta {
        runtime: Default::default(),
        claude_permission_mode: Default::default(),
        parent_session_id: None,
        id: session_id.clone(),
        title: title.clone(),
        created_at: crate::util::now_ts_secs(),
        updated_at: 0,
        skill: None,
        creator: "external".into(),
        cwd: String::new(),
        project: None,
        project_name: None,
        mission_id: None,
        agents: vec![crate::state_fs::sessions::SessionMember::new(&agent_id)],
        legacy: Default::default(),
        user_id: None,
        compact_threshold: None,
        compact_focus: None,
        title_locked: true,
        withheld_tools: Vec::new(),
        params: Default::default(),
    };
    meta.params.insert("host".into(), host.unwrap_or("claude-code").into());
    meta.set_cwd(&root_str);
    meta.set_member_model(&agent_id, model.clone());
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
            ..Default::default()
        };
        if permission_mode.is_none() && write.is_none() {
            perms.set_path_mode(&root_str, PermissionMode::Read);
        } else {
            perms.set_path_mode("*", PermissionMode::Read);
            if let Some(grant) = mode { perms.set_path_mode(&root_str, grant); }
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
    let run = tokio::spawn(drive(
        state.clone(),
        session_id.clone(),
        root,
        agent_id,
        prompt.to_string(),
        mode.is_some(),
        model,
    ));
    match tokio::time::timeout(std::time::Duration::from_secs(WAIT_SECS), run).await {
        Ok(Ok(result)) => result,
        Ok(Err(e)) => Err(format!("agent run failed: {e}")),
        Err(_) => Ok(still_running(&session_id)),
    }
}

/// How long a call waits for the run before handing back its session.
const WAIT_SECS: u64 = 45;

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
    if let Some(model) = model { engine.model_id = model; }

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
    let extra: &[&str] = if write { WRITE_TOOLS } else { &[] };
    let declared: Vec<String> = AGENT_RUN_TOOLS
        .iter()
        .chain(extra)
        .map(|s| s.to_string())
        .collect();
    engine.cfg.mission_allowed_tools = crate::engine::tool_scope::compute_tool_scope(&declared);

    // Permissions come from the permission.json written above — initialize_loop
    // reloads them at the top of the run.
    let result = engine.run_agent_loop(Some(&session_id)).await;
    engine.set_run_id(None);
    drop(engine);

    match result {
        Ok(_) => {
            let _ = state
                .manager
                .finish_agent_run(
                    &run_id,
                    crate::engine::agent::AgentRunStatus::Completed,
                    None,
                )
                .await;
            let _ = state
                .events_tx
                .send(crate::server::ServerEvent::StateUpdated);
            Ok(final_reply(&session_id)
                .unwrap_or_else(|| "(the agent finished without a textual reply)".to_string()))
        }
        Err(e) => {
            let _ = state
                .manager
                .finish_agent_run(
                    &run_id,
                    crate::engine::agent::AgentRunStatus::Failed,
                    Some(e.to_string()),
                )
                .await;
            Err(format!("agent run failed: {e}"))
        }
    }
}

fn still_running(session: &str) -> String {
    format!("Still running in Linggen session {session} (it keeps going). Poll agent_status for the reply.")
}

fn validate_options(session: Option<&str>, write: Option<&str>, model: Option<&str>) -> Result<(), String> {
    if session.is_some() && write.is_some() { return Err("write_dir cannot be used with session_id".into()); }
    if session.is_some() && model.is_some() { return Err("model cannot be used with session_id".into()); }
    Ok(())
}

fn validate_model(model: &str, resolved: Option<String>, ids: &[String]) -> Result<String, String> {
    resolved.ok_or_else(|| format!("unknown model '{model}'. Available: {}", ids.join(", ")))
}

fn session_meta(state: &ServerState, session: &str) -> Result<crate::state_fs::sessions::SessionMeta, String> {
    let meta = state.manager.global_sessions.get_session_meta(session)
        .map_err(|e| format!("cannot read session: {e}"))?;
    require_session(session, meta)
}

fn require_session<T>(session: &str, meta: Option<T>) -> Result<T, String> {
    meta.ok_or_else(|| format!("unknown session '{session}'"))
}

pub async fn status(state: &Arc<ServerState>, session: &str) -> Result<String, String> {
    session_meta(state, session)?;
    let work = super::api::sessions::session_running_work(state, session).await;
    Ok(serde_json::json!({"running": work.turn, "tasks": work.tasks,
        "subagents": work.subagents, "reply": if work.turn { None } else { final_reply(session) }}).to_string())
}

pub async fn stop(state: &Arc<ServerState>, session: &str) -> Result<String, String> {
    session_meta(state, session)?;
    let runs = state.manager.list_agent_runs(&PathBuf::new(), Some(session)).await.map_err(|e| e.to_string())?;
    let mut stopped = Vec::new();
    for run in runs.iter().filter(|r| r.status == crate::engine::agent::AgentRunStatus::Running && r.parent_run_id.is_none()) {
        let cancelled = state.manager.cancel_run_tree(&run.run_id).await.map_err(|e| e.to_string())?;
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

fn format_models(models: &[crate::config::ModelConfig], default: Option<&str>) -> String {
    serde_json::json!({"models": models.iter().map(|m| serde_json::json!({
        "id": m.id, "provider": m.provider, "default": Some(m.id.as_str()) == default
    })).collect::<Vec<_>>()}).to_string()
}

pub async fn models(state: &Arc<ServerState>) -> String {
    let config = state.manager.get_config_snapshot().await;
    format_models(&config.models, config.routing.default_models.first().map(String::as_str))
}

fn worktree_decision(linked: bool, clean: bool, merged: bool) -> Result<(), String> {
    if !linked { return Err("worktree check failed: not a linked worktree root".into()); }
    if !clean { return Err("clean check failed: worktree has changes".into()); }
    if !merged { return Err("merged check failed: HEAD is not contained in a remote branch".into()); }
    Ok(())
}

fn git(cwd: &std::path::Path, args: &[&str]) -> Result<String, String> {
    let output = std::process::Command::new("git").current_dir(cwd).args(args).output()
        .map_err(|e| format!("git failed: {e}"))?;
    if !output.status.success() { return Err(format!("git {} failed: {}", args.join(" "), String::from_utf8_lossy(&output.stderr))); }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub async fn close(state: &Arc<ServerState>, session: &str, remove: bool) -> Result<String, String> {
    let meta = session_meta(state, session)?;
    let stopped = stop(state, session).await?;
    if !remove { return Ok(stopped); }
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
    let common = git(cwd, &["rev-parse", "--path-format=absolute", "--git-common-dir"])?;
    let main = std::path::Path::new(&common).parent().ok_or("no main checkout")?;
    git(main, &["worktree", "remove", "--", &root])?;
    Ok(serde_json::json!({"stopped": stopped, "removed_worktree": root}).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mcp_agent_permission_mapping() {
        for (mode, expected) in [("plan", None), ("default", Some(PermissionMode::Edit)), ("acceptEdits", Some(PermissionMode::Edit)), ("auto", Some(PermissionMode::Admin)), ("bypassPermissions", Some(PermissionMode::Admin))] {
            assert_eq!(permission_mapping(Some(mode), true).unwrap(), expected);
            assert_eq!(permission_mapping(Some(mode), false).unwrap(), None);
        }
        assert!(permission_mapping(Some("bad"), true).is_err());
        assert_eq!(permission_mapping(None, true).unwrap(), Some(PermissionMode::Admin));
    }
    #[test]
    fn mcp_agent_worktree_safety() {
        assert!(worktree_decision(true, true, true).is_ok());
        assert!(worktree_decision(false, true, true).unwrap_err().contains("worktree"));
        assert!(worktree_decision(true, false, true).unwrap_err().contains("clean"));
        assert!(worktree_decision(true, true, false).unwrap_err().contains("merged"));
    }
    #[test]
    fn mcp_agent_models_format() {
        let model: crate::config::ModelConfig = serde_json::from_value(serde_json::json!({"id":"a", "provider":"openai", "url":"", "model":"a"})).unwrap();
        let result: serde_json::Value = serde_json::from_str(&format_models(&[model], Some("a"))).unwrap();
        assert_eq!(result["models"][0]["default"], true);
        assert_eq!(result["models"][0]["provider"], "openai");
    }
    #[test]
    fn mcp_agent_rejects_incompatible_options() {
        assert!(validate_options(Some("s"), Some("/tmp"), None).is_err());
        assert!(validate_options(Some("s"), None, Some("m")).is_err());
    }
    #[test]
    fn mcp_agent_rejects_unknown_session() {
        assert!(require_session::<()>("missing", None).unwrap_err().contains("unknown session"));
    }
    #[test]
    fn mcp_agent_rejects_unknown_model() {
        assert!(validate_model("bad", None, &["good".into()]).unwrap_err().contains("good"));
    }
}
