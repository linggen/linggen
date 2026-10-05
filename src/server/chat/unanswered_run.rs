//! A run that ended without a reply, for the chat's history. A turn's tool
//! calls are observation rows, which the history views leave out: a run cut
//! off before its final reply (an engine restart, a cancel) showed as nothing
//! at all — the user's message, then silence. Each such run comes back as one
//! row of its own (`"run": "interrupted"`, its calls in `tools`), placed after
//! the run's last row, so the chat can show the work and say it stopped.
//!
//! A run that answered keeps its look: its calls stay out, as before. A turn
//! the user never saw open (a `[HIDDEN]` kickoff, which may end in silence)
//! leaves no trace either. The last run of a session that is still running is
//! the live turn, which the chat already shows as it streams.

use super::restored_tools::{cut_args, tool_row, ToolRow};
use crate::state_fs::sessions::ChatMsg;
use serde_json::{json, Value};

/// One call of an unanswered run: the tool, its (cut) arguments, and whether
/// its result came back.
#[derive(Debug, PartialEq)]
pub(crate) struct RunCall {
    pub tool: String,
    pub args: Value,
    pub done: bool,
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
        let tools: Vec<Value> = self
            .calls
            .iter()
            .map(|c| {
                json!({
                    "tool": c.tool,
                    "args": serde_json::to_string(&c.args).unwrap_or_default(),
                    "done": c.done,
                })
            })
            .collect();
        json!({
            "type": "message",
            "id": format!("run-{}", self.started),
            "from": self.agent,
            "to": self.to,
            "ts": self.started,
            "ended": self.ended,
            "task_id": null,
            "run": "interrupted",
            "tools": tools,
        })
    }
}

/// The run being read: its calls so far, and whether a hidden kickoff opened it.
struct Open {
    run: UnansweredRun,
    hidden: bool,
}

/// Every run in `history` whose tool calls got no reply. `running`: the
/// session has a run in flight, so its last run is still live, not cut off.
pub(crate) fn unanswered_runs(history: &[ChatMsg], running: bool) -> Vec<UnansweredRun> {
    let mut out = Vec::new();
    let mut open: Option<Open> = None;
    // Whether the turn now being answered was opened by a hidden kickoff.
    let mut hidden_turn = false;
    let close = |open: &mut Option<Open>, out: &mut Vec<UnansweredRun>| {
        if let Some(o) = open.take() {
            if !o.hidden {
                out.push(o.run);
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
                    let o = open.get_or_insert_with(|| Open {
                        run: UnansweredRun {
                            after: i,
                            agent: m.from_id.clone(),
                            to: m.to_id.clone(),
                            started: m.timestamp,
                            ended: m.timestamp,
                            calls: Vec::new(),
                        },
                        hidden: hidden_turn,
                    });
                    o.run.calls.push(RunCall {
                        tool: name,
                        args: cut_args(args),
                        done: false,
                    });
                    o.run.after = i;
                    o.run.ended = m.timestamp;
                }
                _ => {
                    let Some(o) = open.as_mut() else { continue };
                    if let Some(ToolRow::Result { name, .. }) = tool_row(m, &o.run.agent) {
                        if let Some(c) = o.run.calls.iter_mut().find(|c| !c.done && c.tool == name)
                        {
                            c.done = true;
                        }
                        o.run.after = i;
                        o.run.ended = m.timestamp;
                    }
                }
            }
            continue;
        }
        // A spoken row: the run's own words answer it; a new turn's opener
        // (the user, or another agent addressing it) means it never did.
        if let Some(agent) = open.as_ref().map(|o| o.run.agent.clone()) {
            if m.from_id == agent {
                open = None;
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

/// The history as a view maps it (`row`, which may drop a row), with each
/// unanswered run's row (`run_row`) placed after the run's last row.
pub(crate) fn with_unanswered_runs<T>(
    history: Vec<ChatMsg>,
    running: bool,
    mut row: impl FnMut(ChatMsg) -> Option<T>,
    mut run_row: impl FnMut(&UnansweredRun) -> T,
) -> Vec<T> {
    let runs = unanswered_runs(&history, running);
    let mut runs = runs.iter().peekable();
    let mut out = Vec::with_capacity(history.len() + runs.len());
    for (i, m) in history.into_iter().enumerate() {
        out.extend(row(m));
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
            agent_id: "ling".into(),
            from_id: from.into(),
            to_id: to.into(),
            content: content.into(),
            timestamp: ts,
            is_observation: obs,
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
        let calls: Vec<(&str, bool)> = r.calls.iter().map(|c| (c.tool.as_str(), c.done)).collect();
        assert_eq!(
            calls,
            [
                ("WeeklyScan", true),
                ("WebSearch", true),
                ("SaveWeekly", false)
            ]
        );
        assert_eq!(r.meta()["run"], "interrupted");
        assert_eq!(r.meta()["tools"][0]["args"], r#"{"q":"x"}"#);
    }

    #[test]
    fn an_answered_run_keeps_its_look() {
        let h = vec![
            user("q", 1),
            call("Read", 2),
            result("Read", 3),
            reply("done", 4),
        ];
        assert!(unanswered_runs(&h, false).is_empty());
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
        let rows = with_unanswered_runs(
            h,
            false,
            |m| (!m.is_observation).then_some(m.content),
            |r| format!("run:{}", r.calls.len()),
        );
        assert_eq!(rows, ["q", "run:1", "again"]);
    }
}
