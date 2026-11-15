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
        })
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
            "world": crate::perception::state::read_lines(session_id, false),
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

#[derive(serde::Deserialize)]
struct RecentActivityArgs {
    /// How many entries, newest first. Default 20.
    #[serde(default)]
    limit: Option<usize>,
}

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
        "What has changed lately — deletes, syncs, backups, imports, devices coming and \
         going — newest first, each with who did it and how long ago. Covers this machine \
         AND the user's other one (their phone), listed separately, so it answers \"what \
         happened\" whichever device they meant. Call it when they ask what has been going \
         on, or when the doorbell in your prompt says something did and you need more than \
         the headline. Returns plain lines, not JSON. Last few days only; older days are gone."
    }
    fn tier(&self) -> PermissionMode {
        PermissionMode::Read
    }
    fn args_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "limit": { "type": "integer", "description": "How many entries, newest first. Default 20." }
            }
        })
    }
    fn legacy_schema_entry(&self) -> Value {
        json!({
            "name": "recent_activity",
            "args": { "limit": "number?" },
            "returns": "recent changes to this machine, newest first, as plain lines",
            "notes": "What changed here lately and who did it. The doorbell in your prompt \
                      says whether it is worth calling."
        })
    }
    async fn execute(&self, _tools: &Tools, call: ToolCall) -> Result<ToolResult> {
        let args: RecentActivityArgs =
            serde_json::from_value(call.args).unwrap_or(RecentActivityArgs { limit: None });
        let limit = args.limit.unwrap_or(20).clamp(1, 200);
        let here = crate::perception::activity::log().lines(limit);
        // The merge §6 asks for, at read time. The state block carries one line
        // of the other machine's history; this is the rest of it. Without this
        // the agent answers "what happened" from this machine's log alone, and
        // a song the user deleted on their phone a minute ago reads as nothing
        // having happened at all.
        let (peer_host, there) = crate::perception::state::peer_history();

        let mut out = Vec::new();
        if !here.is_empty() {
            out.push("On this Mac:".to_string());
            out.extend(here.iter().map(|l| format!("  {l}")));
        }
        if !there.is_empty() {
            if !out.is_empty() {
                out.push(String::new());
            }
            out.push(format!(
                "On {}:",
                peer_host.unwrap_or_else(|| "their other device".into())
            ));
            out.extend(there.iter().take(limit).map(|l| format!("  {l}")));
        }
        if out.is_empty() {
            return Ok(ToolResult::Success(
                "Nothing has changed on this machine in the last few days.".to_string(),
            ));
        }
        Ok(ToolResult::Success(out.join("\n")))
    }
}
