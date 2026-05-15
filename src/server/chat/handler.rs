use crate::agent_manager::AgentManager;
use crate::server::chat::helpers::{
    emit_queue_updated, persist_and_emit_to_store, queue_key, queue_preview,
};
use crate::server::{AgentStatusKind, QueuedChatItem, ServerEvent, ServerState};
use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use super::skill_dispatch::{run_skill_dispatch, run_trigger_dispatch};
use super::structured::run_structured_loop;
use super::types::ChatRequest;
use super::ChatRunCtx;

pub(super) fn parse_explicit_target_prefix(message: &str) -> Option<(&str, &str)> {
    let rest = message.strip_prefix('@')?;
    let space_idx = rest.find(' ')?;
    let candidate = rest[..space_idx].trim();
    let body = rest[space_idx + 1..].trim_start();
    if candidate.is_empty() {
        return None;
    }
    if !candidate
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return None;
    }
    Some((candidate, body))
}

/// Generate a session title from the first few words of the user's message.
fn auto_session_title(message: &str) -> String {
    let words: Vec<&str> = message.split_whitespace().collect();
    if words.is_empty() {
        return "New Chat".to_string();
    }
    let first: String = words.iter().take(6).copied().collect::<Vec<_>>().join(" ");
    if first.chars().count() > 50 {
        let s: String = first.chars().take(47).collect();
        format!("{}...", s.trim_end())
    } else if words.len() > 6 {
        format!("{first}...")
    } else {
        first
    }
}

/// Expand the request's `project_root` (handles `~`, `~/...`, empty) into an
/// absolute filesystem path.
fn resolve_request_root(req_root: &str) -> PathBuf {
    let expanded = if req_root.is_empty() || req_root == "~" {
        dirs::home_dir().unwrap_or_else(|| std::env::current_dir().unwrap_or_default())
    } else if let Some(rest) = req_root.strip_prefix("~/") {
        dirs::home_dir().unwrap_or_default().join(rest)
    } else {
        PathBuf::from(req_root)
    };
    crate::util::resolve_path(&expanded)
}

/// Ensure a session exists for this request. Auto-creates a fresh `sess-…`
/// id when none was sent; otherwise inserts the requested id into the
/// session store if it isn't there yet so the Web UI can list it.
/// Returns the resolved session id.
async fn ensure_session(
    state: &Arc<ServerState>,
    req: &ChatRequest,
    project_root_str: &str,
    session_creator: &str,
) -> Option<String> {
    let global_sessions = &state.manager.global_sessions;
    let now = crate::util::now_ts_secs();
    let title = auto_session_title(&req.message);

    let make_meta = |id: String| crate::state_fs::sessions::SessionMeta {
        id,
        title: title.clone(),
        created_at: now,
        skill: req.skill_name.clone(),
        creator: session_creator.into(),
        cwd: Some(project_root_str.to_string()),
        project: None,
        project_name: None,
        mission_id: req.mission_id.clone(),
        model_id: req.model_id.clone(),
        user_id: req.user_id.clone(),
        compact_threshold: None,
        compact_focus: None,
    };

    if let Some(sid) = req.session_id.clone() {
        let exists = matches!(global_sessions.get_session_meta(&sid), Ok(Some(_)));
        if !exists {
            let _ = global_sessions.add_session(&make_meta(sid.clone()));
        }
        return Some(sid);
    }

    let new_id = format!("sess-{}-{}", now, &uuid::Uuid::new_v4().to_string()[..8]);
    let _ = global_sessions.add_session(&make_meta(new_id.clone()));
    let _ = state.events_tx.send(ServerEvent::SessionCreated {
        session_id: new_id.clone(),
        title,
        creator: session_creator.into(),
        project: Some(project_root_str.to_string()),
        project_name: std::path::Path::new(project_root_str)
            .file_name()
            .map(|n| n.to_string_lossy().to_string()),
        skill: req.skill_name.clone(),
        mission_id: req.mission_id.clone(),
    });
    Some(new_id)
}

/// Resolve the effective `(target_agent_id, clean_message)` pair.
///
/// Honors a leading `@agent_id ` prefix when the named agent exists in the
/// project; otherwise the request's `agent_id` and message stand as-is.
async fn route_target(
    state: &Arc<ServerState>,
    req: &ChatRequest,
    root: &PathBuf,
) -> (String, String) {
    if let Some((candidate, body)) = parse_explicit_target_prefix(&req.message) {
        let candidate_id = candidate.to_string();
        if state.manager.agent_exists(root, &candidate_id).await {
            return (candidate_id, body.to_string());
        }
    }
    (req.agent_id.clone(), req.message.clone())
}

/// When the agent is busy, enqueue this turn, broadcast the queue update,
/// cancel any pending AskUser for the agent+session (so the running loop
/// unblocks), and forward the message through the interrupt channel.
/// Returns the queued item (if any) so the spawned task can dequeue it
/// once the lock is acquired.
async fn enqueue_if_busy(
    state: &Arc<ServerState>,
    was_busy: bool,
    project_root_str: &str,
    effective_session_id: &str,
    target_id: &str,
    clean_msg: &str,
) -> Option<QueuedChatItem> {
    if !was_busy {
        return None;
    }
    let item = QueuedChatItem {
        id: format!(
            "{}-{}",
            crate::util::now_ts_ms(),
            state.queue_seq.fetch_add(1, Ordering::Relaxed)
        ),
        agent_id: target_id.to_string(),
        session_id: effective_session_id.to_string(),
        preview: queue_preview(clean_msg),
        timestamp: crate::util::now_ts_secs(),
    };

    let key = queue_key(project_root_str, effective_session_id, target_id);
    {
        let mut guard = state.queued_chats.lock().await;
        guard.entry(key).or_default().push(item.clone());
    }
    emit_queue_updated(state, project_root_str, effective_session_id, target_id).await;

    // Cancel any pending AskUser for this agent+session so the tool
    // unblocks immediately and the loop can pick up the new message.
    {
        let mut pending = state.pending_ask_user.lock().await;
        pending.retain(|_, entry| {
            !(entry.agent_id == target_id
                && entry.session_id.as_deref() == Some(effective_session_id))
        });
    }

    // Send through interrupt channel so the running loop sees the message.
    {
        let interrupt_guard = state.interrupt_tx.lock().await;
        let ikey = queue_key(project_root_str, effective_session_id, target_id);
        if let Some(tx) = interrupt_guard.get(&ikey) {
            let _ = tx.send(clean_msg.to_string());
        }
    }
    Some(item)
}

/// Pop the queued item out of the chat queue (now that the engine lock is
/// held) and persist+emit the user message that was held back at submit.
async fn dequeue_and_emit(
    state: &Arc<ServerState>,
    events_tx: &tokio::sync::broadcast::Sender<ServerEvent>,
    queued_id: &str,
    project_root: &str,
    session_id: &str,
    target_id: &str,
    clean_msg: &str,
    response_session_id: Option<&str>,
) {
    let key = queue_key(project_root, session_id, target_id);
    {
        let mut guard = state.queued_chats.lock().await;
        if let Some(items) = guard.get_mut(&key) {
            items.retain(|item| item.id != queued_id);
            if items.is_empty() {
                guard.remove(&key);
            }
        }
    }
    emit_queue_updated(state, project_root, session_id, target_id).await;
    persist_and_emit_to_store(
        &state.manager.global_sessions,
        events_tx,
        target_id,
        "user",
        target_id,
        clean_msg,
        response_session_id,
        false,
    )
    .await;
}

/// Resolve and pin the engine's effective model for this turn.
///
/// Consumers (proxy room) MUST be restricted to the room's `shared_models` —
/// never fall back to the owner's default, which would leak access to a
/// model the consumer isn't allowed to use.
async fn resolve_effective_model(
    engine: &mut crate::engine::AgentEngine,
    is_consumer: bool,
    shared_models: &[String],
    req_model_id: Option<&str>,
    session_id: Option<&str>,
    manager: &Arc<AgentManager>,
) {
    let consumer_default = || -> Option<String> {
        shared_models
            .iter()
            .find(|id| engine.model_manager.has_model(id))
            .cloned()
    };
    let pin_session_model = |meta: Option<crate::state_fs::sessions::SessionMeta>,
                             new_model: Option<String>| async move {
        let Some(mut meta) = meta else { return };
        if meta.model_id.as_deref() == new_model.as_deref() {
            return;
        }
        meta.model_id = new_model;
        let _ = manager.global_sessions.update_session_meta(&meta);
    };

    let session_meta =
        session_id.and_then(|sid| manager.global_sessions.get_session_meta(sid).ok().flatten());

    if let Some(mid) = req_model_id {
        let ok = engine.model_manager.has_model(mid)
            && (!is_consumer || shared_models.iter().any(|m| m == mid));
        if ok {
            engine.model_id = mid.to_string();
            pin_session_model(session_meta, Some(mid.to_string())).await;
            return;
        }
        // Requested model unavailable / not shared with this consumer.
        // Fall back to a shared model (consumer) or the configured default
        // (owner), and clear stale pinning so the session stops requesting
        // the dead id.
        let fallback = if is_consumer {
            consumer_default().unwrap_or_else(|| engine.default_model_id.clone())
        } else {
            engine.default_model_id.clone()
        };
        tracing::warn!(
            "Session '{}' requested model '{}' which is unavailable for {} — falling back to '{}'",
            session_id.unwrap_or("?"),
            mid,
            if is_consumer { "consumer" } else { "owner" },
            fallback
        );
        engine.model_id = fallback;
        pin_session_model(session_meta, None).await;
        return;
    }

    if is_consumer {
        // Consumer chose "Default" — pick a shared model, never the owner's.
        engine.model_id = consumer_default().unwrap_or_else(|| engine.default_model_id.clone());
    } else {
        engine.model_id = engine.default_model_id.clone();
    }
}

/// Promote a session created by the mission scheduler to a user-owned
/// session the moment a real user takes over the conversation.
///
/// The mission scheduler never goes through `chat_handler` (it calls
/// `dispatch_mission_prompt` directly), so any request landing here on a
/// mission-creator session means a human picked it up. Reset the
/// permission mode that the scheduler had forced to Auto, and invalidate
/// the cached system prompt so the next loop rebuilds with the new tier.
async fn promote_mission_session_to_user(
    engine: &mut crate::engine::AgentEngine,
    state: &Arc<ServerState>,
    session_id: Option<&str>,
) {
    let Some(sid) = session_id else { return };
    let Ok(Some(mut meta)) = state.manager.global_sessions.get_session_meta(sid) else {
        return;
    };
    if meta.creator != "mission" {
        return;
    }
    meta.creator = "user".to_string();
    let _ = state.manager.global_sessions.update_session_meta(&meta);

    let cfg = state.manager.get_config_snapshot().await;
    engine.cfg.tool_permission_mode = cfg.agent.tool_permission_mode;
    engine.cached_system_prompt = None;
}

/// Repopulate `chat_history` from the session store when the engine was
/// freshly created (e.g. after a model change invalidated the engine
/// cache). System messages and observations are skipped — only user/
/// assistant turns rejoin the in-memory history.
async fn restore_chat_history_if_empty(
    engine: &mut crate::engine::AgentEngine,
    manager: &Arc<AgentManager>,
    session_id: Option<&str>,
) {
    if !engine.chat_history.is_empty() {
        return;
    }
    let sid = session_id.unwrap_or("default");
    let Ok(msgs) = manager.global_sessions.get_chat_history(sid) else {
        return;
    };
    for m in &msgs {
        if m.is_observation || m.from_id == "system" {
            continue;
        }
        // Session owns the conversation — any non-user message is assistant
        // context regardless of which agent produced it.
        let role = if m.from_id == "user" { "user" } else { "assistant" };
        engine
            .chat_history
            .push(crate::message::ChatMessage::new(role, &m.content));
    }
    if !engine.chat_history.is_empty() {
        tracing::info!(
            "Restored {} chat_history messages from session store",
            engine.chat_history.len()
        );
    }
}

/// Activate a session-bound skill (set on the session meta, e.g. for
/// skill-embed sessions) so its SKILL.md context, allow-skills scope, and
/// declared permission grants take effect for this turn.
///
/// Skips silently when no skill is bound, when policy blocks it, when the
/// skill name doesn't resolve, or when one is already active. The
/// permission grants are applied only when the session is interactive —
/// non-interactive (mission/consumer) sessions don't auto-elevate.
async fn apply_session_bound_skill(engine: &mut crate::engine::AgentEngine, ctx: &ChatRunCtx) {
    if engine.active_skill.is_some() {
        return;
    }
    let Some(sid) = ctx.session_id.as_deref() else { return };
    let bound_skill_name = match ctx.manager.global_sessions.get_session_meta(sid) {
        Ok(meta) => meta.and_then(|m| m.skill),
        Err(e) => {
            tracing::warn!("Failed to read session meta for {}: {}", sid, e);
            return;
        }
    };
    let Some(skill_name) = bound_skill_name else { return };

    if !ctx.policy.is_skill_allowed(&skill_name) {
        tracing::info!("Session-bound skill '{}' blocked by policy", skill_name);
        return;
    }
    let Some(skill) = ctx.manager.skill_manager.get_skill(&skill_name).await else {
        return;
    };
    tracing::info!("Session-bound skill activated: {}", skill.name);

    // Ensure session_dir is populated so skill-permission saves persist.
    // Otherwise run_agent_loop later loads permission.json from disk and
    // clobbers the in-memory grants we're about to apply.
    let sdir = crate::paths::global_sessions_dir().join(sid);
    if engine.session_dir.is_none() {
        engine.session_dir = Some(sdir.clone());
    }

    // Hydrate session_permissions from disk BEFORE activate_skill —
    // otherwise write_skill_grants merges SKILL.md grants on top of the
    // empty in-memory default and saves, clobbering any runtime grants
    // (e.g. pulse's workspace_path) that the skill iframe PATCHed onto
    // permission.json before the first user turn. Mirrors the same
    // preload the slash-command path does in skill_dispatch.rs.
    engine.session_permissions =
        crate::engine::permission::SessionPermissions::load(&sdir);

    if let crate::engine::ActivationOutcome::Activated { grants_changed: true } =
        engine.activate_skill(skill, crate::engine::ActivationMode::SessionBound).await
    {
        let _ = ctx.events_tx.send(ServerEvent::StateUpdated);
    }
}

/// Route the turn to one of three flows in order: slash-command skill
/// dispatch, user-defined trigger prefix match, or the structured agent
/// loop. The first match wins.
async fn dispatch_turn(
    ctx: &ChatRunCtx,
    engine: &mut crate::engine::AgentEngine,
    manager: &Arc<AgentManager>,
    clean_msg: &str,
) {
    if clean_msg.trim_start().starts_with('/') {
        run_skill_dispatch(ctx, engine).await;
        return;
    }
    if let Some((skill_name, remaining)) = manager.skill_manager.match_trigger(clean_msg).await {
        run_trigger_dispatch(ctx, engine, &skill_name, &remaining).await;
        return;
    }
    run_structured_loop(ctx, engine).await;
}

pub(crate) async fn chat_handler(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<ChatRequest>,
) -> impl IntoResponse {
    let root = resolve_request_root(&req.project_root);
    let project_root_str = root.to_string_lossy().to_string();

    let session_creator: &str = if req.mission_id.is_some() {
        "mission"
    } else if req.skill_name.is_some() {
        "skill"
    } else {
        "user"
    };

    let session_id = ensure_session(&state, &req, &project_root_str, session_creator).await;
    let effective_session_id = session_id.clone().unwrap_or_else(|| "default".to_string());
    let events_tx = state.events_tx.clone();

    let (target_id, clean_msg) = route_target(&state, &req, &root).await;

    let agent = match state
        .manager
        .get_or_create_session_agent(&effective_session_id, &root, &target_id)
        .await
    {
        Ok(a) => a,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };

    let was_busy = agent.try_lock().is_err();
    let queued_item = enqueue_if_busy(
        &state, was_busy, &project_root_str, &effective_session_id, &target_id, &clean_msg,
    )
    .await;

    if !was_busy {
        // Persist + emit. Images are ephemeral (sent inline as base64 for the
        // current turn, not persisted).
        persist_and_emit_to_store(
            &state.manager.global_sessions,
            &events_tx,
            &target_id,
            "user",
            &target_id,
            &clean_msg,
            session_id.as_deref(),
            false,
        )
        .await;
    }
    // Queued messages are persisted when dequeued so they don't appear in
    // chat before the agent picks them up.

    let session_id_response = session_id.clone();
    let events_tx_clone = events_tx.clone();
    let target_id_clone = target_id.clone();
    let clean_msg_clone = clean_msg.clone();
    let root_clone = root.clone();
    let manager = state.manager.clone();
    let state_clone = state.clone();
    let queued_item_id = queued_item.as_ref().map(|q| q.id.clone());
    let session_id_for_queue = effective_session_id.clone();
    let project_root_for_queue = project_root_str.clone();
    let req_user_type = req.user_type;
    let req_model_id = req.model_id.clone();
    let req_images = req.images.clone();

    tokio::spawn(async move {
        let mut engine = agent.lock().await;

        // Refresh the engine's model_manager from live state so it sees
        // models registered after engine creation — most notably proxy
        // models added when the user joins a room mid-session.
        engine.model_manager = state.manager.models.read().await.clone();

        if let Some(queued_id) = queued_item_id.as_deref() {
            dequeue_and_emit(
                &state_clone,
                &events_tx_clone,
                queued_id,
                &project_root_for_queue,
                &session_id_for_queue,
                &target_id_clone,
                &clean_msg_clone,
                session_id.as_deref(),
            )
            .await;
        }

        let is_consumer = req_user_type == "consumer";
        let shared_models = if is_consumer {
            crate::server::rtc::room_config::load_room_config().shared_models
        } else {
            Vec::new()
        };
        resolve_effective_model(
            &mut engine,
            is_consumer,
            &shared_models,
            req_model_id.as_deref(),
            session_id.as_deref(),
            &state.manager,
        )
        .await;

        engine.tools.builtins.set_session_id(session_id.clone());

        // Clear mission-only restrictions — user-initiated chats should never
        // be restricted by permission tiers (those apply only to automated
        // scheduler runs via apply_permission_tier).
        engine.cfg.mission_allowed_tools = None;
        engine.cfg.bash_allow_prefixes = None;

        promote_mission_session_to_user(&mut engine, &state_clone, session_id.as_deref()).await;

        let policy =
            crate::engine::session_policy::SessionPolicy::from_user_type(&req_user_type);
        policy.apply(&mut engine);

        let model_label = engine.model_id.clone();
        state_clone
            .send_agent_status(
                target_id_clone.clone(),
                AgentStatusKind::ModelLoading,
                Some(format!("Loading model: {model_label}")),
                None,
                session_id.clone(),
            )
            .await;
        let ctx = ChatRunCtx {
            state: state_clone.clone(),
            manager: manager.clone(),
            events_tx: events_tx_clone.clone(),
            root: root_clone,
            agent_id: target_id_clone.clone(),
            session_id: session_id.clone(),
            clean_msg: clean_msg_clone.clone(),
            images: req_images,
            policy,
        };

        restore_chat_history_if_empty(&mut engine, &manager, ctx.session_id.as_deref()).await;
        apply_session_bound_skill(&mut engine, &ctx).await;

        dispatch_turn(&ctx, &mut engine, &manager, &clean_msg_clone).await;

        // Emit TurnComplete so the Web UI has a single finalizer.
        let _ = state_clone.events_tx.send(ServerEvent::TurnComplete {
            agent_id: target_id_clone.clone(),
            duration_ms: None,
            context_tokens: None,
            parent_id: None,
            session_id: session_id.clone(),
            run_id: None,
            parent_run_id: None,
        });

        // Post-turn: if this owner session just hit a consolidation
        // interval, fire the memory consolidation tick off the turn
        // (non-blocking — reads what it needs from `engine` here, then
        // spawns). No-op for consumer/mission/sub-N sessions.
        super::consolidation::maybe_fire_consolidation(&ctx, &engine);
        state_clone
            .send_agent_status(
                target_id_clone,
                AgentStatusKind::Idle,
                Some("Idle".to_string()),
                None,
                session_id.clone(),
            )
            .await;
    });

    let status = if was_busy { "queued" } else { "started" };
    Json(serde_json::json!({ "status": status, "session_id": session_id_response }))
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::parse_explicit_target_prefix;

    #[test]
    fn parse_explicit_target_prefix_accepts_valid_mention() {
        let parsed = parse_explicit_target_prefix("@coder please review src/main.rs");
        assert_eq!(parsed, Some(("coder", "please review src/main.rs")));
    }

    #[test]
    fn parse_explicit_target_prefix_rejects_missing_body() {
        let parsed = parse_explicit_target_prefix("@coder");
        assert_eq!(parsed, None);
    }

    #[test]
    fn parse_explicit_target_prefix_rejects_invalid_agent_token() {
        let parsed = parse_explicit_target_prefix("@coder! please review");
        assert_eq!(parsed, None);
    }
}
