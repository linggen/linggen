//! What a resident agent perceives: sense and recent_activity.

use super::super::{ToolCall, ToolResult, Tools};
use super::Tool;
use crate::engine::permission::PermissionMode;
use anyhow::Result;
use async_trait::async_trait;
use chrono::Timelike;
use serde_json::{json, Value};

/// What is going on around the user this instant: are they here, how busy the
/// machine has been, what time it is.
///
/// Gathered once and used two ways — the `sense` tool serialises it, and the
/// prompt builder renders it into the session's "Right now" block. One source
/// so a companion reading the block and a companion calling the tool can never
/// disagree about the room.
pub(crate) struct RightNow {
    pub state: &'static str,
    pub focused: bool,
    pub typing: bool,
    pub idle_seconds: u64,
    pub beat_age_seconds: u64,
    pub active_runs: usize,
    pub runs_today: usize,
    pub local_time: String,
    pub hour: u32,
    pub part_of_day: &'static str,
    /// The Linggen page in front, for the world's "in front" line.
    pub front: Option<crate::engine::agent::Front>,
    pub now: u64,
}

impl RightNow {
    /// `None` when there is no agent manager to read — nothing to sense.
    pub(crate) fn gather(tools: &Tools) -> Option<Self> {
        let manager = tools.get_manager()?;
        let now = crate::util::now_ts_secs();

        // Presence — the three-state read from the latest client beat.
        let p = manager.presence_snapshot();
        let beat_age = now.saturating_sub(p.updated_at);
        let idle = now.saturating_sub(p.last_input_at);
        let state = p.state(now);

        // Other sessions' work only: counting our own run would mean reporting
        // the very turn that is asking.
        let own_session = tools.session_id.as_deref();
        let runs: Vec<_> = manager
            .run_store
            .list_runs(None)
            .into_iter()
            .filter(|r| Some(r.session_id.as_str()) != own_session)
            .collect();
        let active_runs = runs
            .iter()
            .filter(|r| matches!(r.status, crate::engine::agent::AgentRunStatus::Running))
            .count();

        let lt = chrono::Local::now();
        let secs_since_midnight =
            lt.hour() as u64 * 3600 + lt.minute() as u64 * 60 + lt.second() as u64;
        let today_start = now.saturating_sub(secs_since_midnight);
        let runs_today = runs.iter().filter(|r| r.started_at >= today_start).count();

        let hour = lt.hour();
        Some(Self {
            state,
            focused: p.focused,
            typing: p.typing,
            idle_seconds: idle,
            beat_age_seconds: beat_age,
            active_runs,
            runs_today,
            local_time: lt.format("%H:%M").to_string(),
            hour,
            part_of_day: match hour {
                5..=11 => "morning",
                12..=16 => "afternoon",
                17..=21 => "evening",
                _ => "night",
            },
            front: manager.presence_front(now),
            now,
        })
    }

    /// A look at the world that peeks — it never stamps the doorbell read.
    pub(crate) fn world_look<'a>(&self, session_id: Option<&'a str>) -> crate::perception::world::Look<'a> {
        crate::perception::world::Look {
            session_id,
            mark: false,
            door: true,
            front: Some(self.front.clone()),
            now: self.now,
        }
    }

    /// The `sense` tool's wire shape. Unchanged from when the tool built it
    /// inline — an agent that still calls `sense` sees exactly what it always
    /// did, plus the world block's own readings under `world`, so a companion
    /// reading the block and one calling the tool can never disagree about the
    /// machine either.
    pub(crate) fn to_json(&self, session_id: Option<&str>) -> Value {
        json!({
            "presence": {
                "state": self.state,
                "focused": self.focused,
                "typing": self.typing,
                "idle_seconds": self.idle_seconds,
                "beat_age_seconds": self.beat_age_seconds,
            },
            "work": { "active_runs": self.active_runs, "runs_today": self.runs_today },
            "tempo": {
                "local_time": self.local_time,
                "hour": self.hour,
                "part_of_day": self.part_of_day,
            },
            // A glance does not consume the doorbell — the turn that follows
            // still deserves to be told what happened.
            "world": crate::perception::world::read_lines(&self.world_look(session_id)),
        })
    }
}

pub struct SenseTool;
#[async_trait]
impl Tool for SenseTool {
    fn name(&self) -> &'static str {
        "sense"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["Sense"]
    }
    fn description(&self) -> &'static str {
        "Glance at the room before you react: whether the user is here (typing), \
         present but reading, or away; how busy the day is; the hour. Your \
         perception — read it to decide whether and how to respond. Never read it aloud."
    }
    fn tier(&self) -> PermissionMode {
        PermissionMode::Read
    }
    fn args_schema(&self) -> Value {
        json!({ "type": "object", "properties": {} })
    }
    fn legacy_schema_entry(&self) -> Value {
        json!({
            "name": "sense",
            "args": {},
            "returns": "presence + work + tempo snapshot (JSON)",
            "notes": "Glance at the room: presence (typing|present_reading|away), how busy \
                      the day is, the hour. Your perception — decide from it; never read it aloud."
        })
    }
    async fn execute(&self, tools: &Tools, _call: ToolCall) -> Result<ToolResult> {
        let Some(now) = RightNow::gather(tools) else {
            return Ok(ToolResult::Success(
                json!({ "error": "no environment to sense" }).to_string(),
            ));
        };
        Ok(ToolResult::Success(
            now.to_json(tools.session_id.as_deref()).to_string(),
        ))
    }
}

// ---------------------------------------------------------------------------
// recent_activity — what changed on this machine, and who did it
// ---------------------------------------------------------------------------

/// History as a tool, deliberately (`doc/perception-spec.md` §3): the log grows,
/// and an agent whose context fills with its own event stream remembers the
/// user's clicks and forgets the user. The doorbell in the prompt says whether
/// this is worth calling; this is what it costs when it is.
pub struct RecentActivityTool;
#[async_trait]
impl Tool for RecentActivityTool {
    fn name(&self) -> &'static str {
        "recent_activity"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["RecentActivity"]
    }
    fn description(&self) -> &'static str {
        "Read this Mac's activity log: what changed in Linggen and its apps, who did it, \
         and stretches of use (verb `used`: which Linggen page was in front, with \
         `seconds`). Kept 400 days. Filters: since, until (ISO time, YYYY-MM-DD, today, \
         yesterday, now, -30m, -6h, -3d; days start at local midnight), app, verb, text \
         (in object or detail), limit (rows, newest first; default 20). totals: true \
         adds counts and summed seconds per app and verb, with a readable time, for the \
         span shown; with totals, rows come only when limit is set. Read the totals; do \
         not add durations yourself. A plain call also lists the other device's latest \
         lines. Returns JSON."
    }
    fn tier(&self) -> PermissionMode {
        PermissionMode::Read
    }
    fn args_schema(&self) -> Value {
        let time = "ISO time, YYYY-MM-DD, today, yesterday, now, or -30m / -6h / -3d";
        json!({
            "type": "object",
            "properties": {
                "since": { "type": "string", "description": time },
                "until": { "type": "string", "description": format!("{time}. Default now.") },
                "app": { "type": "string", "description": "Only this app." },
                "verb": { "type": "string", "description": "Only this verb." },
                "text": { "type": "string", "description": "Only records whose object or detail contains this." },
                "limit": { "type": "integer", "description": "How many rows, newest first. Default 20, at most 200." },
                "totals": { "type": "boolean", "description": "Add counts and summed durations per app and verb for the span." }
            }
        })
    }
    fn legacy_schema_entry(&self) -> Value {
        json!({
            "name": "recent_activity",
            "args": {
                "since": "string?", "until": "string?", "app": "string?", "verb": "string?",
                "text": "string?", "limit": "number?", "totals": "boolean?"
            },
            "returns": "JSON: span, rows (newest first), totals per app and verb when asked",
            "notes": "This Mac's activity log, kept 400 days; totals sum durations so you never add them."
        })
    }
    async fn execute(&self, _tools: &Tools, call: ToolCall) -> Result<ToolResult> {
        let args: crate::perception::query::Args = match serde_json::from_value(call.args) {
            Ok(a) => a,
            Err(e) => {
                return Ok(ToolResult::Success(json!({ "error": e.to_string() }).to_string()));
            }
        };
        let out = crate::perception::query::read(&args);
        Ok(ToolResult::Success(out.to_string()))
    }
}
