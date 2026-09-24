//! The `data` payload of each UI event, as a type.
//!
//! Every payload the mapping sends is one of these structs, serialized into
//! `UiEvent.data`. Under `cargo test` each derives `TS` and exports into
//! `ui/src/types/generated/` (scripts/gen-types.sh), so the UI reads the same
//! shapes the engine writes instead of `any`.

use crate::engine::events::QueuedChatItem;
use crate::engine::tools::AskUserQuestion;
use crate::engine::{AgentOutcome, Plan};
use serde::Serialize;

#[cfg(test)]
use ts_rs::TS;

/// `message`: one chat line.
#[derive(Serialize)]
#[cfg_attr(test, derive(TS), ts(export))]
pub(super) struct MessageData {
    pub from: String,
    pub to: String,
    pub role: String,
    pub run_id: Option<String>,
    pub parent_agent_id: Option<String>,
}

/// `token` carrying a reasoning token.
#[derive(Serialize)]
#[cfg_attr(test, derive(TS), ts(export))]
pub(super) struct TokenData {
    pub thinking: bool,
}

/// `text_segment`.
#[derive(Serialize)]
#[cfg_attr(test, derive(TS), ts(export))]
pub(super) struct TextSegmentData {
    pub parent_id: Option<String>,
}

/// `content_block` / `start`.
#[derive(Serialize)]
#[cfg_attr(test, derive(TS), ts(export))]
pub(super) struct ContentBlockStartData {
    pub block_id: String,
    pub block_type: String,
    pub tool: Option<String>,
    pub args: Option<String>,
    pub parent_id: Option<String>,
    pub run_id: Option<String>,
    pub parent_run_id: Option<String>,
}

/// `content_block` / `update`. A tool's extra fields (e.g. an edit's diff)
/// ride flat beside the fixed ones and win on a clash.
#[derive(Serialize)]
#[cfg_attr(test, derive(TS), ts(export))]
pub(super) struct ContentBlockUpdateData {
    pub block_id: String,
    pub status: Option<String>,
    pub summary: Option<String>,
    pub is_error: Option<bool>,
    pub parent_id: Option<String>,
    pub run_id: Option<String>,
    pub parent_run_id: Option<String>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// `turn_complete`.
#[derive(Serialize)]
#[cfg_attr(test, derive(TS), ts(export))]
pub(super) struct TurnCompleteData {
    pub duration_ms: Option<u64>,
    pub context_tokens: Option<usize>,
    pub parent_id: Option<String>,
    pub run_id: Option<String>,
    pub parent_run_id: Option<String>,
}

/// `ask_user`.
#[derive(Serialize)]
#[cfg_attr(test, derive(TS), ts(export))]
pub(super) struct AskUserData {
    pub question_id: String,
    pub questions: Vec<AskUserQuestion>,
}

/// `widget_resolved`.
#[derive(Serialize)]
#[cfg_attr(test, derive(TS), ts(export))]
pub(super) struct WidgetResolvedData {
    pub widget_id: String,
}

/// `followups`.
#[derive(Serialize)]
#[cfg_attr(test, derive(TS), ts(export))]
pub(super) struct FollowupsData {
    pub items: Vec<String>,
    pub run_id: Option<String>,
}

/// `model_fallback`.
#[derive(Serialize)]
#[cfg_attr(test, derive(TS), ts(export))]
pub(super) struct ModelFallbackData {
    pub preferred_model: String,
    pub actual_model: String,
    pub reason: String,
}

/// `tool_progress`.
#[derive(Serialize)]
#[cfg_attr(test, derive(TS), ts(export))]
pub(super) struct ToolProgressData {
    pub tool: String,
    pub line: String,
    pub stream: String,
}

/// `activity`: an agent's status line.
#[derive(Serialize)]
#[cfg_attr(test, derive(TS), ts(export))]
pub(super) struct ActivityData {
    pub status: String,
    pub parent_id: Option<String>,
    pub run_id: Option<String>,
    pub parent_run_id: Option<String>,
}

/// `queue`.
#[derive(Serialize)]
#[cfg_attr(test, derive(TS), ts(export))]
pub(super) struct QueueData {
    pub items: Vec<QueuedChatItem>,
}

/// `run` / `outcome`.
#[derive(Serialize)]
#[cfg_attr(test, derive(TS), ts(export))]
pub(super) struct OutcomeData {
    pub outcome: AgentOutcome,
}

/// `run` / `context_usage`.
#[derive(Serialize)]
#[cfg_attr(test, derive(TS), ts(export))]
pub(super) struct ContextUsageData {
    pub agent_id: String,
    pub stage: String,
    pub message_count: usize,
    pub char_count: usize,
    pub estimated_tokens: usize,
    pub token_limit: Option<usize>,
    pub actual_prompt_tokens: Option<usize>,
    pub actual_completion_tokens: Option<usize>,
    pub compressed: bool,
    pub summary_count: usize,
}

/// `run` / `subagent_spawned`.
#[derive(Serialize)]
#[cfg_attr(test, derive(TS), ts(export))]
pub(super) struct SubagentSpawnedData {
    pub subagent_id: String,
    pub task: String,
    pub subagent_run_id: Option<String>,
    pub parent_run_id: Option<String>,
}

/// `run` / `subagent_result`.
#[derive(Serialize)]
#[cfg_attr(test, derive(TS), ts(export))]
pub(super) struct SubagentResultData {
    pub subagent_id: String,
    pub outcome: AgentOutcome,
    pub subagent_run_id: Option<String>,
    pub parent_run_id: Option<String>,
}

/// `run` / `plan_update`.
#[derive(Serialize)]
#[cfg_attr(test, derive(TS), ts(export))]
pub(super) struct PlanUpdateData {
    pub plan: Plan,
}

/// `run` / `resync`.
#[derive(Serialize)]
#[cfg_attr(test, derive(TS), ts(export))]
pub(super) struct ResyncData {
    pub reason: String,
    pub lagged_count: Option<u64>,
}

/// `notification` for a new session. (Mission notifications carry the
/// engine's `NotificationPayload` as-is.)
#[derive(Serialize)]
#[cfg_attr(test, derive(TS), ts(export))]
pub(super) struct SessionCreatedData {
    /// Always `"session_created"`.
    #[cfg_attr(test, ts(type = "\"session_created\""))]
    pub kind: String,
    pub session_id: String,
    pub title: String,
    pub creator: String,
    pub project: Option<String>,
    pub project_name: Option<String>,
    pub skill: Option<String>,
    pub mission_id: Option<String>,
}

/// `app_launched`.
#[derive(Serialize)]
#[cfg_attr(test, derive(TS), ts(export))]
pub(super) struct AppLaunchedData {
    pub skill: String,
    pub launcher: String,
    pub url: String,
    pub title: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

/// `working_folder`.
#[derive(Serialize)]
#[cfg_attr(test, derive(TS), ts(export))]
pub(super) struct WorkingFolderData {
    pub cwd: String,
    pub project: Option<String>,
    pub project_name: Option<String>,
}

/// `room_chat`.
#[derive(Serialize)]
#[cfg_attr(test, derive(TS), ts(export))]
pub(super) struct RoomChatData {
    pub sender_id: String,
    pub sender_name: String,
    pub avatar_url: Option<String>,
    pub text: String,
}

/// `skill_save_changed`.
#[derive(Serialize)]
#[cfg_attr(test, derive(TS), ts(export))]
pub(super) struct SkillSaveChangedData {
    pub skill: String,
    pub version: u64,
    pub conflicts: Vec<String>,
}

/// `quests_changed`.
#[derive(Serialize)]
#[cfg_attr(test, derive(TS), ts(export))]
pub(super) struct QuestsChangedData {
    pub app: String,
}

/// `device_topic`.
#[derive(Serialize)]
#[cfg_attr(test, derive(TS), ts(export))]
pub(super) struct DeviceTopicData {
    pub topic: String,
    pub op: String,
    #[cfg_attr(test, ts(type = "unknown"))]
    pub payload: serde_json::Value,
    pub from_device: Option<String>,
}

/// `pet_voice`.
#[derive(Serialize)]
#[cfg_attr(test, derive(TS), ts(export))]
pub(super) struct PetVoiceData {
    pub muted: bool,
}

/// `pet_speak`.
#[derive(Serialize)]
#[cfg_attr(test, derive(TS), ts(export))]
pub(super) struct PetSpeakData {
    pub text: String,
    pub emotion: Option<String>,
    pub voice: bool,
}

/// `pet_express`.
#[derive(Serialize)]
#[cfg_attr(test, derive(TS), ts(export))]
pub(super) struct PetExpressData {
    pub emotion: Option<String>,
    pub action: Option<String>,
}
