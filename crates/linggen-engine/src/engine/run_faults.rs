//! What went wrong inside one run, as facts — no judgement of whether it
//! matters.
//!
//! The engine already hands every tool result to the model; this keeps the
//! few that ended in failure after the model has read them, so a caller that
//! records the run (a mission's `runs.jsonl`) can say so. What counts:
//!
//! - a skill-declared command tool that exited non-zero, or was killed with no
//!   exit status (a timeout, a signal);
//! - a tool call that errored (bad arguments, a timeout, an execution error) —
//!   except the error a user cancel hands back, which the run's own status
//!   already says;
//! - a call the engine refused (permission, tool scope, a withheld tool).
//!
//! What does not: a built-in tool's ordinary answer (a search with no
//! matches, a `Bash` command's non-zero exit — the model's own probe, which
//! `grep` exits 1 on every miss), a cached replay of an earlier result, and
//! anything inside a delegated agent's run (its own engine's).

use serde::{Deserialize, Serialize};

/// The most failures one run keeps by name; the rest are only counted.
pub const MAX_KEPT: usize = 8;

/// The longest an error tail may be, in characters. It is a line or two of a
/// tool's error output — never its whole output, which may hold private data.
pub const TAIL_MAX_CHARS: usize = 160;

/// How many trailing lines of error output a tail keeps.
const TAIL_LINES: usize = 2;

/// How a tool call failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FailureKind {
    /// A declared command exited non-zero (or with no exit status).
    Exit,
    /// The call errored before or while running.
    Error,
    /// The engine refused to run it.
    Refused,
}

/// One tool call that ended in failure.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolFailure {
    pub tool: String,
    pub kind: FailureKind,
    /// The exit status, for [`FailureKind::Exit`]; `None` there means the
    /// command was killed (a timeout, a signal) and left none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit: Option<i32>,
    /// The last line or two of its error output, flattened and capped at
    /// [`TAIL_MAX_CHARS`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tail: Option<String>,
}

impl ToolFailure {
    /// Whether `other` is the same failure: the same tool, failing the same
    /// way with the same exit status. The tail may differ (a timestamp).
    pub fn same_as(&self, other: &ToolFailure) -> bool {
        self.tool == other.tool && self.kind == other.kind && self.exit == other.exit
    }

    /// "fetch exited with status 3", "fetch was refused" — a plain phrase.
    pub fn phrase(&self) -> String {
        match (self.kind, self.exit) {
            (FailureKind::Exit, Some(code)) => format!("{} exited with status {code}", self.tool),
            (FailureKind::Exit, None) => format!("{} was killed with no exit status", self.tool),
            (FailureKind::Error, _) => format!("{} errored", self.tool),
            (FailureKind::Refused, _) => format!("{} was refused", self.tool),
        }
    }
}

/// The failures of one run, as the engine saw them.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RunFaults {
    /// The first [`MAX_KEPT`] failures, in order.
    pub failures: Vec<ToolFailure>,
    /// Every failure, the kept ones included.
    pub count: u32,
    /// The run stopped because it reached its step limit (`max_iters`).
    pub hit_limit: bool,
}

impl RunFaults {
    pub fn is_empty(&self) -> bool {
        self.count == 0 && !self.hit_limit
    }

    pub fn push(&mut self, failure: ToolFailure) {
        self.count = self.count.saturating_add(1);
        if self.failures.len() < MAX_KEPT {
            self.failures.push(failure);
        }
    }

    /// A declared command's result: a failure unless it exited 0.
    pub fn note_exit(&mut self, tool: &str, exit: Option<i32>, stderr: &str) {
        if exit == Some(0) {
            return;
        }
        self.push(ToolFailure {
            tool: tool.to_string(),
            kind: FailureKind::Exit,
            exit,
            tail: tail(stderr),
        });
    }

    /// A tool call that errored, with the error's own text.
    pub fn note_error(&mut self, tool: &str, error: &str) {
        self.push(ToolFailure {
            tool: tool.to_string(),
            kind: FailureKind::Error,
            exit: None,
            tail: tail(error),
        });
    }

    /// A call the engine refused, with the line it showed for it.
    pub fn note_refused(&mut self, tool: &str, notice: &str) {
        self.push(ToolFailure {
            tool: tool.to_string(),
            kind: FailureKind::Refused,
            exit: None,
            tail: tail(notice),
        });
    }
}

/// The last [`TAIL_LINES`] non-empty lines of `text`, each flattened (control
/// characters become spaces — the tail is read back inside an agent's
/// prompt, where a newline would start a line of its own), joined with " / "
/// and capped at [`TAIL_MAX_CHARS`]. `None` when there is nothing to show.
pub fn tail(text: &str) -> Option<String> {
    let lines: Vec<String> = text
        .lines()
        .map(flatten)
        .filter(|l| !l.is_empty())
        .collect();
    let start = lines.len().saturating_sub(TAIL_LINES);
    let joined = lines[start..].join(" / ");
    (!joined.is_empty()).then(|| cap(&joined, TAIL_MAX_CHARS))
}

fn flatten(line: &str) -> String {
    line.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// `s` cut to `max` characters, with "…" when it was cut.
pub fn cap(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_zero_exit_is_not_a_failure() {
        let mut f = RunFaults::default();
        f.note_exit("fetch", Some(0), "warning: slow\n");
        assert!(f.is_empty());
    }

    #[test]
    fn a_non_zero_exit_keeps_the_last_two_lines_of_stderr() {
        let mut f = RunFaults::default();
        f.note_exit("fetch", Some(3), "starting\nstep 2\ncould not reach host\n\n");
        assert_eq!(f.count, 1);
        let got = &f.failures[0];
        assert_eq!(got.kind, FailureKind::Exit);
        assert_eq!(got.exit, Some(3));
        assert_eq!(got.tail.as_deref(), Some("step 2 / could not reach host"));
        assert_eq!(got.phrase(), "fetch exited with status 3");
    }

    #[test]
    fn a_killed_command_is_a_failure_with_no_status() {
        let mut f = RunFaults::default();
        f.note_exit("fetch", None, "");
        assert_eq!(f.failures[0].exit, None);
        assert_eq!(f.failures[0].tail, None);
        assert_eq!(f.failures[0].phrase(), "fetch was killed with no exit status");
    }

    #[test]
    fn the_tail_is_capped_and_one_line() {
        let long = format!("{}\u{1b}[31m\tend", "x".repeat(400));
        let t = tail(&long).unwrap();
        assert_eq!(t.chars().count(), TAIL_MAX_CHARS);
        assert!(t.ends_with('…'));
        assert!(!t.chars().any(char::is_control), "{t:?}");
    }

    #[test]
    fn only_the_first_failures_are_kept_but_all_are_counted() {
        let mut f = RunFaults::default();
        for i in 0..(MAX_KEPT + 3) {
            f.note_error(&format!("t{i}"), "boom");
        }
        assert_eq!(f.failures.len(), MAX_KEPT);
        assert_eq!(f.count as usize, MAX_KEPT + 3);
    }

    #[test]
    fn the_same_failure_is_the_same_tool_kind_and_status() {
        let a = ToolFailure {
            tool: "fetch".into(),
            kind: FailureKind::Exit,
            exit: Some(3),
            tail: Some("at 01:00".into()),
        };
        let b = ToolFailure {
            tail: Some("at 02:00".into()),
            ..a.clone()
        };
        let other_status = ToolFailure {
            exit: Some(4),
            ..a.clone()
        };
        assert!(a.same_as(&b));
        assert!(!a.same_as(&other_status));
    }
}
