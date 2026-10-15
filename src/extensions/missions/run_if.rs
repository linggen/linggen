//! A mission's `run-if` precondition: a cheap shell check run right before a
//! scheduled dispatch. Exit 0 runs the mission; anything else — a no, a
//! timeout, a spawn failure — records a skip with a reason, before any
//! session or model call. A person's trigger is never gated. See
//! `doc/mission-spec.md`, "run-if".

use super::MissionLoader;
use crate::engine::mission::record::{Mission, MissionRunEntry, SkipKind};
use crate::engine::skill::Skill;
use crate::engine::skill_tool::SkillToolDef;
use crate::engine::tools::ToolResult;
use std::path::{Path, PathBuf};
use tracing::info;

/// How long a check may take before it counts as unanswered.
pub(super) const TIMEOUT_MS: u64 = 10_000;

/// A reason is one short line.
const REASON_MAX_CHARS: usize = 120;

/// What fired a dispatch. Only `Scheduled` is gated by `run-if`.
pub(super) enum Trigger {
    /// Cron fire or catch-up.
    Scheduled,
    /// A person's "Run now" (`POST /api/missions/{id}/trigger`), optionally
    /// scoped to a day, optionally attended.
    Manual { day: Option<String>, attended: bool },
}

/// The check's answer.
#[derive(Debug, PartialEq)]
pub(super) enum Verdict {
    Go,
    Skip(String),
}

/// Gate a dispatch: `true` = go on. A scheduled trigger of a mission with a
/// `run-if` runs the check; a no is recorded as a `run-if` skip.
pub(super) async fn admit(
    missions: &MissionLoader,
    mission: &Mission,
    cwd: &Path,
    skill: Option<&Skill>,
    trigger: &Trigger,
    timeout_ms: u64,
) -> bool {
    let Trigger::Scheduled = trigger else {
        return true;
    };
    let Some(cmd) = mission.run_if.clone() else {
        return true;
    };
    let tool = check_tool(cmd, skill_dir(missions, mission, skill), skill, timeout_ms);
    let cwd = cwd.to_path_buf();
    let verdict = tokio::task::spawn_blocking(move || check(&tool, &cwd))
        .await
        .unwrap_or_else(|e| Verdict::Skip(format!("run-if check failed: {e}")));
    let Verdict::Skip(reason) = verdict else {
        return true;
    };
    info!("Mission '{}': run-if said no — {}", mission.id, reason);
    record_skip(missions, &mission.id, reason);
    false
}

/// The owning skill's folder, for `SKILL_DIR`: the loaded skill's, else
/// where the loader keeps that skill. None for a user mission.
fn skill_dir(
    missions: &MissionLoader,
    mission: &Mission,
    skill: Option<&Skill>,
) -> Option<PathBuf> {
    skill
        .and_then(|s| s.skill_dir.clone())
        .or_else(|| mission.skill.as_ref().map(|s| missions.skills_dir.join(s)))
}

/// The check as a skill shell command, so it gets exactly what a skill
/// tool's command gets: `$SKILL_DIR`, `PATH`, the skill's senses,
/// `LINGGEN_LANGUAGE`, a process-group kill on timeout.
fn check_tool(
    cmd: String,
    skill_dir: Option<PathBuf>,
    skill: Option<&Skill>,
    timeout_ms: u64,
) -> SkillToolDef {
    SkillToolDef {
        name: "run-if".into(),
        description: "A mission's run-if check.".into(),
        cmd,
        endpoint: None,
        tier: Some("read".into()),
        args: Default::default(),
        returns: None,
        timeout_ms,
        max_output_bytes: 4096,
        page_only: true,
        others_read: Vec::new(),
        pet: false,
        remote: false,
        skill_name: skill.map(|s| s.name.clone()),
        skill_dir,
        senses: skill.map(|s| s.senses.clone()).unwrap_or_default(),
    }
}

/// Run the check (blocking) in `cwd`.
fn check(tool: &SkillToolDef, cwd: &Path) -> Verdict {
    let mut env: Vec<(String, String)> = crate::reach::tool_env().into_iter().collect();
    if let Some(dir) = &tool.skill_dir {
        env.push(("SKILL_DIR".into(), dir.to_string_lossy().into_owned()));
    }
    match tool.execute(&serde_json::json!({}), cwd, &env) {
        Ok(ToolResult::CommandOutput {
            exit_code, stdout, ..
        }) => verdict_of(exit_code, &stdout),
        Ok(_) => Verdict::Skip("run-if check failed".into()),
        Err(e) => Verdict::Skip(cap(&format!("run-if check failed: {e}"))),
    }
}

fn verdict_of(exit_code: Option<i32>, stdout: &str) -> Verdict {
    match exit_code {
        Some(0) => Verdict::Go,
        None => Verdict::Skip("run-if check failed: no answer in time".into()),
        Some(code) => Verdict::Skip(reason_of(stdout).unwrap_or(format!("run-if exited {code}"))),
    }
}

/// The first non-empty stdout line, trimmed and capped.
fn reason_of(stdout: &str) -> Option<String> {
    let line = stdout.lines().map(str::trim).find(|l| !l.is_empty())?;
    Some(cap(line))
}

fn cap(text: &str) -> String {
    if text.chars().count() <= REASON_MAX_CHARS {
        return text.to_string();
    }
    let cut: String = text.chars().take(REASON_MAX_CHARS - 1).collect();
    format!("{}…", cut.trim_end())
}

fn record_skip(missions: &MissionLoader, mission_id: &str, reason: String) {
    let entry = MissionRunEntry {
        run_id: super::scheduler::new_run_id(),
        triggered_at: crate::util::now_ts_secs(),
        status: "skipped".into(),
        skipped: true,
        skip: Some(SkipKind::RunIf),
        reason: Some(reason),
        ..Default::default()
    };
    let _ = missions.append_mission_run(mission_id, &entry);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (MissionLoader, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        (MissionLoader::with_dir(dir.path().to_path_buf()), dir)
    }

    fn mission_with(store: &MissionLoader, run_if: &str) -> Mission {
        let md =
            format!("---\nschedule: \"0 9 * * *\"\nenabled: true\nrun-if: '{run_if}'\n---\nBody\n");
        store.write_mission_raw("m", &md).unwrap()
    }

    async fn gate(store: &MissionLoader, m: &Mission, trigger: Trigger, ms: u64) -> bool {
        let cwd = std::env::temp_dir();
        admit(store, m, &cwd, None, &trigger, ms).await
    }

    fn verdict(cmd: &str, ms: u64) -> Verdict {
        check(
            &check_tool(cmd.into(), None, None, ms),
            &std::env::temp_dir(),
        )
    }

    #[test]
    fn run_if_round_trips_through_the_file() {
        let (store, _dir) = store();
        let m = mission_with(&store, "perl x.pl --has-holdings");
        assert_eq!(m.run_if.as_deref(), Some("perl x.pl --has-holdings"));
        let md = super::super::parser::mission_to_md(&m);
        assert!(md.contains("run-if: perl x.pl --has-holdings"), "{md}");
    }

    #[test]
    fn exit_zero_goes_and_a_no_carries_its_first_line() {
        assert_eq!(verdict("true", 5_000), Verdict::Go);
        assert_eq!(
            verdict("printf '\\n  no holdings  \\nmore\\n'; exit 1", 5_000),
            Verdict::Skip("no holdings".into())
        );
        assert_eq!(
            verdict("exit 3", 5_000),
            Verdict::Skip("run-if exited 3".into())
        );
    }

    #[test]
    fn a_long_reason_is_capped() {
        let Verdict::Skip(reason) = verdict("printf 'x%.0s' $(seq 300); exit 1", 5_000) else {
            panic!("expected a skip");
        };
        assert_eq!(reason.chars().count(), REASON_MAX_CHARS);
        assert!(reason.ends_with('…'));
    }

    #[test]
    fn a_check_that_never_answers_is_a_skip() {
        let start = std::time::Instant::now();
        let Verdict::Skip(reason) = verdict("sleep 5", 200) else {
            panic!("expected a skip");
        };
        assert!(reason.starts_with("run-if check failed"), "{reason}");
        assert!(start.elapsed() < std::time::Duration::from_secs(3));
    }

    #[test]
    fn the_check_gets_a_skill_commands_env() {
        let dir = tempfile::tempdir().unwrap();
        let tool = check_tool(
            "test \"$SKILL_DIR\" = \"$(pwd)\" && test -n \"$LINGGEN_LANGUAGE\"".into(),
            Some(dir.path().canonicalize().unwrap()),
            None,
            5_000,
        );
        assert_eq!(
            check(&tool, &dir.path().canonicalize().unwrap()),
            Verdict::Go
        );
    }

    #[tokio::test]
    async fn a_passing_check_dispatches_and_records_nothing() {
        let (store, _dir) = store();
        let m = mission_with(&store, "true");
        assert!(gate(&store, &m, Trigger::Scheduled, 5_000).await);
        assert!(store.list_mission_runs("m").unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_failing_check_records_a_run_if_skip_without_a_session() {
        let (store, _dir) = store();
        let m = mission_with(&store, "echo no holdings; exit 1");
        assert!(!gate(&store, &m, Trigger::Scheduled, 5_000).await);
        let runs = store.list_mission_runs("m").unwrap();
        assert_eq!(runs.len(), 1);
        let run = &runs[0];
        assert!(run.skipped);
        assert_eq!(run.status, "skipped");
        assert_eq!(run.skip, Some(SkipKind::RunIf));
        assert_eq!(run.reason.as_deref(), Some("no holdings"));
        assert!(run.session_id.is_none());
        assert!(run.fills_slot());
    }

    #[tokio::test]
    async fn a_timed_out_check_skips() {
        let (store, _dir) = store();
        let m = mission_with(&store, "sleep 5");
        assert!(!gate(&store, &m, Trigger::Scheduled, 200).await);
        let runs = store.list_mission_runs("m").unwrap();
        assert_eq!(runs[0].skip, Some(SkipKind::RunIf));
    }

    #[tokio::test]
    async fn a_manual_trigger_ignores_run_if() {
        let (store, _dir) = store();
        let m = mission_with(&store, "false");
        let manual = Trigger::Manual {
            day: None,
            attended: false,
        };
        assert!(gate(&store, &m, manual, 5_000).await);
        assert!(store.list_mission_runs("m").unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_skill_missions_check_finds_its_skill_dir_without_the_loaded_skill() {
        let dir = tempfile::tempdir().unwrap();
        let skills = dir.path().join("skills");
        let def = skills.join("cfo/missions/weekly");
        std::fs::create_dir_all(&def).unwrap();
        std::fs::write(skills.join("cfo/SKILL.md"), "---\nname: cfo\n---\n").unwrap();
        std::fs::write(
            def.join("mission.md"),
            "---\nschedule: \"0 19 * * 0\"\nenabled: true\nrun-if: 'test -f \"$SKILL_DIR/SKILL.md\"'\n---\nBody\n",
        )
        .unwrap();
        let store = MissionLoader::with_dirs(dir.path().join("missions"), skills);
        let m = store.reload_one("cfo:weekly").unwrap();
        assert_eq!(m.skill.as_deref(), Some("cfo"));
        assert!(gate(&store, &m, Trigger::Scheduled, 5_000).await);
        assert!(store.list_mission_runs("cfo:weekly").unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_mission_without_run_if_always_goes() {
        let (store, _dir) = store();
        let m = store
            .write_mission_raw(
                "m",
                "---\nschedule: \"0 9 * * *\"\nenabled: true\n---\nBody\n",
            )
            .unwrap();
        assert!(gate(&store, &m, Trigger::Scheduled, 5_000).await);
    }
}
