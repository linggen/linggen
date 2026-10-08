//! A run's tool calls, for the chat's history. A turn's tool calls are
//! observation rows, which the history views leave out; this brings them back
//! on the rows the chat shows, each with the status it ended in — `done` or
//! `failed`, the words the live block's update uses — so a reload shows a
//! call as it looked live.
//!
//! - A run that spoke: each of its agent's spoken rows carries, in `tools`,
//!   the calls made since the agent's previous words. Live, those calls sat
//!   just above these words in the turn's bubble.
//! - A run that ended without a reply (an engine restart, a cancel): one row
//!   of its own (`"run": "interrupted"`, its calls in `tools`), placed after
//!   the run's last row, so the chat can show the work and say it stopped. A
//!   turn the user never saw open (a `[HIDDEN]` kickoff, which may end in
//!   silence) leaves no such row. The last run of a session that is still
//!   running is the live turn, which the chat already shows as it streams.
//!
//! A call's status is its result row's: failed when the row says so
//! (`ChatMsg::failed`, saved since 2026-10-06) or, on an older row, when its
//! text is the engine's error or refusal form. A call whose result never came
//! back never ended: failed.

use super::restored_tools::{cut_args, tool_row, ToolRow};
use crate::engine::tool_render::tool_call_status;
use crate::state_fs::sessions::ChatMsg;
use serde_json::{json, Value};
use std::collections::HashMap;

/// One call of a run: the tool, its (cut) arguments, and its result —
/// `Some(failed)` once its row came back.
#[derive(Debug, PartialEq)]
pub(crate) struct RunCall {
    pub tool: String,
    pub args: Value,
    pub result: Option<bool>,
}

impl RunCall {
    /// A call with no result never ended: failed.
    pub(crate) fn failed(&self) -> bool {
        self.result.unwrap_or(true)
    }

    /// The call as a history row carries it.
    fn json(&self) -> Value {
        json!({
            "tool": self.tool,
            "args": serde_json::to_string(&self.args).unwrap_or_default(),
            "status": tool_call_status(self.failed()),
        })
    }
}

/// The calls, as a row's `tools`.
fn calls_json(calls: &[RunCall]) -> Value {
    Value::Array(calls.iter().map(RunCall::json).collect())
}

/// A spoken row's header, with the calls made before it (`tools`) if any.
pub(crate) fn with_tools(mut meta: Value, calls: &[RunCall]) -> Value {
    if calls.is_empty() {
        return meta;
    }
    if let Some(m) = meta.as_object_mut() {
        m.insert("tools".into(), calls_json(calls));
    }
    meta
}

/// How a result row saved before the `failed` flag (2026-10-06) opens when
/// the call failed or was refused: the engine's error forms, then each
/// refusal `refuse_saved` wrote (prompts/system-reminder.toml and the
/// restriction gates in engine/tool_exec.rs).
const LEGACY_FAILED_OPENINGS: &[&str] = &[
    "tool_error:",
    "tool_not_allowed:",
    "Tool execution failed for tool=",
    "Tool execution blocked for safety:",
    "Permission denied by user",
    "Permission prompt timed out.",
    "Bash command not allowed by this mission's permission tier.",
];

/// Refusals whose opening names the tool or skill: `Tool 'X' is not …`,
/// `Skill 'X' is not available to consumers.`
fn legacy_named_refusal(text: &str) -> bool {
    let named = |prefix: &str, rest: &[&str]| {
        text.strip_prefix(prefix)
            .and_then(|t| t.split_once("' "))
            .is_some_and(|(_, tail)| rest.iter().any(|r| tail.starts_with(r)))
    };
    named(
        "Tool '",
        &["is not allowed for this agent.", "is not available."],
    ) || named("Skill '", &["is not available to consumers."])
}

/// Did the call this result row answers fail? The row's flag; on a row saved
/// before the flag, its text in the engine's error or refusal form.
fn result_failed(m: &ChatMsg, text: &str) -> bool {
    m.failed
        || LEGACY_FAILED_OPENINGS.iter().any(|o| text.starts_with(o))
        || legacy_named_refusal(text)
}

/// A run's calls with no reply after them.
#[derive(Debug, PartialEq)]
pub(crate) struct UnansweredRun {
    /// Index of the run's last row in the history; the run's row goes after it.
    pub after: usize,
    pub agent: String,
    pub to: String,
    /// Timestamp of the run's first call — where it sorts in the chat.
    pub started: u64,
    pub ended: u64,
    pub calls: Vec<RunCall>,
}

impl UnansweredRun {
    /// The row's header, shaped like every other history row's, plus the run.
    pub(crate) fn meta(&self) -> Value {
        json!({
            "type": "message",
            "id": format!("run-{}", self.started),
            "from": self.agent,
            "to": self.to,
            "ts": self.started,
            "ended": self.ended,
            "task_id": null,
            "run": "interrupted",
            "tools": calls_json(&self.calls),
        })
    }
}

/// Every run's calls in a history.
#[derive(Debug, Default)]
pub(crate) struct RunCalls {
    /// A spoken row's index → the calls its agent made since its last words.
    pub answered: HashMap<usize, Vec<RunCall>>,
    pub unanswered: Vec<UnansweredRun>,
}

/// The run being read: its calls so far, and whether a hidden kickoff opened it.
struct Open {
    run: UnansweredRun,
    hidden: bool,
}

impl Open {
    fn new(i: usize, m: &ChatMsg, hidden: bool) -> Self {
        Open {
            run: UnansweredRun {
                after: i,
                agent: m.from_id.clone(),
                to: m.to_id.clone(),
                started: m.timestamp,
                ended: m.timestamp,
                calls: Vec::new(),
            },
            hidden,
        }
    }

    /// A row of the run: it ends no earlier than this.
    fn reach(&mut self, i: usize, m: &ChatMsg) {
        self.run.after = i;
        self.run.ended = m.timestamp;
    }

    /// A result row: it answers the first call of that name still waiting.
    fn result(&mut self, i: usize, m: &ChatMsg) {
        let Some(ToolRow::Result { name, text }) = tool_row(m, &self.run.agent) else {
            return;
        };
        let waiting = self
            .run
            .calls
            .iter_mut()
            .find(|c| c.result.is_none() && c.tool == name);
        if let Some(c) = waiting {
            c.result = Some(result_failed(m, &text));
        }
        self.reach(i, m);
    }
}

/// Every run's calls in `history`. `running`: the session has a run in
/// flight, so its last run is still live, not cut off.
pub(crate) fn run_calls(history: &[ChatMsg], running: bool) -> RunCalls {
    let mut out = RunCalls::default();
    let mut open: Option<Open> = None;
    // Whether the turn now being answered was opened by a hidden kickoff.
    let mut hidden_turn = false;
    let close = |open: &mut Option<Open>, out: &mut RunCalls| {
        if let Some(o) = open.take() {
            if !o.hidden {
                out.unanswered.push(o.run);
            }
        }
    };

    for (i, m) in history.iter().enumerate() {
        if m.is_observation {
            match tool_row(m, &m.from_id) {
                Some(ToolRow::Call { name, args }) => {
                    if open.as_ref().is_some_and(|o| o.run.agent != m.from_id) {
                        close(&mut open, &mut out);
                    }
                    let o = open.get_or_insert_with(|| Open::new(i, m, hidden_turn));
                    o.run.calls.push(RunCall {
                        tool: name,
                        args: cut_args(args),
                        result: None,
                    });
                    o.reach(i, m);
                }
                _ => {
                    if let Some(o) = open.as_mut() {
                        o.result(i, m);
                    }
                }
            }
            continue;
        }
        // A spoken row: the run's own words answer it (and carry its calls);
        // a new turn's opener (the user, or another agent addressing it)
        // means it never did.
        if let Some(agent) = open.as_ref().map(|o| o.run.agent.clone()) {
            if m.from_id == agent {
                let calls = open.take().map(|o| o.run.calls).unwrap_or_default();
                out.answered.insert(i, calls);
            } else if m.from_id == "user" || m.to_id == agent {
                close(&mut open, &mut out);
            }
        }
        if m.from_id == "user" {
            hidden_turn = m.content.contains("[HIDDEN]");
        }
    }
    if !running {
        close(&mut open, &mut out);
    }
    out
}

/// The history as a view maps it (`row`, given each row and the calls made
/// before it; it may drop a row), with each unanswered run's row (`run_row`)
/// placed after the run's last row.
pub(crate) fn with_run_calls<T>(
    history: Vec<ChatMsg>,
    running: bool,
    mut row: impl FnMut(ChatMsg, &[RunCall]) -> Option<T>,
    mut run_row: impl FnMut(&UnansweredRun) -> T,
) -> Vec<T> {
    let RunCalls {
        mut answered,
        unanswered,
    } = run_calls(&history, running);
    let mut runs = unanswered.iter().peekable();
    let mut out = Vec::with_capacity(history.len() + unanswered.len());
    for (i, m) in history.into_iter().enumerate() {
        let calls = answered.remove(&i).unwrap_or_default();
        out.extend(row(m, &calls));
        while let Some(run) = runs.next_if(|r| r.after == i) {
            out.push(run_row(run));
        }
    }
    out
}

/// Is a top-level run of this session in flight?
pub(crate) async fn session_running(
    manager: &crate::engine::agent::AgentManager,
    session_id: &str,
) -> bool {
    manager
        .list_agent_runs(&std::path::PathBuf::new(), Some(session_id))
        .await
        .map(|runs| {
            runs.iter().any(|r| {
                r.parent_run_id.is_none()
                    && r.status == crate::engine::agent::AgentRunStatus::Running
            })
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(from: &str, to: &str, content: &str, obs: bool, ts: u64) -> ChatMsg {
        ChatMsg {
            via: None,
            agent_id: "ling".into(),
            from_id: from.into(),
            to_id: to.into(),
            content: content.into(),
            timestamp: ts,
            is_observation: obs,
            client_id: None,
            failed: false,
        }
    }
    fn user(text: &str, ts: u64) -> ChatMsg {
        row("user", "ling", text, false, ts)
    }
    fn reply(text: &str, ts: u64) -> ChatMsg {
        row("ling", "user", text, false, ts)
    }
    fn call(name: &str, ts: u64) -> ChatMsg {
        let c = format!(r#"{{"type":"tool","tool":"{name}","args":{{"q":"x"}}}}"#);
        row("ling", "user", &c, true, ts)
    }
    fn result(name: &str, ts: u64) -> ChatMsg {
        row("system", "ling", &format!("Tool {name}: ok"), true, ts)
    }
    /// A failed call's result row, as saved since the row carries the flag.
    fn failed(name: &str, text: &str, ts: u64) -> ChatMsg {
        ChatMsg {
            failed: true,
            ..row("system", "ling", &format!("Tool {name}: {text}"), true, ts)
        }
    }
    fn unanswered_runs(h: &[ChatMsg], running: bool) -> Vec<UnansweredRun> {
        run_calls(h, running).unanswered
    }
    /// Each call's tool and the status a row carries for it.
    fn statuses(tools: &Value) -> Vec<(String, String)> {
        tools
            .as_array()
            .into_iter()
            .flatten()
            .map(|t| {
                (
                    t["tool"].as_str().unwrap_or_default().to_string(),
                    t["status"].as_str().unwrap_or_default().to_string(),
                )
            })
            .collect()
    }

    #[test]
    fn a_run_cut_off_before_its_reply_comes_back_with_its_calls() {
        let h = vec![
            user("Write this week's report.", 1),
            row(
                "system",
                "ling",
                "Starting autonomous loop for task: x",
                true,
                1,
            ),
            call("WeeklyScan", 2),
            result("WeeklyScan", 3),
            call("WebSearch", 4),
            result("WebSearch", 5),
            call("SaveWeekly", 6),
        ];
        let runs = unanswered_runs(&h, false);
        assert_eq!(runs.len(), 1);
        let r = &runs[0];
        assert_eq!((r.after, r.started, r.ended), (6, 2, 6));
        assert_eq!(r.agent, "ling");
        let calls: Vec<(&str, bool)> = r
            .calls
            .iter()
            .map(|c| (c.tool.as_str(), c.failed()))
            .collect();
        assert_eq!(
            calls,
            [
                ("WeeklyScan", false),
                ("WebSearch", false),
                ("SaveWeekly", true)
            ]
        );
        assert_eq!(r.meta()["run"], "interrupted");
        assert_eq!(r.meta()["tools"][2]["status"], "failed", "it never ended");
        assert_eq!(r.meta()["tools"][0]["args"], r#"{"q":"x"}"#);
    }

    #[test]
    fn an_answered_runs_calls_ride_on_its_words() {
        let h = vec![
            user("q", 1),
            call("Read", 2),
            result("Read", 3),
            reply("Reading on.", 4),
            call("Write", 5),
            failed("Write", "Permission denied by user: Write on 'a.txt'.", 6),
            reply("Left it.", 7),
        ];
        let rc = run_calls(&h, false);
        assert!(rc.unanswered.is_empty());
        assert_eq!(rc.answered.len(), 2);
        let tools = |i: usize| statuses(&with_tools(json!({}), &rc.answered[&i])["tools"]);
        assert_eq!(tools(3), [("Read".to_string(), "done".to_string())]);
        assert_eq!(tools(6), [("Write".to_string(), "failed".to_string())]);
        assert!(
            with_tools(json!({"from": "ling"}), &[])
                .get("tools")
                .is_none(),
            "a row with no calls before it carries none"
        );
    }

    #[test]
    fn a_stopped_call_is_failed() {
        // Cancelled mid-call: the result row says so; no reply follows.
        let h = vec![
            user("Wait a while.", 1),
            call("Bash", 2),
            failed("Bash", "tool_error: tool=Bash error=run cancelled", 3),
        ];
        let runs = unanswered_runs(&h, false);
        assert_eq!(
            statuses(&runs[0].meta()["tools"]),
            [("Bash".to_string(), "failed".to_string())]
        );
    }

    #[test]
    fn a_row_saved_before_the_flag_reads_its_text() {
        let old = |text: &str| row("system", "ling", &format!("Tool Bash: {text}"), true, 3);
        for (text, status) in [
            ("tool_error: tool=Bash error=run cancelled", "failed"),
            ("tool_not_allowed: tool=Bash reason=pet_scoped", "failed"),
            ("Tool execution failed for tool='Bash'. Error: boom.", "failed"),
            ("Tool execution blocked for safety: tool_error: tool=Bash error=precondition_failed", "failed"),
            ("Permission denied by user: Bash on 'rm -rf x'.\nThe user has denied this action.", "failed"),
            ("Permission denied by user for Bash 'ls'. User says: no", "failed"),
            ("Permission prompt timed out. Stopping.", "failed"),
            ("Bash command not allowed by this mission's permission tier. Allowed: git", "failed"),
            ("Tool 'Bash' is not allowed for this agent. Use one of [Read].", "failed"),
            ("Tool 'Bash' is not available. Allowed: Read", "failed"),
            ("Skill 'cfo' is not available to consumers.", "failed"),
            ("Bash output (exit_code: Some(0)):", "done"),
            ("Tool 'Bash' answered: fine", "done"),
        ] {
            let h = vec![user("q", 1), call("Bash", 2), old(text)];
            let runs = unanswered_runs(&h, false);
            assert_eq!(runs[0].meta()["tools"][0]["status"], status, "{text}");
        }
    }

    #[test]
    fn calls_after_the_last_words_with_no_reply_are_unanswered() {
        let h = vec![
            user("q", 1),
            reply("Let me look.", 2),
            call("Read", 3),
            result("Read", 4),
        ];
        let runs = unanswered_runs(&h, false);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].after, 3);
    }

    #[test]
    fn a_live_run_is_not_cut_off_but_an_earlier_one_is() {
        let h = vec![user("a", 1), call("Read", 2), user("b", 3), call("Grep", 4)];
        let runs = unanswered_runs(&h, true);
        assert_eq!(
            runs.len(),
            1,
            "the first run never replied; the second is live"
        );
        assert_eq!(runs[0].calls[0].tool, "Read");
        assert_eq!(unanswered_runs(&h, false).len(), 2);
    }

    #[test]
    fn a_hidden_kickoff_leaves_no_trace() {
        // It may end in SILENT: silence leaves nothing in the chat.
        let h = vec![
            user("[HIDDEN] a moment", 1),
            call("Read", 2),
            result("Read", 3),
        ];
        assert!(unanswered_runs(&h, false).is_empty());
    }

    #[test]
    fn the_run_row_lands_after_the_runs_last_row() {
        let h = vec![
            user("q", 1),
            call("Read", 2),
            result("Read", 3),
            user("again", 4),
        ];
        let rows = with_run_calls(
            h,
            false,
            |m, _| (!m.is_observation).then_some(m.content),
            |r| format!("run:{}", r.calls.len()),
        );
        assert_eq!(rows, ["q", "run:1", "again"]);
    }
}
