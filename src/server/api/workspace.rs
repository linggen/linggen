use crate::engine::tool_render::sanitize_message_for_ui;
use crate::server::ServerState;
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use ignore::WalkBuilder;
use serde::{Deserialize, Serialize};

/// Expand `~` to the user's home directory.
fn expand_project_root(raw: &str) -> PathBuf {
    if raw == "~" {
        dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
    } else if raw.starts_with("~/") {
        dirs::home_dir().unwrap_or_default().join(&raw[2..])
    } else {
        PathBuf::from(raw)
    }
}
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::util::LockExt;

/// How long the known-roots set is reused before sessions are re-listed —
/// the @-mention picker searches on every keystroke.
const KNOWN_ROOTS_TTL: Duration = Duration::from_secs(5);

static KNOWN_ROOTS: Mutex<Option<(Instant, HashSet<PathBuf>)>> = Mutex::new(None);

/// Resolve a caller-supplied `project_root` to a root the engine already
/// knows — a session's cwd or project, or a project it has opened. The
/// endpoints below read files under the root they are given, so an
/// arbitrary root (`/`, `/etc`) would make them "read any file on disk".
async fn known_root(state: &ServerState, raw: &str) -> Result<PathBuf, StatusCode> {
    let canonical = expand_project_root(raw)
        .canonicalize()
        .map_err(|_| StatusCode::NOT_FOUND)?;
    // A cached miss re-lists once: a session created a moment ago is known.
    for fresh in [false, true] {
        if is_known_root(&canonical, &known_roots(state, fresh).await) {
            return Ok(canonical);
        }
    }
    tracing::warn!(
        "[workspace] refused unknown project_root {}",
        canonical.display()
    );
    Err(StatusCode::FORBIDDEN)
}

fn is_known_root(canonical: &Path, roots: &HashSet<PathBuf>) -> bool {
    canonical.parent().is_some() && roots.contains(canonical)
}

async fn known_roots(state: &ServerState, fresh: bool) -> HashSet<PathBuf> {
    if !fresh {
        if let Some((at, roots)) = KNOWN_ROOTS.lock_ok().as_ref() {
            if at.elapsed() < KNOWN_ROOTS_TTL {
                return roots.clone();
            }
        }
    }
    let mut raw: Vec<String> = state
        .manager
        .projects
        .lock()
        .await
        .keys()
        .cloned()
        .collect();
    for meta in state
        .manager
        .global_sessions
        .list_sessions()
        .unwrap_or_default()
    {
        raw.extend(meta.cwd);
        raw.extend(meta.project);
    }
    let roots: HashSet<PathBuf> = raw
        .iter()
        .filter_map(|r| expand_project_root(r).canonicalize().ok())
        .collect();
    *KNOWN_ROOTS.lock_ok() = Some((Instant::now(), roots.clone()));
    roots
}

#[derive(Deserialize)]
pub(crate) struct FileQuery {
    project_root: String,
    path: Option<String>,
}

pub(crate) async fn list_files(
    State(state): State<Arc<ServerState>>,
    Query(query): Query<FileQuery>,
) -> impl IntoResponse {
    let canonical_root = match known_root(&state, &query.project_root).await {
        Ok(r) => r,
        Err(code) => return code.into_response(),
    };
    let rel_path = query.path.unwrap_or_default();
    if rel_path.contains("..") {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let full_path = canonical_root.join(&rel_path);
    let full_path = full_path.canonicalize().unwrap_or(full_path);
    if !full_path.starts_with(&canonical_root) {
        return StatusCode::BAD_REQUEST.into_response();
    }

    if !full_path.exists() {
        return StatusCode::NOT_FOUND.into_response();
    }

    let mut entries = Vec::new();
    if let Ok(dir) = std::fs::read_dir(full_path) {
        for entry in dir.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            entries.push(serde_json::json!({
                "name": name,
                "isDir": is_dir,
                "path": if rel_path.is_empty() { name } else { format!("{}/{}", rel_path, name) }
            }));
        }
    }
    Json(entries).into_response()
}

#[derive(Deserialize)]
pub(crate) struct FileSearchQuery {
    project_root: String,
    query: Option<String>,
    limit: Option<usize>,
}

pub(crate) async fn search_files(
    State(state): State<Arc<ServerState>>,
    Query(query): Query<FileSearchQuery>,
) -> impl IntoResponse {
    let canonical_root = match known_root(&state, &query.project_root).await {
        Ok(r) => r,
        Err(code) => return code.into_response(),
    };

    let limit = query.limit.unwrap_or(50);
    let search = query.query.as_deref().unwrap_or("").to_lowercase();

    let walker = WalkBuilder::new(&canonical_root)
        .standard_filters(true)
        .hidden(true)
        .build();

    let mut results: Vec<(String, bool)> = Vec::new();
    for entry in walker {
        let entry = match entry {
            Ok(v) => v,
            Err(_) => continue,
        };
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        let abs_path = entry.path();
        let rel = match abs_path.strip_prefix(&canonical_root) {
            Ok(r) => r,
            Err(_) => continue,
        };
        let rel_str = rel.to_string_lossy().to_string();
        if rel_str.is_empty() {
            continue;
        }
        if !search.is_empty() && !rel_str.to_lowercase().contains(&search) {
            continue;
        }
        results.push((rel_str, is_dir));
    }

    // Sort: exact filename matches first, then by path length, then alphabetical
    results.sort_by(|(a_path, _), (b_path, _)| {
        let a_name = a_path.rsplit('/').next().unwrap_or(a_path).to_lowercase();
        let b_name = b_path.rsplit('/').next().unwrap_or(b_path).to_lowercase();
        let a_exact = a_name.contains(&search);
        let b_exact = b_name.contains(&search);
        b_exact
            .cmp(&a_exact)
            .then_with(|| a_path.len().cmp(&b_path.len()))
            .then_with(|| a_path.cmp(b_path))
    });

    results.truncate(limit);

    let entries: Vec<serde_json::Value> = results
        .into_iter()
        .map(|(path, is_dir)| {
            let name = path.rsplit('/').next().unwrap_or(&path).to_string();
            serde_json::json!({
                "name": name,
                "isDir": is_dir,
                "path": path,
            })
        })
        .collect();

    Json(entries).into_response()
}

pub(crate) async fn read_file_api(
    State(state): State<Arc<ServerState>>,
    Query(query): Query<FileQuery>,
) -> impl IntoResponse {
    let rel_path = match query.path {
        Some(p) => p,
        None => return StatusCode::BAD_REQUEST.into_response(),
    };
    if rel_path.contains("..") {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let canonical_root = match known_root(&state, &query.project_root).await {
        Ok(r) => r,
        Err(code) => return code.into_response(),
    };
    let full_path = canonical_root.join(&rel_path);
    let full_path = full_path.canonicalize().unwrap_or(full_path);
    if !full_path.starts_with(&canonical_root) {
        return StatusCode::BAD_REQUEST.into_response();
    }

    match std::fs::read_to_string(full_path) {
        Ok(content) => Json(serde_json::json!({ "content": content })).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

#[derive(Serialize)]
struct WorkspaceStateResponse {
    active_task: Option<(crate::state_fs::StateFile, String)>,
    user_stories: Option<(crate::state_fs::StateFile, String)>,
    tasks: Vec<(crate::state_fs::StateFile, String)>,
    messages: Vec<(crate::state_fs::StateFile, String)>,
}

#[derive(Deserialize)]
pub(crate) struct ProjectQuery {
    project_root: String,
    session_id: Option<String>,
}

pub(crate) async fn get_workspace_state(
    State(state): State<Arc<ServerState>>,
    Query(query): Query<ProjectQuery>,
) -> impl IntoResponse {
    // Project context is optional — session messages are stored globally and
    // should still load even when the project directory no longer exists, or
    // isn't one the engine knows (never open a project for an arbitrary path:
    // that would make it a known root for the file endpoints).
    let known = known_root(&state, &query.project_root).await.ok();
    let project = match known {
        Some(root) => state.manager.get_or_create_project(root).await.ok(),
        None => None,
    };
    let (active_task, user_stories, tasks) = if let Some(ctx) = project {
        (
            ctx.state_fs.read_file("active.md").ok(),
            ctx.state_fs.read_file("user-stories.md").ok(),
            ctx.state_fs.list_tasks().unwrap_or_default(),
        )
    } else {
        (None, None, Vec::new())
    };

    let messages = match query.session_id.as_deref() {
        Some(sid) if !sid.is_empty() => state
            .manager
            .global_sessions
            .get_chat_history(sid)
            .unwrap_or_default(),
        _ => Vec::new(),
    };

    let mapped_messages: Vec<(crate::state_fs::StateFile, String)> = messages
        .into_iter()
        .filter(|m| !m.content.contains("[HIDDEN]"))
        .filter_map(|m| {
            let cleaned = sanitize_message_for_ui(&m.from_id, &m.content)?;
            Some((
                crate::state_fs::StateFile::Message {
                    id: format!("msg-{}", m.timestamp),
                    from: m.from_id,
                    to: m.to_id,
                    ts: m.timestamp,
                    task_id: None,
                },
                cleaned,
            ))
        })
        .collect();

    Json(WorkspaceStateResponse {
        active_task,
        user_stories,
        tasks,
        messages: mapped_messages,
    })
    .into_response()
}

pub(crate) async fn get_agent_tree(
    State(state): State<Arc<ServerState>>,
    Query(query): Query<ProjectQuery>,
) -> impl IntoResponse {
    let root_path = expand_project_root(&query.project_root);
    let repo_name = root_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    let running_agents: HashSet<String> = state
        .manager
        .list_agent_runs(&root_path, None)
        .await
        .map(|runs| {
            runs.into_iter()
                .filter(|run| run.status == crate::engine::agent::AgentRunStatus::Running)
                .map(|run| run.agent_id)
                .collect()
        })
        .unwrap_or_default();

    let activities = state
        .manager
        .list_working_places_for_repo(&query.project_root)
        .await;

    // Build a simple tree structure from in-memory working-place entries.
    let mut tree = serde_json::Map::new();
    for act in activities {
        if !running_agents.contains(&act.agent_id) {
            continue;
        }
        let parts: Vec<&str> = act.file_path.split('/').collect();
        let mut current = &mut tree;
        for (i, part) in parts.iter().enumerate() {
            if i == parts.len() - 1 {
                current.insert(
                    part.to_string(),
                    serde_json::json!({
                        "type": "file",
                        "agent": act.agent_id,
                        "status": "working",
                        "path": act.file_path,
                        "last_modified": act.last_modified,
                    }),
                );
            } else {
                let entry = current
                    .entry(part.to_string())
                    .or_insert(serde_json::json!({
                        "type": "dir",
                        "children": {}
                    }));
                let Some(obj) = entry.as_object_mut() else {
                    break;
                };
                let Some(children) = obj.get_mut("children") else {
                    break;
                };
                let Some(children_obj) = children.as_object_mut() else {
                    break;
                };
                current = children_obj;
            }
        }
    }

    // Wrap in a root node for the repo
    let root_tree = serde_json::json!({
        repo_name: {
            "type": "dir",
            "path": query.project_root,
            "children": tree
        }
    });

    Json(root_tree).into_response()
}

// ── Direct Bash execution (`!command` in UI) ────────────────────────────

#[derive(Deserialize)]
pub(crate) struct BashRequest {
    pub project_root: String,
    pub command: String,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default = "default_bash_timeout")]
    pub timeout_ms: u64,
}

fn default_bash_timeout() -> u64 {
    30_000
}

/// Kill a timed-out command's whole process group (it was spawned leading one).
fn kill_group(pgid: Option<u32>) {
    #[cfg(unix)]
    if let Some(pgid) = pgid {
        // SAFETY: a signal to a group this server created; no memory is touched.
        unsafe {
            libc::kill(-(pgid as i32), libc::SIGKILL);
        }
    }
    #[cfg(not(unix))]
    let _ = pgid;
}

/// POST /api/bash — run a shell command directly (CC `!` shortcut).
///
/// Tracks cwd per session (same sentinel as the agent Bash tool) so that
/// `! cd /path` persists for subsequent `!` commands.
pub(crate) async fn run_bash_api(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<BashRequest>,
) -> impl IntoResponse {
    use crate::engine::tools::search_exec_find_git_root;
    use crate::server::ServerEvent;
    use std::process::Stdio;
    use std::time::Duration;

    use crate::util::CWD_SENTINEL;

    // Resolve cwd: use per-session stored cwd if available, else project_root.
    let base_cwd: PathBuf = if let Some(sid) = &req.session_id {
        let guard = state.user_bash_cwd.lock().await;
        guard
            .get(sid)
            .cloned()
            .unwrap_or_else(|| expand_project_root(&req.project_root))
    } else {
        expand_project_root(&req.project_root)
    };

    // Fall back to home directory when the session's cwd no longer exists
    // (e.g. project directory was deleted). This lets the user `! cd` elsewhere.
    let base_cwd = if !base_cwd.is_dir() {
        dirs::home_dir().unwrap_or_else(|| PathBuf::from("/tmp"))
    } else {
        base_cwd
    };

    let timeout = Duration::from_millis(req.timeout_ms);

    // Wrap command with cwd sentinel (same as agent Bash tool).
    let wrapped_cmd = crate::util::wrap_with_cwd_sentinel(&req.command);

    // Own process group + kill-on-drop: a timeout kills the command and every
    // child it started, instead of leaving them running behind the reply.
    let mut cmd = tokio::process::Command::new("sh");
    cmd.arg("-c")
        .arg(&wrapped_cmd)
        .current_dir(&base_cwd)
        .env("PATH", crate::util::shell_path())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    cmd.process_group(0);
    let child = cmd.spawn();

    let child = match child {
        Ok(c) => c,
        Err(e) => {
            return Json(serde_json::json!({
                "exit_code": -1,
                "stdout": "",
                "stderr": format!("Failed to spawn: {e}"),
            }))
            .into_response();
        }
    };
    let pgid = child.id();

    let (code, mut stdout, stderr) =
        match tokio::time::timeout(timeout, child.wait_with_output()).await {
            Ok(Ok(output)) => {
                let code = output.status.code().unwrap_or(-1);
                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                (code, stdout, stderr)
            }
            Ok(Err(e)) => (-1, String::new(), format!("Command error: {e}")),
            Err(_) => {
                // The dropped future already killed `sh`; take its group too.
                kill_group(pgid);
                (-1, String::new(), "Command timed out".to_string())
            }
        };

    // Strip the cwd sentinel and update per-session cwd. Use substring match
    // (not whole-line) so commands whose last line of output has no trailing
    // newline — e.g. `cat file_without_final_newline` — still strip cleanly;
    // otherwise the sentinel glues onto the user content (`}__LINGGEN_CWD__`)
    // and leaks into the caller.
    let sentinel_with_nl = format!("{}\n", CWD_SENTINEL);
    if let Some(sent_pos) = stdout.rfind(&sentinel_with_nl) {
        let after_sent = sent_pos + sentinel_with_nl.len();
        let pwd_end = stdout[after_sent..]
            .find('\n')
            .map(|i| after_sent + i)
            .unwrap_or(stdout.len());
        let new_cwd = PathBuf::from(stdout[after_sent..pwd_end].to_string());
        if new_cwd.is_absolute() && new_cwd.exists() {
            if let Some(sid) = &req.session_id {
                let old_cwd: Option<PathBuf> = {
                    let guard = state.user_bash_cwd.lock().await;
                    guard.get(sid).cloned()
                };
                {
                    let mut guard = state.user_bash_cwd.lock().await;
                    guard.insert(sid.clone(), new_cwd.clone());
                }

                // Emit WorkingFolderChanged if cwd actually changed.
                if old_cwd.as_ref() != Some(&new_cwd) {
                    let git_root = search_exec_find_git_root(&new_cwd);
                    let project = git_root.as_ref().map(|p| p.to_string_lossy().to_string());
                    let project_name = git_root
                        .as_ref()
                        .and_then(|p| p.file_name())
                        .map(|n| n.to_string_lossy().to_string());
                    // Update session metadata
                    let cwd_str = new_cwd.to_string_lossy().to_string();
                    if let Ok(Some(mut meta)) = state.manager.global_sessions.get_session_meta(sid)
                    {
                        meta.cwd = Some(cwd_str.clone());
                        meta.project = project.clone();
                        meta.project_name = project_name.clone();
                        let _ = state.manager.global_sessions.update_session_meta(&meta);
                    }
                    let _ = state.events_tx.send(ServerEvent::WorkingFolderChanged {
                        session_id: sid.clone(),
                        cwd: cwd_str,
                        project,
                        project_name,
                    });
                }
            }
        }
        // Strip [sentinel, end-of-pwd-line] inclusive of the pwd's trailing \n.
        let strip_end = (pwd_end + 1).min(stdout.len());
        stdout.replace_range(sent_pos..strip_end, "");
    }

    Json(serde_json::json!({
        "exit_code": code,
        "stdout": stdout,
        "stderr": stderr,
    }))
    .into_response()
}

#[cfg(test)]
mod known_root_tests {
    use super::*;

    #[test]
    fn only_a_known_root_itself_is_served_never_the_filesystem_root() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().canonicalize().unwrap();
        let roots: HashSet<PathBuf> = [project.clone(), PathBuf::from("/")].into();
        assert!(is_known_root(&project, &roots));
        assert!(
            !is_known_root(Path::new("/"), &roots),
            "`/` is never a root"
        );
        assert!(!is_known_root(Path::new("/etc"), &roots));
        assert!(
            !is_known_root(&project.join("sub"), &roots),
            "exact roots only"
        );
    }
}
