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
    /// The pet expresses on its avatar body — a sustained mood and/or a
    /// one-shot gesture. Emitted by the `Express` tool; carries no agent text.
    /// Generic across pets/mascots (Yinyue today, others later).
    PetExpress {
        emotion: Option<String>,
        action: Option<String>,
    },
    TaskUpdate {
        agent_id: String,
        task: String,
    },
    Outcome {
        agent_id: String,
        outcome: AgentOutcome,
    },
    Message {
        from: String,
        to: String,
        content: String,
        /// Unique run_id of the emitting agent — set for subagents so the
        /// UI can route the message into the SubagentPane instead of
        /// leaking it into the parent chat. None for top-level messages.
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
    },
    PlanUpdate {
        agent_id: String,
        plan: Plan,
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
