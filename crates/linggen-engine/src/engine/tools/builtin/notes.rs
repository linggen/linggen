//! An agent's own notes: note_write and note_done. Reading them needs no
//! tool — the open notes are in front of the agent at every turn
//! (`engine::prompt::notes`).
//!
//! Two tools, not one with an op: writing takes text and a time, finishing
//! takes only an id (`doc/app-action-spec.md` — one tool per verb unless the
//! params are the same).

use super::super::{ToolCall, ToolResult, Tools};
use super::Tool;
use crate::engine::permission::PermissionMode;
use crate::notes;
use anyhow::Result;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Deserialize)]
struct WriteArgs {
    text: String,
    #[serde(default)]
    look_again: Option<String>,
    #[serde(default)]
    id: Option<String>,
}

#[derive(Deserialize)]
struct DoneArgs {
    id: String,
}

pub struct NoteWriteTool;
#[async_trait]
impl Tool for NoteWriteTool {
    fn name(&self) -> &'static str {
        notes::WRITE_TOOL
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["NoteWrite"]
    }
    fn description(&self) -> &'static str {
        "Write a note of your own: at most 200 characters, a few short lines. Only you see \
         it; it is in front of you at every turn of yours, with its age, until you finish it \
         with note_done. You may hold 8 open notes. look_again (optional, local time) wakes \
         you once at that time with this note as the reason: +40m, +3h, +2d, HH:MM (the next \
         time the clock reads it), tomorrow HH:MM, or YYYY-MM-DD HH:MM; a time already past \
         is refused. After it wakes you the note stays open without a time. With id, \
         rewrites that note: its text and look-again are replaced. Returns the note."
    }
    fn tier(&self) -> PermissionMode {
        PermissionMode::Read
    }
    fn args_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "text": { "type": "string", "description": "The note, at most 200 characters." },
                "look_again": { "type": "string", "description": "When to wake you with it: +40m, +3h, +2d, HH:MM, tomorrow HH:MM, or YYYY-MM-DD HH:MM (local)." },
                "id": { "type": "string", "description": "An open note's id, to rewrite it." }
            },
            "required": ["text"]
        })
    }
    fn legacy_schema_entry(&self) -> Value {
        json!({
            "name": notes::WRITE_TOOL,
            "args": { "text": "string", "look_again": "string?", "id": "string?" },
            "returns": "the note (id, text, look_again) or why it was refused",
            "notes": "Your own note, in front of you each turn until note_done; look_again wakes you once."
        })
    }
    async fn execute(&self, tools: &Tools, call: ToolCall) -> Result<ToolResult> {
        let args: WriteArgs = match serde_json::from_value(call.args) {
            Ok(a) => a,
            Err(e) => return Ok(refused(format!("invalid args: {e}"))),
        };
        let agent = tools.agent_id().unwrap_or("agent").to_string();
        Ok(answer(write(&agent, args, chrono::Local::now())))
    }
}

pub struct NoteDoneTool;
#[async_trait]
impl Tool for NoteDoneTool {
    fn name(&self) -> &'static str {
        notes::DONE_TOOL
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["NoteDone"]
    }
    fn description(&self) -> &'static str {
        "Finish one of your notes by its id: it is no longer shown to you and will not wake you."
    }
    fn tier(&self) -> PermissionMode {
        PermissionMode::Read
    }
    fn args_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": { "id": { "type": "string", "description": "The note's id, such as n3." } },
            "required": ["id"]
        })
    }
    fn legacy_schema_entry(&self) -> Value {
        json!({
            "name": notes::DONE_TOOL,
            "args": { "id": "string" },
            "returns": "the finished note, or why there is none"
        })
    }
    async fn execute(&self, tools: &Tools, call: ToolCall) -> Result<ToolResult> {
        let args: DoneArgs = match serde_json::from_value(call.args) {
            Ok(a) => a,
            Err(e) => return Ok(refused(format!("invalid args: {e}"))),
        };
        let agent = tools.agent_id().unwrap_or("agent").to_string();
        let done = notes::change(&agent, |book| book.finish(&args.id));
        Ok(answer(
            done.map(|n| json!({ "finished": n.id, "text": n.text })),
        ))
    }
}

/// Write or rewrite one note of `agent`'s, at `now`.
fn write<Tz: chrono::TimeZone>(
    agent: &str,
    args: WriteArgs,
    now: chrono::DateTime<Tz>,
) -> Result<Value, String> {
    let look_again = args
        .look_again
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| crate::perception::when::resolve_ahead(s, &now).map(|t| t.timestamp() as u64))
        .transpose()?;
    let at = now.timestamp() as u64;
    let device = crate::perception::host_name();
    let note = notes::change(agent, |book| {
        match args.id.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            Some(id) => book.rewrite(id, &args.text, look_again),
            None => book.write(&args.text, look_again, &device, at),
        }
    })?;
    Ok(json!({
        "id": note.id,
        "text": note.text,
        "look_again": note.look_again_at.map(local_time),
    }))
}

/// A unix time as the agent writes one: local `YYYY-MM-DD HH:MM`.
fn local_time(at: u64) -> String {
    chrono::DateTime::from_timestamp(at as i64, 0)
        .map(|t| {
            t.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_default()
}

fn answer(result: Result<Value, String>) -> ToolResult {
    match result {
        Ok(v) => ToolResult::Success(v.to_string()),
        Err(e) => refused(e),
    }
}

fn refused(why: String) -> ToolResult {
    ToolResult::Success(json!({ "error": why }).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{FixedOffset, TimeZone};

    fn args(text: &str, look: Option<&str>, id: Option<&str>) -> WriteArgs {
        WriteArgs {
            text: text.into(),
            look_again: look.map(String::from),
            id: id.map(String::from),
        }
    }

    fn now() -> chrono::DateTime<FixedOffset> {
        FixedOffset::east_opt(0)
            .unwrap()
            .with_ymd_and_hms(2026, 11, 2, 10, 0, 0)
            .unwrap()
    }

    /// One agent id per test: the tests share this process's notes folder.
    #[test]
    fn a_note_with_a_look_again_is_kept_and_a_past_time_is_refused() {
        let agent = "test-notes-write";
        let out = write(
            agent,
            args("the parcel arrives tomorrow", Some("+40m"), None),
            now(),
        )
        .unwrap();
        let id = out["id"].as_str().unwrap().to_string();
        let book = notes::read(agent);
        assert_eq!(
            book.notes()[0].look_again_at,
            Some(now().timestamp() as u64 + 2400)
        );
        let err = write(agent, args("x", Some("2026-11-01 09:00"), None), now()).unwrap_err();
        assert!(err.contains("already passed"), "{err}");
        assert_eq!(
            notes::read(agent).notes().len(),
            1,
            "a refused time writes nothing"
        );
        // A new time on the open note.
        write(
            agent,
            args("the parcel, again", Some("tomorrow 09:00"), Some(&id)),
            now(),
        )
        .unwrap();
        let book = notes::read(agent);
        assert_eq!(book.notes().len(), 1);
        assert_eq!(book.notes()[0].text, "the parcel, again");
        assert_eq!(
            book.notes()[0].look_again_at,
            Some(now().timestamp() as u64 + 23 * 3600)
        );
        let _ = std::fs::remove_file(crate::paths::notes_dir().join(format!("{agent}.json")));
    }

    #[test]
    fn the_descriptions_state_the_limits_the_store_keeps() {
        let d = NoteWriteTool.description();
        assert!(
            d.contains(&format!("{} characters", notes::TEXT_MAX)),
            "{d}"
        );
        assert!(
            d.contains(&format!("{} open notes", notes::OPEN_MAX)),
            "{d}"
        );
        assert!(d.contains(crate::perception::when::AHEAD_FORMS), "{d}");
    }
}
