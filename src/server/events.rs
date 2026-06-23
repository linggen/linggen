//! `ServerEvent` and the event-payload types broadcast over WebRTC data channels.
//!
//! Each variant maps either to an internal engine event
//! ([`from_agent_event`](ServerEvent::from_agent_event)) or to a transport-layer
//! signal (queue updates, app launches, room control).

use crate::engine::tools::AskUserQuestion;
use serde::{Deserialize, Serialize};

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
#[derive(Debug, Clone, Serialize)]
pub struct QueuedChatItem {
    pub id: String,
    pub agent_id: String,
    pub session_id: String,
    pub preview: String,
    pub timestamp: u64,
}
/// Discriminated payload for the Notification event.
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
    /// An interactive/agent run failed (the agent loop errored out). Yinyue's
    /// watch loop turns this into a brief in-character apology to the user.
    /// Carries the failing agent so she can skip her own failures (no self-loop).
    RunFailed {
        agent_id: String,
        session_id: Option<String>,
    },
}
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum ServerEvent {
    StateUpdated,
    /// The pet has something to say — a pushed "speak" cue for every surface
    /// (pet / menubar / web overlay). Carries the line and an optional emotion;
    /// the surface fetches the audio from `/api/tts` and renders the bubble +
    /// expression. Global (no session) so it reaches all of the user's surfaces.
    /// Generic across pets/mascots (Yinyue today, others later).
    PetSpeak {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        emotion: Option<String>,
    },
    /// The pet expresses on its avatar — a sustained mood and/or a one-shot
    /// gesture (no speech). Emitted by the `Express` tool. Global, like Speak.
    /// Generic across pets/mascots (Yinyue today, others later).
    PetExpress {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        emotion: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        action: Option<String>,
    },
    Message {
        from: String,
        to: String,
        content: String,
        session_id: Option<String>,
        /// Set when this message comes from a subagent — lets the UI
        /// route it into the SubagentPane instead of the main chat
        /// (matching how the activity/content-block events carry
        /// these). Both fall back to None for top-level (depth-0)
        /// messages.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        run_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_agent_id: Option<String>,
    },
    SubagentSpawned {
        parent_id: String,
        subagent_id: String,
        task: String,
        session_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subagent_run_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_run_id: Option<String>,
    },
    SubagentResult {
        parent_id: String,
        subagent_id: String,
        outcome: crate::engine::AgentOutcome,
        session_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subagent_run_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_run_id: Option<String>,
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
        session_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        run_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_run_id: Option<String>,
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
        session_id: Option<String>,
    },
    Outcome {
        agent_id: String,
        outcome: crate::engine::AgentOutcome,
        session_id: Option<String>,
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
        session_id: Option<String>,
    },
    MissionTriggered {
        mission_id: String,
        agent_id: String,
        project_root: String,
        session_id: Option<String>,
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
        session_id: Option<String>,
    },
    AskUser {
        agent_id: String,
        question_id: String,
        questions: Vec<crate::engine::tools::AskUserQuestion>,
        session_id: Option<String>,
    },
    /// Generic "widget resolved" event — dismisses any interactive widget
    /// (AskUser permission, plan approval, etc.) on all connected clients.
    WidgetResolved {
        widget_id: String,
        session_id: Option<String>,
    },
    ModelFallback {
        agent_id: String,
        preferred_model: String,
        actual_model: String,
        reason: String,
        session_id: Option<String>,
    },
    ToolProgress {
        agent_id: String,
        tool: String,
        line: String,
        stream: String, // "stdout" | "stderr"
        session_id: Option<String>,
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
        session_id: Option<String>,
    },
    /// A new content block started within the current assistant turn.
    ContentBlockStart {
        agent_id: String,
        block_id: String,
        block_type: String,
        tool: Option<String>,
        args: Option<String>,
        parent_id: Option<String>,
        session_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        run_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_run_id: Option<String>,
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
        session_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        run_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_run_id: Option<String>,
    },
    /// Signal that the assistant turn is complete (single finalizer).
    TurnComplete {
        agent_id: String,
        duration_ms: Option<u64>,
        context_tokens: Option<usize>,
        parent_id: Option<String>,
        session_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        run_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_run_id: Option<String>,
    },
    /// Working folder changed — agent cd'd to a new directory.
    WorkingFolderChanged {
        session_id: String,
        cwd: String,
        project: Option<String>,
        project_name: Option<String>,
    },
    /// Room chat message — relayed between all peers in a proxy room.
    RoomChat {
        sender_id: String,
        sender_name: String,
        avatar_url: Option<String>,
        text: String,
    },
    /// Owner disabled the room — all consumer peers should disconnect.
    RoomDisabled,
}

impl ServerEvent {
    /// Convert a 1:1 `AgentEvent` variant into the corresponding `ServerEvent`.
    /// Returns `None` for variants that require special handling (AgentStatus, TaskUpdate).
    pub(crate) fn from_agent_event(event: crate::engine::agent::AgentEvent, session_id: Option<String>) -> Option<Self> {
        use crate::engine::agent::AgentEvent;
        match event {
            AgentEvent::StateUpdated => Some(Self::StateUpdated),
            AgentEvent::Message { from, to, content, run_id, parent_id } => {
                Some(Self::Message {
                    from,
                    to,
                    content,
                    session_id,
                    run_id,
                    parent_agent_id: parent_id,
                })
            }
            AgentEvent::SubagentSpawned { parent_id, subagent_id, task, subagent_run_id, parent_run_id } => {
                Some(Self::SubagentSpawned { parent_id, subagent_id, task, session_id, subagent_run_id, parent_run_id })
            }
            AgentEvent::SubagentResult { parent_id, subagent_id, outcome, subagent_run_id, parent_run_id } => {
                Some(Self::SubagentResult { parent_id, subagent_id, outcome, session_id, subagent_run_id, parent_run_id })
            }
            AgentEvent::Outcome { agent_id, outcome } => {
                Some(Self::Outcome { agent_id, outcome, session_id })
            }
            AgentEvent::ContextUsage {
                agent_id, stage, message_count, char_count, estimated_tokens,
                token_limit, actual_prompt_tokens, actual_completion_tokens,
                compressed, summary_count,
            } => Some(Self::ContextUsage {
                agent_id, stage, message_count, char_count, estimated_tokens,
                token_limit, actual_prompt_tokens, actual_completion_tokens,
                compressed, summary_count, session_id,
            }),
            AgentEvent::PlanUpdate { agent_id, plan } => {
                Some(Self::PlanUpdate { agent_id, plan, session_id })
            }
            AgentEvent::PetExpress { emotion, action } => {
                Some(Self::PetExpress { emotion, action })
            }
            AgentEvent::TextSegment { agent_id, text, parent_id } => {
                Some(Self::TextSegment { agent_id, text, parent_id, session_id })
            }
            AgentEvent::ModelFallback { agent_id, preferred_model, actual_model, reason } => {
                Some(Self::ModelFallback { agent_id, preferred_model, actual_model, reason, session_id })
            }
            AgentEvent::ToolProgress { agent_id, tool, line, stream } => {
                Some(Self::ToolProgress { agent_id, tool, line, stream, session_id })
            }
            AgentEvent::ContentBlockStart { agent_id, block_id, block_type, tool, args, parent_id, run_id, parent_run_id } => {
                tracing::debug!("ContentBlockStart: agent={} type={} tool={:?}", agent_id, block_type, tool);
                Some(Self::ContentBlockStart { agent_id, block_id, block_type, tool, args, parent_id, session_id, run_id, parent_run_id })
            }
            AgentEvent::ContentBlockUpdate { agent_id, block_id, status, summary, is_error, parent_id, extra, run_id, parent_run_id } => {
                Some(Self::ContentBlockUpdate { agent_id, block_id, status, summary, is_error, parent_id, extra, session_id, run_id, parent_run_id })
            }
            AgentEvent::TurnComplete { agent_id, duration_ms, context_tokens, parent_id, run_id, parent_run_id } => {
                Some(Self::TurnComplete { agent_id, duration_ms, context_tokens, parent_id, session_id, run_id, parent_run_id })
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
