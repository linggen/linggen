//! A mission's `prepare` step: work the engine does itself when a run
//! starts, before the run's first turn — no model, no tool call, nothing the
//! run's grants have to allow. The memory scan's walk of the session logs
//! (`ling mem scan $DAY`) is the first user: the run then only reads what it
//! wrote. See `doc/mission-spec.md`, "prepare".
//!
//! The step is arguments to the engine's own binary (the running `ling`,
//! never one found on `PATH`), not a shell line: no shell parses it, so a
//! mission file can name a `ling` verb and nothing else. `$DAY` in an
//! argument is the run's day. It runs on every trigger, a person's too — the
//! run needs what it makes.

use super::outcome::RunEnd;
use crate::engine::mission::record::Mission;
use crate::engine::run_faults::RunFaults;
use std::path::Path;
use std::time::Duration;

/// How long a step may take. A day of session logs is seconds; a stuck step
/// must not hold the mission's one-run slot.
const TIMEOUT: Duration = Duration::from_secs(300);

/// The name a failure is recorded under.
const STEP: &str = "prepare";

/// Run `mission`'s step, if it has one, in `cwd`. `Err` is the end the run
/// gets instead of starting: failed, with the step's exit status and the
/// last lines of its stderr.
pub(super) async fn run(mission: &Mission, day: Option<&str>, cwd: &Path) -> Result<(), RunEnd> {
    if mission.prepare.is_empty() {
        return Ok(());
    }
    let args = arguments(&mission.prepare, day)?;
    let exe = std::env::current_exe().map_err(|e| failed_to_start(&e.to_string()))?;
    let mut cmd = tokio::process::Command::new(exe);
    cmd.args(&args)
        .current_dir(cwd)
        .env("PATH", crate::util::shell_path())
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    let output = match tokio::time::timeout(TIMEOUT, cmd.output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(e)) => return Err(failed_to_start(&e.to_string())),
        Err(_) => return Err(exited(None, "prepare: no answer in time")),
    };
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(exited(output.status.code(), &stderr))
}

/// The step's arguments with `$DAY` filled in. A step that names `$DAY` on
/// a run with no day cannot run.
fn arguments(prepare: &[String], day: Option<&str>) -> Result<Vec<String>, RunEnd> {
    let needs_day = prepare.iter().any(|a| a.contains("$DAY"));
    match day {
        Some(day) => Ok(prepare.iter().map(|a| a.replace("$DAY", day)).collect()),
        None if needs_day => Err(failed_to_start(
            "this mission runs for a day; none was given",
        )),
        None => Ok(prepare.to_vec()),
    }
}

fn exited(code: Option<i32>, stderr: &str) -> RunEnd {
    let mut faults = RunFaults::default();
    faults.note_exit(STEP, code, stderr);
    end(faults, "the prepare step failed")
}

fn failed_to_start(error: &str) -> RunEnd {
    let mut faults = RunFaults::default();
    faults.note_error(STEP, error);
    end(faults, error)
}

fn end(faults: RunFaults, error: &str) -> RunEnd {
    RunEnd {
        faults,
        ..RunEnd::failed(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str], day: Option<&str>) -> Result<Vec<String>, RunEnd> {
        let list: Vec<String> = list.iter().map(|s| s.to_string()).collect();
        arguments(&list, day)
    }

    #[test]
    fn the_day_is_filled_in() {
        let got = args(&["mem", "scan", "$DAY"], Some("2026-11-10"))
            .ok()
            .unwrap();
        assert_eq!(got, ["mem", "scan", "2026-11-10"]);
    }

    #[test]
    fn a_step_that_needs_a_day_fails_without_one() {
        let end = args(&["mem", "scan", "$DAY"], None).err().unwrap();
        assert_eq!(end.status, "failed");
        assert_eq!(end.faults.failures[0].tool, "prepare");
    }

    #[test]
    fn a_failed_step_keeps_its_status_and_stderr_tail() {
        let end = exited(
            Some(1),
            "starting\nling mem scan needs jq, which is not on PATH\n",
        );
        assert_eq!(end.status, "failed");
        let failure = &end.faults.failures[0];
        assert_eq!((failure.tool.as_str(), failure.exit), ("prepare", Some(1)));
        assert_eq!(
            failure.tail.as_deref(),
            Some("starting / ling mem scan needs jq, which is not on PATH")
        );
    }
}
