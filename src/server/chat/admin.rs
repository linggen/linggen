use crate::server::{ServerEvent, ServerState};
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use std::path::PathBuf;
use std::sync::Arc;

use super::types::{
    AskUserResponseRequest, ClearChatRequest, CompactChatRequest, CompactConfigRequest,
    SystemPromptQuery,
};
use crate::engine::ActivationMode;

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
    match state
        .manager
        .global_sessions
        .clear_chat_history(&session_id)
    {
        Ok(removed) => {
            // Clear in-memory chat history for this session's engine.
            // Clone the engine out and drop the map guard before waiting on
            // it: the engine lock is held for a whole turn, and holding the
            // map meanwhile would stall every other session's lookup.
            for engine_mutex in state.manager.session_engines_of(&session_id).await {
                let mut engine = engine_mutex.lock().await;
                engine.chat_history.clear();
                engine.observations.clear();
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
    let session_meta = state
        .manager
        .global_sessions
        .get_session_meta(sid)
        .ok()
        .flatten();
    // An existing session's own cwd first: where its turns stand.
    let mut candidates: Vec<String> = Vec::new();
    if let Some(ref m) = session_meta {
        if let Some(cwd) = m.cwd.as_deref().filter(|s| !s.is_empty()) {
            candidates.push(cwd.to_string());
        }
        if let Some(proj) = m.project.as_deref().filter(|s| !s.is_empty()) {
            candidates.push(proj.to_string());
        }
    }
    let q_root = query.project_root.trim();
    if !q_root.is_empty() {
        candidates.push(q_root.to_string());
    }
    candidates.push("/".to_string());
    let root = candidates
        .iter()
        .find_map(|p| PathBuf::from(p).canonicalize().ok())
        .unwrap_or_else(|| PathBuf::from("/"));

    // `agent_id=` selects the member whose prompt to preview. None named
    // (or a frontend that serialized "no agent"): the session's default
    // responder, the one a message naming nobody reaches.
    let members = match &session_meta {
        Some(meta) => {
            let skill = match meta.skill.as_deref() {
                Some(name) => state.manager.skills.get_skill(name).await,
                None => None,
            };
            super::members::effective(meta, skill.as_ref())
        }
        None => super::members::default_members(None),
    };
    let default = super::members::default_responder(&members);
    let agent_id = match query.agent_id.trim() {
        "" | "undefined" | "null" => default.as_str(),
        other => other,
    };

    // Build the prompt on a throwaway engine instead of locking the live one.
    // The live engine holds its lock for the duration of an agent turn —
    // long enough for the control-channel RPC (30s) to time out and leave the
    // Copy-System-Prompt button silently unusable. A fresh engine yields the
    // same prompt without touching live state.
    let not_found = || {
        (
            StatusCode::NOT_FOUND,
            format!("Agent '{}' not found", agent_id),
        )
            .into_response()
    };
    let Ok(mut engine) = state.manager.spawn_delegation_engine(&root, agent_id).await else {
        return not_found();
    };
    // A person's turn: the owner policy, as the chat path applies it.
    crate::engine::session_policy::SessionPolicy::owner().apply(&mut engine);
    // Seated as the member it is, on the model it runs on there.
    seat_for_export(&state, &mut engine, &root, &members, agent_id).await;

    // Apply session-bound skill or mission so the exported prompt matches what
    // the model actually sees during a chat turn. Without this, the export
    // shows a "cold engine" view missing SKILL.md / mission body.
    if let Some(meta) = session_meta {
        engine.withheld_tools.extend(meta.withheld_tools.clone());
        // The same creator rule a real turn follows: a skill's or a
        // mission's session gets no core block and no memory protocol.
        if super::handler::turn_creator(meta.mission_id.as_deref(), None, meta.skill.as_deref())
            != "user"
        {
            engine.prompt_profile.include_memory = false;
        }
        if let Some(ref skill_name) = meta.skill {
            if let Some(skill) = state.manager.skills.reload_one(skill_name).await {
                engine.activate_skill(skill, ActivationMode::Export).await;
            }
        }
        if let Some(ref mission_id) = meta.mission_id {
            let mission = state
                .missions
                .reload_one(mission_id)
                .or_else(|| state.missions.get_mission(mission_id).ok().flatten());
            if let Some(mission) = mission {
                // The same entry the scheduler's dispatch uses, so the export
                // shows the prompt and tools the real run has.
                crate::extensions::missions::enter::enter_mission(
                    &mut engine,
                    &mission,
                    &state.missions,
                    &state.manager.skills,
                )
                .await;
            }
        }
    }

    export_prompt(engine)
}

/// Seat the export's engine as its turn is seated (`handler::seat_member`):
/// another member at the lead's table uses the lead's tools; the companion
/// gets her Pet tuning; each runs on the model it has in the session.
async fn seat_for_export(
    state: &Arc<ServerState>,
    engine: &mut crate::engine::AgentEngine,
    root: &std::path::Path,
    members: &[crate::state_fs::sessions::SessionMember],
    agent_id: &str,
) {
    let lead = super::members::lead(members);
    if lead != agent_id {
        engine.session_tools = state
            .manager
            .agents
            .find(root, &lead)
            .await
            .ok()
            .flatten()
            .map(|spec| spec.spec.tools);
        engine.session_lead = Some(lead);
    }
    if super::members::is_companion(agent_id) {
        let pet = state.manager.get_config_snapshot().await.pet;
        crate::server::resident::tune_companion(engine, &pet);
    }
    let member = members
        .iter()
        .find(|m| m.id == agent_id)
        .cloned()
        .unwrap_or_else(|| crate::state_fs::sessions::SessionMember::new(agent_id));
    if let Some(model) = super::members::model_of(&state.manager, root, &member).await {
        engine.model_id = model;
    }
}

/// The prompt and tools `engine` would send, as the export shows them.
fn export_prompt(mut engine: crate::engine::AgentEngine) -> axum::response::Response {
    // Preview, not a turn: this export renders the prompt for a human to read,
    // and rendering the doorbell is what marks it read. Without this, clicking
    // "Copy System Prompt" makes the next real turn say "nothing new since you
    // last looked" about things the agent was never told.
    let (messages, allowed_tools, _) = engine.prepare_loop_messages(
        "(export)",
        true,
        crate::engine::prompt::PromptPurpose::Preview,
    );
    let system_prompt = messages
        .first()
        .map(|m| m.content.clone())
        .unwrap_or_default();
    // Tool schemas are delivered to the model via the native function-calling
    // `tools` API parameter — not embedded in the system prompt text. Expose
    // them alongside so the debug export shows the full model-facing surface.
    //
    // Provider-aware: route through the same `wire_tool_def` each provider
    // uses on the live request path. Single source of truth — the export
    // shape matches the wire shape exactly. Falls back to the canonical
    // OpenAI-nested form if the model_id isn't registered (rare).
    let canonical_tools = engine.offered_tool_definitions(allowed_tools.as_ref());
    let provider = engine
        .model_manager
        .provider_kind(&engine.model_id)
        .unwrap_or("ollama");
    let tools: Vec<serde_json::Value> = canonical_tools
        .iter()
        .filter_map(|t| match provider {
            "ollama" => crate::provider::ollama::wire_tool_def(t),
            "anthropic" => crate::provider::anthropic::wire_tool_def(t),
            // Every other provider speaks the OpenAI-compatible API.
            _ => crate::provider::openai::wire_tool_def(t),
        })
        .collect();
    Json(serde_json::json!({
        "system_prompt": system_prompt,
        "tools": tools,
        "provider": provider,
        "model_id": engine.model_id,
    }))
    .into_response()
}

/// `/compact`: the session's one summary, now (`compact_rows`). Every
/// member's cached thread starts over from it on its next turn.
pub(crate) async fn compact_chat_api(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<CompactChatRequest>,
) -> impl IntoResponse {
    let session_id = req
        .session_id
        .clone()
        .unwrap_or_else(|| "default".to_string());
    let root = match PathBuf::from(&req.project_root).canonicalize() {
        Ok(r) => r,
        Err(e) => return (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    };
    let _turn = state.manager.session_turn(&session_id).lock_owned().await;
    let members = super::members::of_session(&state.manager, &session_id).await;
    let Some(models) = super::compact_rows::session_models(&state.manager, &root, &members).await
    else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "no model to summarize with",
        )
            .into_response();
    };
    let focus = req.focus.clone().filter(|f| !f.is_empty()).or_else(|| {
        state
            .manager
            .global_sessions
            .get_session_meta(&session_id)
            .ok()
            .flatten()
            .and_then(|m| m.compact_focus)
    });
    let result = super::compact_rows::compact_session(
        &state.manager,
        &session_id,
        &models,
        focus.as_deref(),
    )
    .await;
    if result.is_some() {
        for engine in state.manager.session_engines_of(&session_id).await {
            let mut engine = engine.lock().await;
            engine.chat_history.clear();
            engine.observations.clear();
        }
    }
    let _ = state.events_tx.send(ServerEvent::StateUpdated);

    match result {
        Some(summary) => {
            let referenced_files: Vec<String> = extract_file_references(&summary)
                .into_iter()
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect();
            Json(serde_json::json!({
                "compacted": true,
                "summary": summary,
                "referenced_files": referenced_files,
            }))
            .into_response()
        }
        None => Json(serde_json::json!({
            "compacted": false,
            "summary": "Nothing to compact — context is too small.",
        }))
        .into_response(),
    }
}

/// Set the per-session auto-compaction threshold and/or focus hint. Applied
/// to the live engine when one exists and persisted to session.yaml either
/// way, so ordering against the chat path's engine creation doesn't matter —
/// skills replay this call on iframe load (same pattern as runtime grants).
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

    // Never create an engine here: engine creation belongs to the chat path,
    // which knows the session's real project root. Creating one from this
    // out-of-band call could pin the session to the wrong workspace. Update
    // the live engine when one exists; otherwise merge against the persisted
    // session.yaml values — engine creation re-applies them (agent/mod.rs).
    let engines = state.manager.session_engines_of(&session_id).await;
    let meta = state
        .manager
        .global_sessions
        .get_session_meta(&session_id)
        .ok()
        .flatten();
    let (mut threshold, mut focus) = (
        meta.as_ref().and_then(|m| m.compact_threshold),
        meta.and_then(|m| m.compact_focus),
    );
    if let Some(t) = req.threshold {
        threshold = Some(t.clamp(0.1, 0.99));
    }
    if let Some(f) = req.focus {
        focus = if f.is_empty() { None } else { Some(f) };
    }

    for arc in engines {
        let mut engine = arc.lock().await;
        engine.compact_threshold = threshold;
        engine.compact_focus = focus.clone();
    }

    // Persist to session.yaml so the config survives engine restart and is
    // picked up when the chat path creates the engine. Best-effort: log on
    // failure but still return the merged state.
    if let Err(e) =
        state
            .manager
            .global_sessions
            .set_compact_config(&session_id, threshold, focus.clone())
    {
        tracing::warn!("compact_config: persist to session.yaml failed: {e}");
    }

    Json(serde_json::json!({
        "ok": true,
        "threshold": threshold,
        "focus": focus,
    }))
    .into_response()
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
            let path = rest
                .split_whitespace()
                .next()
                .unwrap_or("")
                .trim_matches('`');
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
