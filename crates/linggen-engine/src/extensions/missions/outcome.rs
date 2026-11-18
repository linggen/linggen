//! How a mission run ended, as its record keeps it — and the one line the
//! activity log gets when something in it went wrong.

use crate::engine::mission::record::{Mission, MissionRunEntry};
use crate::engine::run_faults::{self, RunFaults};
use crate::perception::activity_verbs::FAULT_VERB;
use crate::provider::models::RunUsage;

/// What a finished run leaves in its record.
#[derive(Debug, Default)]
pub struct RunEnd {
    pub status: &'static str,
    pub usage: Option<RunUsage>,
    pub faults: RunFaults,
    /// The engine error of a `failed` run.
    pub error: Option<String>,
}

impl RunEnd {
    pub fn failed(error: String) -> Self {
        Self {
            status: "failed",
            error: Some(error),
            ..Default::default()
        }
    }

    /// Write this end into the run's record.
    pub fn apply(self, entry: &mut MissionRunEntry) {
        entry.status = self.status.to_string();
        if self.usage.is_some() {
            entry.usage = self.usage;
        }
        entry.set_faults(self.faults);
        entry.error = self
            .error
            .as_deref()
            .and_then(run_faults::tail)
            .map(|e| run_faults::cap(&e, run_faults::TAIL_MAX_CHARS));
    }
}

/// What went wrong in a run, in a few plain words: its first failed tool
/// call, else that the run failed, else that it stopped at its step limit.
/// `None` for a run that had nothing go wrong.
pub fn what_went_wrong(entry: &MissionRunEntry) -> Option<String> {
    if let Some(first) = entry.failures.first() {
        return Some(first.phrase());
    }
    if entry.status == "failed" {
        return Some("the run failed".to_string());
    }
    entry
        .hit_limit
        .then(|| "the run stopped at its step limit".to_string())
}

/// One plain activity record for a run that had something go wrong — so
/// whoever reads the log later sees it. Nothing for a clean run.
pub fn record_fault(mission: &Mission, entry: &MissionRunEntry) {
    let Some(what) = what_went_wrong(entry) else {
        return;
    };
    let label = mission.name.as_deref().unwrap_or(&mission.id);
    crate::perception::activity::record_detail(
        "system",
        "system",
        FAULT_VERB,
        Some(format!("{label}: {what}")),
        Some(serde_json::json!({
            "mission": mission.id,
            "run_id": entry.run_id,
            "failure_count": entry.failure_count,
        })),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::run_faults::{FailureKind, ToolFailure};

    #[test]
    fn no_skill_may_declare_the_fault_verb() {
        assert!(crate::perception::activity::ENGINE_VERBS.contains(&FAULT_VERB));
    }

    #[test]
    fn a_failed_run_keeps_a_capped_last_line_of_its_error() {
        let mut entry = MissionRunEntry::default();
        let long = format!("provider said:\n{}", "e".repeat(500));
        RunEnd::failed(long).apply(&mut entry);
        assert_eq!(entry.status, "failed");
        let error = entry.error.unwrap();
        assert_eq!(error.chars().count(), run_faults::TAIL_MAX_CHARS);
        assert!(error.starts_with("provider said: / eee"), "{error}");
    }

    #[test]
    fn a_clean_end_adds_nothing_to_the_record() {
        let mut entry = MissionRunEntry::default();
        RunEnd {
            status: "completed",
            ..Default::default()
        }
        .apply(&mut entry);
        let line = serde_json::to_string(&entry).unwrap();
        for field in ["failures", "failure_count", "hit_limit", "error"] {
            assert!(!line.contains(field), "{field} in {line}");
        }
        assert_eq!(what_went_wrong(&entry), None);
    }

    #[test]
    fn the_first_failed_call_names_what_went_wrong() {
        let mut faults = RunFaults::default();
        faults.push(ToolFailure {
            tool: "fetch".into(),
            kind: FailureKind::Exit,
            exit: Some(3),
            tail: None,
        });
        let mut entry = MissionRunEntry::default();
        RunEnd {
            status: "completed",
            faults,
            ..Default::default()
        }
        .apply(&mut entry);
        assert!(entry.faulted());
        assert_eq!(
            what_went_wrong(&entry).as_deref(),
            Some("fetch exited with status 3")
        );
    }

    #[test]
    fn a_faulted_run_leaves_one_plain_activity_line_and_a_clean_one_none() {
        let md = "---\nname: Harbor digest\nschedule: \"0 1 * * *\"\nenabled: true\n---\nBody\n";
        let mission = super::super::parser::parse_mission_md("harbor-digest", md).unwrap();
        let ours = || {
            crate::perception::activity::log()
                .recent()
                .into_iter()
                .filter(|a| a.verb == FAULT_VERB)
                .filter(|a| {
                    a.object
                        .as_deref()
                        .is_some_and(|o| o.starts_with("Harbor digest"))
                })
                .collect::<Vec<_>>()
        };
        record_fault(&mission, &MissionRunEntry::default());
        assert!(ours().is_empty(), "a clean run was logged");

        let mut entry = MissionRunEntry {
            run_id: "mission-run-1".into(),
            status: "completed".into(),
            ..Default::default()
        };
        let mut faults = RunFaults::default();
        faults.note_exit("fetch", Some(3), "private detail\nhost unreachable");
        entry.set_faults(faults);
        record_fault(&mission, &entry);
        let rows = ours();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].by, "system");
        let line = rows[0].line_on("this-mac");
        assert!(
            line.starts_with(
                "the Mac recorded a failure in Harbor digest: fetch exited with status 3"
            ),
            "{line}"
        );
        assert!(!line.contains("private detail"), "{line}");
    }
}
