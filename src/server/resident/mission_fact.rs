//! The fact she is handed when an unattended mission run had something go
//! wrong: which mission, when, what failed, and how many runs in a row the
//! same thing has — read from the run's record, worded as facts. Whether it
//! is worth a word, and which words, is hers.

use crate::engine::mission::record::MissionRunEntry;
use crate::extensions::missions::streak::{self, Streak};

use super::ServerState;

/// The most failed calls named in one fact; the rest are counted.
const NAMED: usize = 3;

/// Her kickoff for the run `run_id` of `mission_id`, or `None` when there is
/// nothing to hand her: the run is not on record, a person watched it
/// (attended), or nothing in it went wrong.
pub(super) fn for_run(
    state: &ServerState,
    mission_id: &str,
    label: &str,
    run_id: &str,
) -> Option<String> {
    let runs = state.missions.list_mission_runs(mission_id).ok()?;
    let entry = runs.iter().find(|r| r.run_id == run_id)?;
    if entry.attended || !entry.faulted() {
        return None;
    }
    let streak = streak::streak(&runs, run_id);
    Some(kickoff(label, entry, streak.as_ref()))
}

/// The kickoff: the facts, then the herald's usual choice — a word, or SILENT.
pub(super) fn kickoff(label: &str, entry: &MissionRunEntry, streak: Option<&Streak>) -> String {
    format!(
        "You've been woken by a background event on the user's machine. The background \
         job \"{label}\" ran unattended at {} (status: {}). What the engine recorded about \
         the run:\n{}\n\nDecide whether it's worth telling the user. If so, reply with one or \
         two brief sentences in your voice. Your reply will be SPOKEN ALOUD, so write plain \
         prose, no markdown. If it's not worth interrupting them, reply with exactly the \
         single word SILENT and nothing else. Never nag.",
        local_time(entry.triggered_at),
        entry.status,
        fact_lines(entry, streak).join("\n"),
    )
}

/// One line per fact, in the record's own terms.
fn fact_lines(entry: &MissionRunEntry, streak: Option<&Streak>) -> Vec<String> {
    let mut lines: Vec<String> = entry
        .failures
        .iter()
        .take(NAMED)
        .map(failure_line)
        .collect();
    let unnamed = (entry.failure_count as usize).saturating_sub(lines.len());
    if unnamed > 0 {
        lines.push(format!("- {unnamed} more tool call(s) failed"));
    }
    if let Some(error) = entry.error.as_deref().filter(|_| entry.status == "failed") {
        lines.push(format!("- the run failed: \"{error}\""));
    }
    if entry.hit_limit {
        lines.push("- the run stopped at its step limit, before a final reply".to_string());
    }
    lines.extend(streak.map(streak_line));
    lines
}

fn failure_line(f: &crate::engine::run_faults::ToolFailure) -> String {
    match f.tail.as_deref() {
        Some(tail) => format!("- {}; the last of its error output: \"{tail}\"", f.phrase()),
        None => format!("- {}", f.phrase()),
    }
}

fn streak_line(s: &Streak) -> String {
    if s.runs <= 1 {
        return "- no earlier run is on record with the same failure".to_string();
    }
    format!(
        "- the same failure showed in each of the last {} runs, on {} different day(s), since {}",
        s.runs,
        s.days,
        streak::local_day(s.since).format("%Y-%m-%d"),
    )
}

fn local_time(secs: u64) -> String {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_opt(secs as i64, 0)
        .single()
        .map(|t| t.format("%H:%M on %Y-%m-%d").to_string())
        .unwrap_or_else(|| "an unknown time".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::run_faults::{FailureKind, ToolFailure};
    use crate::extensions::missions::streak::Signature;

    fn failed_fetch() -> ToolFailure {
        ToolFailure {
            tool: "fetch".into(),
            kind: FailureKind::Exit,
            exit: Some(3),
            tail: Some("host unreachable".into()),
        }
    }

    fn entry() -> MissionRunEntry {
        MissionRunEntry {
            run_id: "r".into(),
            triggered_at: 1_789_002_000,
            status: "completed".into(),
            failures: vec![failed_fetch()],
            failure_count: 1,
            ..Default::default()
        }
    }

    #[test]
    fn the_fact_names_the_tool_its_status_its_line_and_the_streak() {
        let streak = Streak {
            signature: Signature::Tool(failed_fetch()),
            runs: 5,
            days: 5,
            since: 1_788_656_400,
        };
        let text = kickoff("Harbor digest", &entry(), Some(&streak));
        assert!(text.contains("\"Harbor digest\" ran unattended"), "{text}");
        assert!(text.contains("(status: completed)"), "{text}");
        assert!(
            text.contains(
                "- fetch exited with status 3; the last of its error output: \"host unreachable\""
            ),
            "{text}"
        );
        assert!(
            text.contains("each of the last 5 runs, on 5 different day(s), since "),
            "{text}"
        );
        assert!(text.contains("SILENT"), "{text}");
        assert!(!text.to_lowercase().contains("important"), "{text}");
    }

    #[test]
    fn more_failures_than_named_are_counted_and_an_error_is_quoted() {
        let mut e = entry();
        e.failures = vec![failed_fetch(); 5];
        e.failure_count = 9;
        e.status = "failed".into();
        e.error = Some("provider quota reached".into());
        let lines = fact_lines(&e, None);
        assert_eq!(lines.len(), NAMED + 2, "{lines:?}");
        assert!(
            lines.contains(&"- 6 more tool call(s) failed".to_string()),
            "{lines:?}"
        );
        assert!(lines.contains(&"- the run failed: \"provider quota reached\"".to_string()));
    }

    #[test]
    fn a_first_failure_says_no_earlier_run_showed_it() {
        let streak = Streak {
            signature: Signature::Tool(failed_fetch()),
            runs: 1,
            days: 1,
            since: 1_789_002_000,
        };
        let lines = fact_lines(&entry(), Some(&streak));
        assert_eq!(
            lines.last().unwrap(),
            "- no earlier run is on record with the same failure"
        );
    }
}
