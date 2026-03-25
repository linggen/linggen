mod agent_api;
mod chat_api;
pub(crate) mod chat_helpers;
mod config_api;
pub(crate) mod mission_scheduler;
mod missions_api;
mod marketplace_api;
mod projects_api;
pub(crate) mod rtc;
mod storage_api;
mod workspace_api;

use crate::agent_manager::AgentManager;
use axum::{
    extract::State,
    http::Uri,
    response::{
        sse::{Event, Sse},
        IntoResponse, Response,
    },
    routing::{delete, get, patch, post},
    Router,
};
use rust_embed::RustEmbed;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::broadcast;
use tokio::sync::mpsc;
use tokio::sync::Mutex;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;
use tracing::info;

use agent_api::{
    cancel_agent_run, cancel_tool_execution,
    run_agent, set_task,
};
use chat_api::{approve_plan_handler, ask_user_response_handler, chat_handler, clear_chat_history_api, compact_chat_api, edit_plan_handler, get_system_prompt_api, pending_ask_user_handler, reject_plan_handler};
use config_api::{get_config_api, get_credentials_api, get_models_health, update_config_api, update_credentials_api, get_codex_auth_status, start_codex_auth_login, codex_auth_logout};
use projects_api::{
    add_project, create_session, delete_agent_file_api, delete_skill_file_api,
    get_agent_context_api, get_agent_file_api, get_skill_file_api, list_agent_children_api,
    get_status_api,
    list_agent_files_api, list_agent_runs_api, list_agents_api, list_models_api, list_projects,
    delete_unified_session, get_skill_session_state, list_all_sessions, list_sessions, list_skill_files_api, list_skill_sessions, list_skills, reload_agents, reload_skills,
    remove_project, remove_session_api, remove_skill_session_api,
    rename_session_api, resolve_session_api, upsert_agent_file_api, upsert_skill_file_api,
};
use marketplace_api::{builtin_skills_install, builtin_skills_install_all, builtin_skills_list, clawhub_scan, community_search, marketplace_install, marketplace_move_to_global, marketplace_uninstall};
use missions_api::{
    create_mission, delete_mission, delete_mission_session, get_mission,
    get_mission_session_state, list_mission_runs, list_mission_sessions, list_missions,
    trigger_mission, update_mission,
};
use storage_api::{storage_roots, storage_tree, storage_read_file, storage_write_file, storage_delete_file};
use workspace_api::{get_agent_tree, get_workspace_state, list_files, read_file_api, run_bash_api, search_files};

#[derive(RustEmbed)]
#[folder = "ui/dist/"]
struct Assets;

pub struct ServerState {
    pub manager: Arc<AgentManager>,
    pub dev_mode: bool,
    pub port: u16,
    pub events_tx: broadcast::Sender<ServerEvent>,
    pub skill_manager: Arc<crate::skills::SkillManager>,
    pub prompt_store: Arc<crate::prompts::PromptStore>,
    pub queued_chats: Arc<Mutex<HashMap<String, Vec<QueuedChatItem>>>>,
    /// Senders for interrupt messages keyed by queue_key. Used to inject user
    /// messages into a running agent loop so the model can adapt mid-run.
    pub interrupt_tx: Arc<Mutex<HashMap<String, mpsc::UnboundedSender<String>>>>,
    /// Pending AskUser questions waiting for user responses.
    /// Keyed by unique question_id. The oneshot sender delivers the user's answer.
    pub pending_ask_user: Arc<Mutex<HashMap<String, crate::engine::tools::PendingAskUser>>>,
    status_seq: AtomicU64,
    active_statuses: Arc<Mutex<HashMap<String, ActiveStatusRecord>>>,
    pub queue_seq: AtomicU64,
    pub event_seq: AtomicU64,
    /// Accumulated token usage per session (in-memory, resets on restart).
    /// Key: "{project_root}:{session_id}", Value: (prompt_tokens, completion_tokens).
    pub session_tokens: Arc<Mutex<HashMap<String, (usize, usize)>>>,
    /// Maps running agent_id → session_id so SSE events can be tagged with their session.
    /// Uses `std::sync::RwLock` so SSE readers never block on each other and only
    /// briefly contend with writers (register/deregister are single insert/remove).
    pub agent_sessions: Arc<std::sync::RwLock<HashMap<String, String>>>,
    /// Random token required for WHIP endpoint authentication.
    /// Generated at startup, passed to the UI via /api/status.
    pub whip_token: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatusKind {
    Idle,
    ModelLoading,
    Thinking,
    CallingTool,
    Working,
}

impl AgentStatusKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::ModelLoading => "model_loading",
            Self::Thinking => "thinking",
            Self::CallingTool => "calling_tool",
            Self::Working => "working",
        }
    }

    pub fn from_str_loose(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "idle" => Self::Idle,
            "model_loading" => Self::ModelLoading,
            "thinking" => Self::Thinking,
            "calling_tool" => Self::CallingTool,
            "working" => Self::Working,
            _ => Self::Working,
        }
    }
}

#[derive(Debug, Clone)]
struct ActiveStatusRecord {
    status_id: String,
    status: AgentStatusKind,
    detail: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct QueuedChatItem {
    pub id: String,
    pub agent_id: String,
    pub session_id: String,
    pub preview: String,
    pub timestamp: u64,
}

/// Discriminated payload for the Notification SSE event.
/// Add new variants here to introduce new notification types.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NotificationPayload {
    MissionCompleted {
        mission_id: String,
        mission_name: String,
        status: String,
        run_id: String,
        session_id: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum ServerEvent {
    StateUpdated,
    Message {
        from: String,
        to: String,
        content: String,
        session_id: Option<String>,
    },
    SubagentSpawned {
        parent_id: String,
        subagent_id: String,
        task: String,
    },
    SubagentResult {
        parent_id: String,
        subagent_id: String,
        outcome: crate::engine::AgentOutcome,
    },
    AgentStatus {
        agent_id: String,
        status: String,
        detail: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        status_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        lifecycle: Option<String>, // "doing" | "done"
        #[serde(skip_serializing_if = "Option::is_none")]
        parent_agent_id: Option<String>,
    },
    QueueUpdated {
        project_root: String,
        session_id: String,
        agent_id: String,
        items: Vec<QueuedChatItem>,
    },
    ContextUsage {
        agent_id: String,
        stage: String,
        message_count: usize,
        char_count: usize,
        estimated_tokens: usize,
        #[serde(skip_serializing_if = "Option::is_none")]
        token_limit: Option<usize>,
        #[serde(skip_serializing_if = "Option::is_none")]
        actual_prompt_tokens: Option<usize>,
        #[serde(skip_serializing_if = "Option::is_none")]
        actual_completion_tokens: Option<usize>,
        compressed: bool,
        summary_count: usize,
    },
    Outcome {
        agent_id: String,
        outcome: crate::engine::AgentOutcome,
    },
    Token {
        agent_id: String,
        token: String,
        done: bool,
        thinking: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
    },
    PlanUpdate {
        agent_id: String,
        plan: crate::engine::Plan,
    },
    MissionTriggered {
        mission_id: String,
        agent_id: String,
        project_root: String,
    },
    Notification(NotificationPayload),
    /// A new session was created — used to update the unified session list in real-time.
    SessionCreated {
        session_id: String,
        title: String,
        creator: String,
        project: Option<String>,
        project_name: Option<String>,
        skill: Option<String>,
        mission_id: Option<String>,
    },
    TextSegment {
        agent_id: String,
        text: String,
        parent_id: Option<String>,
    },
    AskUser {
        agent_id: String,
        question_id: String,
        questions: Vec<crate::engine::tools::AskUserQuestion>,
        session_id: Option<String>,
    },
    ModelFallback {
        agent_id: String,
        preferred_model: String,
        actual_model: String,
        reason: String,
    },
    ToolProgress {
        agent_id: String,
        tool: String,
        line: String,
        stream: String, // "stdout" | "stderr"
    },
    Resync {
        reason: String,
        lagged_count: Option<u64>,
    },
    /// An app-enabled skill was launched (web, bash, or url).
    AppLaunched {
        skill: String,
        launcher: String,
        url: String,
        title: String,
        width: Option<u32>,
        height: Option<u32>,
    },
    /// A new content block started within the current assistant turn.
    ContentBlockStart {
        agent_id: String,
        block_id: String,
        block_type: String,
        tool: Option<String>,
        args: Option<String>,
        parent_id: Option<String>,
    },
    /// Update an existing content block (status change, result summary).
    ContentBlockUpdate {
        agent_id: String,
        block_id: String,
        status: Option<String>,
        summary: Option<String>,
        is_error: Option<bool>,
        parent_id: Option<String>,
        /// Optional extra payload (e.g. diff data for Edit/Write tools).
        extra: Option<serde_json::Value>,
    },
    /// Signal that the assistant turn is complete (single finalizer).
    TurnComplete {
        agent_id: String,
        duration_ms: Option<u64>,
        context_tokens: Option<usize>,
        parent_id: Option<String>,
    },
}

impl ServerEvent {
    /// Convert a 1:1 `AgentEvent` variant into the corresponding `ServerEvent`.
    /// Returns `None` for variants that require special handling (AgentStatus, TaskUpdate).
    fn from_agent_event(event: crate::agent_manager::AgentEvent) -> Option<Self> {
        use crate::agent_manager::AgentEvent;
        match event {
            AgentEvent::StateUpdated => Some(Self::StateUpdated),
            AgentEvent::Message { from, to, content } => {
                Some(Self::Message { from, to, content, session_id: None })
            }
            AgentEvent::SubagentSpawned { parent_id, subagent_id, task } => {
                Some(Self::SubagentSpawned { parent_id, subagent_id, task })
            }
            AgentEvent::SubagentResult { parent_id, subagent_id, outcome } => {
                Some(Self::SubagentResult { parent_id, subagent_id, outcome })
            }
            AgentEvent::Outcome { agent_id, outcome } => {
                Some(Self::Outcome { agent_id, outcome })
            }
            AgentEvent::ContextUsage {
                agent_id, stage, message_count, char_count, estimated_tokens,
                token_limit, actual_prompt_tokens, actual_completion_tokens,
                compressed, summary_count,
            } => Some(Self::ContextUsage {
                agent_id, stage, message_count, char_count, estimated_tokens,
                token_limit, actual_prompt_tokens, actual_completion_tokens,
                compressed, summary_count,
            }),
            AgentEvent::PlanUpdate { agent_id, plan } => {
                Some(Self::PlanUpdate { agent_id, plan })
            }
            AgentEvent::TextSegment { agent_id, text, parent_id } => {
                Some(Self::TextSegment { agent_id, text, parent_id })
            }
            AgentEvent::ModelFallback { agent_id, preferred_model, actual_model, reason } => {
                Some(Self::ModelFallback { agent_id, preferred_model, actual_model, reason })
            }
            AgentEvent::ToolProgress { agent_id, tool, line, stream } => {
                Some(Self::ToolProgress { agent_id, tool, line, stream })
            }
            AgentEvent::ContentBlockStart { agent_id, block_id, block_type, tool, args, parent_id } => {
                tracing::debug!("SSE ContentBlockStart: agent={} type={} tool={:?}", agent_id, block_type, tool);
                Some(Self::ContentBlockStart { agent_id, block_id, block_type, tool, args, parent_id })
            }
            AgentEvent::ContentBlockUpdate { agent_id, block_id, status, summary, is_error, parent_id, extra } => {
                Some(Self::ContentBlockUpdate { agent_id, block_id, status, summary, is_error, parent_id, extra })
            }
            AgentEvent::TurnComplete { agent_id, duration_ms, context_tokens, parent_id } => {
                Some(Self::TurnComplete { agent_id, duration_ms, context_tokens, parent_id })
            }
            // AgentStatus and TaskUpdate need special handling — return None.
            AgentEvent::AgentStatus { .. } | AgentEvent::TaskUpdate { .. } => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiEvent {
    pub id: String,
    pub seq: u64,
    pub rev: u64,
    pub ts_ms: u64,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_root: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

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

const UI_PHASE_SYNC: &str = "sync";
const UI_PHASE_OUTCOME: &str = "outcome";
const UI_PHASE_CONTEXT_USAGE: &str = "context_usage";
const UI_PHASE_SUBAGENT_SPAWNED: &str = "subagent_spawned";
const UI_PHASE_SUBAGENT_RESULT: &str = "subagent_result";
const UI_PHASE_PLAN_UPDATE: &str = "plan_update";
const UI_PHASE_DOING: &str = "doing";
const UI_PHASE_DONE: &str = "done";
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
        ServerEvent::Message { from, to, content, session_id } => {
            let cleaned = crate::server::chat_helpers::sanitize_message_for_ui(&from, &content)?;
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
                    session_id: None,
                    project_root: None,
                    data: Some(json!({ "status": "idle", "parent_id": parent_agent_id })),
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
                session_id: None,
                project_root: None,
                data: Some(json!({ "status": status, "parent_id": parent_agent_id })),
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
        ServerEvent::Outcome { agent_id, outcome } => Some(UiEvent {
            id: format!("run-outcome-{agent_id}-{seq}"),
            seq,
            rev: seq,
            ts_ms,
            kind: UI_KIND_RUN.to_string(),
            phase: Some(UI_PHASE_OUTCOME.to_string()),
            text: Some("Run outcome".to_string()),
            agent_id: Some(agent_id),
            session_id: None,
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
        } => Some(UiEvent {
            id: format!("run-context-{agent_id}"),
            seq,
            rev: seq,
            ts_ms,
            kind: UI_KIND_RUN.to_string(),
            phase: Some(UI_PHASE_CONTEXT_USAGE.to_string()),
            text: None,
            agent_id: Some(agent_id.clone()),
            session_id: None,
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
        } => Some(UiEvent {
            id: format!("run-subagent-spawned-{subagent_id}-{seq}"),
            seq,
            rev: seq,
            ts_ms,
            kind: UI_KIND_RUN.to_string(),
            phase: Some(UI_PHASE_SUBAGENT_SPAWNED.to_string()),
            text: Some(format!("Spawned subagent {}", subagent_id)),
            agent_id: Some(parent_id),
            session_id: None,
            project_root: None,
            data: Some(json!({ "subagent_id": subagent_id, "task": task })),
        }),
        ServerEvent::SubagentResult {
            parent_id,
            subagent_id,
            outcome,
        } => Some(UiEvent {
            id: format!("run-subagent-result-{subagent_id}-{seq}"),
            seq,
            rev: seq,
            ts_ms,
            kind: UI_KIND_RUN.to_string(),
            phase: Some(UI_PHASE_SUBAGENT_RESULT.to_string()),
            text: Some(format!("Subagent {} returned", subagent_id)),
            agent_id: Some(parent_id),
            session_id: None,
            project_root: None,
            data: Some(json!({ "subagent_id": subagent_id, "outcome": outcome })),
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
        ServerEvent::PlanUpdate { agent_id, plan } => Some(UiEvent {
            id: format!("run-plan-{agent_id}-{seq}"),
            seq,
            rev: seq,
            ts_ms,
            kind: UI_KIND_RUN.to_string(),
            phase: Some(UI_PHASE_PLAN_UPDATE.to_string()),
            text: Some("Plan updated".to_string()),
            agent_id: Some(agent_id),
            session_id: None,
            project_root: None,
            data: Some(json!({ "plan": plan })),
        }),
        ServerEvent::MissionTriggered {
            mission_id,
            agent_id,
            project_root,
        } => Some(UiEvent {
            id: format!("mission-trigger-{mission_id}-{seq}"),
            seq,
            rev: seq,
            ts_ms,
            kind: UI_KIND_ACTIVITY.to_string(),
            phase: Some(UI_PHASE_DOING.to_string()),
            text: Some("Mission triggered".to_string()),
            agent_id: Some(agent_id),
            session_id: None,
            project_root: Some(project_root),
            data: Some(json!({ "status": "mission_triggered", "mission_id": mission_id })),
        }),
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
        ServerEvent::Notification(ref payload) => {
            let data = serde_json::to_value(payload).ok();
            let text = match payload {
                NotificationPayload::MissionCompleted { mission_name, status, .. } => {
                    format!("Mission '{}' {}", mission_name, status)
                }
            };
            let id_str = match payload {
                NotificationPayload::MissionCompleted { mission_id, .. } => {
                    format!("notif-mission-{mission_id}-{seq}")
                }
            };
            Some(UiEvent {
                id: id_str,
                seq,
                rev: seq,
                ts_ms,
                kind: "notification".to_string(),
                phase: None,
                text: Some(text),
                agent_id: None,
                session_id: Some("global".to_string()),
                project_root: None,
                data,
            })
        }
        ServerEvent::TextSegment {
            agent_id,
            text,
            parent_id,
        } => Some(UiEvent {
            id: format!("text-seg-{agent_id}-{seq}"),
            seq,
            rev: seq,
            ts_ms,
            kind: UI_KIND_TEXT_SEGMENT.to_string(),
            phase: None,
            text: Some(text),
            agent_id: Some(agent_id),
            session_id: None,
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
        ServerEvent::ModelFallback {
            agent_id,
            preferred_model,
            actual_model,
            reason,
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
            session_id: None,
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
        } => Some(UiEvent {
            id: format!("tool-progress-{agent_id}-{seq}"),
            seq,
            rev: seq,
            ts_ms,
            kind: "tool_progress".to_string(),
            phase: None,
            text: Some(line.clone()),
            agent_id: Some(agent_id),
            session_id: None,
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
        } => Some(UiEvent {
            id: format!("app-launched-{skill}-{seq}"),
            seq,
            rev: seq,
            ts_ms,
            kind: "app_launched".to_string(),
            phase: None,
            text: Some(format!("Launched app: {}", title)),
            agent_id: None,
            session_id: Some("global".to_string()),
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
                session_id: None,
                project_root: None,
                data: Some(json!({
                    "block_id": block_id,
                    "block_type": block_type,
                    "tool": tool,
                    "args": args,
                    "parent_id": parent_id,
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
        } => {
            let mut data_obj = json!({
                "block_id": block_id,
                "status": status,
                "summary": summary,
                "is_error": is_error,
                "parent_id": parent_id,
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
                session_id: None,
                project_root: None,
                data: Some(data_obj),
            })
        }
        ServerEvent::TurnComplete {
            agent_id,
            duration_ms,
            context_tokens,
            parent_id,
        } => Some(UiEvent {
            id: format!("turn-complete-{agent_id}-{seq}"),
            seq,
            rev: seq,
            ts_ms,
            kind: UI_KIND_TURN_COMPLETE.to_string(),
            phase: None,
            text: None,
            agent_id: Some(agent_id),
            session_id: None,
            project_root: None,
            data: Some(json!({
                "duration_ms": duration_ms,
                "context_tokens": context_tokens,
                "parent_id": parent_id,
            })),
        }),
    }
}

impl ServerState {
    pub async fn send_agent_status(
        &self,
        agent_id: String,
        status: AgentStatusKind,
        detail: Option<String>,
        parent_agent_id: Option<String>,
    ) {
        let mut done_event: Option<ServerEvent> = None;
        let mut status_id: Option<String> = None;
        let mut lifecycle: Option<String> = None;

        {
            let mut active = self.active_statuses.lock().await;
            if status == AgentStatusKind::Idle {
                if let Some(prev) = active.remove(&agent_id) {
                    done_event = Some(ServerEvent::AgentStatus {
                        agent_id: agent_id.clone(),
                        status: prev.status.as_str().to_string(),
                        detail: prev.detail,
                        status_id: Some(prev.status_id),
                        lifecycle: Some(UI_PHASE_DONE.to_string()),
                        parent_agent_id: parent_agent_id.clone(),
                    });
                }
            } else {
                if let Some(prev) = active.get(&agent_id).cloned() {
                    if prev.status != status {
                        done_event = Some(ServerEvent::AgentStatus {
                            agent_id: agent_id.clone(),
                            status: prev.status.as_str().to_string(),
                            detail: prev.detail,
                            status_id: Some(prev.status_id),
                            lifecycle: Some(UI_PHASE_DONE.to_string()),
                            parent_agent_id: parent_agent_id.clone(),
                        });
                        active.remove(&agent_id);
                    } else {
                        status_id = Some(prev.status_id.clone());
                        lifecycle = Some(UI_PHASE_DOING.to_string());
                        active.insert(
                            agent_id.clone(),
                            ActiveStatusRecord {
                                status_id: prev.status_id,
                                status,
                                detail: detail.clone(),
                            },
                        );
                    }
                }

                if status_id.is_none() {
                    let next_id =
                        format!("status-{}", self.status_seq.fetch_add(1, Ordering::Relaxed));
                    status_id = Some(next_id.clone());
                    lifecycle = Some(UI_PHASE_DOING.to_string());
                    active.insert(
                        agent_id.clone(),
                        ActiveStatusRecord {
                            status_id: next_id,
                            status,
                            detail: detail.clone(),
                        },
                    );
                }
            }
        }

        if let Some(done) = done_event {
            let _ = self.events_tx.send(done);
        }

        let _ = self.events_tx.send(ServerEvent::AgentStatus {
            agent_id,
            status: status.as_str().to_string(),
            detail,
            status_id,
            parent_agent_id,
            lifecycle,
        });
    }
}

pub struct ServerHandle {
    pub task: tokio::task::JoinHandle<anyhow::Result<()>>,
    pub port: u16,
}

pub async fn prepare_server(
    manager: Arc<AgentManager>,
    skill_manager: Arc<crate::skills::SkillManager>,
    port: u16,
    dev_mode: bool,
    mut agent_events_rx: mpsc::UnboundedReceiver<crate::agent_manager::AgentEvent>,
) -> anyhow::Result<ServerHandle> {
    info!("linggen server starting on port {}...", port);

    // SSE can be bursty (tool/status steps). Use a larger buffer to reduce lag drops.
    let (events_tx, _) = broadcast::channel(4096);

    let prompt_store = Arc::new(crate::prompts::PromptStore::load(
        Some(&crate::prompts::PromptStore::default_override_dir()),
    ));

    let state = Arc::new(ServerState {
        manager,
        dev_mode,
        port,
        events_tx,
        skill_manager,
        prompt_store,
        queued_chats: Arc::new(Mutex::new(HashMap::new())),
        interrupt_tx: Arc::new(Mutex::new(HashMap::new())),
        pending_ask_user: Arc::new(Mutex::new(HashMap::new())),
        status_seq: AtomicU64::new(1),
        active_statuses: Arc::new(Mutex::new(HashMap::new())),
        queue_seq: AtomicU64::new(1),
        event_seq: AtomicU64::new(1),
        session_tokens: Arc::new(Mutex::new(HashMap::new())),
        agent_sessions: Arc::new(std::sync::RwLock::new(HashMap::new())),
        whip_token: uuid::Uuid::new_v4().to_string(),
    });

    // Bridge internal AgentManager events to SSE for the UI.
    {
        let state_clone = state.clone();
        tokio::spawn(async move {
            while let Some(event) = agent_events_rx.recv().await {
                match event {
                    // Special cases that need extra logic beyond a 1:1 mapping.
                    crate::agent_manager::AgentEvent::AgentStatus {
                        agent_id, status, detail, parent_id,
                    } => {
                        state_clone
                            .send_agent_status(agent_id, AgentStatusKind::from_str_loose(&status), detail, parent_id)
                            .await;
                    }
                    crate::agent_manager::AgentEvent::TaskUpdate { .. } => {
                        let _ = state_clone.events_tx.send(ServerEvent::StateUpdated);
                    }
                    // All other variants have a 1:1 ServerEvent equivalent.
                    other => {
                        // Accumulate token usage from ContextUsage events.
                        if let crate::agent_manager::AgentEvent::ContextUsage {
                            agent_id: ctx_agent_id,
                            actual_prompt_tokens: Some(prompt),
                            actual_completion_tokens: Some(completion),
                            ..
                        } = &other {
                            let sid = state_clone.agent_sessions.read().unwrap()
                                .get(ctx_agent_id).cloned()
                                .unwrap_or_else(|| "current".to_string());
                            let mut tokens = state_clone.session_tokens.lock().await;
                            let entry = tokens.entry(sid).or_insert((0, 0));
                            entry.0 += prompt;
                            entry.1 += completion;
                        }
                        // Register subagent → parent's session_id so SSE events
                        // from subagents are tagged with the correct session.
                        if let crate::agent_manager::AgentEvent::SubagentSpawned {
                            parent_id, subagent_id, ..
                        } = &other {
                            let sid = state_clone.agent_sessions.read().unwrap()
                                .get(parent_id).cloned();
                            if let Some(sid) = sid {
                                state_clone.agent_sessions.write().unwrap()
                                    .insert(subagent_id.clone(), sid);
                            }
                        }
                        // Deregister subagent when it finishes (delayed so final
                        // events like idle AgentStatus still get enriched).
                        if let crate::agent_manager::AgentEvent::SubagentResult {
                            subagent_id, ..
                        } = &other {
                            let sessions_ref = state_clone.agent_sessions.clone();
                            let key = subagent_id.clone();
                            tokio::spawn(async move {
                                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                                sessions_ref.write().unwrap().remove(&key);
                            });
                        }
                        if let Some(se) = ServerEvent::from_agent_event(other) {
                            let _ = state_clone.events_tx.send(se);
                        }
                    }
                }
            }
        });
    }

    let app = Router::new()
        .route("/api/projects", get(list_projects))
        .route("/api/projects", post(add_project))
        .route("/api/projects", delete(remove_project))
        .route("/api/agents", get(list_agents_api))
        .route("/api/agent-files", get(list_agent_files_api))
        .route("/api/agent-file", get(get_agent_file_api))
        .route("/api/agent-file", post(upsert_agent_file_api))
        .route("/api/agent-file", delete(delete_agent_file_api))
        .route("/api/agent-runs", get(list_agent_runs_api))
        .route("/api/agent-children", get(list_agent_children_api))
        .route("/api/agent-context", get(get_agent_context_api))
        .route("/api/models", get(list_models_api))
        .route("/api/models/health", get(get_models_health))
        .route("/api/config", get(get_config_api).post(update_config_api))
        .route("/api/credentials", get(get_credentials_api).put(update_credentials_api))
        .route("/api/auth/codex/status", get(get_codex_auth_status))
        .route("/api/auth/codex/login", post(start_codex_auth_login))
        .route("/api/auth/codex/logout", post(codex_auth_logout))
        .route("/api/skills", get(list_skills))
        .route("/api/skills/reload", post(reload_skills))
        .route("/api/agents/reload", post(reload_agents))
        .route("/api/community-skills/search", get(community_search))
        .route("/api/marketplace/install", post(marketplace_install))
        .route("/api/marketplace/uninstall", delete(marketplace_uninstall))
        .route("/api/marketplace/move-to-global", post(marketplace_move_to_global))
        .route("/api/builtin-skills", get(builtin_skills_list))
        .route("/api/builtin-skills/install", post(builtin_skills_install))
        .route("/api/builtin-skills/install-all", post(builtin_skills_install_all))
        .route("/api/clawhub/scan", get(clawhub_scan))
        .route("/api/skill-files", get(list_skill_files_api))
        .route("/api/skill-file", get(get_skill_file_api))
        .route("/api/skill-file", post(upsert_skill_file_api))
        .route("/api/skill-file", delete(delete_skill_file_api))
        .route("/api/sessions", get(list_sessions))
        .route("/api/sessions/all", get(list_all_sessions).delete(delete_unified_session))
        .route("/api/sessions", post(create_session))
        .route("/api/sessions", patch(rename_session_api))
        .route("/api/sessions", delete(remove_session_api))
        .route("/api/skill-sessions", get(list_skill_sessions))
        .route("/api/skill-sessions", delete(remove_skill_session_api))
        .route("/api/skill-sessions/state", get(get_skill_session_state))
        .route("/api/sessions/resolve", post(resolve_session_api))
        .route("/api/task", post(set_task))
        .route("/api/run", post(run_agent))
        .route("/api/agent-cancel", post(cancel_agent_run))
        .route("/api/tool-cancel", post(cancel_tool_execution))
        .route("/api/missions", get(list_missions).post(create_mission))
        .route("/api/missions/sessions/state", get(get_mission_session_state))
        .route("/api/missions/{id}", get(get_mission).put(update_mission).delete(delete_mission))
        .route("/api/missions/{id}/runs", get(list_mission_runs))
        .route("/api/missions/{id}/sessions", get(list_mission_sessions))
        .route("/api/missions/{mission_id}/sessions/{session_id}", delete(delete_mission_session))
        .route("/api/missions/{id}/trigger", post(trigger_mission))
        .route("/api/chat", post(chat_handler))
        .route("/api/chat/clear", post(clear_chat_history_api))
        .route("/api/chat/compact", post(compact_chat_api))
        .route("/api/chat/system-prompt", get(get_system_prompt_api))
        .route("/api/plan/approve", post(approve_plan_handler))
        .route("/api/plan/edit", post(edit_plan_handler))
        .route("/api/plan/reject", post(reject_plan_handler))
        .route("/api/ask-user-response", post(ask_user_response_handler))
        .route("/api/pending-ask-user", get(pending_ask_user_handler))
        .route("/api/workspace/tree", get(get_agent_tree))
        .route("/api/files", get(list_files))
        .route("/api/files/search", get(search_files))
        .route("/api/file", get(read_file_api))
        .route("/api/workspace/state", get(get_workspace_state))
        .route("/api/bash", post(run_bash_api))
        .route("/api/events", get(events_handler))
        .route("/api/rtc/whip", post(rtc::whip_handler))
        .route("/api/rtc/token", get(rtc::whip_token_handler))
        .route("/api/status", get(get_status_api))
        .route("/api/health", get(health_handler))
        .route("/api/utils/pick-folder", get(pick_folder))
        .route("/api/utils/ollama-status", get(get_ollama_status))
        .route("/api/storage/roots", get(storage_roots))
        .route("/api/storage/tree", get(storage_tree))
        .route("/api/storage/file", get(storage_read_file).put(storage_write_file).delete(storage_delete_file))
        .route("/apps/{skill_name}/{*file_path}", get(serve_app_file))
        .fallback(static_handler)
        .with_state(state.clone());

    // Spawn the cron mission scheduler.
    {
        let scheduler_state = state.clone();
        tokio::spawn(mission_scheduler::mission_scheduler_loop(scheduler_state));
    }

    // Spawn remote relay tasks (heartbeat + offer polling) if remote.toml exists.
    rtc::relay::spawn_relay_tasks(state.clone());

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", port)).await?;
    let actual_port = listener.local_addr()?.port();
    info!("Server running on http://localhost:{}", actual_port);

    let task = tokio::spawn(async move {
        axum::serve(listener, app).await?;
        Ok(())
    });

    Ok(ServerHandle {
        task,
        port: actual_port,
    })
}

pub async fn start_server(
    manager: Arc<AgentManager>,
    skill_manager: Arc<crate::skills::SkillManager>,
    port: u16,
    dev_mode: bool,
    agent_events_rx: mpsc::UnboundedReceiver<crate::agent_manager::AgentEvent>,
) -> anyhow::Result<()> {
    let handle = prepare_server(manager, skill_manager, port, dev_mode, agent_events_rx).await?;
    handle.task.await??;
    Ok(())
}

/// Serve static files from an app-enabled skill's directory.
/// Route: GET /apps/{skill_name}/{*file_path}
async fn serve_app_file(
    State(state): State<Arc<ServerState>>,
    axum::extract::Path((skill_name, file_path)): axum::extract::Path<(String, String)>,
) -> Response {
    let build_err = |status: u16, msg: &str| -> Response {
        Response::builder()
            .status(status)
            .header("Content-Type", "text/plain")
            .body(axum::body::Body::from(msg.to_string()))
            .unwrap_or_else(|_| Response::new(axum::body::Body::from("internal server error")))
    };

    // Look up the skill.
    let skill = state.skill_manager.get_skill(&skill_name).await;
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

    match tokio::fs::read(&canonical_full).await {
        Ok(content) => {
            let mime = mime_guess::from_path(&full_path).first_or_octet_stream();
            Response::builder()
                .header("Content-Type", mime.as_ref())
                .header("X-Frame-Options", "ALLOWALL")
                .body(axum::body::Body::from(content))
                .unwrap_or_else(|_| Response::new(axum::body::Body::from("internal server error")))
        }
        Err(_) => build_err(404, &format!("File not found: {}", file_path_clean)),
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

    let path = if path.is_empty() { "index.html" } else { path };

    // Allow embedding in iframes (e.g. VS Code webview compact mode).
    // Disable frame restrictions entirely via X-Frame-Options and use a
    // permissive frame-ancestors that explicitly lists non-network schemes
    // used by VS Code webviews.
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

/// Optional query parameter for session-scoped SSE filtering.
#[derive(Deserialize, Default)]
struct EventsQuery {
    #[serde(default)]
    session_id: Option<String>,
}

async fn events_handler(
    State(state): State<Arc<ServerState>>,
    axum::extract::Query(query): axum::extract::Query<EventsQuery>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>> {
    let rx = state.events_tx.subscribe();
    let agent_sessions = state.agent_sessions.clone();
    let filter_session = query.session_id;
    let stream = BroadcastStream::new(rx).filter_map(move |msg| {
        let event = match msg {
            Ok(event) => event,
            Err(_) => ServerEvent::Resync {
                reason: "broadcast_lag".into(),
                lagged_count: None,
            },
        };
        let seq = state.event_seq.fetch_add(1, Ordering::Relaxed);
        let mut ui_msg = map_server_event_to_ui_message(event, seq)?;
        // Enrich with session_id from agent_sessions map if not already set.
        if ui_msg.session_id.is_none() {
            if let Some(aid) = &ui_msg.agent_id {
                if let Ok(map) = agent_sessions.read() {
                    if let Some(sid) = map.get(aid) {
                        ui_msg.session_id = Some(sid.clone());
                    } else if filter_session.is_some() {
                        tracing::debug!(
                            "SSE enrich: agent '{}' not in agent_sessions (kind={}, keys={:?})",
                            aid, ui_msg.kind, map.keys().collect::<Vec<_>>(),
                        );
                    }
                }
            }
        }
        // Server-side session filter.
        // Events with session_id "global" always pass through to all clients.
        let is_global_event = ui_msg.session_id.as_deref() == Some("global");

        if !is_global_event {
            if let Some(ref wanted) = filter_session {
                // Client requested a specific session — drop events from other sessions.
                match &ui_msg.session_id {
                    Some(event_sid) if event_sid != wanted => {
                        return None;
                    }
                    None => {
                        // Events without session_id after enrichment: this means
                        // the agent isn't registered for any session (e.g. a mission
                        // event before agent_sessions registration). Drop it to
                        // prevent cross-session bleed.
                        return None;
                    }
                    _ => {} // session matches — pass through
                }
            } else {
                // No session filter (main page with no active session).
                // Drop content events tagged with a real session_id —
                // they belong to app skills or scoped sessions, not the main view.
                if let Some(ref sid) = ui_msg.session_id {
                    if sid != "global" {
                        let is_passthrough = matches!(ui_msg.kind.as_str(),
                            "agent_status" | "activity" | "run"
                        );
                        if !is_passthrough {
                            return None;
                        }
                    }
                }
            }
        }
        let data = serde_json::to_string(&ui_msg).unwrap_or_default();
        Some(Ok(Event::default().data(data)))
    });

    // Cap connection lifetime to 30 minutes so lingering TCP sockets are reclaimed.
    // Clients (EventSource / TUI) auto-reconnect seamlessly.
    let start = std::time::Instant::now();
    let max_lifetime = std::time::Duration::from_secs(30 * 60);
    let stream = stream.take_while(move |_| start.elapsed() < max_lifetime);

    Sse::new(stream).keep_alive(
        axum::response::sse::KeepAlive::new()
            .interval(std::time::Duration::from_secs(15))
            .text("ping"),
    )
}

async fn health_handler() -> impl IntoResponse {
    axum::Json(json!({ "ok": true }))
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
            },
            ServerEvent::SubagentSpawned {
                parent_id: "ling".into(),
                subagent_id: "coder".into(),
                task: "fix bug".into(),
            },
            ServerEvent::SubagentResult {
                parent_id: "ling".into(),
                subagent_id: "coder".into(),
                outcome: crate::engine::AgentOutcome::None,
            },
            ServerEvent::AgentStatus {
                agent_id: "ling".into(),
                status: "thinking".into(),
                detail: Some("Analyzing code".into()),
                status_id: None,
                lifecycle: Some("doing".into()),
                parent_agent_id: None,
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
            },
            ServerEvent::Outcome {
                agent_id: "ling".into(),
                outcome: crate::engine::AgentOutcome::None,
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
            },
            ServerEvent::MissionTriggered {
                mission_id: "mission-1".into(),
                agent_id: "ling".into(),
                project_root: "/tmp".into(),
            },
            ServerEvent::TextSegment {
                agent_id: "ling".into(),
                text: "some text".into(),
                parent_id: None,
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
            },
            ServerEvent::ToolProgress {
                agent_id: "ling".into(),
                tool: "Bash".into(),
                line: "building...".into(),
                stream: "stdout".into(),
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
            },
            ServerEvent::ContentBlockUpdate {
                agent_id: "ling".into(),
                block_id: "cb-1".into(),
                status: Some("done".into()),
                summary: Some("Read 42 lines".into()),
                is_error: Some(false),
                parent_id: None,
                extra: None,
            },
            ServerEvent::TurnComplete {
                agent_id: "ling".into(),
                duration_ms: Some(1200),
                context_tokens: Some(5000),
                parent_id: None,
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
