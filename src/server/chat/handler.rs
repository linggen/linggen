use crate::engine::agent::AgentManager;
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

/// Lead-in filler the auto-titler strips before picking words. Matched
/// case-insensitively at a word boundary so "hide" is never read as
/// "hi" + "de". Longest match wins so "i want to" beats bare "i".
const TITLE_FILLER: &[&str] = &[
    // greetings / interjections
    "hi", "hello", "hey", "yo", "hiya", "sup", "greetings",
    // politeness / acks
    "ok", "okay", "please", "pls", "plz", "thanks", "thx", "btw", "anyway", "so", "well", "just",
    // soft asks (longer phrases first so they take precedence over shorter prefixes)
    "i would like to", "i'd like to", "i want to", "i need to",
    "could you please", "would you please", "can you please",
    "do you know", "do you", "did you", "can you", "could you", "would you",
    "will you", "should you", "are you", "is it", "is there", "is the",
    "let's", "lets", "let me",
];

/// Skip leading filler tokens + punctuation so the title reflects the
/// substantive part of the message instead of "hi, do you …".
fn strip_leading_filler(message: &str) -> String {
    let mut s = message.trim().to_string();
    loop {
        let trimmed = s
            .trim_start_matches(|c: char| {
                c.is_whitespace() || matches!(c, ',' | '.' | '!' | '?' | ';' | ':' | '—' | '-')
            })
            .to_string();
        s = trimmed;
        let lower = s.to_lowercase();
        let mut matched_len = 0usize;
        for phrase in TITLE_FILLER {
            if !lower.starts_with(phrase) {
                continue;
            }
            // Require a word boundary after the match so "hide" doesn't
            // get stripped to "de".
            let after = lower[phrase.len()..].chars().next();
            let boundary = after.is_none_or(|c| !c.is_alphanumeric() && c != '\'');
            if boundary && phrase.len() > matched_len {
                matched_len = phrase.len();
            }
        }
        if matched_len == 0 {
            break;
        }
        s = s[matched_len..].to_string();
    }
    s
}

/// Generate a session title from the first ~3 substantive words of the
/// user's message, after stripping greetings / soft-ask leads (`hi,`,
/// `can you`, `i want to`, …) and leading punctuation. Falls back to
/// the raw message when stripping leaves nothing.
fn auto_session_title(message: &str) -> String {
    const MAX_WORDS: usize = 3;
    const MAX_CHARS: usize = 30;

    let stripped = strip_leading_filler(message);
    let source = if stripped.split_whitespace().next().is_some() {
        stripped
    } else {
        message.trim().to_string()
    };

    let words: Vec<&str> = source.split_whitespace().collect();
    if words.is_empty() {
        return "New Chat".to_string();
    }
    let first: String = words.iter().take(MAX_WORDS).copied().collect::<Vec<_>>().join(" ");
    if first.chars().count() > MAX_CHARS {
        let s: String = first.chars().take(MAX_CHARS - 3).collect();
        format!("{}...", s.trim_end())
    } else if words.len() > MAX_WORDS {
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
        // The title here is already derived from the user message via
        // `auto_session_title`, so it's a real title — lock it.
        title_locked: true,
    };

    if let Some(sid) = req.session_id.clone() {
        let exists = matches!(global_sessions.get_session_meta(&sid), Ok(Some(_)));
        if !exists {
            let _ = global_sessions.add_session(&make_meta(sid.clone()));
        } else {
            maybe_auto_rename(state, &sid, &req.message).await;
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

/// If the session was created with a placeholder title (UI's time-based
/// "Chat May 22, 3:20 PM" or "New Chat"), replace it with a title derived
/// from the user's first substantive message and lock it.
///
/// Best-effort: any read/write failure is swallowed — chat must not block
/// on a cosmetic rename.
async fn maybe_auto_rename(state: &Arc<ServerState>, session_id: &str, message: &str) {
    let store = &state.manager.global_sessions;
    let Ok(Some(meta)) = store.get_session_meta(session_id) else {
        return;
    };
    if meta.title_locked {
        return;
    }
    let trimmed = message.trim();
    if trimmed.is_empty() {
        return;
    }
    // Strip the optional `@agent ` routing prefix so the rename reflects
    // the actual user intent, not the routing token.
    let body = parse_explicit_target_prefix(trimmed)
        .map(|(_, rest)| rest)
        .unwrap_or(trimmed);
    if body.trim().is_empty() {
        return;
    }
    let new_title = auto_session_title(body);
    if new_title == meta.title {
        return;
    }
    if store.rename_session(session_id, &new_title).is_ok() {
        let _ = state.events_tx.send(ServerEvent::StateUpdated);
    }
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
        return;
    }

    // Owner, no explicit model. Prefer the bound skill's declared default
    // (SKILL.md `model:`) so an app ships its own model without touching the
    // engine-wide default; fall back to the global default otherwise. A
    // per-skill user override is layered on top by passing it as the session's
    // pinned model (req_model_id), which already wins above.
    if let Some(skill_name) = session_meta.as_ref().and_then(|m| m.skill.clone()) {
        if let Some(skill) = manager.skills.reload_one(&skill_name).await {
            if let Some(model) = skill.model.filter(|m| engine.model_manager.has_model(m)) {
                engine.model_id = model;
                return;
            }
        }
    }
    engine.model_id = engine.default_model_id.clone();
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
/// cache, or after `/clear` emptied it). System messages and observations
/// are skipped — only user/assistant turns rejoin the in-memory history.
///
/// `current_user_msg` is the message about to enter the turn. The chat
/// handler persists each incoming user message to the session store
/// **before** spawning the engine task (so other clients see it
/// immediately via the event broadcast), so the same message shows up in
/// `get_chat_history` here. `push_user_turn_with_recall` will then push
/// it into `chat_history` again — leaving a duplicate. Trim the trailing
/// entry when it's the current message so the post-push state is exactly
/// one copy.
async fn restore_chat_history_if_empty(
    engine: &mut crate::engine::AgentEngine,
    manager: &Arc<AgentManager>,
    session_id: Option<&str>,
    current_user_msg: &str,
) {
    if !engine.chat_history.is_empty() {
        return;
    }
    let sid = session_id.unwrap_or("default");
    let Ok(mut msgs) = manager.global_sessions.get_chat_history(sid) else {
        return;
    };
    // Drop the trailing entry if it matches the just-persisted current
    // user message; push_user_turn_with_recall will add it back exactly
    // once. Only the very last entry can be the current message (the
    // persist write was the most recent op).
    if let Some(last) = msgs.last() {
        if !last.is_observation && last.from_id == "user" && last.content == current_user_msg {
            msgs.pop();
        }
    }
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
    let Some(skill) = ctx.manager.skills.reload_one(&skill_name).await else {
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
    if let Some((skill_name, remaining)) = manager.skills.match_trigger(clean_msg).await {
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

        // Skill- and mission-created sessions don't write to the user's
        // biographical memory and shouldn't have the core block + memory
        // protocol injected into their system prompt. The owner-vs-consumer
        // policy is too coarse to express this — every owner-machine session
        // gets `include_memory=true` by default — so we narrow it here once
        // the live creator is known.
        if session_creator != "user" {
            engine.prompt_profile.include_memory = false;
            engine.cached_system_prompt = None;
        }

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

        restore_chat_history_if_empty(
            &mut engine,
            &manager,
            ctx.session_id.as_deref(),
            &clean_msg_clone,
        )
        .await;
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

        // Owner turns also catch up any mission whose `catchup_hours` is set
        // and whose last run is older than that threshold (e.g. the `dream`
        // consolidate+evict mission, when its daily cron was missed because
        // the machine was off/asleep). Cheap, non-blocking, guarded; no-op
        // unless something is overdue.
        if engine.prompt_profile.include_memory {
            crate::extensions::missions::scheduler::maybe_fire_catchup_missions(ctx.state.clone());
        }
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
    use super::{auto_session_title, parse_explicit_target_prefix};

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

    #[test]
    fn auto_title_three_word_short_kept_whole() {
        assert_eq!(auto_session_title("fix login bug"), "fix login bug");
    }

    #[test]
    fn auto_title_truncates_long_run_to_three_words() {
        let t = auto_session_title("one two three four five six");
        assert_eq!(t, "one two three...");
    }

    #[test]
    fn auto_title_strips_leading_greeting() {
        // "hi, do you know my cat" → drop "hi,", drop "do you know" → "my cat"
        assert_eq!(
            auto_session_title("hi, do you know my cat"),
            "my cat"
        );
    }

    #[test]
    fn auto_title_strips_polite_lead() {
        assert_eq!(
            auto_session_title("please add a button to the toolbar"),
            "add a button..."
        );
    }

    #[test]
    fn auto_title_strips_soft_ask() {
        assert_eq!(
            auto_session_title("i want to debug a stack overflow"),
            "debug a stack..."
        );
    }

    #[test]
    fn auto_title_word_boundary_keeps_real_words() {
        // "hide" must not get its "hi" prefix stripped.
        assert_eq!(auto_session_title("hide the sidebar on mobile"), "hide the sidebar...");
    }

    #[test]
    fn auto_title_falls_back_when_strip_empties() {
        // Pure filler — fall back to the raw message instead of "New Chat".
        assert_eq!(auto_session_title("hi"), "hi");
    }

    #[test]
    fn auto_title_empty_returns_placeholder() {
        assert_eq!(auto_session_title("   "), "New Chat");
    }
}
