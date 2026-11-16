//! The memory scan and solve, started from the engine (doc/memory-spec.md §
//! Scan and solve): `ling mem scan` on a fixture home's session logs, and the
//! `memory-scan` / `memory-solve` missions run through the trigger route.

use crate::support::model::Agent;
use crate::support::{text, tool, Setup, World};
use serde_json::{json, Value};
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant, SystemTime};

/// A past day (the store stamps only days that are over); the world's clock
/// is UTC (`hermetic_env`).
const DAY: &str = "2026-11-10";
/// Noon of `DAY`, UTC, in seconds.
const NOON: i64 = 1_794_312_000;
const CC_SESSION: &str = "0b5e6c1a-1111-4222-8333-444455556666";
const LING_SESSION: &str = "sess-1794312000-a1e7a1e7";
const OUT: &str = "~/.linggen/memory/.scan-output.jsonl";

/// Whether the scan's tools are here (jq is not on every machine).
fn scan_tools() -> bool {
    ["jq", "perl"].iter().all(|t| {
        ["/usr/bin", "/bin"]
            .iter()
            .any(|d| Path::new(d).join(t).is_file())
    })
}

/// Set a file's modification time to noon of `DAY` — the scan picks a
/// session by the day its file last changed.
fn date_file(path: &Path) {
    let when = SystemTime::UNIX_EPOCH + Duration::from_secs(NOON as u64);
    let file = std::fs::File::options().write(true).open(path).unwrap();
    file.set_modified(when).unwrap();
}

/// One Claude Code session and one Linggen session from `DAY`, and one
/// Claude Code session from the day before, under `home`.
fn write_sessions(home: &Path) {
    let cc = home.join(".claude/projects/-work-harbor");
    std::fs::create_dir_all(&cc).unwrap();
    let line = |role: &str, ts: &str, text: &str| {
        json!({"type": role, "cwd": "/work/harbor", "timestamp": ts,
               "message": {"role": role, "content": text}})
        .to_string()
    };
    let today = [
        line(
            "user",
            "2026-11-10T12:00:00Z",
            "Alex here. For Harbor I always want tabs, not spaces.",
        ),
        line(
            "assistant",
            "2026-11-10T12:00:05Z",
            "Noted: tabs for Harbor.",
        ),
        line(
            "user",
            "2026-11-10T12:01:00Z",
            "We picked SQLite over Postgres: the app is single-user.",
        ),
    ];
    let cc_file = cc.join(format!("{CC_SESSION}.jsonl"));
    std::fs::write(&cc_file, today.join("\n") + "\n").unwrap();
    date_file(&cc_file);
    let older = cc.join("9f9f9f9f-0000-4000-8000-000000000000.jsonl");
    let before = [
        line(
            "user",
            "2026-11-09T09:00:00Z",
            "An older day: Alex asked about the ferry timetable.",
        ),
        line(
            "user",
            "2026-11-09T09:01:00Z",
            "And the harbour tide tables.",
        ),
    ];
    std::fs::write(&older, before.join("\n") + "\n").unwrap();
    let file = std::fs::File::options().write(true).open(&older).unwrap();
    file.set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs((NOON - 86_400) as u64))
        .unwrap();

    let ling = home.join(".linggen/sessions").join(LING_SESSION);
    std::fs::create_dir_all(&ling).unwrap();
    std::fs::write(
        ling.join("session.yaml"),
        "title: Garden plans\ncreator: user\n",
    )
    .unwrap();
    let msg = |from: &str, to: &str, t: i64, text: &str| {
        json!({"from_id": from, "to_id": to, "content": text, "timestamp": t,
               "is_observation": false})
        .to_string()
    };
    let rows = [
        msg("user", "ling", NOON, "My cat Pico turned 3 this month."),
        msg("ling", "user", NOON + 10, "Happy birthday to Pico!"),
        msg(
            "user",
            "ling",
            NOON + 60,
            "I water the garden every Sunday morning.",
        ),
    ];
    let messages = ling.join("messages.jsonl");
    std::fs::write(&messages, rows.join("\n") + "\n").unwrap();
    date_file(&messages);
}

/// `ling mem scan <args>` with the world's environment, `path` as `$PATH`.
fn scan(w_env: &[(String, String)], path: Option<&str>, args: &[&str]) -> std::process::Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_ling"));
    cmd.arg("mem").arg("scan").args(args).env_clear();
    for (k, v) in w_env {
        cmd.env(k, v);
    }
    if let Some(path) = path {
        cmd.env("PATH", path);
    }
    cmd.output().expect("run ling mem scan")
}

fn candidates(home: &Path) -> Vec<Value> {
    let text = std::fs::read_to_string(home.join(".linggen/memory/.scan-output.jsonl"))
        .expect("the candidates file");
    text.lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

/// The scan writes one day's sessions — and only that day's — into the
/// candidates file, header first, as scan.sh did; a missing tool is one
/// plain line and exit 1.
#[test]
fn ling_mem_scan_writes_one_days_candidates() {
    if !scan_tools() {
        eprintln!("SKIPPED (passes on nothing): no jq or perl in /usr/bin");
        return;
    }
    let home = crate::support::home::Home::new();
    write_sessions(&home.home());
    let env = crate::support::process::hermetic_env(&home);

    let out = scan(&env, None, &[DAY]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{stdout} {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        stdout.starts_with(&format!("scan: window={DAY} found=2 scanned=2 skipped=0")),
        "{stdout}"
    );
    let rows = candidates(&home.home());
    assert_eq!(rows.len(), 3, "{rows:?}");
    let header = &rows[0];
    assert_eq!(header["_meta"], true);
    assert_eq!(header["window"], DAY);
    assert_eq!(
        (
            header["scanned_from"].as_str(),
            header["scanned_to"].as_str()
        ),
        (Some(DAY), Some(DAY))
    );
    assert_eq!(header["sessions_scanned"], 2);
    let cc = &rows[1];
    assert_eq!(cc["source"], "CC");
    assert!(cc["filepath"]
        .as_str()
        .unwrap()
        .ends_with(&format!("{CC_SESSION}.jsonl")));
    let transcript = cc["transcript"].as_str().unwrap();
    assert!(
        transcript.starts_with("[SESSION_CWD]: /work/harbor"),
        "{transcript}"
    );
    assert!(
        transcript.contains("[user]: Alex here. For Harbor I always want tabs"),
        "{transcript}"
    );
    let ling = &rows[2];
    assert_eq!(ling["source"], "Linggen");
    assert!(ling["transcript"]
        .as_str()
        .unwrap()
        .contains("I water the garden every Sunday"));
    assert!(
        !rows
            .iter()
            .any(|r| r.to_string().contains("ferry timetable")),
        "another day leaked"
    );

    // A wrong window: the script's usage line, its exit status.
    let bad = scan(&env, None, &["someday"]);
    assert_eq!(bad.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&bad.stderr).starts_with("Usage: ling mem scan"));

    // No jq: one plain line, exit 1, and the candidates file untouched.
    let bin = home.root.join("nojq");
    std::fs::create_dir_all(&bin).unwrap();
    for (name, real) in [("bash", "/bin/bash"), ("perl", "/usr/bin/perl")] {
        std::os::unix::fs::symlink(real, bin.join(name)).unwrap();
    }
    let missing = scan(&env, Some(bin.to_str().unwrap()), &[DAY]);
    assert_eq!(missing.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&missing.stderr),
        "ling mem scan needs jq, which is not on PATH\n"
    );
    assert_eq!(candidates(&home.home()).len(), 3);
}

/// Put the built `ling` first on the engine's `$PATH` (`hermetic_env` puts
/// `<root>/bin` there), so the mission's `ling mem scan` is this build's.
fn link_ling(w: &World) {
    let bin = w.home.root.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_ling"), bin.join("ling")).unwrap();
}

/// Trigger a built-in memory mission; its session id.
async fn trigger(w: &World, mission: &str, body: Value) -> String {
    let resp = w
        .api
        .post(&format!("/api/missions/{mission}/trigger"), body)
        .await;
    resp["session_id"]
        .as_str()
        .unwrap_or_else(|| panic!("no session: {resp}"))
        .to_string()
}

/// The run in `sid`, once it is on record as finished.
async fn finished_run(w: &World, mission: &str, sid: &str) -> Value {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(60) {
        let runs = w.api.get(&format!("/api/missions/{mission}/runs")).await;
        let newest = runs["runs"][0].clone();
        if newest["session_id"] == sid && newest["status"] != "running" {
            return newest;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    panic!(
        "the {mission} run in {sid} never finished:\n{}",
        w.engine.log()
    );
}

/// The text of every tool result in a session, joined.
fn tool_results(w: &World, sid: &str) -> String {
    w.raw_rows(sid)
        .iter()
        .map(|r| r.to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

fn add(content: &str, session: &str, cwd: Option<&str>) -> Value {
    let mut args = json!({
        "content": content, "type": "preference", "from": "user", "tier": "episodic",
        "occurred_at": format!("{DAY}T12:00:00Z"), "source_session": session,
    });
    if let Some(cwd) = cwd {
        args["cwd"] = json!(cwd);
    }
    args
}

/// Settings → Memory's Scan for a day: the run calls `ling mem scan`, reads
/// the candidates, stages one row per session carrying the session it came
/// from, and stamps the day scanned — all inside the mission's two grants
/// (nothing refused).
#[tokio::test]
async fn a_scan_run_stages_the_days_sessions_and_stamps_the_day() {
    if !crate::support::memory::embedder_available() || !scan_tools() {
        eprintln!("SKIPPED (passes on nothing): no embedding model, or no jq/perl");
        return;
    }
    let list = format!("jq -c '{{source, filepath, user_turns, transcript_bytes}}' {OUT}");
    let script = vec![
        tool("Bash", json!({"command": format!("ling mem scan {DAY}")})),
        tool("Bash", json!({"command": list})),
        tool(
            "mcp__memory__memory_list",
            json!({"tier": "episodic", "day": DAY, "limit": 100}),
        ),
        text(&format!("SCAN {DAY} found=2 scanned=2\nTODO 2 3")),
        tool(
            "Read",
            json!({"path": OUT, "line_range": [2, 2], "max_bytes": 250000}),
        ),
        tool(
            "mcp__memory__memory_add",
            add(
                "Alex wants tabs, not spaces, in Harbor.",
                CC_SESSION,
                Some("/work/harbor"),
            ),
        ),
        text(&format!("STAGE 2 {CC_SESSION} rows=1")),
        tool(
            "Read",
            json!({"path": OUT, "line_range": [3, 3], "max_bytes": 250000}),
        ),
        tool(
            "mcp__memory__memory_add",
            add(
                "Alex waters the garden every Sunday morning.",
                LING_SESSION,
                None,
            ),
        ),
        text(&format!("STAGE 3 {LING_SESSION} rows=1")),
        tool("mcp__memory__memory_harvest_day", json!({"date": DAY})),
        text("DONE"),
    ];
    let w = World::start(Setup {
        embedder: true,
        ..Setup::default().script(Agent::Memory, script)
    })
    .await;
    write_sessions(&w.home.home());
    link_ling(&w);

    let sid = trigger(&w, "memory-scan", json!({"day": DAY, "attended": true})).await;
    let run = finished_run(&w, "memory-scan", &sid).await;
    assert_eq!(run["status"], "completed", "{run}");
    assert!(
        run.get("failure_count").is_none(),
        "a call failed or was refused: {run}"
    );
    assert_eq!(run["attended"], true, "{run}");
    w.assert_all_scripted();

    let seen = tool_results(&w, &sid);
    assert!(
        seen.contains(&format!("scan: window={DAY} found=2 scanned=2")),
        "the scan never ran"
    );
    let model_saw: String = w
        .model
        .calls_of(Agent::Memory)
        .iter()
        .map(|c| c.text())
        .collect();
    assert!(
        model_saw.contains("I water the garden every Sunday"),
        "the candidate was never read"
    );
    assert!(
        seen.contains(&format!("scanned {DAY}: its session logs staged")),
        "no report line"
    );

    let staged = w
        .memory
        .call("list", json!({"tier": "episodic", "day": DAY}))
        .await;
    let rows = staged["facts"]
        .as_array()
        .or(staged.as_array())
        .cloned()
        .unwrap_or_default();
    let mut sessions: Vec<&str> = rows
        .iter()
        .filter_map(|r| r["source_session"].as_str())
        .collect();
    sessions.sort();
    assert_eq!(sessions, [CC_SESSION, LING_SESSION], "{staged}");
    let days = w.memory.call("days", json!({})).await;
    let day = days["days"]
        .as_array()
        .and_then(|d| d.iter().find(|r| r["date"] == DAY))
        .cloned()
        .unwrap_or_else(|| panic!("no {DAY}: {days}"));
    assert_eq!(day["scanned"], true, "{day}");
    assert_eq!(day["dreamed"], false, "a scan marks nothing dreamed: {day}");
}

/// Settings → Memory's Solve: the run reads the queue, asks the user (it is
/// attended), and a second Solve while it waits is refused with 409; the
/// answer closes the item.
#[tokio::test]
async fn a_solve_run_asks_once_refuses_a_second_start_and_closes_the_item() {
    let script = vec![
        tool("mcp__memory__memory_role", json!({})),
        tool("mcp__memory__memory_issues", json!({})),
        tool(
            "AskUser",
            json!({"questions": [{
                "question": "Two notes on Harbor's database: SQLite, or Postgres? I think SQLite.",
                "header": "Harbor",
                "options": [{"label": "SQLite"}, {"label": "Postgres"}],
            }]}),
        ),
        tool(
            "mcp__memory__memory_issue_resolve",
            json!({"id": "iss-harbor-db", "outcome": "resolved", "note": "Alex said SQLite"}),
        ),
        text("SOLVED iss-harbor-db \"SQLite\"\nSolved 1, dismissed 0, left open 0.\nDONE"),
    ];
    // She is told the run waits on the user; she stays quiet here.
    let setup = Setup::default()
        .script(Agent::Memory, script)
        .script(Agent::Yinyue, vec![text("SILENT")]);
    let w = World::start(setup).await;
    let issues = json!({"issues": [{
        "id": "iss-harbor-db", "kind": "contradiction", "row_ids": [],
        "note": "Harbor uses SQLite, or Postgres?", "created_at": "2026-11-10T12:00:00Z",
        "status": "open",
    }]});
    let sidecar = w.home.linggen_home().join("memory/.issues.json");
    std::fs::create_dir_all(sidecar.parent().unwrap()).unwrap();
    std::fs::write(&sidecar, issues.to_string()).unwrap();

    let sid = trigger(&w, "memory-solve", json!({"attended": true})).await;
    let start = Instant::now();
    let ask = loop {
        if let Some(ask) = w.api.pending(&sid, Some("memory")).await.into_iter().next() {
            break ask;
        }
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "the solve never asked"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    let again = w
        .api
        .post_status(
            "/api/missions/memory-solve/trigger",
            json!({"attended": true}),
        )
        .await;
    assert_eq!(again, 409, "a second solve started while one was running");

    w.api.answer(&ask, "SQLite").await;
    let run = finished_run(&w, "memory-solve", &sid).await;
    assert_eq!(run["status"], "completed", "{run}");
    assert!(run.get("failure_count").is_none(), "{run}");
    w.assert_all_scripted();
    let open = w.memory.call("issues", json!({})).await;
    assert!(
        !open.to_string().contains("iss-harbor-db"),
        "still open: {open}"
    );
}
