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
        crate::support::skipped("ling_mem_scan_writes_one_days_candidates", "no jq or perl in /usr/bin");
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

#[test]
fn ling_mem_scan_sessions_lists_one_day_without_candidates() {
    if !scan_tools() {
        crate::support::skipped("ling_mem_scan_sessions_lists_one_day_without_candidates", "no jq or perl in /usr/bin");
        return;
    }
    let home = crate::support::home::Home::new();
    write_sessions(&home.home());
    let env = crate::support::process::hermetic_env(&home);
    let candidates = home.home().join(".linggen/memory/.scan-output.jsonl");
    let out = scan(&env, None, &["--sessions", DAY]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(out.stderr.is_empty());
    let rows: Vec<Value> = String::from_utf8(out.stdout).unwrap().lines()
        .map(|line| serde_json::from_str(line).unwrap()).collect();
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|row| row["date"] == DAY && row["filepath"].is_string()));
    assert!(!candidates.exists());
    std::fs::create_dir_all(candidates.parent().unwrap()).unwrap();
    std::fs::write(&candidates, "unchanged\n").unwrap();
    assert!(scan(&env, None, &["--sessions", DAY]).status.success());
    assert_eq!(std::fs::read_to_string(candidates).unwrap(), "unchanged\n");
}

#[test]
fn ling_mem_scan_resolves_the_owner_without_a_running_engine() {
    if !scan_tools() {
        crate::support::skipped("ling_mem_scan_resolves_the_owner_without_a_running_engine", "jq or perl missing");
        return;
    }
    let home = crate::support::home::Home::new();
    write_sessions(&home.home());
    let sessions = home.linggen_home().join("sessions");
    for (id, stamp) in [("sess-owner", "u-owner"), ("sess-other", "u-owner-extra")] {
        let dir = sessions.join(id);
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("session.yaml"), format!("creator: user\nuser_id: {stamp}\n"))
            .unwrap();
        let file = dir.join("messages.jsonl");
        std::fs::copy(sessions.join(LING_SESSION).join("messages.jsonl"), &file).unwrap();
        date_file(&file);
    }
    let account = home.linggen_home().join("account.toml");
    std::fs::write(&account, "api_token = 'unused'\nuser_id = 'u-owner'\n").unwrap();
    let mut env = crate::support::process::hermetic_env(&home);
    env.push(("LINGGEN_SCAN_OWNER_ID".into(), "u-owner-extra".into()));
    let list = || {
        let out = scan(&env, None, &["--sessions", DAY]);
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8(out.stdout).unwrap()
    };
    let signed_in = list();
    assert!(signed_in.contains("sess-owner/messages.jsonl"), "{signed_in}");
    assert!(!signed_in.contains("sess-other/messages.jsonl"), "{signed_in}");
    std::fs::write(&account, "not valid TOML\n").unwrap();
    let unreadable = scan(&env, None, &["--sessions", DAY]);
    assert!(!unreadable.status.success());
    assert!(unreadable.stdout.is_empty());
    assert!(String::from_utf8_lossy(&unreadable.stderr).contains("cannot resolve owner identity"));
    std::fs::remove_file(account).unwrap();
    let signed_out = list();
    assert!(!signed_out.contains("sess-owner/messages.jsonl"), "{signed_out}");
    assert!(!signed_out.contains("sess-other/messages.jsonl"), "{signed_out}");
    assert!(signed_out.contains(LING_SESSION), "{signed_out}");
}

#[test]
fn ling_mem_scan_sessions_requires_a_day() {
    let home = crate::support::home::Home::new();
    let env = crate::support::process::hermetic_env(&home);
    for args in [&["--sessions"][..], &["--sessions", "week"], &["--sessions", "3d"]] {
        let out = scan(&env, None, args);
        assert_eq!(out.status.code(), Some(2));
        assert!(out.stdout.is_empty());
        assert_eq!(String::from_utf8_lossy(&out.stderr),
            "ling mem scan --sessions needs one day, YYYY-MM-DD\n");
    }
}

#[test]
fn ling_mem_scan_extract_reads_one_session_without_candidates() {
    if !scan_tools() {
        crate::support::skipped("ling_mem_scan_extract_reads_one_session_without_candidates", "no jq or perl in /usr/bin");
        return;
    }
    let home = crate::support::home::Home::new();
    let env = crate::support::process::hermetic_env(&home);
    let file = home.home().join("session.jsonl");
    let rows = [
        json!({"type":"user", "timestamp":"2026-11-10T12:00:00Z", "message":{"role":"user", "content":"Alex likes gardens."}}),
        json!({"type":"assistant", "timestamp":"2026-11-10T12:00:01Z", "message":{"role":"assistant", "content":[{"type":"text", "text":"Alex grows herbs."}]}}),
    ];
    std::fs::write(&file, rows.iter().map(Value::to_string).collect::<Vec<_>>().join("\n") + "\n").unwrap();
    let file = file.to_str().unwrap();
    let out = scan(&env, None, &["--extract", file, "--source", "CC", "--day", DAY, "--max-chars", "0"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(String::from_utf8_lossy(&out.stdout), "[user]: Alex likes gardens.\n[assistant]: Alex grows herbs.\n");
    assert!(out.stderr.is_empty());
    assert!(!home.home().join(".linggen/memory/.scan-output.jsonl").exists());
    let missing = scan(&env, None, &["--extract", "/missing-session", "--source", "CC"]);
    assert_eq!(missing.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&missing.stderr).contains("file not found"));
    let default_day = scan(&env, None, &["--extract", file, "--source", "CC", "--max-chars", "0"]);
    assert!(default_day.status.success());
}

#[test]
fn ling_mem_scan_extract_refuses_bad_arguments() {
    let home = crate::support::home::Home::new();
    let env = crate::support::process::hermetic_env(&home);
    let cases: &[(&[&str], &str)] = &[
        (&["--extract", "file"], "--extract needs --source"),
        (&["--extract", "file", "--source", "bad"], "--source must be CC, Codex, OpenClaw, or Linggen"),
        (&["--extract", "file", "--source", "CC", "--day", "week"], "--day needs one day, YYYY-MM-DD"),
        (&["--extract", "file", "--source", "CC", "--max-chars", "abc"], "--max-chars needs a non-negative whole number"),
        (&["--extract", "file", "--sessions", DAY], "--extract cannot be combined with --sessions"),
        (&["--extract", "file", "--source", "CC", "week"], "--extract does not take a window"),
        (&["--source", "CC"], "--source needs --extract"),
        (&["--day", DAY], "--day needs --extract"),
        (&["--max-chars", "10"], "--max-chars needs --extract"),
    ];
    for (args, message) in cases {
        let out = scan(&env, None, args);
        assert_eq!(out.status.code(), Some(2), "{args:?}");
        assert!(out.stdout.is_empty());
        assert_eq!(String::from_utf8_lossy(&out.stderr), format!("ling mem scan {message}\n"));
    }
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

/// Read one candidate line of the scan file: its opening only (`head`), or
/// the whole of it.
fn read_line(n: u64, head: bool) -> crate::support::model::Reply {
    let max = if head { 600 } else { 250_000 };
    tool(
        "Read",
        json!({"path": OUT, "line_range": [n, n], "max_bytes": max}),
    )
}

/// Turn 1 of a scan: the header, each candidate's opening, the day's rows.
fn plan_turn() -> Vec<crate::support::model::Reply> {
    vec![
        read_line(1, true),
        read_line(2, true),
        read_line(3, true),
        tool(
            "mcp__memory__memory_search",
            json!({"op": "list", "tier": "episodic", "day": DAY, "limit": 100}),
        ),
        tool(
            "mcp__memory__memory_search",
            json!({"op": "list", "tier": "semantic", "day": DAY, "limit": 100}),
        ),
    ]
}

/// Wait until the mission's newest run is on record as running — its one-run
/// slot is taken by then.
async fn until_running(w: &World, mission: &str) {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(30) {
        let runs = w.api.get(&format!("/api/missions/{mission}/runs")).await;
        if runs["runs"][0]["status"] == "running" {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("the {mission} run never started");
}

/// Settings → Memory's Scan for a day: the ENGINE walks the session logs
/// before the first turn (the model calls no Bash — it has none); the run
/// reads the candidates, stages one row per session carrying the session it
/// came from, and stamps the day scanned, with nothing refused. A second
/// Scan while it runs gets 409. Scanned again, the run's own day listing
/// shows each session's id — what the covered-session skip reads.
#[tokio::test]
async fn a_scan_run_stages_the_days_sessions_and_stamps_the_day() {
    if !crate::support::memory::embedder_available() || !scan_tools() {
        crate::support::skipped("a_scan_run_stages_the_days_sessions_and_stamps_the_day", "no embedding model, or no jq/perl");
        return;
    }
    // The first run plans from the header and the day's rows alone (the
    // fixture config's step limit is 12 a run); the second reads each
    // candidate's opening as well.
    let mut script = vec![
        read_line(1, true),
        tool(
            "mcp__memory__memory_search",
            json!({"op": "list", "tier": "episodic", "day": DAY, "limit": 100}),
        ),
    ];
    script.extend([
        text(&format!("SCAN {DAY} found=2 scanned=2\nTODO 2 3")),
        read_line(2, false),
        tool(
            "mcp__memory__memory_add",
            add(
                "Alex wants tabs, not spaces, in Harbor.",
                CC_SESSION,
                Some("/work/harbor"),
            ),
        ),
        text(&format!("STAGE 2 {CC_SESSION} rows=1")),
        read_line(3, false),
        tool(
            "mcp__memory__memory_add",
            add(
                "Alex waters the garden every Sunday morning.",
                LING_SESSION,
                None,
            ),
        ),
        text(&format!("STAGE 3 {LING_SESSION} rows=1")),
        tool("mcp__memory__memory_upkeep", json!({"op": "harvest_day", "date": DAY})),
        text("DONE"),
    ]);
    // The same day scanned again: both sessions already have rows.
    script.extend(plan_turn());
    script.extend([
        tool("mcp__memory__memory_upkeep", json!({"op": "harvest_day", "date": DAY})),
        text(&format!(
            "SKIP {CC_SESSION} covered\nSKIP {LING_SESSION} covered\nCLEAN"
        )),
    ]);
    let w = World::start(Setup {
        embedder: true,
        ..Setup::default().script(Agent::Memory, script)
    })
    .await;
    write_sessions(&w.home.home());

    let body = json!({"day": DAY, "attended": true});
    let sid = trigger(&w, "memory-scan", body.clone()).await;
    until_running(&w, "memory-scan").await;
    let again = w
        .api
        .post_status("/api/missions/memory-scan/trigger", body.clone())
        .await;
    assert_eq!(again, 409, "a second scan started while one was running");
    let run = finished_run(&w, "memory-scan", &sid).await;
    assert_eq!(run["status"], "completed", "{run}");
    assert!(
        run.get("failure_count").is_none(),
        "a call failed or was refused: {run}"
    );
    assert_eq!(run["attended"], true, "{run}");
    assert!(
        run.get("hit_limit").is_none(),
        "stopped at its step limit: {run}"
    );
    w.assert_all_scripted();

    let calls = w.model.calls_of(Agent::Memory);
    let prompts: String = calls.iter().map(|c| c.body.to_string()).collect();
    assert!(
        prompts.contains("Procedure — scan one day"),
        "the scan procedure is not in the run's prompt"
    );
    assert!(
        !prompts.contains("\"name\":\"Bash\""),
        "the scan run was offered Bash"
    );
    let model_saw: String = calls.iter().map(|c| c.text()).collect();
    assert!(
        prompts.contains(&format!("\\\"window\\\":\\\"{DAY}\\\"")),
        "the engine's scan never reached the model"
    );
    assert!(
        model_saw.contains("I water the garden every Sunday"),
        "the candidate was never read"
    );
    let seen = tool_results(&w, &sid);
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
    let day = day_record(&w).await;
    assert_eq!(day["scanned"], true, "{day}");
    assert_eq!(day["dreamed"], false, "a scan marks nothing dreamed: {day}");

    let again = trigger(&w, "memory-scan", body).await;
    let run = finished_run(&w, "memory-scan", &again).await;
    assert_eq!(run["status"], "completed", "{run}");
    assert!(run.get("failure_count").is_none(), "{run}");
    w.assert_all_scripted();
    let listed = tool_results(&w, &again);
    for session in [CC_SESSION, LING_SESSION] {
        assert!(
            listed.contains(&format!("source_session\\\":\\\"{session}")),
            "the day listing does not show {session}: {listed}"
        );
    }
}

/// `DAY`'s record in the `days` rollup, if it has one.
async fn day_record(w: &World) -> Value {
    let days = w.memory.call("days", json!({})).await;
    days["days"]
        .as_array()
        .and_then(|d| d.iter().find(|r| r["date"] == DAY))
        .cloned()
        .unwrap_or(Value::Null)
}

/// A mission whose `prepare` step fails: the memory scan's own shape, with
/// a window `ling mem scan` refuses.
const BROKEN_SCAN: &str = "---\n\
name: Broken scan\n\
enabled: false\n\
agent: memory\n\
cwd: ~/.linggen\n\
prepare: [mem, scan, $DAY-x]\n\
kickoff-day:\n  - \"Read the candidates for $DAY.\"\n\
allowed-tools: [mcp__memory, Read]\n\
---\n\nScan a day.\n";

/// The engine's step fails: the run ends failed before any model call, its
/// record names the step, its exit status and the tail of its error, the
/// session says why, and the day stays unscanned.
#[tokio::test]
async fn a_failing_prepare_step_ends_the_run_failed_and_unstamped() {
    let w = World::start(Setup::default()).await;
    let dir = w.home.linggen_home().join("missions/broken-scan");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("mission.md"), BROKEN_SCAN).unwrap();

    let sid = trigger(&w, "broken-scan", json!({"day": DAY, "attended": true})).await;
    let run = finished_run(&w, "broken-scan", &sid).await;
    assert_eq!(run["status"], "failed", "{run}");
    let failure = &run["failures"][0];
    assert_eq!(failure["tool"], "prepare", "{run}");
    assert_eq!(failure["exit"], 1, "{run}");
    assert!(
        failure["tail"]
            .as_str()
            .unwrap_or("")
            .starts_with("Usage: ling mem scan"),
        "{run}"
    );
    assert!(
        w.model.calls_of(Agent::Memory).is_empty(),
        "the model was called"
    );
    assert!(
        tool_results(&w, &sid).contains("This run did not start"),
        "the session does not say why"
    );
    assert_ne!(
        day_record(&w).await["scanned"],
        true,
        "a failed scan stamped the day"
    );
}

/// Settings → Memory's Solve: the run reads the queue, asks the user (it is
/// attended), and a second Solve while it waits is refused with 409; the
/// answer closes the item.
#[tokio::test]
async fn a_solve_run_asks_once_refuses_a_second_start_and_closes_the_item() {
    let script = vec![
        tool("mcp__memory__memory_upkeep", json!({"op": "role"})),
        tool("mcp__memory__memory_upkeep", json!({"op": "issues"})),
        tool(
            "AskUser",
            json!({"questions": [{
                "question": "Two notes on Harbor's database: SQLite, or Postgres? I think SQLite.",
                "header": "Harbor",
                "options": [{"label": "SQLite"}, {"label": "Postgres"}],
            }]}),
        ),
        tool(
            "mcp__memory__memory_upkeep",
            json!({"op": "issue_resolve", "id": "iss-harbor-db", "outcome": "resolved", "note": "Alex said SQLite"}),
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

/// Which of Linggen's own sessions the scan reads: a session the person
/// spoke in, whoever made it — never another person's day, a mission's or a
/// host-made session, or a private app's chat. The fixture is a script
/// beside this file (`memory_scan_day.sh`): the scan is shell, and so is
/// its test.
#[test]
fn the_scan_reads_a_session_the_person_spoke_in() {
    if !scan_tools() {
        crate::support::skipped("the_scan_reads_a_session_the_person_spoke_in", "jq or perl missing");
        return;
    }
    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/system/memory_scan_day.sh");
    let out = Command::new("bash").arg(script).output().expect("bash");
    assert!(
        out.status.success(),
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Memory on the engine's port: a host reaches it at the one door `/mcp`
/// (twelve tools, memory's five among them, and a store's answer through
/// them); an old plugin's `/mcp/memory` is told to update and runs nothing;
/// the engine's own agents reach memory's five tools at `/mem/mcp`, where a
/// retired name is unknown and nothing of the engine's dream, agents or
/// browser is listed.
#[tokio::test]
async fn hosts_reach_memory_at_the_one_door_and_the_engines_own_at_its_own() {
    let w = World::start(Setup::default()).await;
    let names = |list: &Value| -> Vec<String> {
        list["result"]["tools"]
            .as_array()
            .unwrap_or_else(|| panic!("{list}"))
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_string())
            .collect()
    };
    let rpc = |method: &str, params: Value| json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params});
    let door = w.api.post("/mcp", rpc("tools/list", json!({}))).await;
    let door = names(&door);
    assert_eq!(door.len(), 12, "{door:?}");
    assert!(door.contains(&"memory_upkeep".to_string()) && door.contains(&"agent_run".to_string()));
    let call = rpc("tools/call", json!({"name": "memory_upkeep", "arguments": {"op": "days"}}));
    let days = w.api.post("/mcp", call).await;
    let text = days["result"]["content"][0]["text"].as_str().unwrap_or_else(|| panic!("{days}"));
    assert!(serde_json::from_str::<Value>(text).unwrap().get("days").is_some(), "{text}");
    let call = rpc("tools/call", json!({"name": "memory_days", "arguments": {}}));
    let old = w.api.post("/mcp", call).await;
    assert_eq!(old["result"]["structuredContent"]["code"], "UPGRADE_PLUGIN", "{old}");

    let status = w.api.post_status("/mcp/memory", rpc("initialize", json!({}))).await;
    assert_eq!(status, 410);

    let own = w.api.post("/mem/mcp", rpc("tools/list", json!({}))).await;
    assert_eq!(
        names(&own),
        ["memory_search", "memory_add", "memory_update", "memory_delete", "memory_upkeep"]
    );
    let ops = &own["result"]["tools"][4]["inputSchema"]["properties"]["op"]["enum"];
    assert_eq!(ops.as_array().map(Vec::len), Some(11), "{ops}");
    for engine_only in ["dream_run", "dream_status"] {
        assert!(!ops.as_array().unwrap().contains(&json!(engine_only)), "{ops}");
    }
    let dream = w
        .api
        .post("/mem/mcp", rpc("tools/call", json!({"name": "memory_upkeep", "arguments": {"op": "dream_run"}})))
        .await;
    assert_eq!(dream["result"]["isError"], true, "{dream}");
    for engine_only in ["agent_run", "browser_open"] {
        let call = rpc("tools/call", json!({"name": engine_only, "arguments": {}}));
        let refused = w.api.post("/mem/mcp", call).await;
        assert_eq!(refused["error"]["message"], format!("unknown tool: {engine_only}"), "{refused}");
    }
    let retired = w
        .api
        .post("/mem/mcp", rpc("tools/call", json!({"name": "memory_get", "arguments": {"id": "x"}})))
        .await;
    assert_eq!(retired["error"]["message"], "unknown tool: memory_get", "{retired}");
    let days = w
        .api
        .post("/mem/mcp", rpc("tools/call", json!({"name": "memory_upkeep", "arguments": {"op": "days"}})))
        .await;
    let text = days["result"]["content"][0]["text"].as_str().unwrap_or_else(|| panic!("{days}"));
    assert!(serde_json::from_str::<Value>(text).unwrap().get("days").is_some(), "{text}");
}

/// A host still on an old plugin — it calls the retired memory door, or a
/// retired tool name at the one door — is remembered: which host and when,
/// once per host per day, where `ling status` and Yinyue's problems read it.
#[tokio::test]
async fn an_old_plugins_call_is_remembered_for_the_person() {
    let w = World::start(Setup::default()).await;
    let rpc = |method: &str, params: Value| json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params});
    let as_host = |ua: &'static str| reqwest::Client::builder().user_agent(ua).build().unwrap();
    let base = w.engine.base_url();
    let claude = as_host("claude-code/2.1.4 (cli)");
    for _ in 0..2 {
        let res = claude.post(format!("{base}/mcp/memory")).json(&rpc("initialize", json!({}))).send().await.unwrap();
        assert_eq!(res.status().as_u16(), 410);
    }
    let codex = as_host("codex_cli_rs/0.167.0");
    let call = rpc("tools/call", json!({"name": "memory_list", "arguments": {}}));
    let old: Value = codex.post(format!("{base}/mcp")).json(&call).send().await.unwrap().json().await.unwrap();
    assert_eq!(old["result"]["structuredContent"]["code"], "UPGRADE_PLUGIN", "{old}");

    let record = w.home.linggen_home().join("activity/old-plugin.json");
    let seen: Value = serde_json::from_str(&std::fs::read_to_string(&record).unwrap()).unwrap();
    let hosts: Vec<&String> = seen.as_object().unwrap().keys().collect();
    assert_eq!(hosts, ["claude-code", "codex"], "{seen}");
    assert_eq!(seen["claude-code"]["called"], "/mcp/memory");
    assert_eq!(seen["codex"]["called"], "memory_list");
    let today = seen["claude-code"]["day"].clone();
    assert!(today.as_str().is_some_and(|d| d.len() == 10), "{seen}");
}
