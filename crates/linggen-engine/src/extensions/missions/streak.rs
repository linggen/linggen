//! How long the same failure has been happening, counted from `runs.jsonl` —
//! the one thing about a failure a model woken for a single run cannot know.

use crate::engine::mission::record::MissionRunEntry;
use crate::engine::run_faults::ToolFailure;

/// What a faulted run is known by: its first tool failure, else its own
/// error, else its step limit.
#[derive(Debug, Clone, PartialEq)]
pub enum Signature {
    Tool(ToolFailure),
    Errored,
    Limit,
}

impl Signature {
    pub fn of(run: &MissionRunEntry) -> Option<Self> {
        if let Some(first) = run.failures.first() {
            return Some(Self::Tool(first.clone()));
        }
        if run.status == "failed" {
            return Some(Self::Errored);
        }
        run.hit_limit.then_some(Self::Limit)
    }

    /// Whether `run` shows this failure too.
    fn seen_in(&self, run: &MissionRunEntry) -> bool {
        match self {
            Self::Tool(f) => run.failures.iter().any(|g| g.same_as(f)),
            Self::Errored => run.status == "failed",
            Self::Limit => run.hit_limit,
        }
    }
}

/// The same failure, run after run.
#[derive(Debug, Clone, PartialEq)]
pub struct Streak {
    pub signature: Signature,
    /// Runs in a row that showed it, this one included.
    pub runs: u32,
    /// The distinct local days those runs started on.
    pub days: u32,
    /// When the first of them started (unix seconds).
    pub since: u64,
}

/// A run that ran to an end of its own — the only kind that is evidence
/// either way. Skips, runs still going, runs a dead daemon cut off and runs
/// the person cancelled say nothing about whether the failure is still there.
fn ran(run: &MissionRunEntry) -> bool {
    !run.skipped && !matches!(run.status.as_str(), "running" | "interrupted" | "cancelled")
}

/// The streak of the failure `run_id` ended with, over `runs` (newest
/// first, as `list_mission_runs` gives them). `None` when that run is not
/// there or had nothing go wrong. A run recorded before failures were kept
/// shows none, so it ends the streak — the count never claims what the
/// record cannot say.
pub fn streak(runs: &[MissionRunEntry], run_id: &str) -> Option<Streak> {
    let at = runs.iter().position(|r| r.run_id == run_id)?;
    let signature = Signature::of(&runs[at])?;
    let mut count = 0u32;
    let mut since = runs[at].triggered_at;
    let mut days: Vec<chrono::NaiveDate> = Vec::new();
    for run in runs[at..].iter().filter(|r| ran(r)) {
        if !signature.seen_in(run) {
            break;
        }
        count += 1;
        since = run.triggered_at;
        let day = local_day(run.triggered_at);
        if !days.contains(&day) {
            days.push(day);
        }
    }
    Some(Streak {
        signature,
        runs: count,
        days: days.len() as u32,
        since,
    })
}

pub fn local_day(secs: u64) -> chrono::NaiveDate {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_opt(secs as i64, 0)
        .single()
        .map(|t| t.date_naive())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::run_faults::FailureKind;

    const DAY: u64 = 86_400;
    /// 2026-09-10 01:00 UTC — a night, whatever the local zone's offset.
    const NIGHT: u64 = 1_789_002_000;

    fn fetch_failed(code: i32) -> ToolFailure {
        ToolFailure {
            tool: "fetch".into(),
            kind: FailureKind::Exit,
            exit: Some(code),
            tail: Some("could not reach host".into()),
        }
    }

    fn run(id: &str, at: u64, status: &str, failures: Vec<ToolFailure>) -> MissionRunEntry {
        MissionRunEntry {
            run_id: id.into(),
            triggered_at: at,
            status: status.into(),
            failure_count: failures.len() as u32,
            failures,
            ..Default::default()
        }
    }

    #[test]
    fn five_nights_of_the_same_exit_count_as_five() {
        let mut runs: Vec<_> = (0..5)
            .map(|i| {
                run(
                    &format!("r{i}"),
                    NIGHT + i * DAY,
                    "completed",
                    vec![fetch_failed(3)],
                )
            })
            .collect();
        runs.insert(0, run("clean", NIGHT - DAY, "completed", vec![]));
        runs.reverse(); // newest first
        let s = streak(&runs, "r4").unwrap();
        assert_eq!((s.runs, s.days), (5, 5));
        assert_eq!(s.since, NIGHT);
        assert_eq!(s.signature, Signature::Tool(fetch_failed(3)));
    }

    #[test]
    fn a_clean_run_has_no_streak() {
        let runs = vec![run("r", NIGHT, "completed", vec![])];
        assert_eq!(streak(&runs, "r"), None);
        assert_eq!(streak(&runs, "missing"), None);
    }

    #[test]
    fn another_exit_status_ends_the_streak() {
        let runs = vec![
            run("new", NIGHT + DAY, "completed", vec![fetch_failed(3)]),
            run("old", NIGHT, "completed", vec![fetch_failed(4)]),
        ];
        assert_eq!(streak(&runs, "new").unwrap().runs, 1);
    }

    #[test]
    fn skips_and_cancels_are_passed_over_and_catch_ups_share_a_day() {
        let mut skipped = run("skip", NIGHT + DAY + 60, "skipped", vec![]);
        skipped.skipped = true;
        let runs = vec![
            run(
                "late",
                NIGHT + DAY + 3600,
                "completed",
                vec![fetch_failed(3)],
            ),
            skipped,
            run("cancel", NIGHT + DAY + 30, "cancelled", vec![]),
            run("early", NIGHT + DAY, "completed", vec![fetch_failed(3)]),
            run("night", NIGHT, "completed", vec![fetch_failed(3)]),
        ];
        let s = streak(&runs, "late").unwrap();
        assert_eq!((s.runs, s.days), (3, 2));
    }

    #[test]
    fn an_old_record_without_failures_ends_the_streak() {
        let line =
            r#"{"run_id":"old","triggered_at":1789002000,"status":"completed","skipped":false}"#;
        let old: MissionRunEntry = serde_json::from_str(line).unwrap();
        assert!(old.failures.is_empty() && !old.faulted());
        let runs = vec![
            run("new", NIGHT + DAY, "completed", vec![fetch_failed(3)]),
            old,
        ];
        assert_eq!(streak(&runs, "new").unwrap().runs, 1);
    }

    #[test]
    fn errored_runs_in_a_row_are_one_streak() {
        let mut limited = run("limit", NIGHT - DAY, "completed", vec![]);
        limited.hit_limit = true;
        let runs = vec![
            run("a", NIGHT + DAY, "failed", vec![]),
            run("b", NIGHT, "failed", vec![]),
            limited,
        ];
        let s = streak(&runs, "a").unwrap();
        assert_eq!(s.signature, Signature::Errored);
        assert_eq!(s.runs, 2);
        assert_eq!(streak(&runs, "limit").unwrap().signature, Signature::Limit);
    }
}
