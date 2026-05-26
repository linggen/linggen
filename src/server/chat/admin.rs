use crate::server::{ServerEvent, ServerState};
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use std::path::PathBuf;
use std::sync::Arc;

use crate::engine::ActivationMode;
use super::types::{
    AskUserResponseRequest, ClearChatRequest, CompactChatRequest, CompactConfigRequest,
    SystemPromptQuery,
};

pub(crate) async fn clear_chat_history_api(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<ClearChatRequest>,
) -> impl IntoResponse {
    let session_id = req
        .session_id
        .clone()
        .unwrap_or_else(|| "default".to_string());
    // project_root not needed — chat history is stored globally by session ID.
    // Skipping canonicalize avoids failures when the project directory is deleted.
    match state.manager.global_sessions.clear_chat_history(&session_id) {
        Ok(removed) => {
            // Clear in-memory chat history for this session's engine.
            {
                let engines = state.manager.session_engines.lock().await;
                if let Some(engine_mutex) = engines.get(&session_id) {
                    let mut engine = engine_mutex.lock().await;
                    engine.chat_history.clear();
                    engine.observations.clear();
                }
            }
            let _ = state.events_tx.send(ServerEvent::StateUpdated);
            Json(serde_json::json!({ "removed": removed })).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

pub(crate) async fn get_system_prompt_api(
    State(state): State<Arc<ServerState>>,
    Query(query): Query<SystemPromptQuery>,
) -> impl IntoResponse {
    let sid = query.session_id.as_deref().unwrap_or("default");
    // Resolve a usable filesystem root. Try, in order:
    //   1. The query's `project_root` (when non-empty) — usually what the UI sends.
    //   2. The session's stored `cwd` / `project` — for skill-embed sessions
    //      that send empty project_root.
    //   3. `"/"` — last-resort sentinel that always canonicalizes.
    // A stale `selectedProjectRoot` in the UI used to 400 here; that's a
    // session/UI staleness issue, not something the user needs to fix
    // before exporting the prompt.
    let session_meta = state.manager.global_sessions.get_session_meta(sid).ok().flatten();
    let mut candidates: Vec<String> = Vec::new();
    let q_root = query.project_root.trim();
    if !q_root.is_empty() {
        candidates.push(q_root.to_string());
    }
    if let Some(ref m) = session_meta {
        if let Some(cwd) = m.cwd.as_deref().filter(|s| !s.is_empty()) {
            candidates.push(cwd.to_string());
        }
        if let Some(proj) = m.project.as_deref().filter(|s| !s.is_empty()) {
            candidates.push(proj.to_string());
        }
    }
    candidates.push("/".to_string());
    let root = candidates
        .iter()
        .find_map(|p| PathBuf::from(p).canonicalize().ok())
        .unwrap_or_else(|| PathBuf::from("/"));

    // Skill-embed sessions render the chat sidebar with no project selection,
    // so the frontend's selectedAgent is undefined and the query may carry
    // `agent_id=` or `agent_id=undefined`. Default to the canonical "ling"
    // agent — there's only one agent in the registry; this avoids a 404 on
    // the Copy button regardless of how the frontend serialized "no agent".
    let agent_id = match query.agent_id.trim() {
        "" | "undefined" | "null" => "ling",
        other => other,
    };

    // Build the prompt on a throwaway engine instead of locking the live one.
    // The live engine holds its lock for the duration of an agent turn —
    // long enough for the control-channel RPC (30s) to time out and leave the
    // Copy-System-Prompt button silently unusable. A fresh engine yields the
    // same prompt without touching live state.
    let mut engine = match state.manager.spawn_delegation_engine(&root, agent_id).await {
        Ok(e) => e,
        Err(_) => {
            return (StatusCode::NOT_FOUND, format!("Agent '{}' not found", agent_id))
                .into_response()
        }
    };

    // Apply session-bound skill or mission so the exported prompt matches what
    // the model actually sees during a chat turn. Without this, the export
    // shows a "cold engine" view missing SKILL.md / mission body.
    if let Ok(Some(meta)) = state.manager.global_sessions.get_session_meta(sid) {
        if let Some(ref skill_name) = meta.skill {
            if let Some(skill) = state.manager.skills.get_skill(skill_name).await {
                engine.activate_skill(skill, ActivationMode::Export).await;
            }
        }
        if let Some(ref mission_id) = meta.mission_id {
            if let Ok(Some(mission)) = state.manager.missions.get_mission(mission_id) {
                // Mirror what mission_scheduler does at dispatch time:
                // - inject the mission body via active_mission
                // - apply allowed-tools so the `tools` array and
                //   system-prompt reflect the real run.
                engine.active_mission = Some(crate::engine::ActiveMission {
                    name: mission.name.clone().unwrap_or_else(|| mission.id.clone()),
                    description: mission.description.clone(),
                    body: mission.prompt.clone(),
                    mission_dir: Some(state.manager.missions.mission_dir(&mission.id)),
                });
                if !mission.allowed_tools.is_empty() {
                    engine.cfg.mission_allowed_tools =
                        Some(mission.allowed_tools.iter().cloned().collect());
                }
                // Mirror `scheduler::dispatch_mission_prompt_public`: mission
                // sessions strip the core/memory_protocol blocks from the
                // system prompt and invalidate any cached prompt. Without
                // this, the "Copy System Prompt" debug export shows a
                // prompt the actual run never had — making bugs look like
                // they're elsewhere than they are.
                engine.prompt_profile.include_memory = false;
                engine.cached_system_prompt = None;
            }
        }
    }

    let (messages, allowed_tools, _) = engine.prepare_loop_messages("(export)", true);
    let system_prompt = messages.first().map(|m| m.content.clone()).unwrap_or_default();
    // Tool schemas are delivered to the model via the native function-calling
    // `tools` API parameter — not embedded in the system prompt text. Expose
    // them alongside so the debug export shows the full model-facing surface.
    let tools = engine.tools.oai_tool_definitions(allowed_tools.as_ref());
    Json(serde_json::json!({
        "system_prompt": system_prompt,
        "tools": tools,
    }))
    .into_response()
}

pub(crate) async fn compact_chat_api(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<CompactChatRequest>,
) -> impl IntoResponse {
    let session_id = req
        .session_id
        .clone()
        .unwrap_or_else(|| "default".to_string());
    let agent_id = req.agent_id.clone().unwrap_or_else(|| "ling".to_string());
    let root = match PathBuf::from(&req.project_root).canonicalize() {
        Ok(r) => r,
        Err(e) => return (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    };
    let focus = req.focus.as_deref();

    match state
        .manager
        .get_or_create_session_agent(&session_id, &root, &agent_id)
        .await
    {
        Ok(agent_mutex) => {
            let mut engine = agent_mutex.lock().await;
            // Compact the same effective context auto-compact sees: the
            // durable chat_history PLUS the live tool/observation outputs,
            // where the bulk (e.g. fetched Reddit trees) actually lives.
            // /compact runs outside the agent loop, so a finished run's
            // observations are still resident — auto-compact reaches the
            // same content via prepare_loop_messages. Without this, /compact
            // on a tool-heavy session is a no-op ("nothing to compact").
            let mut messages = std::mem::take(&mut engine.chat_history);
            let obs_rendered: Vec<String> = engine
                .observations
                .iter()
                .map(|o| engine.observation_for_model(o))
                .collect();
            messages.extend(
                obs_rendered
                    .into_iter()
                    .map(|c| crate::message::ChatMessage::new("user", c)),
            );
            let result = engine.force_compact(&mut messages, focus).await;

            let referenced_files: Vec<String> = messages
                .iter()
                .flat_map(|m| extract_file_references(&m.content))
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect();

            // Rewrite the persisted session file with the compacted messages.
            if result.is_some() {
                // Observation bodies are now folded into the summary held in
                // chat_history; drop them so they aren't re-expanded.
                engine.observations.clear();
                let chat_msgs: Vec<crate::state_fs::sessions::ChatMsg> = messages
                    .iter()
                    .map(|m| {
                        let is_user = m.role == "user" || m.role == "system";
                        crate::state_fs::sessions::ChatMsg {
                            agent_id: agent_id.clone(),
                            from_id: if is_user { "user".to_string() } else { agent_id.clone() },
                            to_id: if is_user { agent_id.clone() } else { "user".to_string() },
                            content: m.content.clone(),
                            timestamp: crate::util::now_ts_secs(),
                            is_observation: m.role == "tool",
                        }
                    })
                    .collect();
                if let Err(e) = state
                    .manager
                    .global_sessions
                    .rewrite_chat_history(&session_id, &chat_msgs)
                {
                    tracing::warn!("Failed to rewrite session after compact: {e}");
                }
            }

            engine.chat_history = messages;
            drop(engine);

            let _ = state.events_tx.send(ServerEvent::StateUpdated);

            match result {
                Some(summary) => Json(serde_json::json!({
                    "compacted": true,
                    "summary": summary,
                    "referenced_files": referenced_files,
                }))
                .into_response(),
                None => Json(serde_json::json!({
                    "compacted": false,
                    "summary": "Nothing to compact — context is too small.",
                }))
                .into_response(),
            }
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// Set the per-session auto-compaction threshold and/or focus hint. Stateless
/// on the engine side — skills persist their own runtime config and replay
/// this call on iframe load (same pattern as runtime permission grants).
///
/// Both fields are independently optional:
/// - `threshold: Some(f)` overrides the default 0.95 trigger fraction.
/// - `threshold: None` keeps whatever override is currently active (no-op for that field).
/// - `focus: Some(s)` sets the per-session summarization focus hint.
/// - `focus: None` keeps whatever focus is currently active.
///
/// To clear an override, send an empty string for `focus` or use a future
/// dedicated clear endpoint — for now skills can just live without sending
/// it again, since runtime-only state resets when the engine is reloaded.
pub(crate) async fn compact_config_api(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<CompactConfigRequest>,
) -> impl IntoResponse {
    let session_id = req
        .session_id
        .clone()
        .unwrap_or_else(|| "default".to_string());
    let agent_id = req.agent_id.clone().unwrap_or_else(|| "ling".to_string());
    let root = match PathBuf::from(&req.project_root).canonicalize() {
        Ok(r) => r,
        Err(e) => return (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    };

    match state
        .manager
        .get_or_create_session_agent(&session_id, &root, &agent_id)
        .await
    {
        Ok(agent_mutex) => {
            let mut engine = agent_mutex.lock().await;
            if let Some(t) = req.threshold {
                engine.compact_threshold = Some(t.clamp(0.1, 0.99));
            }
            if let Some(f) = req.focus {
                engine.compact_focus = if f.is_empty() { None } else { Some(f) };
            }
            let persisted_threshold = engine.compact_threshold;
            let persisted_focus = engine.compact_focus.clone();
            drop(engine);

            // Persist to session.yaml so the config survives engine restart.
            // Best-effort: log on failure but still return the in-memory state
            // since the live engine already has it.
            if let Err(e) = state.manager.global_sessions.set_compact_config(
                &session_id,
                persisted_threshold,
                persisted_focus.clone(),
            ) {
                tracing::warn!("compact_config: persist to session.yaml failed: {e}");
            }

            Json(serde_json::json!({
                "ok": true,
                "threshold": persisted_threshold,
                "focus": persisted_focus,
            }))
            .into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// Extract file paths referenced in message content (e.g. from Read/Edit/Write tool calls).
fn extract_file_references(content: &str) -> Vec<String> {
    let mut files = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed
            .strip_prefix("Reading file ")
            .or_else(|| trimmed.strip_prefix("Editing file "))
            .or_else(|| trimmed.strip_prefix("Writing file "))
            .or_else(|| trimmed.strip_prefix("Read "))
            .or_else(|| trimmed.strip_prefix("Edit "))
            .or_else(|| trimmed.strip_prefix("Write "))
        {
            let path = rest.split_whitespace().next().unwrap_or("").trim_matches('`');
            if !path.is_empty() {
                files.push(path.to_string());
            }
        }
        // Match file_path patterns from tool JSON.
        if let Some(start) = trimmed.find("file_path") {
            if let Some(colon) = trimmed[start..].find(':') {
                let after = trimmed[start + colon + 1..]
                    .trim()
                    .trim_matches('"')
                    .trim_matches(',');
                if !after.is_empty() && (after.contains('/') || after.contains('.')) {
                    files.push(after.to_string());
                }
            }
        }
    }
    files
}

pub(crate) async fn ask_user_response_handler(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<AskUserResponseRequest>,
) -> impl IntoResponse {
    let sender = {
        let mut pending = state.pending_ask_user.lock().await;
        pending.remove(&req.question_id)
    };

    match sender {
        Some(entry) => {
            let session_id = entry.session_id.clone();
            if entry.sender.send(req.answers).is_ok() {
                // Broadcast so all clients (including remote) dismiss the widget.
                let _ = state.events_tx.send(ServerEvent::WidgetResolved {
                    widget_id: req.question_id,
                    session_id,
                });
                Json(serde_json::json!({ "status": "ok" })).into_response()
            } else {
                (
                    StatusCode::GONE,
                    Json(serde_json::json!({ "error": "Question already expired" })),
                )
                    .into_response()
            }
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Unknown question_id" })),
        )
            .into_response(),
    }
}

/// Return any pending AskUser questions so the UI can restore the widget
/// after navigating away and back, or after a page refresh.
pub(crate) async fn pending_ask_user_handler(
    State(state): State<Arc<ServerState>>,
) -> impl IntoResponse {
    let pending = state.pending_ask_user.lock().await;
    let items: Vec<serde_json::Value> = pending
        .iter()
        .map(|(qid, entry)| {
            serde_json::json!({
                "question_id": qid,
                "agent_id": entry.agent_id,
                "questions": entry.questions,
                "session_id": entry.session_id,
            })
        })
        .collect();
    Json(serde_json::json!(items))
}
