//! Server-pushed page state — aggregates all UI-relevant data into one message.
//!
//! Instead of the frontend making 10+ individual HTTP requests on every state
//! change, the server pushes a single `PageState` message over the WebRTC
//! control data channel at 0.5 Hz (every 2 seconds), only when state changed.

use crate::server::ServerState;
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Arc;

/// The frontend's current view context — what session/project is active.
/// Sent by the frontend via `set_view_context` control channel message.
#[derive(Debug, Clone, Default)]
pub struct ViewContext {
    pub session_id: Option<String>,
    pub project_root: Option<String>,
    /// Which UI entry is connected: "main" | "embed" | "consumer".
    /// None means the client hasn't reported yet — treat as "main".
    pub view: Option<String>,
}

/// Aggregated page state pushed to the frontend.
/// All fields are optional — the server omits fields that haven't changed
/// or aren't applicable to the current view context.
#[derive(Debug, Clone, Serialize)]
pub struct PageState {
    // -- User context (always included) --

    /// User type: "owner" or "consumer". Set at connection time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_type: Option<String>,

    /// User's permission level: "admin", "edit", "read", "chat"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission: Option<String>,

    /// Room name (only for room users)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub room_name: Option<String>,

    /// Whether the owner's room is enabled (accepting consumers).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub room_enabled: Option<bool>,

    // -- Global (always included unless compact mode) --

    #[serde(skip_serializing_if = "Option::is_none")]
    pub all_sessions: Option<Vec<serde_json::Value>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub models: Option<Vec<serde_json::Value>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_models: Option<Vec<String>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub skills: Option<Vec<serde_json::Value>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub missions: Option<Vec<serde_json::Value>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending_ask_user: Option<Vec<serde_json::Value>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_counts_by_project: Option<std::collections::HashMap<String, usize>>,

    /// Map of session_id → agent status string for all currently-busy sessions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub busy_sessions: Option<std::collections::HashMap<String, String>>,

    // -- Scoped (based on ViewContext, Admin only) --

    #[serde(skip_serializing_if = "Option::is_none")]
    pub agents: Option<Vec<serde_json::Value>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_runs: Option<Vec<serde_json::Value>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub sessions: Option<Vec<serde_json::Value>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_permission: Option<serde_json::Value>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub files: Option<Vec<serde_json::Value>>,
}

/// Dirty-flag bits for tracking which data categories changed.
pub const DIRTY_GLOBAL: u64 = 0b0000_0001;
pub const DIRTY_SCOPED: u64 = 0b0000_0010;
pub const DIRTY_ALL: u64 = DIRTY_GLOBAL | DIRTY_SCOPED;

/// Build the page state from server state + view context + user context.
/// All data is filtered by user_id. Permission level controls what data categories are included.
pub async fn build_page_state(
    state: &Arc<ServerState>,
    ctx: &ViewContext,
    dirty: u64,
    user: &super::UserContext,
) -> PageState {
    let include_global = (dirty & DIRTY_GLOBAL) != 0;
    let include_scoped = (dirty & DIRTY_SCOPED) != 0;
    let is_admin = user.permission.is_admin();
    let user_id = &user.user_id;

    // Load room config — needed for consumer filtering and owner room_enabled status.
    let room_cfg = super::room_config::load_room_config();

    let mut ps = PageState {
        user_type: Some(user.user_type().to_string()),
        permission: Some(user.permission.as_str().to_string()),
        room_name: user.room_name.clone(),
        room_enabled: if !user.is_consumer { Some(room_cfg.room_enabled) } else { None },
        all_sessions: None,
        models: None,
        default_models: None,
        skills: None,
        missions: None,
        pending_ask_user: None,
        session_counts_by_project: None,
        busy_sessions: None,
        agents: None,
        agent_runs: None,
        sessions: None,
        session_permission: None,
        files: None,
    };

    // Collect user's session IDs for filtering busy_sessions and pending_ask_user
    let user_session_ids: std::collections::HashSet<String> =
        if let Ok(sessions) = state.manager.global_sessions.list_sessions() {
            sessions.iter()
                .filter(|s| match (&s.user_id, user_id.as_str()) {
                    (Some(sid), uid) => sid == uid,
                    (None, "__local__") => true, // legacy sessions belong to local owner
                    (None, _) => is_admin,       // admin sees legacy sessions
                    _ => false,
                })
                .map(|s| s.id.clone())
                .collect()
        } else {
            std::collections::HashSet::new()
        };

    // Wrap room config for consumer filtering
    let room_cfg = if user.is_consumer { Some(room_cfg) } else { None };

    // -- Global data (skip for embed peers — they're pinned to one session
    // and don't render the sidebar/mission list that these fields feed) --
    let is_embed_view = ctx.view.as_deref() == Some("embed");
    if include_global && !is_embed_view {
        // All sessions — filtered by user_id
        if let Ok(sessions) = state.manager.global_sessions.list_sessions() {
            ps.all_sessions = Some(
                sessions.into_iter()
                    .filter(|s| user_session_ids.contains(&s.id))
                    .filter_map(|s| serde_json::to_value(s).ok())
                    .collect(),
            );
        }

        // Missions — admin only
        if is_admin {
            if let Ok(missions) = state.manager.missions.list_all_missions() {
                ps.missions = Some(
                    missions.into_iter()
                        .filter_map(|m| serde_json::to_value(m).ok())
                        .collect(),
                );
            }
        }
    }

    // Models + skills + pending ask-user + busy sessions
    if include_global {
        // Models — admin sees all, others see shared_models only
        let models_guard = state.manager.models.read().await;
        let all_models: Vec<_> = models_guard.list_models().into_iter().cloned().collect();
        drop(models_guard);
        let models = if let Some(ref cfg) = room_cfg {
            let shared: std::collections::HashSet<&str> = cfg.shared_models.iter().map(|s| s.as_str()).collect();
            all_models.into_iter().filter(|m| shared.contains(m.id.as_str())).collect::<Vec<_>>()
        } else {
            all_models
        };
        ps.models = Some(models.into_iter().map(|m| {
            // Only send metadata — strip sensitive fields (api_key, url)
            serde_json::json!({
                "id": m.id,
                "provider": m.provider,
                "model": m.model,
                "tags": m.tags,
                "supports_tools": m.supports_tools,
                "provided_by": m.provided_by,
            })
        }).collect());

        // Default models — admin only
        if is_admin {
            if let Ok((config, _)) = crate::config::Config::load_with_path() {
                ps.default_models = Some(config.routing.default_models.clone());
            }
        }

        // Skills — admin sees all, others see allowed_skills only
        let skills = state.skills.list_skills().await;
        let skills = if let Some(ref cfg) = room_cfg {
            let allowed: std::collections::HashSet<&str> = cfg.allowed_skills.iter().map(|s| s.as_str()).collect();
            skills.into_iter().filter(|s| allowed.contains(s.name.as_str())).collect::<Vec<_>>()
        } else {
            skills
        };
        ps.skills = Some(skills.into_iter().filter_map(|s| {
            let mut v = serde_json::to_value(s).ok()?;
            // Strip large fields — UI only needs metadata for sidebar cards
            v.as_object_mut().map(|m| {
                m.remove("content");
                m.remove("context");
            });
            Some(v)
        }).collect());

        // Embed peers are pinned to one session — narrow user_session_ids to
        // just that session so pending_ask_user and busy_sessions can't leak
        // across sessions owned by the same user.
        let is_embed = ctx.view.as_deref() == Some("embed");
        let pinned = if is_embed { ctx.session_id.as_deref() } else { None };
        let scope_check = |sid: &str| -> bool {
            if let Some(pin) = pinned {
                sid == pin
            } else {
                is_admin || user_session_ids.contains(sid)
            }
        };

        // Pending ask-user — filtered to user's sessions (+ pinned for embed)
        let pending = state.pending_ask_user.lock().await;
        let items: Vec<serde_json::Value> = pending.iter()
            .filter(|(_, entry)| {
                entry.session_id.as_deref()
                    .map_or(is_admin && pinned.is_none(), |sid| scope_check(sid))
            })
            .map(|(qid, entry)| {
                serde_json::json!({
                    "question_id": qid,
                    "agent_id": entry.agent_id,
                    "questions": entry.questions,
                    "session_id": entry.session_id,
                })
            })
            .collect();
        ps.pending_ask_user = Some(items);

        // Busy sessions — filtered to user's sessions (+ pinned for embed)
        let active = state.active_statuses.lock().await;
        let mut busy: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        for (key, record) in active.iter() {
            if let Some(sid) = key.split('|').next() {
                if !sid.is_empty() && scope_check(sid) {
                    busy.entry(sid.to_string())
                        .or_insert_with(|| record.status.as_str().to_string());
                }
            }
        }
        drop(active);
        ps.busy_sessions = Some(busy);
    }

    // -- Scoped data (admin only — project/session context) --
    if include_scoped && is_admin {
        if let Some(ref root) = ctx.project_root {
            let root_buf = PathBuf::from(root);

            if let Ok(agents) = state.manager.list_agents(&root_buf).await {
                ps.agents = Some(agents.into_iter().filter_map(|a| serde_json::to_value(a).ok()).collect());
            }

            if let Ok(all) = state.manager.global_sessions.list_sessions() {
                let project_sessions: Vec<_> = all.into_iter()
                    .filter(|s| s.project.as_deref() == Some(root.as_str()) || s.cwd.as_deref() == Some(root.as_str()))
                    .take(50)
                    .filter_map(|s| serde_json::to_value(s).ok())
                    .collect();
                ps.sessions = Some(project_sessions);
            }
        }

        if let Some(ref session_id) = ctx.session_id {
            let root_buf = PathBuf::from(ctx.project_root.as_deref().unwrap_or(""));
            if let Ok(runs) = state.manager.list_agent_runs(&root_buf, Some(session_id.as_str())).await {
                ps.agent_runs = Some(runs.into_iter().filter_map(|r| serde_json::to_value(r).ok()).collect());
            }

            let session_dir = crate::paths::global_sessions_dir().join(session_id);
            let perms = crate::engine::permission::SessionPermissions::load(&session_dir);
            let mut perm_val = serde_json::to_value(&perms).unwrap_or_default();

            // Prefer the session's own cwd (from SessionMeta) over the UI's
            // ctx.project_root. Skill-embed sessions send empty project_root;
            // without falling back to session.cwd, effective_mode and zone
            // would always be computed against an empty path.
            let effective_root: Option<String> = state
                .manager
                .global_sessions
                .get_session_meta(session_id)
                .ok()
                .flatten()
                .and_then(|m| m.cwd)
                .filter(|s| !s.is_empty())
                .or_else(|| ctx.project_root.clone().filter(|s| !s.is_empty()));

            if let Some(ref root) = effective_root {
                // Effective mode at the session's working folder. No covering
                // grant means no tools there — emit an explicit "chat" rather
                // than omitting the field: the UI keeps the last value it saw,
                // so an absent field would leave the previous folder's mode on
                // the chip after the agent cd's into an ungranted folder.
                let mode = crate::engine::permission::effective_mode_for_path(&perms.path_modes, std::path::Path::new(root))
                    .unwrap_or(crate::engine::permission::PermissionMode::Chat);
                perm_val.as_object_mut().map(|m| m.insert("effective_mode".to_string(), serde_json::Value::String(mode.to_string())));
            }
            ps.session_permission = Some(perm_val);
        }
    }

    ps
}

