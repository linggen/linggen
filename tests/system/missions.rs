//! A mission run's record, and what reaches her when something in it failed
//! (doc/mission-spec.md § Run history, doc/yinyue-companion-spec.md § A
//! mission run that went wrong).

use crate::support::model::Agent;
use crate::support::{text, tool, Setup, World};
use serde_json::{json, Value};
use std::path::Path;
use std::time::{Duration, Instant};

const MISSION: &str = "beacon:nightly";
const HER_LINE: &str = "The beacon check stumbled again tonight.";

/// A made-up skill with one declared tool, `Ping`, whose exit status is
/// whatever `data/exit` holds, and a mission of its own that uses it. Written
/// into the world's home after start: the engine reads skills and their
/// missions from disk on every trigger, once the skills are reloaded.
fn install_beacon(skills: &Path) {
    let dir = skills.join("beacon");
    for sub in ["scripts", "data", "missions/nightly"] {
        std::fs::create_dir_all(dir.join(sub)).expect("beacon dirs");
    }
    std::fs::write(
        dir.join("SKILL.md"),
        "---\n\
         name: beacon\n\
         description: A test skill — checks a lamp.\n\
         cwd: ~/.linggen/skills/beacon\n\
         permission:\n  paths:\n    - { path: ~/.linggen/skills/beacon, mode: edit }\n\
         tools:\n  - name: Ping\n    description: Ping the lamp.\n    \
         cmd: \"sh $SKILL_DIR/scripts/ping.sh\"\n    tier: read\n    timeout_ms: 5000\n\
         ---\n\nPing the lamp.\n",
    )
    .expect("SKILL.md");
    std::fs::write(
        dir.join("scripts/ping.sh"),
        "#!/bin/sh\n\
         code=$(cat \"$(dirname \"$0\")/../data/exit\")\n\
         echo \"rows the model reads\"\n\
         if [ \"$code\" != 0 ]; then echo 'starting' >&2; echo 'beacon: the lamp did not answer' >&2; fi\n\
         exit \"$code\"\n",
    )
    .expect("ping.sh");
    std::fs::write(
        dir.join("missions/nightly/mission.md"),
        "---\n\
         name: Beacon check\n\
         schedule: '0 3 * * *'\n\
         enabled: false\n\
         agent: ling\n\
         cwd: ~/.linggen/skills/beacon\n\
         kickoff:\n  - \"Ping the lamp and say how it went.\"\n\
         ---\n\nPing the lamp once and say how it went in one line.\n",
    )
    .expect("mission.md");
}

fn set_exit(w: &World, code: i32) {
    let path = w.home.linggen_home().join("skills/beacon/data/exit");
    std::fs::write(path, code.to_string()).expect("exit file");
}

/// Trigger the mission unattended and wait until its run is on record as
/// finished; returns that record.
async fn run_once(w: &World) -> Value {
    let mark = w.mark();
    let path = format!("/api/missions/{}/trigger", urlencoding::encode(MISSION));
    let resp = w.api.post(&path, json!({})).await;
    let sid = resp["session_id"]
        .as_str()
        .unwrap_or_else(|| panic!("no session: {resp}"))
        .to_string();
    w.finish_run(&sid, Agent::Ling, mark).await;
    let runs_path = format!("/api/missions/{}/runs", urlencoding::encode(MISSION));
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(10) {
        let runs = w.api.get(&runs_path).await;
        let newest = runs["runs"][0].clone();
        if newest["session_id"] == sid && newest["status"] != "running" {
            return newest;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the run in {sid} was never finalized");
}

fn pet_lines(w: &World) -> Vec<String> {
    w.events()
        .into_iter()
        .filter(|e| e["kind"] == "pet_speak")
        .map(|e| e["data"]["text"].as_str().unwrap_or("").to_string())
        .collect()
}

/// A declared tool exits 3 and the model still closes as usual: the run is
/// recorded with the failure, she is handed one fact naming the tool and the
/// status, and the only line spoken is hers. The same mission with the tool
/// succeeding hands her nothing.
#[tokio::test]
async fn a_failed_tool_in_an_unattended_run_reaches_her_as_one_fact() {
    let ling = vec![
        tool("Ping", json!({})),
        text("The lamp check did not complete. DONE"),
        tool("Ping", json!({})),
        text("The lamp answered. DONE"),
    ];
    let w = World::start(
        Setup::default()
            .script(Agent::Ling, ling)
            .script(Agent::Yinyue, vec![text(HER_LINE)]),
    )
    .await;
    install_beacon(&w.home.linggen_home().join("skills"));
    w.api.post("/api/skills/reload", json!({})).await;

    set_exit(&w, 3);
    let her_mark = w.mark();
    let failed = run_once(&w).await;
    assert_eq!(failed["status"], "completed", "{failed}");
    assert_eq!(failed["failure_count"], 1, "{failed}");
    assert_eq!(failed["failures"][0]["tool"], "Ping", "{failed}");
    assert_eq!(failed["failures"][0]["exit"], 3, "{failed}");
    assert_eq!(
        failed["failures"][0]["tail"],
        "starting / beacon: the lamp did not answer"
    );
    assert!(
        !failed.to_string().contains("rows the model reads"),
        "stdout reached the record: {failed}"
    );

    w.wait_agent(Agent::Yinyue, her_mark).await;
    let hers = w.model.calls_of(Agent::Yinyue);
    assert_eq!(hers.len(), 1, "she was woken {} times", hers.len());
    let (_, fact) = hers[0].thread().last().cloned().expect("her kickoff");
    assert!(fact.contains("\"Beacon check\" ran unattended"), "{fact}");
    assert!(fact.contains("Ping exited with status 3"), "{fact}");
    assert!(fact.contains("beacon: the lamp did not answer"), "{fact}");
    assert!(
        fact.contains("no earlier run is on record with the same failure"),
        "{fact}"
    );
    assert!(
        !fact.contains("rows the model reads"),
        "stdout reached her: {fact}"
    );
    assert_eq!(pet_lines(&w), [HER_LINE], "a line not hers was spoken");

    set_exit(&w, 0);
    let clean = run_once(&w).await;
    assert_eq!(clean["status"], "completed", "{clean}");
    assert!(clean.get("failure_count").is_none(), "{clean}");
    assert!(clean.get("failures").is_none(), "{clean}");
    // Time for a herald to have started, were one coming.
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(
        w.model.calls_of(Agent::Yinyue).len(),
        1,
        "a clean run woke her"
    );
    assert_eq!(pet_lines(&w), [HER_LINE]);

    let listed = w.api.get("/api/missions").await;
    let beacon = listed["missions"]
        .as_array()
        .and_then(|all| all.iter().find(|m| m["id"] == MISSION))
        .cloned()
        .unwrap_or_else(|| panic!("not listed: {listed}"));
    assert_eq!(beacon["last_run"]["status"], "completed", "{beacon}");
    assert_eq!(beacon["last_run"]["failure_count"], 0, "{beacon}");
    w.assert_all_scripted();
}
