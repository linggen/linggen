//! `ling v1-only [<ling arguments>…]` — the one gate a v1 memory mission
//! declares as its `prepare` step (`doc/mission-spec.md`, "prepare").
//!
//! Memory v1 is the ling-mem store: the nightly dream, the day scan and the
//! review-queue solve tidy it. Under `[memory] linggen = "v2"` Linggen's own
//! sessions read the notes and nobody reads that store, so those runs would
//! only spend model tokens. This verb is where "is this a v1 job, and is v1
//! the memory" is decided, for every mission that asks: no mission is named
//! here, and the engine's mission runner knows nothing of memory.
//!
//! - v2: print one plain line and exit `NOTHING_TO_DO`; the run is skipped
//!   with that line as its reason — no model call, no session.
//! - v1: do nothing, or, when the mission has more to prepare, run the
//!   remaining arguments as `ling <arguments>` (the memory scan's walk of the
//!   session logs) and exit as it exits.
//!
//! Reading `[memory] linggen` here is one of the listed places
//! (`own_memory::tests::the_switch_is_read_in_the_listed_places_only`).

use crate::config::Config;
use crate::extensions::missions::NOTHING_TO_DO;
use crate::own_memory::Mode;
use anyhow::{Context, Result};

/// The reason a skipped run shows: what happened, in plain words.
const V2_REASON: &str = "Memory v2 is on, so the v1 memory store is not tidied.";

/// Whether a v1 job may run under `mode`, else the line to say why not.
fn refusal(mode: Mode) -> Option<&'static str> {
    (mode == Mode::V2).then_some(V2_REASON)
}

pub async fn run(rest: &[String], config: &Config) -> Result<()> {
    if let Some(line) = refusal(config.memory.linggen) {
        println!("{line}");
        std::process::exit(NOTHING_TO_DO);
    }
    if rest.is_empty() {
        return Ok(());
    }
    let exe = std::env::current_exe().context("cannot find this program")?;
    let status = tokio::process::Command::new(exe)
        .args(rest)
        .status()
        .await
        .context("cannot run the rest of the step")?;
    std::process::exit(status.code().unwrap_or(1));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v1_runs_its_jobs_and_v2_does_not() {
        assert_eq!(refusal(Mode::V1), None);
        assert_eq!(refusal(Mode::V2), Some(V2_REASON));
    }

    #[test]
    fn the_reason_is_one_short_line() {
        assert!(V2_REASON.split_whitespace().count() <= 20 && !V2_REASON.contains('\n'));
    }
}
