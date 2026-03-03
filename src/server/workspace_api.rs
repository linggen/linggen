use crate::server::chat_helpers::sanitize_message_for_ui;
use crate::server::ServerState;
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use ignore::WalkBuilder;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Deserialize)]
pub(crate) struct FileQuery {
    project_root: String,
    path: Option<String>,
}

pub(crate) async fn list_files(
    State(_state): State<Arc<ServerState>>,
    Query(query): Query<FileQuery>,
) -> impl IntoResponse {
    let project_root = PathBuf::from(&query.project_root);
    let canonical_root = match project_root.canonicalize() {
        Ok(r) => r,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
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
        for entry in dir {
            if let Ok(entry) = entry {
                let name = entry.file_name().to_string_lossy().to_string();
                let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
                entries.push(serde_json::json!({
                    "name": name,
                    "isDir": is_dir,
                    "path": if rel_path.is_empty() { name } else { format!("{}/{}", rel_path, name) }
                }));
            }
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
    State(_state): State<Arc<ServerState>>,
    Query(query): Query<FileSearchQuery>,
) -> impl IntoResponse {
    let project_root = PathBuf::from(&query.project_root);
    let canonical_root = match project_root.canonicalize() {
        Ok(r) => r,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };

    let limit = query.limit.unwrap_or(50);
    let search = query
        .query
        .as_deref()
        .unwrap_or("")
        .to_lowercase();

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
    State(_state): State<Arc<ServerState>>,
    Query(query): Query<FileQuery>,
) -> impl IntoResponse {
    let rel_path = match query.path {
        Some(p) => p,
        None => return StatusCode::BAD_REQUEST.into_response(),
    };
    if rel_path.contains("..") {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let project_root = PathBuf::from(&query.project_root);
    let canonical_root = match project_root.canonicalize() {
        Ok(r) => r,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
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
    let root = PathBuf::from(&query.project_root);
    if let Ok(ctx) = state.manager.get_or_create_project(root).await {
        let active_task = ctx.state_fs.read_file("active.md").ok();
        let user_stories = ctx.state_fs.read_file("user-stories.md").ok();
        let tasks = ctx.state_fs.list_tasks().unwrap_or_default();

        let messages = match query.session_id.as_deref() {
            Some(sid) if !sid.is_empty() => ctx
                .sessions
                .get_chat_history(sid, None)
                .unwrap_or_default(),
            _ => Vec::new(),
        };

        let mapped_messages: Vec<(crate::state_fs::StateFile, String)> = messages
            .into_iter()
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
    } else {
        StatusCode::NOT_FOUND.into_response()
    }
}

pub(crate) async fn get_agent_tree(
    State(state): State<Arc<ServerState>>,
    Query(query): Query<ProjectQuery>,
) -> impl IntoResponse {
    let root_path = PathBuf::from(&query.project_root);
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
                .filter(|run| run.status == crate::project_store::AgentRunStatus::Running)
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
