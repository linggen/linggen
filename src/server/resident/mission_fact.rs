//! The fact she is handed when an unattended mission run had something go
//! wrong: which mission, when, what failed, and how many runs in a row the
//! same thing has — read from the run's record, worded as facts. Whether it
//! is worth a word, and which words, is hers.
//!
//! A mission runs while nobody watches — at night, mostly. A fact handed to
//! her then would be a line said to an empty room, so it waits until quiet
//! hours are over and the person is back at a screen, and only then is she
//! woken with it. It is an unprompted source of its own kind
//! ([`Source::Fault`]): quiet hours hold it, the hourly ceiling neither
//! counts nor holds it, and nothing drops it. A wait through quiet hours is
//! written once to the activity log (verb `hold`). One held run per mission,
//! the newest: a mission failing every night while they are away is one wake
//! when they return, its streak saying how many nights. The hold lives in
//! memory; a restart loses it, never the record.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::engine::mission::record::MissionRunEntry;
use crate::extensions::missions::streak::{self, Streak};

use super::guard::{self, Held as Waits, Source};
use super::{Origin, Reach, ServerState};

/// The most failed calls named in one fact; the rest are counted.
const NAMED: usize = 3;

/// How often the hold checks whether the person is back. Cheap: a lock and
/// the presence reading, no model.
const TICK_SECS: u64 = 5;

/// How much of a mission's name its hold record carries.
const LABEL_CHARS: usize = 30;

/// A run whose fact waits for the person.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Held {
    pub mission_id: String,
    pub label: String,
    pub run_id: String,
}

static HELD: Mutex<Vec<Held>> = Mutex::new(Vec::new());

/// A mission run ended. When there is a fact to hand her, she gets it now
/// if the person is at a screen and it is not quiet hours; otherwise it
/// waits.
pub(super) fn hand_or_hold(state: &Arc<ServerState>, run: Held) {
    if for_run(state, &run).is_none() {
        return; // nothing went wrong, or a person watched it
    }
    let state = state.clone();
    tokio::spawn(async move {
        let quiet = quiet_now(&state).await;
        if quiet.is_none() && super::triggers::user_sees(&state, super::triggers::Sight::Screen) {
            tracing::info!(
                "[yinyue-watch] mission run {} had failures; handing Yinyue the fact",
                run.run_id
            );
            hand(&state, &run).await;
            return;
        }
        tracing::info!(
            "[yinyue-watch] mission run {} had failures; held until quiet hours end and the person is back",
            run.run_id
        );
        record_waits(std::slice::from_ref(&run), quiet.as_ref());
        hold(&mut HELD.lock().unwrap_or_else(|e| e.into_inner()), run);
    });
}

/// Keep `run` as its mission's one held run, replacing an older one.
fn hold(held: &mut Vec<Held>, run: Held) {
    held.retain(|h| h.mission_id != run.mission_id);
    held.push(run);
}

/// What the hold gives up when `ready`: all of it, in the order it was held;
/// nothing, when not ready, however many ticks go by.
fn take_ready(held: &mut Vec<Held>, ready: bool) -> Vec<Held> {
    if ready {
        std::mem::take(held)
    } else {
        Vec::new()
    }
}

/// Quiet hours, if they hold a fault now.
async fn quiet_now(state: &Arc<ServerState>) -> Option<Waits> {
    let pet = state.manager.get_config_snapshot().await.pet;
    super::triggers::unprompted_source(Reach::Open, Origin::Fault)
        .and_then(|source| guard::held_now(&pet, source))
}

/// The activity record of a fault waiting through quiet hours: once per run,
/// however many ticks it waits. Nothing when no guardrail holds it (it waits
/// for a screen only).
fn record_waits(runs: &[Held], quiet: Option<&Waits>) {
    let Some(held) = quiet else {
        return;
    };
    for run in runs {
        guard::record_held(&withheld(run), held);
    }
}

fn withheld(run: &Held) -> guard::Withheld<'static> {
    guard::Withheld {
        what: format!(
            "a fault in {}",
            run.label.chars().take(LABEL_CHARS).collect::<String>()
        ),
        source: Source::Fault.name(),
        key: format!("fault-{}", run.run_id),
    }
}

/// Hand her what the hold kept, once quiet hours are over and the person is
/// back at a screen — one wake per mission, in the order they were held.
pub async fn mission_fact_loop(state: Arc<ServerState>) {
    loop {
        tokio::time::sleep(Duration::from_secs(TICK_SECS)).await;
        let waiting = HELD.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if waiting.is_empty() {
            continue;
        }
        let quiet = quiet_now(&state).await;
        record_waits(&waiting, quiet.as_ref());
        let ready = quiet.is_none() && super::triggers::user_sees(&state, super::triggers::Sight::Screen);
        let taken = take_ready(&mut HELD.lock().unwrap_or_else(|e| e.into_inner()), ready);
        for run in taken {
            hand(&state, &run).await;
        }
    }
}

/// Wake her with the run's fact, built from the record as it is now.
async fn hand(state: &Arc<ServerState>, run: &Held) {
    let Some(fact) = for_run(state, run) else {
        return;
    };
    super::triggers::wake_fault(state.clone(), fact).await;
}

/// Her kickoff for `run`, or `None` when there is nothing to hand her: the
/// run is not on record, a person watched it (attended), or nothing in it
/// went wrong.
fn for_run(state: &ServerState, run: &Held) -> Option<String> {
    let runs = state.missions.list_mission_runs(&run.mission_id).ok()?;
    let entry = runs.iter().find(|r| r.run_id == run.run_id)?;
    if entry.attended || !entry.faulted() {
        return None;
    }
    let streak = streak::streak(&runs, &run.run_id);
    let later = later_clean_run(&runs, &run.run_id);
    let mut lines = fact_lines(entry, streak.as_ref());
    lines.extend(
        later.map(|at| format!("- a later run, at {}, had nothing go wrong", local_time(at))),
    );
    Some(kickoff(&run.label, entry, &lines))
}

/// When the newest run that ran is a later one than `run_id` and nothing
/// went wrong in it: when it started.
fn later_clean_run(runs: &[MissionRunEntry], run_id: &str) -> Option<u64> {
    let newest = runs.iter().find(|r| streak::ran(r))?;
    (newest.run_id != run_id && !newest.faulted()).then_some(newest.triggered_at)
}

/// The kickoff: the facts, then the herald's usual choice — a word, or nothing.
fn kickoff(label: &str, entry: &MissionRunEntry, lines: &[String]) -> String {
    format!(
        "You've been woken by a background event on the user's machine. The background \
         job \"{label}\" ran on its own at {} (status: {}). What the engine recorded about \
         the run:\n{}\n\nDecide whether it's worth telling the user. If so, say one or \
         two brief sentences in your voice. If it's not worth interrupting them, say nothing. \
         Never nag.",
        local_time(entry.triggered_at),
        entry.status,
        lines.join("\n"),
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
        let text = kickoff(
            "Harbor digest",
            &entry(),
            &fact_lines(&entry(), Some(&streak)),
        );
        assert!(
            text.contains("\"Harbor digest\" ran on its own at "),
            "{text}"
        );
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
        assert!(text.contains("say nothing"), "{text}");
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

    fn held(mission: &str, run: &str) -> Held {
        Held {
            mission_id: mission.into(),
            label: mission.into(),
            run_id: run.into(),
        }
    }

    #[test]
    fn the_hold_keeps_one_run_per_mission_the_newest() {
        let mut h = Vec::new();
        hold(&mut h, held("a", "a1"));
        hold(&mut h, held("b", "b1"));
        hold(&mut h, held("a", "a2"));
        assert_eq!(h, vec![held("b", "b1"), held("a", "a2")]);
    }

    /// What the tick asks: someone at a screen and no quiet hours.
    fn goes(rules: &guard::Rules, lines: &guard::Lines, minute: u32, at_screen: bool) -> bool {
        rules
            .check(Source::Fault, 1_790_000_000, minute, lines)
            .is_none()
            && at_screen
    }

    fn rules(per_hour: u32) -> guard::Rules {
        guard::Rules::of(&crate::config::PetConfig {
            unprompted_per_hour: per_hour,
            quiet_hours: "23:00-07:00".into(),
            ..Default::default()
        })
    }

    #[test]
    fn a_fault_at_two_waits_for_quiet_hours_to_end_and_a_screen_to_be_back() {
        let (rules, lines) = (rules(6), guard::Lines::new());
        assert!(!goes(&rules, &lines, 2 * 60, true), "02:00, someone there");
        assert!(
            !goes(&rules, &lines, 6 * 60 + 59, true),
            "a minute before 07:00"
        );
        assert!(
            !goes(&rules, &lines, 7 * 60, false),
            "quiet hours over, nobody there"
        );
        assert!(
            goes(&rules, &lines, 7 * 60, true),
            "07:00 and someone is back"
        );
    }

    #[test]
    fn with_the_hourly_ceiling_reached_a_fault_outside_quiet_hours_still_goes() {
        let rules = rules(2);
        let mut lines = guard::Lines::new();
        lines.note(1_790_000_000 - 20);
        lines.note(1_790_000_000 - 10);
        assert!(goes(&rules, &lines, 12 * 60, true));
        assert_eq!(
            rules.check(Source::Glance, 1_790_000_000, 12 * 60, &lines),
            Some(Waits::Ceiling { per_hour: 2 }),
            "the same ceiling holds a glance"
        );
    }

    #[test]
    fn a_waiting_fault_is_not_lost_while_it_waits_and_goes_in_the_order_held() {
        let mut h = Vec::new();
        hold(&mut h, held("a", "a1"));
        hold(&mut h, held("b", "b1"));
        for _ in 0..500 {
            assert!(take_ready(&mut h, false).is_empty());
        }
        assert_eq!(
            h,
            vec![held("a", "a1"), held("b", "b1")],
            "ticks took nothing"
        );
        assert_eq!(
            take_ready(&mut h, true),
            vec![held("a", "a1"), held("b", "b1")]
        );
        assert!(h.is_empty(), "handed once");
    }

    /// A wait is one record per run, short, without the run's words.
    #[test]
    fn a_wait_through_quiet_hours_is_recorded_once_per_run() {
        let quiet = Waits::Quiet {
            until: "07:00".into(),
        };
        let run = Held {
            label: "A mission with a very long name that goes on and on".into(),
            ..held("m", "wait-run-1")
        };
        let (object, detail) = guard::held_record(&withheld(&run), &quiet).unwrap();
        assert!(
            object.starts_with("a fault in A mission with a very long na"),
            "{object}"
        );
        assert!(object.ends_with("(quiet hours)"), "{object}");
        assert!(object.chars().count() <= 64, "{object}");
        assert_eq!(detail["source"], "fault");
        assert_eq!(detail["reason"], "quiet-hours");
        for _ in 0..100 {
            assert!(guard::held_record(&withheld(&run), &quiet).is_none());
        }
        let other = held("m", "wait-run-2");
        assert!(guard::held_record(&withheld(&other), &quiet).is_some());
    }

    #[test]
    fn a_later_clean_run_is_named_and_a_later_failing_one_is_not() {
        let clean = MissionRunEntry {
            run_id: "later".into(),
            triggered_at: 1_789_088_400,
            status: "completed".into(),
            ..Default::default()
        };
        let runs = vec![clean.clone(), entry()];
        assert_eq!(later_clean_run(&runs, "r"), Some(1_789_088_400));
        assert_eq!(later_clean_run(&runs[1..], "r"), None);
        let still_failing = MissionRunEntry {
            failures: vec![failed_fetch()],
            failure_count: 1,
            ..clean
        };
        assert_eq!(later_clean_run(&[still_failing, entry()], "r"), None);
    }
}
