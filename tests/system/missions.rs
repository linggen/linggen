//! A mission run's record, and what reaches her when something in it failed
//! (doc/mission-spec.md § Run history, doc/yinyue-spec.md § A
//! mission run that went wrong).

use crate::notes::{held, wait_for};
use crate::support::model::Agent;
use crate::support::{say, text, tool, Setup, World};
use crate::unprompted::{quiet_around_now, set_pet};
use serde_json::{json, Value};
use std::{fs, path::Path};
use std::time::{Duration, Instant};

const OLD_DREAM: &str = include_str!("../fixtures/missions-main/mission.md");
const BUILTIN_MISSIONS: &[(&str, &str)] = &[
    ("dream", include_str!("../../missions/dream/mission.md")),
    ("memory-scan", include_str!("../../missions/memory-scan/mission.md")),
    ("memory-solve", include_str!("../../missions/memory-solve/mission.md")),
];

#[tokio::test]
async fn startup_refreshes_a_pre_upgrade_dream_once() {
    let old = OLD_DREAM
        .replacen("schedule: \"0 3 * * *\"", "schedule: '17 5 * * *'", 1)
        .replacen("enabled: true", "enabled: false", 1);
    let mut w = World::start_with_home(Setup::default(), |home| {
        let missions = home.linggen_home().join("missions");
        let dream = missions.join("dream");
        fs::create_dir_all(&dream).unwrap();
        fs::write(missions.join(".builtin-missions-installed"), "dream\n").unwrap();
        fs::write(dream.join("mission.md"), &old).unwrap();
        fs::write(dream.join("runs.jsonl"), HISTORY).unwrap();
        fs::write(dream.join("note.txt"), "keep this").unwrap();
    })
    .await;
    let missions = w.home.linggen_home().join("missions");
    let read = |path: &str| fs::read_to_string(missions.join(path)).unwrap();
    let split = linggen_engine::extensions::frontmatter::split;
    let snapshot = || {
        let mut files = Vec::new();
        for dir in ["", "dream", "memory-scan", "memory-solve", "table-check"] {
            for entry in fs::read_dir(missions.join(dir)).unwrap() {
                let entry = entry.unwrap();
                if entry.file_type().unwrap().is_file() {
                    let path = entry.path();
                    files.push((path.clone(), fs::read(&path).unwrap(), entry.metadata().unwrap().modified().unwrap()));
                }
            }
        }
        files.sort();
        files
    };

    // Check the whole file: kickoff instructions also live in frontmatter.
    for (name, builtin) in BUILTIN_MISSIONS {
        let installed = read(&format!("{name}/mission.md"));
        for retired in ling_mem::catalog::memory::RETIRED {
            assert!(
                !installed.match_indices(retired.name).any(|(at, found)| {
                    !installed[at + found.len()..]
                        .starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_')
                }),
                "installed {name} still names {}", retired.name
            );
        }
        assert_eq!(split(&installed).1, split(builtin).1, "{name} body");
    }
    let installed = read("dream/mission.md");
    let yaml: serde_norway::Value = serde_norway::from_str(split(&installed).0.unwrap()).unwrap();
    assert_eq!(yaml["schedule"].as_str(), Some("17 5 * * *"));
    assert_eq!(yaml["enabled"].as_bool(), Some(false));
    assert_eq!(read("dream/mission.md.prev"), old);
    assert_eq!(read("dream/runs.jsonl"), HISTORY);
    assert_eq!(read("dream/note.txt"), "keep this");
    let before_restart = snapshot();
    w.engine.restart(&w.home).await;
    assert_eq!(snapshot(), before_restart);
}

const HISTORY: &str = "{\"run_id\":\"old-run\",\"triggered_at\":1700000000,\"status\":\"completed\",\"skipped\":false}\n";

const MISSION: &str = "beacon:nightly";
const HER_LINE: &str = "The beacon check stumbled again tonight.";

/// A made-up skill with one declared tool, `Ping`, whose exit status is
/// whatever `data/exit` holds, and a mission of its own that uses it. Written
/// into the world's home after start: the engine reads skills and their
/// missions from disk on every trigger, once the skills are reloaded.
pub(crate) fn install_beacon(skills: &Path) {
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

pub(crate) fn set_exit(w: &World, code: i32) {
    let path = w.home.linggen_home().join("skills/beacon/data/exit");
    std::fs::write(path, code.to_string()).expect("exit file");
}

/// Trigger the mission unattended and wait until its run is on record as
/// finished; returns that record.
pub(crate) async fn run_once(w: &World) -> Value {
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

pub(crate) fn pet_lines(w: &World) -> Vec<String> {
    w.events()
        .into_iter()
        .filter(|e| e["kind"] == "pet_speak")
        .map(|e| e["data"]["text"].as_str().unwrap_or("").to_string())
        .collect()
}

pub(crate) async fn beat_present(w: &World) {
    let body = json!({"surface": "main-ui", "focused": true, "typing": false, "idle_ms": 1000});
    w.api.post("/api/presence", body).await;
}

/// Her kickoffs so far, oldest first: each call's last user row (the world
/// block follows it).
pub(crate) fn her_facts(w: &World) -> Vec<String> {
    w.model
        .calls_of(Agent::Yinyue)
        .iter()
        .filter_map(|c| {
            let thread = c.thread();
            thread
                .iter()
                .rev()
                .find(|(role, _)| role == "user")
                .map(|(_, t)| t.clone())
        })
        .collect()
}

/// A declared tool exits 3 and the model still closes as usual: the run is
/// recorded with the failure, and its fact waits while nobody is at a
/// screen; once someone is, she is handed it — once, naming the tool and the
/// status — and the only line spoken is hers. The same mission with the tool
/// succeeding hands her nothing; failing again while someone is there, she
/// is handed it at once.
#[tokio::test]
async fn a_failed_tool_in_an_unattended_run_reaches_her_as_one_fact() {
    let ling = vec![
        tool("Ping", json!({})),
        text("The lamp check did not complete. DONE"),
        tool("Ping", json!({})),
        text("The lamp answered. DONE"),
        tool("Ping", json!({})),
        text("The lamp check did not complete. DONE"),
    ];
    let w = World::start(Setup::default().script(Agent::Ling, ling).script(
        Agent::Yinyue,
        vec![say(HER_LINE), say(HER_LINE), text("Hello.")],
    ))
    .await;
    install_beacon(&w.home.linggen_home().join("skills"));
    w.api.post("/api/skills/reload", json!({})).await;

    // Nobody at a screen (no surface has beaten): the run fails at "night".
    set_exit(&w, 3);
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
    // The hold checks every 5 s; past one check, nothing has reached her.
    tokio::time::sleep(Duration::from_secs(6)).await;
    assert!(her_facts(&w).is_empty(), "handed to an empty room");

    // Someone is back: she is handed the fact, once.
    let her_mark = w.mark();
    beat_present(&w).await;
    w.wait_agent(Agent::Yinyue, her_mark).await;
    let facts = her_facts(&w);
    assert_eq!(facts.len(), 1, "she was woken {} times", facts.len());
    let fact = &facts[0];
    assert!(fact.contains("\"Beacon check\" ran on its own"), "{fact}");
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
    // Her run is over before her line goes out (it is published after the
    // turn), so the cue is waited for; a second line would still show here
    // and in the checks below.
    w.events_when(|e| e.iter().any(|x| x["kind"] == "pet_speak")).await;
    assert_eq!(pet_lines(&w), [HER_LINE], "a line not hers was spoken");
    // The fault is in her call once: the world block's doorbell, which
    // names the newest record, does not tell it a second time.
    let first = w.model.calls_of(Agent::Yinyue).remove(0);
    let told = first.text().matches("Ping exited with status 3").count();
    assert_eq!(told, 1, "the fault is in her call {told} times");
    assert!(
        first.text().contains("happened since you last looked"),
        "the doorbell is still there: {:?}",
        first.thread()
    );

    // A clean run, someone there: nothing for her.
    set_exit(&w, 0);
    beat_present(&w).await;
    let clean = run_once(&w).await;
    assert_eq!(clean["status"], "completed", "{clean}");
    assert!(clean.get("failure_count").is_none(), "{clean}");
    assert!(clean.get("failures").is_none(), "{clean}");
    // Past a hold check, were anything coming.
    tokio::time::sleep(Duration::from_secs(6)).await;
    assert_eq!(her_facts(&w).len(), 1, "a clean run woke her");
    assert_eq!(pet_lines(&w), [HER_LINE]);

    let listed = w.api.get("/api/missions").await;
    let beacon = listed["missions"]
        .as_array()
        .and_then(|all| all.iter().find(|m| m["id"] == MISSION))
        .cloned()
        .unwrap_or_else(|| panic!("not listed: {listed}"));
    assert_eq!(beacon["last_run"]["status"], "completed", "{beacon}");
    assert_eq!(beacon["last_run"]["failure_count"], 0, "{beacon}");

    // Failing again with someone there: handed at once, no hold.
    set_exit(&w, 3);
    beat_present(&w).await;
    let her_mark = w.mark();
    run_once(&w).await;
    w.wait_agent(Agent::Yinyue, her_mark).await;
    let facts = her_facts(&w);
    assert_eq!(facts.len(), 2, "{facts:?}");
    assert!(
        facts[1].contains("Ping exited with status 3"),
        "{}",
        facts[1]
    );

    // An ordinary turn of hers hears the doorbell whole: the Mac recorded it.
    let mark = w.mark();
    w.api
        .post("/api/yinyue/chat", json!({"text": "Anything else?"}))
        .await;
    w.wait_agent(Agent::Yinyue, mark).await;
    let chat = w.model.calls_of(Agent::Yinyue).remove(2);
    assert!(
        chat.text()
            .contains("most recent: the Mac recorded a failure in Beacon check"),
        "{:?}",
        chat.thread()
    );
    w.assert_all_scripted();
}

/// Two failing-then-closing runs' worth of script: Ling pings and closes.
pub(crate) fn failing_run_script() -> Setup {
    Setup::default()
        .script(
            Agent::Ling,
            vec![
                tool("Ping", json!({})),
                text("The lamp check did not complete. DONE"),
            ],
        )
        .script(Agent::Yinyue, vec![say(HER_LINE)])
}

async fn beacon_world() -> World {
    let w = World::start(failing_run_script()).await;
    install_beacon(&w.home.linggen_home().join("skills"));
    w.api.post("/api/skills/reload", json!({})).await;
    set_exit(&w, 3);
    w
}

/// A fault in the night with someone at a screen: quiet hours hold it, the
/// wait is one `hold` record, and she is handed it, once, when they end —
/// it is not lost along the way.
#[tokio::test]
async fn a_fault_in_quiet_hours_waits_and_comes_when_they_end() {
    let w = beacon_world().await;
    set_pet(&w, json!({ "quiet_hours": quiet_around_now() })).await;
    beat_present(&w).await;
    run_once(&w).await;
    wait_for("a hold record", || !held(&w).is_empty()).await;
    tokio::time::sleep(Duration::from_secs(12)).await;
    beat_present(&w).await;
    tokio::time::sleep(Duration::from_secs(6)).await;
    assert!(her_facts(&w).is_empty(), "handed through quiet hours");
    let records = held(&w);
    assert_eq!(
        records.len(),
        1,
        "one record for the whole wait: {records:?}"
    );
    assert_eq!(records[0]["by"], "yinyue");
    assert_eq!(
        records[0]["object"],
        "a fault in Beacon check (quiet hours)"
    );
    assert_eq!(records[0]["detail"]["source"], "fault");
    assert!(pet_lines(&w).is_empty());

    // Quiet hours end with someone at the screen: the fault comes.
    let her_mark = w.mark();
    set_pet(&w, json!({ "quiet_hours": "" })).await;
    beat_present(&w).await;
    w.wait_agent(Agent::Yinyue, her_mark).await;
    let facts = her_facts(&w);
    assert_eq!(facts.len(), 1, "{facts:?}");
    assert!(
        facts[0].contains("Ping exited with status 3"),
        "{}",
        facts[0]
    );
    // The turn ends before the speech cue is emitted and logged.
    w.events_when(|events| events.iter().any(|e| e["kind"] == "pet_speak"))
        .await;
    assert_eq!(pet_lines(&w), [HER_LINE]);
    assert_eq!(held(&w).len(), 1, "the wait is not recorded again");
    w.assert_all_scripted();
}

/// The hourly ceiling is not for a fault: with the hour's allowance already
/// at nothing, a fault outside quiet hours is handed at once.
#[tokio::test]
async fn the_hourly_ceiling_does_not_hold_a_fault() {
    let w = beacon_world().await;
    set_pet(&w, json!({ "unprompted_per_hour": 0 })).await;
    beat_present(&w).await;
    let her_mark = w.mark();
    run_once(&w).await;
    w.wait_agent(Agent::Yinyue, her_mark).await;
    assert_eq!(her_facts(&w).len(), 1);
    // The turn ends before the speech cue is emitted and logged.
    w.events_when(|events| events.iter().any(|e| e["kind"] == "pet_speak"))
        .await;
    assert_eq!(pet_lines(&w), [HER_LINE]);
    assert!(held(&w).is_empty(), "{:?}", held(&w));
    w.assert_all_scripted();
}

/// A skill with one tool, `Count`, that prints what it counted, and a
/// mission of its own with a two-item kickoff.
fn install_tally(skills: &Path) {
    let dir = skills.join("tally");
    for sub in ["scripts", "missions/trays"] {
        fs::create_dir_all(dir.join(sub)).expect("tally dirs");
    }
    fs::write(
        dir.join("SKILL.md"),
        "---\n\
         name: tally\n\
         description: A test skill — counts trays.\n\
         cwd: ~/.linggen/skills/tally\n\
         tools:\n  - name: Count\n    description: Count a tray.\n    \
         cmd: \"echo the-tray-holds-58\"\n    tier: read\n    timeout_ms: 5000\n\
         ---\n\nCount the trays.\n",
    )
    .expect("SKILL.md");
    fs::write(
        dir.join("missions/trays/mission.md"),
        "---\n\
         name: Tray count\n\
         schedule: '0 3 * * *'\n\
         enabled: false\n\
         agent: ling\n\
         cwd: ~/.linggen/skills/tally\n\
         kickoff:\n  - \"Count the first tray.\"\n  - \"Count the second tray.\"\n\
         ---\n\nCount each tray once and say the number.\n",
    )
    .expect("mission.md");
}

/// A two-item kickoff run whose second item's model call fails keeps its
/// work in the session's thread; the turn that continues the session sees
/// each row of that run once, in the order it happened.
#[tokio::test]
async fn a_failed_kickoff_run_is_continued_with_each_row_once_in_order() {
    let setup = Setup::default()
        .script(
            Agent::Ling,
            vec![
                tool("Count", json!({})),
                text("First tray: 58."),
                tool("Count", json!({})),
                crate::support::error(500, "upstream overloaded"),
                text("Back to the trays."),
            ],
        )
        .script(Agent::Yinyue, vec![say(HER_LINE)]);
    let w = World::start(setup).await;
    install_tally(&w.home.linggen_home().join("skills"));
    w.api.post("/api/skills/reload", json!({})).await;
    let mark = w.mark();
    let path = format!("/api/missions/{}/trigger", urlencoding::encode("tally:trays"));
    let resp = w.api.post(&path, json!({})).await;
    let sid = resp["session_id"].as_str().unwrap_or_else(|| panic!("no session: {resp}")).to_string();
    let _ = w.wait_run(&sid, Agent::Ling, mark).await;

    let root = w.home.linggen_home().join("skills/tally").display().to_string();
    w.turn(&sid, &root, "Go on.", Agent::Ling).await;
    let calls = w.model.calls_of(Agent::Ling);
    let thread = calls.last().expect("the continued turn").thread();
    let at = |what: &str| -> Vec<usize> {
        thread.iter().enumerate().filter(|(_, (_, t))| t.contains(what)).map(|(i, _)| i).collect()
    };
    let reply = at("First tray: 58.");
    let second = at("Count the second tray.");
    let counted = at("[call Count");
    let results: Vec<usize> = thread
        .iter()
        .enumerate()
        .filter(|(_, (role, _))| role == "tool")
        .map(|(i, _)| i)
        .collect();
    let go_on = at("Go on.");
    assert_eq!(reply.len(), 1, "the first reply once: {thread:#?}");
    assert_eq!(second.len(), 1, "the second item once: {thread:#?}");
    assert_eq!(counted.len(), 2, "each call once: {thread:#?}");
    assert_eq!(results.len(), 2, "each result once: {thread:#?}");
    assert_eq!(go_on.len(), 1, "{thread:#?}");
    let order = [counted[0], results[0], reply[0], second[0], counted[1], results[1], go_on[0]];
    assert!(order.windows(2).all(|p| p[0] < p[1]), "out of order {order:?}: {thread:#?}");
}
