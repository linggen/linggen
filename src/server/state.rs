//! `ServerState` — the shared state Axum hands to every handler. Owns
//! the agent manager, event broadcaster, in-memory queues, and the
//! per-status tracking used to dedupe agent-status updates.

use crate::engine::agent::AgentManager;
use crate::server::rtc;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, Mutex};

use super::events::{AgentStatusKind, QueuedChatItem, ServerEvent};
use super::{UI_PHASE_DOING, UI_PHASE_DONE};

pub struct ServerState {
    pub manager: Arc<AgentManager>,
    pub dev_mode: bool,
    pub port: u16,
    /// Connected WebRTC peer count. Drives the idle-shutdown watcher when
    /// `idle_shutdown_secs` is set. Bumped in `rtc::peer::create_peer_inner`.
    pub active_peer_count: Arc<std::sync::atomic::AtomicUsize>,
    pub events_tx: broadcast::Sender<ServerEvent>,
    pub skills: Arc<crate::extensions::skills::SkillLoader>,
    pub prompt_store: Arc<crate::prompts::PromptStore>,
    pub queued_chats: Arc<Mutex<HashMap<String, Vec<QueuedChatItem>>>>,
    /// Senders for interrupt messages keyed by queue_key. Used to inject user
    /// messages into a running agent loop so the model can adapt mid-run.
    pub interrupt_tx: Arc<Mutex<HashMap<String, mpsc::UnboundedSender<String>>>>,
    /// Pending AskUser questions waiting for user responses.
    /// Keyed by unique question_id. The oneshot sender delivers the user's answer.
    pub pending_ask_user: Arc<Mutex<HashMap<String, crate::engine::tools::PendingAskUser>>>,
    pub(super) status_seq: AtomicU64,
    pub(crate) active_statuses: Arc<Mutex<HashMap<String, ActiveStatusRecord>>>,
    pub queue_seq: AtomicU64,
    pub event_seq: AtomicU64,
    /// Accumulated token usage per session (in-memory, resets on restart).
    /// Key: "{project_root}:{session_id}", Value: (prompt_tokens, completion_tokens).
    pub session_tokens: Arc<Mutex<HashMap<String, (usize, usize)>>>,
    /// Random token required for WHIP endpoint authentication.
    /// Generated at startup, passed to the UI via /api/status.
    pub whip_token: String,
    /// Per-session cwd for user `!` bash commands. Key = session_id.
    /// Mirrors the agent's cwd_by_session but for direct user shell commands.
    pub user_bash_cwd: Arc<Mutex<HashMap<String, std::path::PathBuf>>>,
    /// Tracks active proxy room connections (per-room model tracking).
    pub proxy_connections: Arc<rtc::proxy_room::ProxyRoomConnections>,
    /// Persistent token usage for proxy room budget enforcement.
    pub token_usage: Arc<tokio::sync::Mutex<rtc::token_store::TokenUsageStore>>,
    /// In-flight ChatGPT OAuth login task. A new login attempt aborts the
    /// prior one — without this, two `browser_login()` flows can run in
    /// parallel after a failed attempt and the second one's callback hits
    /// the first one's callback server (or vice versa), producing a
    /// "State mismatch" error. Logout also aborts.
    pub codex_login_task: Arc<tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>>,
    /// Session ids with a memory-consolidation tick in flight. The
    /// every-N-turns trigger skips a session already present here so a slow
    /// tick is never overlapped by the next qualifying turn (mirrors the
    /// mission scheduler's per-mission `running` guard). In-memory only —
    /// a daemon restart clears it, which is correct: no tick survives a
    /// restart, so none can still be "running".
    pub consolidation_active: Arc<Mutex<HashSet<String>>>,
}
#[derive(Debug, Clone)]
pub(crate) struct ActiveStatusRecord {
    status_id: String,
    pub(crate) status: AgentStatusKind,
    detail: Option<String>,
}
impl ServerState {
    /// Back-compat shim — callers that don't yet thread run_id can keep using
    /// this. Internally forwards to `send_agent_status_with_ids` with None.
    pub async fn send_agent_status(
        &self,
        agent_id: String,
        status: AgentStatusKind,
        detail: Option<String>,
        parent_agent_id: Option<String>,
        session_id: Option<String>,
    ) {
        self.send_agent_status_with_ids(
            agent_id, status, detail, parent_agent_id, session_id, None, None,
        )
        .await
    }

    /// Full variant that carries the emitting agent's run_id and its
    /// parent's run_id so the UI can route status to the right subagent
    /// even when multiple subagents share the same `agent_id`.
    pub async fn send_agent_status_with_ids(
        &self,
        agent_id: String,
        status: AgentStatusKind,
        detail: Option<String>,
        parent_agent_id: Option<String>,
        session_id: Option<String>,
        run_id: Option<String>,
        parent_run_id: Option<String>,
    ) {
        let mut done_event: Option<ServerEvent> = None;
        let mut status_id: Option<String> = None;
        let mut lifecycle: Option<String> = None;

        // Key by session_id|agent_id so concurrent sessions don't clobber each other.
        let status_key = match &session_id {
            Some(sid) => format!("{}|{}", sid, agent_id),
            None => agent_id.clone(),
        };

        {
            let mut active = self.active_statuses.lock().await;
            if status == AgentStatusKind::Idle {
                if let Some(prev) = active.remove(&status_key) {
                    done_event = Some(ServerEvent::AgentStatus {
                        agent_id: agent_id.clone(),
                        status: prev.status.as_str().to_string(),
                        detail: prev.detail,
                        status_id: Some(prev.status_id),
                        lifecycle: Some(UI_PHASE_DONE.to_string()),
                        parent_agent_id: parent_agent_id.clone(),
                        session_id: session_id.clone(),
                        run_id: run_id.clone(),
                        parent_run_id: parent_run_id.clone(),
                    });
                }
            } else {
                if let Some(prev) = active.get(&status_key).cloned() {
                    if prev.status != status {
                        done_event = Some(ServerEvent::AgentStatus {
                            agent_id: agent_id.clone(),
                            status: prev.status.as_str().to_string(),
                            detail: prev.detail,
                            status_id: Some(prev.status_id),
                            lifecycle: Some(UI_PHASE_DONE.to_string()),
                            parent_agent_id: parent_agent_id.clone(),
                            session_id: session_id.clone(),
                            run_id: run_id.clone(),
                            parent_run_id: parent_run_id.clone(),
                        });
                        active.remove(&status_key);
                    } else {
                        status_id = Some(prev.status_id.clone());
                        lifecycle = Some(UI_PHASE_DOING.to_string());
                        active.insert(
                            status_key.clone(),
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
                        status_key.clone(),
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
            session_id,
            run_id,
            parent_run_id,
        });
    }
}
