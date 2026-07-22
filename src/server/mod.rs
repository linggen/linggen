mod api;
pub(crate) mod bridge;
mod chat;
mod mcp;
mod mcp_agent;
mod events;
pub(crate) mod rtc;
mod state;
mod yinyue_watch;

pub use events::{AgentStatusKind, NotificationPayload, QueuedChatItem, ServerEvent, UiEvent};
pub use state::ServerState;
pub(crate) use state::ActiveStatusRecord;

use events::*;

use crate::engine::agent::AgentManager;
use axum::{
    extract::State,
    http::Uri,
    response::{IntoResponse, Response},
    routing::{delete, get, patch, post, put},
    Router,
};
use rust_embed::RustEmbed;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::broadcast;
use tokio::sync::mpsc;
use tokio::sync::Mutex;
use tracing::info;

use api::agents::{
    cancel_agent_run, cancel_tool_execution, clear_queued_messages,
    delete_agent_file_api, get_agent_file_api,
    list_agent_files_api, list_agent_runs_api, list_agents_api,
    reload_agents, run_agent, set_task,
    upsert_agent_file_api,
};
use api::account::{
    get_account, get_account_callback, post_account_checkout, post_account_login,
    post_account_logout,
};
use api::config::{
    codex_auth_logout, get_claude_auth_status, get_codex_auth_status, get_config_api,
    get_credentials_api, get_models_health, start_codex_auth_login, update_config_api,
    update_credentials_api,
};
use api::marketplace::{
    builtin_skills_install, builtin_skills_list, clawhub_scan, community_search,
    marketplace_install, marketplace_move_to_global, marketplace_uninstall,
};
use api::missions::{
    create_mission, delete_mission, get_mission_file,
    get_mission_session_state, list_mission_runs, list_missions, trigger_mission,
    update_mission, upsert_mission_file,
};
use api::permissions::{get_session_permission, update_session_permission};
use api::rooms::{
    connect_proxy_room_api, disconnect_proxy_room_api, get_room_config, proxy_rooms,
    proxy_status_api, token_usage_api, update_room_config,
};
use api::sessions::{
    create_session, delete_unified_session, get_skill_session_state, list_all_sessions,
    list_sessions, list_skill_sessions, remove_session_api, remove_skill_session_api,
    rename_session_api, resolve_session_api,
};
use api::skills::{
    delete_skill_file_api, get_skill_file_api, list_skill_files_api, list_skills,
    reload_skills, upsert_skill_file_api,
};
use api::status::{get_status_api, list_models_api};
use api::storage::{
    storage_delete_file, storage_read_file, storage_roots, storage_tree, storage_write_file,
};
use api::workspace::{
    get_agent_tree, get_workspace_state, list_files, read_file_api, run_bash_api, search_files,
};
use chat::{
    approve_plan_handler, ask_user_response_handler, chat_handler, clear_chat_history_api,
    compact_chat_api, compact_config_api, edit_plan_handler, get_system_prompt_api,
    pending_ask_user_handler, reject_plan_handler,
};
#[derive(RustEmbed)]
#[folder = "ui/dist/"]
struct Assets;







// ---------------------------------------------------------------------------
// UI event kind/phase constants
// ---------------------------------------------------------------------------

const UI_KIND_MESSAGE: &str = "message";
const UI_KIND_ACTIVITY: &str = "activity";
const UI_KIND_QUEUE: &str = "queue";
const UI_KIND_RUN: &str = "run";
const UI_KIND_TOKEN: &str = "token";
const UI_KIND_TEXT_SEGMENT: &str = "text_segment";
const UI_KIND_CONTENT_BLOCK: &str = "content_block";
const UI_KIND_TURN_COMPLETE: &str = "turn_complete";
const UI_KIND_DEVICE_TOPIC: &str = "device_topic";

const UI_PHASE_SYNC: &str = "sync";
const UI_PHASE_OUTCOME: &str = "outcome";
const UI_PHASE_CONTEXT_USAGE: &str = "context_usage";
const UI_PHASE_SUBAGENT_SPAWNED: &str = "subagent_spawned";
const UI_PHASE_SUBAGENT_RESULT: &str = "subagent_result";
const UI_PHASE_PLAN_UPDATE: &str = "plan_update";
pub(crate) const UI_PHASE_DOING: &str = "doing";
pub(crate) const UI_PHASE_DONE: &str = "done";
const UI_PHASE_RESYNC: &str = "resync";

fn default_status_text(status: AgentStatusKind) -> String {
    match status {
        AgentStatusKind::ModelLoading => "Model loading...".to_string(),
        AgentStatusKind::Thinking => "Thinking...".to_string(),
        AgentStatusKind::CallingTool => "Calling tool...".to_string(),
        AgentStatusKind::Working => "Working...".to_string(),
        AgentStatusKind::Idle => "Idle".to_string(),
    }
}

pub(crate) fn map_server_event_to_ui_message(event: ServerEvent, seq: u64) -> Option<UiEvent> {
    let ts_ms = crate::util::now_ts_ms();
    match event {
        ServerEvent::PetSpeak { text, emotion } => Some(UiEvent {
            id: format!("pet-speak-{seq}"),
            seq,
            rev: seq,
            ts_ms,
            kind: "pet_speak".to_string(),
            phase: None,
            text: Some(text.clone()),
            agent_id: Some("yinyue".to_string()),
            session_id: None, // global → control channel of every surface
            project_root: None,
            data: Some(json!({ "text": text, "emotion": emotion })),
        }),
        ServerEvent::PetExpress { emotion, action } => Some(UiEvent {
            id: format!("pet-express-{seq}"),
            seq,
            rev: seq,
            ts_ms,
            kind: "pet_express".to_string(),
            phase: None,
            text: None,
            agent_id: Some("yinyue".to_string()),
            session_id: None, // global → every surface
            project_root: None,
            data: Some(json!({ "emotion": emotion, "action": action })),
        }),
        // Internal: consumed by Yinyue's watch loop, never a UI banner.
        ServerEvent::AgentChat { .. } => None,
        // Per-peer — handled directly in forward.rs (the present flag depends on
        // the receiving peer's id), so it never goes through this shared mapping.
        ServerEvent::YinyuePresenterChanged => None,
        ServerEvent::Message { from, to, content, session_id, run_id, parent_agent_id } => {
            let cleaned = crate::engine::tool_render::sanitize_message_for_ui(&from, &content)?;
            Some(UiEvent {
                id: format!("msg-{seq}"),
                seq,
                rev: seq,
                ts_ms,
                kind: UI_KIND_MESSAGE.to_string(),
                phase: None,
                text: Some(cleaned),
                agent_id: Some(from.clone()),
                session_id,
                project_root: None,
                data: Some(json!({
                    "from": from,
                    "to": to,
                    "role": if from == "user" { "user" } else { "assistant" },
                    // Subagent routing keys — present only when emitted
                    // from a delegated engine. handleMessage in the UI
                    // uses these to route the bubble into SubagentPane
                    // instead of the parent's main chat (the gap that
                    // was leaking "ENCODED encoded=0" into chat).
                    "run_id": run_id,
                    "parent_agent_id": parent_agent_id,
                })),
            })
        }
        ServerEvent::AgentStatus {
            agent_id,
            status,
            detail,
            status_id,
            lifecycle,
            parent_agent_id,
            session_id,
            run_id,
            parent_run_id,
        } => {
            if status.eq_ignore_ascii_case("idle") && lifecycle.is_none() {
                // Still emit the idle event so the UI can transition agent status.
                return Some(UiEvent {
                    id: format!("act-{seq}"),
                    seq,
                    rev: seq,
                    ts_ms,
                    kind: UI_KIND_ACTIVITY.to_string(),
                    phase: Some(UI_PHASE_DONE.to_string()),
                    text: None,
                    agent_id: Some(agent_id),
                    session_id,
                    project_root: None,
                    data: Some(json!({
                        "status": "idle",
                        "parent_id": parent_agent_id,
                        "run_id": run_id,
                        "parent_run_id": parent_run_id,
                    })),
                });
            }
            let phase = lifecycle.or_else(|| {
                if status.eq_ignore_ascii_case("idle") {
                    Some(UI_PHASE_DONE.to_string())
                } else {
                    Some(UI_PHASE_DOING.to_string())
                }
            });
            let text = detail
                .and_then(|v| {
                    let t = v.trim().to_string();
                    if t.is_empty() {
                        None
                    } else {
                        Some(t)
                    }
                })
                .unwrap_or_else(|| default_status_text(AgentStatusKind::from_str_loose(&status)));
            Some(UiEvent {
                id: status_id.unwrap_or_else(|| format!("activity-{agent_id}-{status}-{seq}")),
                seq,
                rev: seq,
                ts_ms,
                kind: UI_KIND_ACTIVITY.to_string(),
                phase,
                text: Some(text),
                agent_id: Some(agent_id),
                session_id,
                project_root: None,
                data: Some(json!({
                    "status": status,
                    "parent_id": parent_agent_id,
                    "run_id": run_id,
                    "parent_run_id": parent_run_id,
                })),
            })
        }
        ServerEvent::QueueUpdated {
            project_root,
            session_id,
            agent_id,
            items,
        } => Some(UiEvent {
            id: format!("queue-{project_root}|{session_id}|{agent_id}"),
            seq,
            rev: seq,
            ts_ms,
            kind: UI_KIND_QUEUE.to_string(),
            phase: None,
            text: Some(format!(
                "Queued {} message{}",
                items.len(),
                if items.len() == 1 { "" } else { "s" }
            )),
            agent_id: Some(agent_id),
            session_id: Some(session_id),
            project_root: Some(project_root),
            data: Some(json!({ "items": items })),
        }),
        ServerEvent::StateUpdated => Some(UiEvent {
            id: format!("run-sync-{seq}"),
            seq,
            rev: seq,
            ts_ms,
            kind: UI_KIND_RUN.to_string(),
            phase: Some(UI_PHASE_SYNC.to_string()),
            text: Some("State updated".to_string()),
            agent_id: None,
            session_id: Some("global".to_string()),
            project_root: None,
            data: None,
        }),
        ServerEvent::Outcome { agent_id, outcome, session_id } => Some(UiEvent {
            id: format!("run-outcome-{agent_id}-{seq}"),
            seq,
            rev: seq,
            ts_ms,
            kind: UI_KIND_RUN.to_string(),
            phase: Some(UI_PHASE_OUTCOME.to_string()),
            text: Some("Run outcome".to_string()),
            agent_id: Some(agent_id),
            session_id,
            project_root: None,
            data: Some(json!({ "outcome": outcome })),
        }),
        ServerEvent::ContextUsage {
            agent_id,
            stage,
            message_count,
            char_count,
            estimated_tokens,
            token_limit,
            actual_prompt_tokens,
            actual_completion_tokens,
            compressed,
            summary_count,
            session_id,
        } => Some(UiEvent {
            id: format!("run-context-{agent_id}"),
            seq,
            rev: seq,
            ts_ms,
            kind: UI_KIND_RUN.to_string(),
            phase: Some(UI_PHASE_CONTEXT_USAGE.to_string()),
            text: None,
            agent_id: Some(agent_id.clone()),
            session_id,
            project_root: None,
            data: Some(json!({
                "agent_id": agent_id,
                "stage": stage,
                "message_count": message_count,
                "char_count": char_count,
                "estimated_tokens": estimated_tokens,
                "token_limit": token_limit,
                "actual_prompt_tokens": actual_prompt_tokens,
                "actual_completion_tokens": actual_completion_tokens,
                "compressed": compressed,
                "summary_count": summary_count,
            })),
        }),
        ServerEvent::SubagentSpawned {
            parent_id,
            subagent_id,
            task,
            session_id,
            subagent_run_id,
            parent_run_id,
        } => Some(UiEvent {
            id: format!("run-subagent-spawned-{}-{seq}",
                subagent_run_id.as_deref().unwrap_or(&subagent_id)),
            seq,
            rev: seq,
            ts_ms,
            kind: UI_KIND_RUN.to_string(),
            phase: Some(UI_PHASE_SUBAGENT_SPAWNED.to_string()),
            text: Some(format!("Spawned subagent {}", subagent_id)),
            agent_id: Some(parent_id),
            session_id,
            project_root: None,
            data: Some(json!({
                "subagent_id": subagent_id,
                "task": task,
                "subagent_run_id": subagent_run_id,
                "parent_run_id": parent_run_id,
            })),
        }),
        ServerEvent::SubagentResult {
            parent_id,
            subagent_id,
            outcome,
            session_id,
            subagent_run_id,
            parent_run_id,
        } => Some(UiEvent {
            id: format!("run-subagent-result-{}-{seq}",
                subagent_run_id.as_deref().unwrap_or(&subagent_id)),
            seq,
            rev: seq,
            ts_ms,
            kind: UI_KIND_RUN.to_string(),
            phase: Some(UI_PHASE_SUBAGENT_RESULT.to_string()),
            text: Some(format!("Subagent {} returned", subagent_id)),
            agent_id: Some(parent_id),
            session_id,
            project_root: None,
            data: Some(json!({
                "subagent_id": subagent_id,
                "outcome": outcome,
                "subagent_run_id": subagent_run_id,
                "parent_run_id": parent_run_id,
            })),
        }),
        ServerEvent::Token {
            agent_id,
            token,
            done,
            thinking,
            session_id,
        } => Some(UiEvent {
            id: format!("token-{agent_id}-{seq}"),
            seq,
            rev: seq,
            ts_ms,
            kind: UI_KIND_TOKEN.to_string(),
            phase: if done { Some(UI_PHASE_DONE.to_string()) } else { None },
            text: Some(token),
            agent_id: Some(agent_id),
            session_id,
            project_root: None,
            data: if thinking { Some(json!({ "thinking": true })) } else { None },
        }),
        ServerEvent::PlanUpdate { agent_id, plan, session_id } => Some(UiEvent {
            id: format!("run-plan-{agent_id}-{seq}"),
            seq,
            rev: seq,
            ts_ms,
            kind: UI_KIND_RUN.to_string(),
            phase: Some(UI_PHASE_PLAN_UPDATE.to_string()),
            text: Some("Plan updated".to_string()),
            agent_id: Some(agent_id),
            session_id,
            project_root: None,
            data: Some(json!({ "plan": plan })),
        }),
        // MissionTriggered is a lifecycle signal, not a chat activity.
        // SessionCreated + AgentStatus(Working) already convey the visible start
        // to the UI; routing this as activity caused a stray "Mission triggered"
        // line inside the session transcript.
        ServerEvent::MissionTriggered { .. } => None,
        ServerEvent::SessionCreated {
            ref session_id,
            ref title,
            ref creator,
            ref project,
            ref project_name,
            ref skill,
            ref mission_id,
        } => Some(UiEvent {
            id: format!("session-created-{session_id}-{seq}"),
            seq,
            rev: seq,
            ts_ms,
            kind: "notification".to_string(),
            phase: None,
            text: Some(format!("Session created: {title}")),
            agent_id: None,
            session_id: Some("global".to_string()),
            project_root: project.clone(),
            data: Some(json!({
                "kind": "session_created",
                "session_id": session_id,
                "title": title,
                "creator": creator,
                "project": project,
                "project_name": project_name,
                "skill": skill,
                "mission_id": mission_id,
            })),
        }),
        ServerEvent::Notification(ref payload) => match payload {
            NotificationPayload::MissionCompleted {
                mission_id,
                mission_name,
                status,
                ..
            } => Some(UiEvent {
                id: format!("notif-mission-{mission_id}-{seq}"),
                seq,
                rev: seq,
                ts_ms,
                kind: "notification".to_string(),
                phase: None,
                text: Some(format!("Mission '{}' {}", mission_name, status)),
                agent_id: None,
                session_id: Some("global".to_string()),
                project_root: None,
                data: serde_json::to_value(payload).ok(),
            }),
            // Internal signal that drives Yinyue's spoken apology — not a UI
            // banner (the failed turn already renders its own error message).
            NotificationPayload::RunFailed { .. } => None,
            // Internal trigger for Yinyue's herald watch — never a UI banner
            // (the completed turn already renders its own reply).
            NotificationPayload::RunCompleted { .. } => None,
        },
        ServerEvent::TextSegment {
            agent_id,
            text,
            parent_id,
            session_id,
        } => Some(UiEvent {
            id: format!("text-seg-{agent_id}-{seq}"),
            seq,
            rev: seq,
            ts_ms,
            kind: UI_KIND_TEXT_SEGMENT.to_string(),
            phase: None,
            text: Some(text),
            agent_id: Some(agent_id),
            session_id,
            project_root: None,
            data: Some(json!({ "parent_id": parent_id })),
        }),
        ServerEvent::AskUser {
            agent_id,
            question_id,
            questions,
            session_id,
        } => Some(UiEvent {
            id: format!("ask-user-{question_id}"),
            seq,
            rev: seq,
            ts_ms,
            kind: "ask_user".to_string(),
            phase: None,
            text: None,
            agent_id: Some(agent_id),
            session_id,
            project_root: None,
            data: Some(json!({
                "question_id": question_id,
                "questions": questions,
            })),
        }),
        ServerEvent::WidgetResolved {
            widget_id,
            session_id,
        } => Some(UiEvent {
            id: format!("resolved-{widget_id}"),
            seq,
            rev: seq,
            ts_ms,
            kind: "widget_resolved".to_string(),
            phase: None,
            text: None,
            agent_id: None,
            session_id,
            project_root: None,
            data: Some(json!({ "widget_id": widget_id })),
        }),
        ServerEvent::ModelFallback {
            agent_id,
            preferred_model,
            actual_model,
            reason,
            session_id,
        } => Some(UiEvent {
            id: format!("model-fallback-{agent_id}-{seq}"),
            seq,
            rev: seq,
            ts_ms,
            kind: "model_fallback".to_string(),
            phase: None,
            text: Some(format!(
                "Using {} model ({} unavailable: {})",
                actual_model, preferred_model, reason
            )),
            agent_id: Some(agent_id),
            session_id,
            project_root: None,
            data: Some(json!({
                "preferred_model": preferred_model,
                "actual_model": actual_model,
                "reason": reason,
            })),
        }),
        ServerEvent::ToolProgress {
            agent_id,
            tool,
            line,
            stream,
            session_id,
        } => Some(UiEvent {
            id: format!("tool-progress-{agent_id}-{seq}"),
            seq,
            rev: seq,
            ts_ms,
            kind: "tool_progress".to_string(),
            phase: None,
            text: Some(line.clone()),
            agent_id: Some(agent_id),
            session_id,
            project_root: None,
            data: Some(json!({
                "tool": tool,
                "line": line,
                "stream": stream,
            })),
        }),
        ServerEvent::Resync {
            reason,
            lagged_count,
        } => Some(UiEvent {
            id: format!("run-resync-{seq}"),
            seq,
            rev: seq,
            ts_ms,
            kind: UI_KIND_RUN.to_string(),
            phase: Some(UI_PHASE_RESYNC.to_string()),
            text: Some("Resync required".to_string()),
            agent_id: None,
            session_id: Some("global".to_string()),
            project_root: None,
            data: Some(json!({
                "reason": reason,
                "lagged_count": lagged_count,
            })),
        }),
        ServerEvent::AppLaunched {
            skill,
            launcher,
            url,
            title,
            width,
            height,
            session_id,
        } => Some(UiEvent {
            id: format!("app-launched-{skill}-{seq}"),
            seq,
            rev: seq,
            ts_ms,
            kind: "app_launched".to_string(),
            phase: None,
            text: Some(format!("Launched app: {}", title)),
            agent_id: None,
            session_id: Some(session_id.unwrap_or_else(|| "global".to_string())),
            project_root: None,
            data: Some(json!({
                "skill": skill,
                "launcher": launcher,
                "url": url,
                "title": title,
                "width": width,
                "height": height,
            })),
        }),
        ServerEvent::ContentBlockStart {
            agent_id,
            block_id,
            block_type,
            tool,
            args,
            parent_id,
            session_id,
            run_id,
            parent_run_id,
        } => {
            let phase = if block_type == "tool_use" { "start" } else { "start" };
            Some(UiEvent {
                id: format!("cb-start-{block_id}"),
                seq,
                rev: seq,
                ts_ms,
                kind: UI_KIND_CONTENT_BLOCK.to_string(),
                phase: Some(phase.to_string()),
                text: None,
                agent_id: Some(agent_id),
                session_id,
                project_root: None,
                data: Some(json!({
                    "block_id": block_id,
                    "block_type": block_type,
                    "tool": tool,
                    "args": args,
                    "parent_id": parent_id,
                    "run_id": run_id,
                    "parent_run_id": parent_run_id,
                })),
            })
        }
        ServerEvent::ContentBlockUpdate {
            agent_id,
            block_id,
            status,
            summary,
            is_error,
            parent_id,
            extra,
            session_id,
            run_id,
            parent_run_id,
        } => {
            let mut data_obj = json!({
                "block_id": block_id,
                "status": status,
                "summary": summary,
                "is_error": is_error,
                "parent_id": parent_id,
                "run_id": run_id,
                "parent_run_id": parent_run_id,
            });
            // Merge extra fields into the data object so the frontend receives them flat.
            if let Some(extra_val) = &extra {
                if let (Some(base), Some(ext)) = (data_obj.as_object_mut(), extra_val.as_object()) {
                    for (k, v) in ext {
                        base.insert(k.clone(), v.clone());
                    }
                }
            }
            Some(UiEvent {
                id: format!("cb-update-{block_id}-{seq}"),
                seq,
                rev: seq,
                ts_ms,
                kind: UI_KIND_CONTENT_BLOCK.to_string(),
                phase: Some("update".to_string()),
                text: summary.clone(),
                agent_id: Some(agent_id),
                session_id,
                project_root: None,
                data: Some(data_obj),
            })
        }
        ServerEvent::TurnComplete {
            agent_id,
            duration_ms,
            context_tokens,
            parent_id,
            session_id,
            run_id,
            parent_run_id,
        } => Some(UiEvent {
            id: format!("turn-complete-{agent_id}-{seq}"),
            seq,
            rev: seq,
            ts_ms,
            kind: UI_KIND_TURN_COMPLETE.to_string(),
            phase: None,
            text: None,
            agent_id: Some(agent_id),
            session_id,
            project_root: None,
            data: Some(json!({
                "duration_ms": duration_ms,
                "context_tokens": context_tokens,
                "parent_id": parent_id,
                "run_id": run_id,
                "parent_run_id": parent_run_id,
            })),
        }),
        ServerEvent::WorkingFolderChanged {
            session_id,
            cwd,
            project,
            project_name,
        } => Some(UiEvent {
            id: format!("wf-{seq}"),
            seq,
            rev: seq,
            ts_ms,
            kind: "working_folder".to_string(),
            phase: None,
            text: None,
            agent_id: None,
            session_id: Some(session_id),
            project_root: None,
            data: Some(json!({
                "cwd": cwd,
                "project": project,
                "project_name": project_name,
            })),
        }),
        ServerEvent::RoomChat {
            sender_id,
            sender_name,
            avatar_url,
            text,
        } => Some(UiEvent {
            id: format!("room-chat-{seq}"),
            seq,
            rev: seq,
            ts_ms,
            kind: "room_chat".to_string(),
            phase: None,
            text: Some(text.clone()),
            agent_id: None,
            session_id: Some("global".to_string()),
            project_root: None,
            data: Some(json!({
                "sender_id": sender_id,
                "sender_name": sender_name,
                "avatar_url": avatar_url,
                "text": text,
            })),
        }),
        // RoomDisabled is handled directly in peer.rs — no UI event needed.
        ServerEvent::RoomDisabled => None,
        ServerEvent::DeviceTopic {
            topic,
            op,
            payload,
            from_device,
        } => Some(UiEvent {
            id: format!("device-topic-{seq}"),
            seq,
            rev: seq,
            ts_ms,
            kind: UI_KIND_DEVICE_TOPIC.to_string(),
            phase: None,
            text: None,
            agent_id: None,
            // User-level: reaches every surface over the control channel.
            session_id: Some("global".to_string()),
            project_root: None,
            data: Some(json!({
                "topic": topic,
                "op": op,
                "payload": payload,
                "from_device": from_device,
            })),
        }),
    }
}


struct ServerHandle {
    task: tokio::task::JoinHandle<anyhow::Result<()>>,
    port: u16,
}

async fn prepare_server(
    manager: Arc<AgentManager>,
    skills: Arc<crate::extensions::skills::SkillLoader>,
    host: &str,
    port: u16,
    dev_mode: bool,
    idle_shutdown_secs: Option<u64>,
    mut agent_events_rx: mpsc::UnboundedReceiver<(crate::engine::agent::AgentEvent, Option<String>)>,
) -> anyhow::Result<ServerHandle> {
    info!("linggen server starting on {}:{}...", host, port);

    // Events can be bursty (tool/status steps). Use a larger buffer to reduce lag drops.
    let (events_tx, _) = broadcast::channel(4096);

    let prompt_store = Arc::new(crate::prompts::PromptStore::load(
        Some(&crate::prompts::PromptStore::default_override_dir()),
    ));

    // Pet/voice settings drive the TTS engine choice + whether to pre-warm it.
    let pet_cfg = manager.get_config_snapshot().await.pet;

    let state = Arc::new(ServerState {
        manager,
        dev_mode,
        port,
        bound_host: host.to_string(),
        active_peer_count: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        last_activity: Arc::new(AtomicU64::new(unix_secs_now())),
        events_tx,
        skills,
        prompt_store,
        queued_chats: Arc::new(Mutex::new(HashMap::new())),
        interrupt_tx: Arc::new(Mutex::new(HashMap::new())),
        pending_ask_user: Arc::new(Mutex::new(HashMap::new())),
        status_seq: AtomicU64::new(1),
        active_statuses: Arc::new(Mutex::new(HashMap::new())),
        queue_seq: AtomicU64::new(1),
        event_seq: AtomicU64::new(1),
        session_tokens: Arc::new(Mutex::new(HashMap::new())),
        whip_token: uuid::Uuid::new_v4().to_string(),
        user_bash_cwd: Arc::new(Mutex::new(HashMap::new())),
        proxy_connections: Arc::new(rtc::proxy_room::ProxyRoomConnections::new()),
        token_usage: Arc::new(tokio::sync::Mutex::new(rtc::token_store::TokenUsageStore::load())),
        codex_login_task: Arc::new(tokio::sync::Mutex::new(None)),
        tts: api::tts::default_provider(),
        bridge: Arc::new(bridge::BridgeHub::new()),
        current_view: Arc::new(std::sync::Mutex::new(None)),
        yinyue_presenters: Arc::new(std::sync::Mutex::new(Vec::new())),
        next_peer_id: Arc::new(std::sync::atomic::AtomicU64::new(1)),
    });

    // Flush token usage to disk every 30 seconds.
    {
        let usage = state.token_usage.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                usage.lock().await.flush();
            }
        });
    }

    // Push DJ library changes to paired devices instead of making them poll.
    api::dj::spawn_library_watcher(state.clone());

    // Pre-warm ling-mem when any installed skill uses scoped memory
    // (`memory-context`). Those apps auto-recall every turn, so get the
    // possibly-first-time (~100 MB) install + daemon start off the hot path.
    // No-op for daemons with no memory-using skill; on failure the per-call
    // autostart still covers it.
    {
        let skills = state.skills.clone();
        tokio::spawn(async move {
            let uses_memory = skills.list_skills().await.iter().any(|s| {
                s.memory_context
                    .as_deref()
                    .map(|c| !c.trim().is_empty())
                    .unwrap_or(false)
            });
            if uses_memory {
                match crate::engine::tools::memory_tool::autostart().await {
                    Ok(()) => info!("ling-mem pre-warmed (a skill uses scoped memory)"),
                    Err(e) => {
                        info!("ling-mem pre-warm deferred to first use: {e}")
                    }
                }
            }
        });
    }

    // Pre-warm the voice model (Yinyue's TTS) off the hot path — but ONLY when
    // the pet is enabled AND the chosen engine is light enough to prefetch
    // (Kokoro). A heavy engine (Qwen3, ~2 GB) stays lazy: its model downloads on
    // her first actual speak, so a fresh install or a disabled pet never pulls
    // it. The lazy `OnceCell` in the provider handles the on-demand load.
    if pet_cfg.enabled && state.tts.prewarm_on_boot() {
        let tts = state.tts.clone();
        tokio::spawn(async move { tts.prewarm().await });
    }

    // Idle-shutdown watcher: when --idle-shutdown-secs is set, exit the
    // process after that many seconds with zero connected WebRTC peers.
    // Used by bundled apps so the daemon doesn't outlive its last client.
    if let Some(timeout) = idle_shutdown_secs.filter(|t| *t > 0) {
        use std::sync::atomic::Ordering;
        let peers = state.active_peer_count.clone();
        let last_activity = state.last_activity.clone();
        info!("idle-shutdown enabled: exit after {timeout}s with no peers and no activity");
        tokio::spawn(async move {
            let check_interval = std::time::Duration::from_secs(15);
            loop {
                tokio::time::sleep(check_interval).await;
                let now = unix_secs_now();
                // A connected peer is itself a sign of life — keep the activity
                // clock fresh so dropping a peer never trips an instant exit on
                // a stale timestamp.
                if peers.load(Ordering::Relaxed) > 0 {
                    last_activity.store(now, Ordering::Relaxed);
                    continue;
                }
                // No peers: exit only after `timeout` of no activity at all.
                // Bundled-app shells ping /api/health while their window is open
                // (see health_handler), so the daemon survives a window that is
                // open but transiently disconnected.
                if now.saturating_sub(last_activity.load(Ordering::Relaxed)) >= timeout {
                    info!("idle-shutdown: no peers and no activity for {timeout}s, exiting");
                    std::process::exit(0);
                }
            }
        });
    }

    // Bridge internal AgentManager events to the UI (broadcast channel → WebRTC).
    {
        let state_clone = state.clone();
        tokio::spawn(async move {
            while let Some((event, session_id)) = agent_events_rx.recv().await {
                match event {
                    // Special cases that need extra logic beyond a 1:1 mapping.
                    crate::engine::agent::AgentEvent::AgentStatus {
                        agent_id, status, detail, parent_id, run_id, parent_run_id,
                    } => {
                        state_clone
                            .send_agent_status_with_ids(
                                agent_id, AgentStatusKind::from_str_loose(&status), detail,
                                parent_id, session_id, run_id, parent_run_id,
                            )
                            .await;
                    }
                    crate::engine::agent::AgentEvent::TaskUpdate { .. } => {
                        let _ = state_clone.events_tx.send(ServerEvent::StateUpdated);
                    }
                    // All other variants have a 1:1 ServerEvent equivalent.
                    other => {
                        // Intercept __cwd_changed__ progress events → WorkingFolderChanged
                        if let crate::engine::agent::AgentEvent::ToolProgress {
                            ref tool, ref line, ..
                        } = &other {
                            if tool == "__cwd_changed__" {
                                // line = cwd, stream = "project|project_name"
                                let cwd = line.clone();
                                if let crate::engine::agent::AgentEvent::ToolProgress { stream, .. } = &other {
                                    let parts: Vec<&str> = stream.splitn(2, '|').collect();
                                    let project = parts.first().filter(|s| !s.is_empty()).map(|s| s.to_string());
                                    let project_name = parts.get(1).filter(|s| !s.is_empty()).map(|s| s.to_string());
                                    if let Some(ref sid) = session_id {
                                        // Update session metadata
                                        if let Ok(Some(mut meta)) = state_clone.manager.global_sessions.get_session_meta(sid) {
                                            meta.cwd = Some(cwd.clone());
                                            meta.project = project.clone();
                                            meta.project_name = project_name.clone();
                                            let _ = state_clone.manager.global_sessions.update_session_meta(&meta);
                                        }
                                        let _ = state_clone.events_tx.send(ServerEvent::WorkingFolderChanged {
                                            session_id: sid.clone(),
                                            cwd,
                                            project,
                                            project_name,
                                        });
                                    }
                                }
                                continue; // Don't forward as ToolProgress
                            }
                        }
                        // Accumulate token usage from ContextUsage events.
                        if let crate::engine::agent::AgentEvent::ContextUsage {
                            actual_prompt_tokens: Some(prompt),
                            actual_completion_tokens: Some(completion),
                            ..
                        } = &other {
                            let sid = session_id.clone().unwrap_or_else(|| "current".to_string());
                            let mut tokens = state_clone.session_tokens.lock().await;
                            let entry = tokens.entry(sid).or_insert((0, 0));
                            entry.0 += prompt;
                            entry.1 += completion;
                        }
                        if let Some(se) = ServerEvent::from_agent_event(other, session_id) {
                            let _ = state_clone.events_tx.send(se);
                        }
                    }
                }
            }
        });
    }

    let app = Router::new()
        // Agent & model file management (admin HTTP, not proxied for consumers)
        .route("/api/agent-files", get(list_agent_files_api))
        .route("/api/agent-file", get(get_agent_file_api))
        .route("/api/agent-file", post(upsert_agent_file_api))
        .route("/api/agent-file", delete(delete_agent_file_api))
        // Models & skills GETs — used by SharingTab and SkillsTab Settings pages directly.
        // Session list / agents / agent-runs come via page_state only (no GET route).
        .route("/api/models", get(list_models_api))
        .route("/api/skills", get(list_skills))
        .route("/api/models/health", get(get_models_health))
        .route("/api/config", get(get_config_api).post(update_config_api))
        .route("/api/credentials", get(get_credentials_api).put(update_credentials_api))
        .route("/api/auth/codex/status", get(get_codex_auth_status))
        .route("/api/auth/codex/login", post(start_codex_auth_login))
        .route("/api/auth/codex/logout", post(codex_auth_logout))
        .route("/api/auth/claude/status", get(get_claude_auth_status))
        .route("/api/skills/reload", post(reload_skills))
        .route("/api/agents/reload", post(reload_agents))
        .route("/api/community-skills/search", get(community_search))
        .route("/api/marketplace/install", post(marketplace_install))
        .route("/api/marketplace/uninstall", delete(marketplace_uninstall))
        .route("/api/marketplace/move-to-global", post(marketplace_move_to_global))
        .route("/api/builtin-skills", get(builtin_skills_list))
        .route("/api/builtin-skills/install", post(builtin_skills_install))
        .route("/api/skill-files", get(list_skill_files_api))
        .route("/api/skill-file", get(get_skill_file_api))
        .route("/api/skill-file", post(upsert_skill_file_api))
        .route("/api/skill-file", delete(delete_skill_file_api))
        // Session management (create/rename/delete still via HTTP, data via page_state)
        .route("/api/sessions/all", delete(delete_unified_session))
        .route("/api/sessions", post(create_session))
        .route("/api/sessions", patch(rename_session_api))
        .route("/api/sessions", delete(remove_session_api))
        .route("/api/sessions/permission", get(get_session_permission).patch(update_session_permission))
        .route("/api/skill-sessions", get(list_skill_sessions))
        .route("/api/skill-sessions", delete(remove_skill_session_api))
        .route("/api/skill-sessions/state", get(get_skill_session_state))
        .route("/api/task", post(set_task))
        .route("/api/agent-cancel", post(cancel_agent_run))
        .route("/api/queue/clear", post(clear_queued_messages))
        // Missions
        .route("/api/missions", get(list_missions).post(create_mission))
        .route("/api/missions/sessions/state", get(get_mission_session_state))
        .route("/api/missions/{id}", put(update_mission).delete(delete_mission))
        .route("/api/missions/{id}/runs", get(list_mission_runs))
        .route("/api/missions/{id}/trigger", post(trigger_mission))
        .route("/api/mission-file", get(get_mission_file).post(upsert_mission_file))
        // Chat & plan (also accessible via named WebRTC RPC)
        .route("/api/chat", post(chat_handler))
        .route("/api/chat/clear", post(clear_chat_history_api))
        .route("/api/chat/compact", post(compact_chat_api))
        .route("/api/chat/compact_config", post(compact_config_api))
        .route("/api/chat/system-prompt", get(get_system_prompt_api))
        .route("/api/plan/approve", post(approve_plan_handler))
        .route("/api/plan/edit", post(edit_plan_handler))
        .route("/api/plan/reject", post(reject_plan_handler))
        .route("/api/ask-user-response", post(ask_user_response_handler))
        .route("/api/pending-ask-user", get(pending_ask_user_handler))
        // Files & workspace
        .route("/api/workspace/tree", get(get_agent_tree))
        .route("/api/files", get(list_files))
        .route("/api/files/search", get(search_files))
        .route("/api/file", get(read_file_api))
        .route("/api/workspace/state", get(get_workspace_state))
        .route("/api/bash", post(run_bash_api))
        .route("/api/media/manifest", post(api::media::manifest_handler))
        .route(
            "/api/media/ingest",
            post(api::media::ingest_handler)
                // Originals include multi-GB videos; the axum default (2 MB) is
                // far too small for this one route.
                .layer(axum::extract::DefaultBodyLimit::max(32 * 1024 * 1024 * 1024)),
        )
        .route("/api/media/verify", post(api::media::verify_handler))
        .route("/api/media/reconcile", post(api::media::reconcile_handler))
        .route("/api/media/backup", post(api::media::backup_handler))
        .route("/api/media/request-delete", post(api::media::request_delete_handler))
        .route("/api/media/pending-deletes", get(api::media::pending_deletes_handler))
        .route("/api/tts", post(api::tts::tts_handler))
        .route("/api/yinyue/say", post(api::yinyue::say_handler))
        .route("/api/bridge/socket", get(bridge::socket_handler))
        .route("/api/bridge/call", post(bridge::call_handler))
        .route("/api/bridge/status", get(bridge::status_handler))
        .route("/mcp", post(mcp::post_handler).get(mcp::get_handler))
        .route("/api/yinyue/chat", post(api::yinyue::chat_handler))
        .route("/api/presence", post(api::yinyue::presence_handler))
        .route("/api/rtc/whip", post(rtc::whip_handler))
        .route("/api/rtc/token", get(rtc::whip_token_handler))
        .route("/api/status", get(get_status_api))
        .route("/api/account", get(get_account))
        .route("/api/account/login", post(post_account_login))
        .route("/api/account/callback", get(get_account_callback))
        .route("/api/account/logout", post(post_account_logout))
        .route("/api/account/mobile-token", get(api::account::get_mobile_token))
        .route("/api/pair/request", post(api::pair::post_pair_request))
        .route("/api/pair/confirm", post(api::pair::post_pair_confirm))
        .route("/api/pair/qr-confirm", post(api::pair::post_pair_qr_confirm))
        .route("/api/pair/info", get(api::pair::get_pair_info))
        .route("/api/pair/me", get(api::pair::get_pair_me))
        .route("/api/pair/qr", get(api::pair::get_pair_qr))
        .route(
            "/api/pair/devices/{id}",
            patch(api::pair::rename_pair_device).delete(api::pair::delete_pair_device),
        )
        .route("/pair", get(api::pair::get_pair_page))
        .route("/api/topic/publish", post(api::topic::publish))
        .route("/api/dj/library", get(api::dj::get_library))
        .route("/api/dj/file", get(api::dj::get_file))
        .route("/api/dj/devices", get(api::dj::get_devices))
        .route("/api/dj/have", post(api::dj::post_have))
        .route("/api/account/checkout", post(post_account_checkout))
        .route("/api/rooms", axum::routing::any(proxy_rooms))
        .route("/api/rooms/", axum::routing::any(proxy_rooms))
        .route("/api/rooms/{*path}", axum::routing::any(proxy_rooms))
        .route("/api/proxy/connect", post(connect_proxy_room_api))
        .route("/api/proxy/disconnect", post(disconnect_proxy_room_api))
        .route("/api/proxy/status", get(proxy_status_api))
        .route("/api/token-usage", get(token_usage_api))
        .route("/api/room-config", get(get_room_config).post(update_room_config))
        .route("/api/health", get(health_handler))
        .route("/api/utils/pick-folder", get(pick_folder))
        .route("/api/utils/ollama-status", get(get_ollama_status))
        .route("/api/storage/roots", get(storage_roots))
        .route("/api/storage/tree", get(storage_tree))
        .route("/api/storage/file", get(storage_read_file).put(storage_write_file).delete(storage_delete_file))
        .route("/apps/{skill_name}/capability/{tool_name}", post(capability_dispatch))
        .route("/apps/{skill_name}/{*file_path}", get(serve_app_file))
        .fallback(static_handler)
        .with_state(state.clone())
        // LAN gate: outermost layer — see `lan_gate`.
        .layer(axum::middleware::from_fn(lan_gate));

    // Spawn the cron mission scheduler.
    {
        let scheduler_state = state.clone();
        tokio::spawn(crate::extensions::missions::scheduler::mission_scheduler_loop(scheduler_state));
    }

    // Spawn Yinyue's event-reactive watch loop. Taps the same event bus and, on
    // a coarse trigger (first slice: a non-Yinyue mission finishing), wakes the
    // Yinyue agent to decide whether to tell the user. Guards against self-loops
    // and ignores the per-token firehose. See server/yinyue_watch.rs.
    {
        let yinyue_state = state.clone();
        tokio::spawn(yinyue_watch::yinyue_watch_loop(yinyue_state));
        // Ambient life-signs: on a jittered cadence she glances at the day and,
        // now and then, makes one small unprompted remark in her own voice
        // (mostly she stays quiet). Sibling to the watch loop, not a mission.
        tokio::spawn(yinyue_watch::yinyue_ambient_loop(state.clone()));
    }

    // Spawn the agent_run sweeper. Reaps `Running` rows older than the
    // threshold that never got `finish_agent_run` called (panic, dropped
    // future, missing finish on a new exit path). Without this, a stale
    // row keeps the UI spinner stuck on the affected session forever.
    {
        let sweep_state = state.clone();
        const SWEEP_INTERVAL_SECS: u64 = 60;
        const STALE_THRESHOLD_SECS: u64 = 15 * 60; // 15 min — well beyond any normal turn
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(SWEEP_INTERVAL_SECS));
            tick.tick().await; // skip the immediate first tick
            loop {
                tick.tick().await;
                let now = crate::util::now_ts_secs();
                let reaped = sweep_state
                    .manager
                    .run_store
                    .sweep_stale_running(now, STALE_THRESHOLD_SECS);
                if !reaped.is_empty() {
                    tracing::warn!(
                        count = reaped.len(),
                        run_ids = ?reaped,
                        threshold_secs = STALE_THRESHOLD_SECS,
                        "run/sweep: reaped stale Running rows"
                    );
                    let _ = sweep_state
                        .events_tx
                        .send(ServerEvent::StateUpdated);
                }
            }
        });
    }

    // Spawn remote relay tasks (heartbeat + offer polling) if remote.toml exists.
    // One-time cleanup of the retired remote.toml (its token was a second copy
    // of the account credential and drifted; see account::migrate_remote_toml).
    crate::account::migrate_remote_toml();
    rtc::relay::spawn_relay_tasks(state.clone());

    // Auto-connect to joined proxy rooms (linggen server consumer mode)
    // if auto_connect is enabled in room_config.toml.
    {
        let room_cfg = rtc::room_config::load_room_config();
        if room_cfg.auto_connect {
            let auto_state = state.clone();
            tokio::spawn(async move {
                // Small delay to let relay establish first.
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                rtc::proxy_room::auto_connect_joined_rooms(auto_state).await;
            });
        }
    }

    let listener = tokio::net::TcpListener::bind(format!("{}:{}", host, port)).await?;
    let actual_port = listener.local_addr()?.port();
    info!("Server running on http://{}:{}", host, actual_port);
    api::pair::advertise(actual_port, host != "127.0.0.1" && host != "localhost");

    // Anonymous usage telemetry. See src/telemetry/mod.rs for the field list
    // and opt-out paths. Fired here (after listener binds, before serve loop)
    // so a launch is only counted when the daemon is actually up.
    {
        let data_dir = crate::paths::linggen_home().clone();
        let telemetry = crate::telemetry::Telemetry::new("linggen", &data_dir);
        telemetry.launch();
        let system_state = crate::telemetry::read_system_state(&data_dir);
        telemetry.command_with_payload(
            "engine.start",
            serde_json::json!({ "system_state": system_state }),
        );
    }

    let task = tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await?;
        Ok(())
    });

    Ok(ServerHandle {
        task,
        port: actual_port,
    })
}

/// The LAN trust gate (see `api::pair`). Loopback callers — the local web UI,
/// app shells, skills, the simulator — pass untouched. Anything arriving over
/// the network must present a paired-device token; only the health check and
/// the pairing handshake itself stay open (they are the bootstrap).
async fn lan_gate(
    axum::extract::ConnectInfo(addr): axum::extract::ConnectInfo<std::net::SocketAddr>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    if addr.ip().is_loopback() {
        return next.run(req).await;
    }
    let path = req.uri().path();
    if matches!(
        path,
        "/api/health" | "/api/pair/request" | "/api/pair/confirm" | "/api/pair/qr-confirm"
    ) {
        return next.run(req).await;
    }
    let headers = req.headers();
    let token = headers
        .get("x-linggen-device")
        .and_then(|v| v.to_str().ok())
        .or_else(|| {
            headers
                .get(axum::http::header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.strip_prefix("Bearer "))
        })
        // Webviews (phone app_mode surfaces) can't add headers to their
        // subresource requests — they carry the device token as a cookie.
        .or_else(|| {
            headers
                .get(axum::http::header::COOKIE)
                .and_then(|v| v.to_str().ok())
                .and_then(|c| {
                    c.split(';')
                        .map(str::trim)
                        .find_map(|kv| kv.strip_prefix("linggen_device="))
                })
        });
    if token.map(api::pair::is_valid_device_token).unwrap_or(false) {
        return next.run(req).await;
    }
    tracing::info!("[gate] rejected {} {} from {}", req.method(), path, addr);
    (
        axum::http::StatusCode::UNAUTHORIZED,
        axum::Json(serde_json::json!({ "error": "pairing_required" })),
    )
        .into_response()
}

pub async fn start_server(
    manager: Arc<AgentManager>,
    skills: Arc<crate::extensions::skills::SkillLoader>,
    host: &str,
    port: u16,
    dev_mode: bool,
    idle_shutdown_secs: Option<u64>,
    agent_events_rx: mpsc::UnboundedReceiver<(crate::engine::agent::AgentEvent, Option<String>)>,
) -> anyhow::Result<()> {
    let handle = prepare_server(manager, skills, host, port, dev_mode, idle_shutdown_secs, agent_events_rx).await?;
    handle.task.await??;
    Ok(())
}

/// Dispatch a Memory_* tool on behalf of a skill's webpage.
/// Route: POST /apps/{skill_name}/capability/{tool_name}
///
/// Memory is engine-built-in (HTTP to `ling-mem`); this endpoint is a
/// thin wrapper around `tools::memory_tool::call_memory_http` so skill
/// webpages can call Memory_* without hardcoding the daemon URL. The
/// call also rides the existing WebRTC fetch proxy in remote mode
/// (control channel forwards `/apps/*` to the host's linggen server).
///
/// The pre-PR2 implementation gated this on the skill's frontmatter
/// `provides: [memory]`. With memory now engine-built-in there is no
/// `provides` list to consult — every installed skill can invoke
/// Memory_* via this endpoint. Skills the user doesn't trust shouldn't
/// be installed in the first place; the route name + skill_name still
/// shows up in audit logs.
async fn capability_dispatch(
    State(state): State<Arc<ServerState>>,
    axum::extract::Path((skill_name, tool_name)): axum::extract::Path<(String, String)>,
    body: Option<axum::Json<serde_json::Value>>,
) -> Response {
    use axum::http::StatusCode;

    let args = body
        .map(|axum::Json(v)| v)
        .unwrap_or(serde_json::Value::Object(Default::default()));

    if tool_name != "Memory_query" && tool_name != "Memory_write" {
        return (
            StatusCode::NOT_FOUND,
            format!("Unknown built-in tool '{}' on /apps/*/capability/*", tool_name),
        )
            .into_response();
    }

    if state.skills.get_skill(&skill_name).await.is_none() {
        return (StatusCode::NOT_FOUND, format!("Skill '{}' not found", skill_name)).into_response();
    }

    let ling_mem_url = state.manager.get_config_snapshot().await.agent.ling_mem_url;
    match crate::engine::tools::memory_tool::call_memory_http(&ling_mem_url, &tool_name, args).await {
        Ok(data) => axum::Json(data).into_response(),
        Err(e) => {
            let msg = format!("{:#}", e);
            (StatusCode::BAD_GATEWAY, msg).into_response()
        }
    }
}

/// Serve static files from an app-enabled skill's directory.
/// Route: GET /apps/{skill_name}/{*file_path}
async fn serve_app_file(
    State(state): State<Arc<ServerState>>,
    axum::extract::Path((skill_name, file_path)): axum::extract::Path<(String, String)>,
    req: axum::extract::Request,
) -> Response {
    let build_err = |status: u16, msg: &str| -> Response {
        Response::builder()
            .status(status)
            .header("Content-Type", "text/plain")
            .body(axum::body::Body::from(msg.to_string()))
            .unwrap_or_else(|_| Response::new(axum::body::Body::from("internal server error")))
    };

    // Look up the skill.
    let skill = state.skills.get_skill(&skill_name).await;
    let Some(skill) = skill else {
        return build_err(404, &format!("Skill '{}' not found", skill_name));
    };

    // Verify it has app config with web launcher.
    let Some(ref app) = skill.app else {
        return build_err(403, &format!("Skill '{}' is not an app", skill_name));
    };
    if app.launcher != "web" {
        return build_err(403, &format!("Skill '{}' app launcher is '{}', not 'web'", skill_name, app.launcher));
    }

    // Resolve the file within the skill directory.
    let Some(ref skill_dir) = skill.skill_dir else {
        return build_err(500, "Skill directory not available");
    };

    // Sanitize: reject path traversal.
    let file_path_clean = file_path.trim_start_matches('/');
    if file_path_clean.contains("..") {
        return build_err(403, "Path traversal not allowed");
    }

    let full_path = skill_dir.join(file_path_clean);

    // Canonicalize both paths to resolve symlinks and prevent escape.
    let canonical_dir = match tokio::fs::canonicalize(skill_dir).await {
        Ok(p) => p,
        Err(_) => return build_err(500, "Skill directory not accessible"),
    };
    let canonical_full = match tokio::fs::canonicalize(&full_path).await {
        Ok(p) => p,
        Err(_) => return build_err(404, &format!("File not found: {}", file_path_clean)),
    };
    if !canonical_full.starts_with(&canonical_dir) {
        return build_err(403, "Path traversal not allowed");
    }

    // Delegate to tower-http's ServeFile: it speaks Range/206 (video seeking
    // in skill pages needs partial content), conditional requests, and HEAD.
    use tower::ServiceExt as _;
    let range_hdr = req.headers().get("range").and_then(|v| v.to_str().ok()).map(String::from);
    match tower_http::services::ServeFile::new(&canonical_full).oneshot(req).await {
        Ok(res) => {
            tracing::debug!("serve_app_file {} range={:?} -> {}", file_path_clean, range_hdr, res.status());
            let mut res = res.map(axum::body::Body::new);
            let headers = res.headers_mut();
            headers.insert("X-Frame-Options", "ALLOWALL".parse().expect("static header"));
            // No-store on skill assets: skills are user-iterated, often
            // edited mid-session, and ES-module URL caching makes a stale
            // scan.js indistinguishable from a missing feature. Forcing
            // revalidation costs nothing on localhost and removes a sharp
            // edge from the development loop.
            headers.insert("Cache-Control", "no-store".parse().expect("static header"));
            res
        }
        Err(_) => build_err(500, &format!("File not readable: {}", file_path_clean)),
    }
}

async fn static_handler(State(state): State<Arc<ServerState>>, uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');

    let build_response = |builder: axum::http::response::Builder, body: axum::body::Body| -> Response {
        builder.body(body).unwrap_or_else(|_| {
            Response::new(axum::body::Body::from("internal server error"))
        })
    };

    if state.dev_mode {
        // In dev mode, static assets are served by the Vite dev server.
        // Return 404 so the user knows to use the Vite proxy.
        return build_response(
            Response::builder().status(404).header("Content-Type", "text/plain"),
            axum::body::Body::from(
                "Dev mode: static assets are served by Vite. Use the Vite dev server URL instead.",
            ),
        );
    }

    // All surface routes (/, /embed, /consumer) share a single index.html.
    // The JS entry inspects window.location to pick MainApp/EmbedApp/ConsumerApp.
    // This keeps the bundle as a single chunk — required for blob-URL loading
    // through the linggen.dev tunnel (shared chunks with relative imports fail).
    let path = if path.is_empty() { "index.html" } else { path };

    // Allow embedding in iframes (e.g. VS Code webview, skill app iframes).
    let xfo = "X-Frame-Options";
    let xfo_val = "ALLOWALL";

    match Assets::get(path) {
        Some(content) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            build_response(
                Response::builder()
                    .header("Content-Type", mime.as_ref())
                    .header(xfo, xfo_val),
                axum::body::Body::from(content.data),
            )
        }
        None => {
            // Fallback to index.html for SPA routing
            match Assets::get("index.html") {
                Some(index) => build_response(
                    Response::builder()
                        .header("Content-Type", "text/html")
                        .header(xfo, xfo_val),
                    axum::body::Body::from(index.data),
                ),
                None => build_response(
                    Response::builder().status(404),
                    axum::body::Body::from("Not found"),
                ),
            }
        }
    }
}

/// Seconds since the Unix epoch (0 on the impossible clock-before-epoch error).
fn unix_secs_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

async fn health_handler(
    axum::extract::State(state): axum::extract::State<Arc<ServerState>>,
) -> impl IntoResponse {
    // A health ping is a sign of life: bundled-app shells ping this every ~60s
    // while their window is open, which keeps the idle-shutdown clock fresh.
    state
        .last_activity
        .store(unix_secs_now(), std::sync::atomic::Ordering::Relaxed);
    axum::Json(json!({ "ok": true, "version": env!("CARGO_PKG_VERSION") }))
}

async fn pick_folder() -> impl IntoResponse {
    #[cfg(target_os = "macos")]
    {
        let result = tokio::process::Command::new("osascript")
            .arg("-e")
            .arg("POSIX path of (choose folder with prompt \"Select project folder\")")
            .output()
            .await;
        match result {
            Ok(output) if output.status.success() => {
                let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                // osascript returns path with trailing slash — strip it
                let path = path.trim_end_matches('/').to_string();
                if path.is_empty() {
                    return (axum::http::StatusCode::NO_CONTENT, "").into_response();
                }
                axum::Json(serde_json::json!({ "path": path })).into_response()
            }
            Ok(_) => {
                // User cancelled the dialog
                (axum::http::StatusCode::NO_CONTENT, "").into_response()
            }
            Err(e) => {
                tracing::warn!("Folder picker failed: {e}");
                (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        (axum::http::StatusCode::NOT_IMPLEMENTED, "Folder picker not available on this platform").into_response()
    }
}

async fn get_ollama_status(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    let models_guard = state.manager.models.read().await;
    if let Some(client) = models_guard.first_ollama_client() {
        match client.get_ps().await {
            Ok(status) => axum::Json(status).into_response(),
            Err(e) => {
                (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
            }
        }
    } else {
        (
            axum::http::StatusCode::NOT_FOUND,
            "No Ollama models configured",
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ensure every ServerEvent variant maps without panicking.
    /// Acts as a documentation checkpoint — if a new variant is added, this test
    /// will fail to compile until a mapping arm is provided.
    #[test]
    fn all_server_events_mapped() {
        let events: Vec<ServerEvent> = vec![
            ServerEvent::StateUpdated,
            ServerEvent::Message {
                from: "ling".into(),
                to: "user".into(),
                content: "hello".into(),
                session_id: None,
                run_id: None,
                parent_agent_id: None,
            },
            ServerEvent::SubagentSpawned {
                parent_id: "ling".into(),
                subagent_id: "coder".into(),
                task: "fix bug".into(),
                session_id: None,
                subagent_run_id: None,
                parent_run_id: None,
            },
            ServerEvent::SubagentResult {
                parent_id: "ling".into(),
                subagent_id: "coder".into(),
                outcome: crate::engine::AgentOutcome::None,
                session_id: None,
                subagent_run_id: None,
                parent_run_id: None,
            },
            ServerEvent::AgentStatus {
                agent_id: "ling".into(),
                status: "thinking".into(),
                detail: Some("Analyzing code".into()),
                status_id: None,
                lifecycle: Some("doing".into()),
                parent_agent_id: None,
                session_id: None,
                run_id: None,
                parent_run_id: None,
            },
            ServerEvent::QueueUpdated {
                project_root: "/tmp".into(),
                session_id: "s1".into(),
                agent_id: "ling".into(),
                items: vec![],
            },
            ServerEvent::ContextUsage {
                agent_id: "ling".into(),
                stage: "pre".into(),
                message_count: 10,
                char_count: 5000,
                estimated_tokens: 1500,
                token_limit: Some(200_000),
                actual_prompt_tokens: None,
                actual_completion_tokens: None,
                compressed: false,
                summary_count: 0,
                session_id: None,
            },
            ServerEvent::Outcome {
                agent_id: "ling".into(),
                outcome: crate::engine::AgentOutcome::None,
                session_id: None,
            },
            ServerEvent::Token {
                session_id: None,
                agent_id: "ling".into(),
                token: "Hello".into(),
                done: false,
                thinking: false,
            },
            ServerEvent::PlanUpdate {
                agent_id: "ling".into(),
                plan: crate::engine::Plan {
                    summary: "Test plan".into(),
                    status: crate::engine::PlanStatus::Planned,
                    plan_text: String::new(),
                    items: Vec::new(),
                },
                session_id: None,
            },
            ServerEvent::MissionTriggered {
                mission_id: "mission-1".into(),
                agent_id: "ling".into(),
                project_root: "/tmp".into(),
                session_id: None,
            },
            ServerEvent::TextSegment {
                agent_id: "ling".into(),
                text: "some text".into(),
                parent_id: None,
                session_id: None,
            },
            ServerEvent::AskUser {
                agent_id: "ling".into(),
                question_id: "q1".into(),
                questions: vec![],
                session_id: None,
            },
            ServerEvent::ModelFallback {
                agent_id: "ling".into(),
                preferred_model: "gpt-4".into(),
                actual_model: "gpt-3.5".into(),
                reason: "rate_limited".into(),
                session_id: None,
            },
            ServerEvent::ToolProgress {
                agent_id: "ling".into(),
                tool: "Bash".into(),
                line: "building...".into(),
                stream: "stdout".into(),
                session_id: None,
            },
            ServerEvent::Resync {
                reason: "broadcast_lag".into(),
                lagged_count: Some(42),
            },
            ServerEvent::ContentBlockStart {
                agent_id: "ling".into(),
                block_id: "cb-1".into(),
                block_type: "tool_use".into(),
                tool: Some("Read".into()),
                args: Some("foo.rs".into()),
                parent_id: None,
                session_id: None,
                run_id: None,
                parent_run_id: None,
            },
            ServerEvent::ContentBlockUpdate {
                agent_id: "ling".into(),
                block_id: "cb-1".into(),
                status: Some("done".into()),
                summary: Some("Read 42 lines".into()),
                is_error: Some(false),
                parent_id: None,
                extra: None,
                session_id: None,
                run_id: None,
                parent_run_id: None,
            },
            ServerEvent::TurnComplete {
                agent_id: "ling".into(),
                duration_ms: Some(1200),
                context_tokens: Some(5000),
                parent_id: None,
                session_id: None,
                run_id: None,
                parent_run_id: None,
            },
            ServerEvent::RoomChat {
                sender_id: "user-1".into(),
                sender_name: "Alice".into(),
                avatar_url: None,
                text: "Hello room!".into(),
            },
        ];

        for (i, event) in events.into_iter().enumerate() {
            let result = map_server_event_to_ui_message(event, i as u64);
            // All variants should produce Some(...), except Message which may
            // return None if sanitization strips it. We just verify no panics.
            let _ = result;
        }
    }
}
