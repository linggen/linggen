//! What one turn sets for itself never reaches a later turn on the same
//! engine (doc/session-spec.md § How a turn is set up).

use crate::support::model::Agent;
use crate::support::{text, Setup, World};
use serde_json::json;
use std::path::Path;

const MISSION: &str = "tally:count";
const LEFTOVER: &str = "LEFTOVER-BRANCH: count again.";

/// A made-up skill with a mission whose `kickoff-then` branch waits for a
/// reply of exactly `MORE`, which the run itself never gives.
fn install_tally(skills: &Path) {
    let dir = skills.join("tally");
    std::fs::create_dir_all(dir.join("missions/count")).expect("tally dirs");
    std::fs::write(
        dir.join("SKILL.md"),
        "---\nname: tally\ndescription: A test skill — counts.\n\
         cwd: ~/.linggen/skills/tally\n---\n\nCount.\n",
    )
    .expect("SKILL.md");
    std::fs::write(
        dir.join("missions/count/mission.md"),
        format!(
            "---\nname: Tally count\nschedule: '0 3 * * *'\nenabled: false\nagent: ling\n\
             cwd: ~/.linggen/skills/tally\nkickoff:\n  - \"Count once.\"\n\
             kickoff-then:\n  MORE:\n    - \"{LEFTOVER}\"\n---\n\nCount once, in one line.\n"
        ),
    )
    .expect("mission.md");
}

/// A mission run's kickoff belongs to that run. The run ends with its
/// branch unfired; the person's next turn in the
/// session answers `MORE`, and nothing of the mission's kickoff follows.
#[tokio::test]
async fn a_finished_runs_kickoff_never_reaches_a_persons_turn() {
    let ling = vec![text("Counted: one."), text("MORE")];
    let w = World::start(Setup::default().script(Agent::Ling, ling)).await;
    install_tally(&w.home.linggen_home().join("skills"));
    w.api.post("/api/skills/reload", json!({})).await;

    let mark = w.mark();
    let path = format!("/api/missions/{}/trigger", urlencoding::encode(MISSION));
    let resp = w.api.post(&path, json!({})).await;
    let sid = resp["session_id"]
        .as_str()
        .unwrap_or_else(|| panic!("no session: {resp}"))
        .to_string();
    w.finish_run(&sid, Agent::Ling, mark).await;

    w.turn(&sid, &w.work(), "Anything else?", Agent::Ling).await;
    let leaked: Vec<_> = w
        .raw_rows(&sid)
        .into_iter()
        .filter(|r| r["content"].as_str().is_some_and(|c| c.contains("LEFTOVER-BRANCH")))
        .collect();
    assert!(leaked.is_empty(), "the run's kickoff reached the chat: {leaked:#?}");
    assert_eq!(w.model.calls_of(Agent::Ling).len(), 2);
    w.assert_all_scripted();
}
