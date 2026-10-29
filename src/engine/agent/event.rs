//! `AgentEvent` — the event bus message shape between `AgentManager`
//! and the server. Every observable change in an agent run (status,
//! tokens, content blocks, plan updates, subagent lifecycle, context
//! usage) flows through this enum.
//!
//! Lives in its own file because the enum is large (~130 lines of
//! variants) and orthogonal to the lifecycle methods on
//! `AgentManager`. The server's `events.rs` maps these into
//! `ServerEvent` for the WebRTC data channel.

use crate::engine::{AgentOutcome, Plan};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AgentEvent {
    /// Start a task in an already-created independent session.
    StartSession {
        session_id: String,
        message: String,
        from: String,
    },
    /// The pet expresses on its avatar body — a sustained mood and/or a
    /// one-shot gesture. Emitted by the `Express` tool; carries no agent text.
    /// Generic across pets/mascots (Yinyue today, others later).
    PetExpress {
        emotion: Option<String>,
        action: Option<String>,
    },
    /// The pet's voice on this machine was turned off or on (`/mute`, the
    /// `Voice` tool, Settings). Every surface hears it, so one playing her
    /// stops at once.
    PetVoice {
        muted: bool,
    },
    /// One agent sends a one-way message to another. Routed on the bus; a
    /// recipient that watches the bus (Yinyue) picks up messages addressed to it.
    AgentChat {
        from: String,
        to: String,
        message: String,
        /// Optional target app/skill — deliver into that skill's session so the
        /// recipient runs with that app's tools (e.g. "dj" to play music).
        app: Option<String>,
    },
    TaskUpdate {
        agent_id: String,
        task: String,
    },
    /// A native background task started, changed or ended. `notice`, on its
    /// end, is what the agent that started it is told in a turn of its own.
    BackgroundTask {
        agent_id: String,
        task: crate::engine::events::BackgroundTask,
        notice: Option<String>,
    },
    Outcome {
        agent_id: String,
        outcome: AgentOutcome,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        run_id: Option<String>,
    },
    Message {
        from: String,
        to: String,
        content: String,
        /// The run that wrote it. A subagent's message is told apart by
        /// `parent_id` (the UI routes it into the SubagentPane).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        run_id: Option<String>,
        /// agent_id of the parent when this comes from a subagent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_id: Option<String>,
    },
    AgentStatus {
        agent_id: String,
        status: String,
        detail: Option<String>,
        parent_id: Option<String>,
        /// Unique run_id of the emitting agent (distinguishes parallel
        /// subagents that share the same `agent_id`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        run_id: Option<String>,
        /// Unique run_id of the parent agent when this is a subagent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_run_id: Option<String>,
    },
    SubagentSpawned {
        parent_id: String,
        subagent_id: String,
        task: String,
        /// Unique run_id of the spawned subagent — the stable key for UI
        /// tracking when multiple subagents share the same `subagent_id`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subagent_run_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_run_id: Option<String>,
    },
    SubagentResult {
        parent_id: String,
        subagent_id: String,
        outcome: AgentOutcome,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subagent_run_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_run_id: Option<String>,
    },
    ContextUsage {
        agent_id: String,
        stage: String,
        message_count: usize,
        char_count: usize,
        estimated_tokens: usize,
        #[serde(default)]
        token_limit: Option<usize>,
        #[serde(default)]
        actual_prompt_tokens: Option<usize>,
        #[serde(default)]
        actual_completion_tokens: Option<usize>,
        compressed: bool,
        summary_count: usize,
    },
    TextSegment {
        agent_id: String,
        text: String,
        parent_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        run_id: Option<String>,
    },
    PlanUpdate {
        agent_id: String,
        plan: Plan,
    },
    /// What the person would most likely type next (the input's hint).
    /// `run_id`: the turn it follows — a surface drops it once a newer turn
    /// has started.
    Followups {
        agent_id: String,
        items: Vec<String>,
        run_id: Option<String>,
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        run_id: Option<String>,
    },
    /// A new content block started within the current assistant turn.
    ContentBlockStart {
        agent_id: String,
        block_id: String,
        block_type: String, // "text" | "tool_use" | "tool_result" | "thinking"
        tool: Option<String>,
        args: Option<String>,
        parent_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        run_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_run_id: Option<String>,
    },
    /// Update an existing content block (status change, result summary).
    ContentBlockUpdate {
        agent_id: String,
        block_id: String,
        status: Option<String>, // "running" | "done" | "failed"
        summary: Option<String>,
        is_error: Option<bool>,
        parent_id: Option<String>,
        /// Optional extra payload (e.g. diff data for Edit/Write tools).
        extra: Option<serde_json::Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        run_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_run_id: Option<String>,
    },
    /// Signal that the assistant turn is complete.
    TurnComplete {
        agent_id: String,
        duration_ms: Option<u64>,
        context_tokens: Option<usize>,
        parent_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        run_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_run_id: Option<String>,
    },
    StateUpdated,
}

impl AgentEvent {
    /// Stamp the run that sends this event on it — the one place a run's
    /// events get their `run_id` (and `parent_run_id`), so the UI binds each
    /// to its run's bubble. A field already set is kept; an event that
    /// belongs to no run is left alone.
    pub fn stamp_run(&mut self, run: Option<&str>, parent_run: Option<&str>) {
        let fill = |slot: &mut Option<String>, v: Option<&str>| {
            if slot.is_none() {
                *slot = v.map(str::to_string);
            }
        };
        match self {
            Self::Outcome { run_id, .. }
            | Self::Message { run_id, .. }
            | Self::TextSegment { run_id, .. }
            | Self::ToolProgress { run_id, .. }
            | Self::Followups { run_id, .. } => fill(run_id, run),
            Self::AgentStatus {
                run_id,
                parent_run_id,
                ..
            }
            | Self::ContentBlockStart {
                run_id,
                parent_run_id,
                ..
            }
            | Self::ContentBlockUpdate {
                run_id,
                parent_run_id,
                ..
            }
            | Self::TurnComplete {
                run_id,
                parent_run_id,
                ..
            } => {
                fill(run_id, run);
                fill(parent_run_id, parent_run);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::AgentEvent;

    /// A run's events take its id where they carry none, and keep one they
    /// were given (a subagent's own run inside its parent's).
    #[test]
    fn a_runs_events_are_stamped_with_it() {
        let mut seg = AgentEvent::TextSegment {
            agent_id: "ling".into(),
            text: "x".into(),
            parent_id: None,
            run_id: None,
        };
        seg.stamp_run(Some("r1"), None);
        assert!(matches!(seg, AgentEvent::TextSegment { run_id: Some(ref r), .. } if r == "r1"));
        let mut done = AgentEvent::TurnComplete {
            agent_id: "ling".into(),
            duration_ms: None,
            context_tokens: None,
            parent_id: None,
            run_id: Some("own".into()),
            parent_run_id: None,
        };
        done.stamp_run(Some("r1"), Some("p1"));
        let AgentEvent::TurnComplete {
            run_id,
            parent_run_id,
            ..
        } = done
        else {
            unreachable!()
        };
        assert_eq!(run_id.as_deref(), Some("own"));
        assert_eq!(parent_run_id.as_deref(), Some("p1"));
    }
}
