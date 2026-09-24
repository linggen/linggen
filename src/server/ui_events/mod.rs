//! ServerEvent → UiEvent: the one shape every surface reads off the wire.
//!
//! The dispatch below only routes a variant to its family; each family
//! module builds its events with [`Ui`] so every event carries the same
//! seq/rev/ts header without repeating it.

mod activity;
mod chat;
mod data;
mod global;
mod pet;

use crate::engine::events::{ServerEvent, UiEvent};

pub(crate) const UI_KIND_MESSAGE: &str = "message";
pub(crate) const UI_KIND_ACTIVITY: &str = "activity";
pub(crate) const UI_KIND_QUEUE: &str = "queue";
pub(crate) const UI_KIND_RUN: &str = "run";
pub(crate) const UI_KIND_TOKEN: &str = "token";
pub(crate) const UI_KIND_TEXT_SEGMENT: &str = "text_segment";
pub(crate) const UI_KIND_CONTENT_BLOCK: &str = "content_block";
pub(crate) const UI_KIND_TURN_COMPLETE: &str = "turn_complete";
pub(crate) const UI_KIND_DEVICE_TOPIC: &str = "device_topic";
pub(crate) const UI_KIND_SKILL_SAVE_CHANGED: &str = "skill_save_changed";
pub(crate) const UI_KIND_QUESTS_CHANGED: &str = "quests_changed";

pub(crate) const UI_PHASE_SYNC: &str = "sync";
pub(crate) const UI_PHASE_OUTCOME: &str = "outcome";
pub(crate) const UI_PHASE_CONTEXT_USAGE: &str = "context_usage";
pub(crate) const UI_PHASE_SUBAGENT_SPAWNED: &str = "subagent_spawned";
pub(crate) const UI_PHASE_SUBAGENT_RESULT: &str = "subagent_result";
pub(crate) const UI_PHASE_PLAN_UPDATE: &str = "plan_update";
pub(crate) const UI_PHASE_DOING: &str = "doing";
pub(crate) const UI_PHASE_DONE: &str = "done";
pub(crate) const UI_PHASE_RESYNC: &str = "resync";

/// The session id that reaches every surface over its control channel.
const GLOBAL: &str = "global";

/// The header every event of one mapping shares.
#[derive(Clone, Copy)]
struct Ui {
    seq: u64,
    ts_ms: u64,
}

impl Ui {
    fn event(self, id: String, kind: &str) -> UiEvent {
        UiEvent {
            id,
            seq: self.seq,
            rev: self.seq,
            ts_ms: self.ts_ms,
            kind: kind.to_string(),
            phase: None,
            text: None,
            agent_id: None,
            session_id: None,
            project_root: None,
            data: None,
        }
    }
}

impl UiEvent {
    fn phase(mut self, phase: &str) -> Self {
        self.phase = Some(phase.to_string());
        self
    }
    fn text(mut self, text: impl Into<String>) -> Self {
        self.text = Some(text.into());
        self
    }
    fn agent(mut self, agent_id: impl Into<String>) -> Self {
        self.agent_id = Some(agent_id.into());
        self
    }
    fn session(mut self, session_id: Option<String>) -> Self {
        self.session_id = session_id;
        self
    }
    fn global(mut self) -> Self {
        self.session_id = Some(GLOBAL.to_string());
        self
    }
    fn project_root(mut self, project_root: Option<String>) -> Self {
        self.project_root = project_root;
        self
    }
    fn data(mut self, data: impl serde::Serialize) -> Self {
        self.data = serde_json::to_value(data).ok();
        self
    }
}

pub(crate) fn map_server_event_to_ui_message(event: ServerEvent, seq: u64) -> Option<UiEvent> {
    use ServerEvent::*;
    let ui = Ui {
        seq,
        ts_ms: crate::util::now_ts_ms(),
    };
    match &event {
        PetVoice { .. } | PetSpeak { .. } | PetExpress { .. } => pet::map(event, ui),
        Message { .. }
        | Token { .. }
        | TextSegment { .. }
        | ContentBlockStart { .. }
        | ContentBlockUpdate { .. }
        | TurnComplete { .. }
        | AskUser { .. }
        | WidgetResolved { .. }
        | Followups { .. }
        | ModelFallback { .. }
        | ToolProgress { .. } => chat::map(event, ui),
        AgentStatus { .. }
        | QueueUpdated { .. }
        | StateUpdated
        | Outcome { .. }
        | ContextUsage { .. }
        | SubagentSpawned { .. }
        | SubagentResult { .. }
        | PlanUpdate { .. }
        | Resync { .. } => activity::map(event, ui),
        SessionCreated { .. }
        | Notification(_)
        | AppLaunched { .. }
        | WorkingFolderChanged { .. }
        | RoomChat { .. }
        | SkillSaveChanged { .. }
        | QuestsChanged { .. }
        | DeviceTopic { .. } => global::map(event, ui),
        // Internal: consumed by Yinyue's watch loop, never a UI banner.
        AgentChat { .. } => None,
        // Per-peer — handled directly in forward.rs (the present flag depends on
        // the receiving peer's id), so it never goes through this shared mapping.
        YinyuePresenterChanged => None,
        // MissionTriggered is a lifecycle signal, not a chat activity.
        // SessionCreated + AgentStatus(Working) already convey the visible start
        // to the UI; routing this as activity caused a stray "Mission triggered"
        // line inside the session transcript.
        MissionTriggered { .. } => None,
        // RoomDisabled is handled directly in peer.rs — no UI event needed.
        RoomDisabled => None,
    }
}

#[cfg(test)]
mod tests;
