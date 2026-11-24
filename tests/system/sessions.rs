//! Prewritten sessions (tests/fixtures/home/sessions): a shared table, a
//! compacted chat, a run cut off before its reply — read back the way the
//! engine and the chat page read them.

use crate::support::engine::RunEnd;
use crate::support::model::Agent;
use crate::support::{snap, snap_text, text, tool, Setup, World};
use serde_json::json;

fn work_meta(w: &World, sid: &str) -> serde_json::Value {
    serde_norway::from_str(&std::fs::read_to_string(w.home.linggen_home().join("sessions").join(sid).join("session.yaml")).unwrap()).unwrap()
}

#[tokio::test]
async fn work_create_code() {
    let w = World::start(Setup::default()).await;
    let sid = w.api.create_session(json!({"title":"code", "work":"code"})).await;
    assert_eq!(work_meta(&w, &sid)["work"], "code");
    let prompt = w.api.export(&sid, "ling", &w.work()).await;
    assert!(prompt["system_prompt"].as_str().unwrap_or("").contains("## Working on code"));
}

#[tokio::test]
async fn native_model_receives_imported_rules_but_not_secret_or_chat_imports() {
    let w = World::start(Setup::default().script(Agent::Ling, vec![text("Done.")])).await;
    let root = w.work();
    let project = std::path::Path::new(&root);
    let secret = w.home.linggen_home().join("account.toml");
    std::fs::write(&secret, "INVENTED-ACCOUNT-SECRET").unwrap();
    std::fs::write(project.join("rule.md"), "IMPORTED-COMPILER-CONVENTION").unwrap();
    std::fs::write(project.join("chat-only.md"), "UNIMPORTED-CHAT-FILE").unwrap();
    std::fs::write(project.join("CLAUDE.md"), format!("@rule.md\n@{}", secret.display())).unwrap();
    let sid = w.api.create_session(json!({"title":"Rules", "project_root":root, "work":"code"})).await;
    w.turn(&sid, &root, "The literal text is @chat-only.md", Agent::Ling).await;
    let calls = w.model.calls_of(Agent::Ling);
    let sent = calls.first().expect("native model request").body.to_string();
    assert!(sent.contains("IMPORTED-COMPILER-CONVENTION") && sent.contains("Imported from"), "{sent}");
    assert!(sent.contains("not loaded: outside the project"), "{sent}");
    assert!(!sent.contains("INVENTED-ACCOUNT-SECRET"));
    assert!(!sent.contains("UNIMPORTED-CHAT-FILE"));
}

#[cfg(unix)]
#[tokio::test]
async fn coding_prompt_loads_personal_rules_before_project_rules_and_refreshes_next_turn() {
    use std::fs::{create_dir, write};
    let w = World::start(Setup::default().script(Agent::Ling, vec![text("Done."); 3])).await;
    let home = w.home.home();
    let personal = home.join(".claude/CLAUDE.md");
    let style = home.join("style.md");
    create_dir(personal.parent().unwrap()).unwrap();
    write(&personal, "ORIGINAL-PERSONAL-RULE\n@~/style.md").unwrap();
    write(&style, "ORIGINAL-PERSONAL-STYLE").unwrap();
    write(w.home.work().join("AGENTS.md"), "LOCAL-PROJECT-RULE").unwrap();
    let root = w.work();
    let sid = w.api.create_session(json!({"title":"Code", "project_root":root, "work":"code"})).await;
    let mut sent = Vec::new();
    for message in ["First turn", "Second turn", "Third turn"] {
        w.turn(&sid, &root, message, Agent::Ling).await;
        sent.push(w.model.calls_of(Agent::Ling).last().unwrap().body.to_string());
        match message {
            "First turn" => write(&personal, "UPDATED-PERSONAL-RULE\n@~/style.md").unwrap(),
            "Second turn" => write(&style, "UPDATED-PERSONAL-STYLE").unwrap(),
            _ => {}
        }
    }
    assert!(sent[0].contains("Personal") && sent[0].contains(personal.to_str().unwrap()));
    assert!(sent[0].find("ORIGINAL-PERSONAL-RULE").unwrap() < sent[0].find("ORIGINAL-PERSONAL-STYLE").unwrap());
    assert!(sent[0].find("ORIGINAL-PERSONAL-STYLE").unwrap() < sent[0].find("LOCAL-PROJECT-RULE").unwrap());
    assert!(sent[1].contains("UPDATED-PERSONAL-RULE") && sent[1].contains("ORIGINAL-PERSONAL-STYLE"));
    assert!(sent[2].contains("UPDATED-PERSONAL-RULE") && sent[2].contains("UPDATED-PERSONAL-STYLE"));
    assert!(!sent[2].contains("ORIGINAL-PERSONAL-STYLE"));
}

#[tokio::test]
async fn daily_chat_app_and_yinyue_receive_neither_personal_rule_nor_import() {
    use std::fs::{create_dir, write};
    let w = World::start(Setup {
        daily_session: Some(true),
        ..Setup::default()
    }.script(Agent::Ling, vec![text("Hi."); 2]).script(Agent::Yinyue, vec![text("Hello.")])).await;
    let home = w.home.home();
    let personal = home.join(".claude/CLAUDE.md");
    create_dir(personal.parent().unwrap()).unwrap();
    write(&personal, "PERSONAL-CHAT-CONTROL\n@~/style.md").unwrap();
    write(home.join("style.md"), "IMPORTED-CHAT-CONTROL").unwrap();
    let root = w.work();
    let day = w.api.get("/api/sessions/day").await;
    let day = day["id"].as_str().expect("daily session");
    crate::day_session::turn_at(&w, day, None, "Hello", Agent::Ling).await;
    let app = w.api.create_session(json!({"title":"App", "skill":"dice"})).await;
    crate::day_session::turn_at(&w, &app, Some("dice"), "Hello", Agent::Ling).await;
    let code = w.api.create_session(json!({"title":"Code", "project_root":root, "work":"code"})).await;
    w.turn(&code, &root, "@银月 hello", Agent::Yinyue).await;
    for agent in [Agent::Ling, Agent::Yinyue] {
        let calls = w.model.calls_of(agent);
        assert_eq!(calls.len(), if agent == Agent::Ling { 2 } else { 1 });
        for call in calls {
            let sent = call.body.to_string();
            assert!(!sent.contains("PERSONAL-CHAT-CONTROL") && !sent.contains("IMPORTED-CHAT-CONTROL"));
        }
    }
}

#[tokio::test]
async fn personal_import_refusal_reaches_the_provider_but_protected_contents_do_not() {
    use std::fs::{create_dir, write};
    let w = World::start(Setup::default().script(Agent::Ling, vec![text("Done.")])).await;
    let home = w.home.home();
    let personal = home.join(".claude/CLAUDE.md");
    create_dir(personal.parent().unwrap()).unwrap();
    create_dir(home.join(".ssh")).unwrap();
    write(home.join(".ssh/test-key"), "INVENTED-PERSONAL-KEY").unwrap();
    write(w.home.linggen_home().join("account.toml"), "INVENTED-PERSONAL-ACCOUNT").unwrap();
    write(&personal, "@~/.ssh/test-key\n@~/.linggen/account.toml").unwrap();
    let root = w.work();
    let sid = w.api.create_session(json!({"title":"Code", "project_root":root, "work":"code"})).await;
    w.turn(&sid, &root, "Check the rules", Agent::Ling).await;
    let sent = w.model.calls_of(Agent::Ling).first().expect("native model request").body.to_string();
    for token in ["~/.ssh/test-key", "~/.linggen/account.toml"] {
        assert!(sent.contains(&format!("[Import @{token} not loaded: protected or requires approval.]")));
    }
    assert!(!sent.contains("INVENTED-PERSONAL-KEY") && !sent.contains("INVENTED-PERSONAL-ACCOUNT"));
}

#[tokio::test]
async fn work_create_app_refused() {
    let w = World::start(Setup::default()).await;
    let dir = w.home.linggen_home().join("sessions");
    let before = std::fs::read_dir(&dir).unwrap().count();
    assert_eq!(w.api.post_status("/api/sessions", json!({"title":"app", "skill":"dice", "work":"code"})).await, 400);
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), before);
}

#[tokio::test]
async fn work_update_switch() {
    let w = World::start(Setup::default()).await;
    let sid = w.api.create_session(json!({"title":"chat"})).await;
    let client = reqwest::Client::new();
    for (value, expected, status) in [("code", json!("code"), 200), ("none", json!(null), 200), ("bad", json!(null), 400)] {
        let response = client.patch(format!("{}/api/sessions", w.engine.base_url())).json(&json!({"session_id":sid, "work":value})).send().await.unwrap();
        assert_eq!(response.status(), status);
        assert_eq!(work_meta(&w, &sid)["work"], expected);
    }
}

/// A session's compaction settings: each partial change keeps the other
/// field, and a session with no file to keep them in is an error, not ok.
#[tokio::test]
async fn compact_config_merges_and_refuses_an_unkept_session() {
    let w = World::start(Setup::default()).await;
    let sid = w.api.create_session(json!({"title":"chat"})).await;
    let threshold = w.api.post("/api/chat/compact_config", json!({"session_id": sid, "threshold": 0.5})).await;
    assert_eq!(threshold["ok"], true, "{threshold}");
    let focus = w.api.post("/api/chat/compact_config", json!({"session_id": sid, "focus": "Keep the paths."})).await;
    assert_eq!((focus["threshold"].as_f64(), focus["focus"].as_str()), (Some(0.5), Some("Keep the paths.")), "{focus}");
    let kept = work_meta(&w, &sid);
    assert_eq!((kept["compact_threshold"].as_f64(), kept["compact_focus"].as_str()), (Some(0.5), Some("Keep the paths.")));

    let missing = json!({"session_id": "sess-1790000099-0000dead", "threshold": 0.5});
    assert_eq!(w.api.post_status("/api/chat/compact_config", missing).await, 404);
}

#[tokio::test]
async fn permission_patch_reports_a_failed_save() {
    let w = World::start(Setup::default()).await;
    let sid = w.api.create_session(json!({"title":"chat"})).await;
    let file = w.home.linggen_home().join("sessions").join(&sid).join("permission.json");
    if file.is_file() {
        std::fs::remove_file(&file).unwrap();
    }
    std::fs::create_dir(&file).unwrap();
    let response = reqwest::Client::new()
        .patch(format!("{}/api/sessions/permission", w.engine.base_url()))
        .json(&json!({"session_id":sid, "path":w.work(), "mode":"read"}))
        .send().await.unwrap();
    assert_eq!(response.status(), 500);
    assert!(file.is_dir());
}

const SHARED: &str = "sess-1790000001-5a1ed001";
const COMPACTED: &str = "sess-1790000002-c0ac7ed2";
const INTERRUPTED: &str = "sess-1790000003-1e7e5ed3";

/// A Claude session from 2.5.1 (the fixture, written as the SDK host wrote
/// it): its history opens, tool rows as tools, not bubbles.
const OLD_CLAUDE: &str = "sess-1790000004-c1a0de04";

/// Rule: an old Claude session is never lost and never a dead end. It opens
/// read-only — no turn, no model change — renames as before, and continues
/// in a new Linggen session in its folder whose first model call carries
/// its conversation. Clearing and deleting it work as before.
#[tokio::test]
async fn an_old_claude_session_opens_read_only_and_continues_in_linggen() {
    let w = World::start(Setup::default().script(Agent::Ling, vec![text("First car at 06:15, as before.")])).await;
    let history = w.api.history(OLD_CLAUDE, &w.work()).await;
    let shown = history["messages"].as_array().cloned().unwrap_or_default();
    assert_eq!(shown.len(), 2, "tool observations became bubbles: {history}");
    assert!(shown[1].to_string().contains("first car at 06:15"), "{history}");

    let client = reqwest::Client::new();
    let base = w.engine.base_url();
    let said = client
        .post(format!("{base}/api/chat"))
        .json(&json!({"session_id": OLD_CLAUDE, "project_root": w.work(), "message": "Hello"}))
        .send()
        .await
        .unwrap();
    assert_eq!(said.status(), 410, "an old Claude session took a turn");
    assert!(said.text().await.unwrap().contains("read-only"));
    let model = client
        .patch(format!("{base}/api/sessions"))
        .json(&json!({"session_id": OLD_CLAUDE, "agent_id": "ling", "model_id": "test"}))
        .send()
        .await
        .unwrap();
    assert_eq!(model.status(), 410, "an old Claude session changed its model");
    w.api.patch("/api/sessions", json!({"session_id": OLD_CLAUDE, "title": "Tram 28"})).await;
    let dir = w.home.linggen_home().join("sessions").join(OLD_CLAUDE);
    let meta = std::fs::read_to_string(dir.join("session.yaml")).unwrap();
    assert!(meta.contains("title: Tram 28"), "{meta}");
    assert!(meta.contains("runtime: claude_agent_sdk"), "a rename made it a Linggen session: {meta}");

    let made = w.api.post("/api/sessions/continue", json!({"session_id": OLD_CLAUDE})).await;
    let sid = made["id"].as_str().unwrap_or_else(|| panic!("no new session: {made}")).to_string();
    let named = made["model"].as_str().unwrap_or_else(|| panic!("the new session names no model: {made}")).to_string();
    assert_ne!(sid, OLD_CLAUDE);
    assert_eq!(made["cwd"], w.work(), "the new session left the old one's folder");
    let made_meta = std::fs::read_to_string(w.home.linggen_home().join("sessions").join(&sid).join("session.yaml")).unwrap();
    assert!(made_meta.contains("work: code"), "the new session is not a Coding session: {made_meta}");
    w.turn(&sid, &w.work(), "When is the first car?", Agent::Ling).await;
    let call = w.model.calls_of(Agent::Ling).pop().expect("the new session's call");
    let opening = call.thread().first().cloned().unwrap_or_default();
    assert_eq!(opening.0, "system", "the carried conversation is not a system note");
    assert!(opening.1.contains("Move the tram 28 timetable into tram28.md."), "{opening:?}");
    assert!(opening.1.contains("first car at 06:15"), "{opening:?}");
    assert!(!opening.1.contains("Tool Write"), "tool rows were carried: {opening:?}");
    assert!(opening.1.contains(&named), "the first row does not name the model {named}: {opening:?}");
    assert!(opening.1.contains("Left behind"), "the first row does not say what was left out: {opening:?}");

    // What the 2.5.1 host kept beside the conversation: a resume pointer and
    // the subagents' and usage records. Clear takes them all.
    assert!(dir.join("claude.json").exists(), "the fixture has no resume pointer");
    for name in ["claude-subagents.json", "claude-usage.json"] {
        std::fs::write(dir.join(name), "{}").unwrap();
    }
    w.api.post("/api/chat/clear", json!({"session_id": OLD_CLAUDE})).await;
    let cleared = w.api.history(OLD_CLAUDE, &w.work()).await;
    assert_eq!(cleared["messages"].as_array().map_or(0, Vec::len), 0, "{cleared}");
    for name in ["claude.json", "claude-subagents.json", "claude-usage.json"] {
        assert!(!dir.join(name).exists(), "a cleared old session still has {name}");
    }
    assert!(dir.join("session.yaml").exists(), "clear removed the session itself");
    w.api.delete("/api/sessions/all", json!({"session_id": OLD_CLAUDE})).await;
    assert!(!dir.exists(), "the old session was not deleted");
    w.assert_all_scripted();
}

/// Rule: every door that refuses an old Claude session says why, in the same
/// words, in one short line.
#[tokio::test]
async fn every_door_refuses_an_old_claude_session_with_the_same_line() {
    let w = World::start(Setup::default()).await;
    let client = reqwest::Client::new();
    let base = w.engine.base_url();
    let work = w.work();
    let doors = [
        client.post(format!("{base}/api/chat")).json(&json!({"session_id": OLD_CLAUDE, "project_root": work, "message": "Hello"})),
        client.post(format!("{base}/api/chat/compact")).json(&json!({"session_id": OLD_CLAUDE, "project_root": work})),
        client.patch(format!("{base}/api/sessions")).json(&json!({"session_id": OLD_CLAUDE, "agent_id": "ling", "model_id": "test"})),
        client.patch(format!("{base}/api/sessions/permission")).json(&json!({"session_id": OLD_CLAUDE, "path": work, "mode": "edit"})),
        client.get(format!("{base}/api/skill-sessions/state")).query(&[("skill", "any"), ("session_id", OLD_CLAUDE)]),
    ];
    let mut lines = Vec::new();
    for door in doors {
        let answer = door.send().await.unwrap();
        assert_eq!(answer.status(), 410);
        lines.push(answer.text().await.unwrap());
    }
    assert!(lines.windows(2).all(|pair| pair[0] == pair[1]), "the doors say different things: {lines:?}");
    assert!(lines[0].split_whitespace().count() <= 20, "{}", lines[0]);
    assert!(lines[0].contains("read-only"), "{}", lines[0]);
}

/// Rule: a request that still names the Claude Agent SDK runtime (a 2.5.1
/// page or script) gets a Linggen session, and it runs natively.
#[tokio::test]
async fn a_request_for_a_claude_sdk_session_gets_a_linggen_session() {
    let w = World::start(Setup::default().script(Agent::Ling, vec![text("Here, natively.")])).await;
    let sid = w
        .api
        .create_session(json!({"title": "Claude", "runtime": "claude_agent_sdk", "project_root": w.work()}))
        .await;
    let meta = std::fs::read_to_string(w.home.linggen_home().join("sessions").join(&sid).join("session.yaml")).unwrap();
    assert!(!meta.contains("claude"), "{meta}");
    w.turn(&sid, &w.work(), "Hello", Agent::Ling).await;
    w.assert_all_scripted();
}

/// Each member's thread, built from the one session thread: his own Look as
/// native call/result pairs, hers as labelled dialogue — and for her, his
/// Look as labelled text carrying only what it declares for others.
#[tokio::test]
async fn shared_session_builds_each_members_thread() {
    let w = World::start(
        Setup::default()
            .script(Agent::Ling, vec![text("Your roll.")])
            .script(Agent::Yinyue, vec![text("Good luck.")]),
    )
    .await;
    w.turn(SHARED, "", "Roll again?", Agent::Ling).await;
    w.turn(SHARED, "", "@银月 wish me luck", Agent::Yinyue)
        .await;

    let his = w.model.calls_of(Agent::Ling).pop().expect("his call");
    let hers = w.model.calls_of(Agent::Yinyue).pop().expect("her call");
    let thread = his.thread();
    let native_call = thread
        .iter()
        .any(|(r, t)| r == "assistant" && t.contains("[call Look"));
    let native_result = thread.iter().any(|(r, _)| r == "tool");
    assert!(
        native_call && native_result,
        "his Look is not a native call/result pair"
    );
    assert!(hers.text().contains("Ling used Look"));
    assert!(
        !hers.text().contains("Never mention this guide"),
        "she read his guide"
    );
    snap("shared_ling_thread", &w, &his);
    snap("shared_yinyue_thread", &w, &hers);
    w.assert_all_scripted();
}

/// A compacted session: the next thread starts at the summary row, and no
/// row before it reaches the model.
#[tokio::test]
async fn compacted_session_thread_starts_at_the_summary() {
    let w =
        World::start(Setup::default().script(Agent::Ling, vec![text("Yes, every thirty.")])).await;
    w.turn(
        COMPACTED,
        &w.work(),
        "Still every thirty on Sundays?",
        Agent::Ling,
    )
    .await;
    let call = w.model.calls_of(Agent::Ling).pop().expect("his call");
    let opening = call.thread().first().cloned().unwrap_or_default();
    assert_eq!(opening.0, "system", "the summary is not a system note");
    assert!(opening
        .1
        .contains("SUMMARY: Alex asked about the Cacilhas ferry"));
    assert!(
        !call.text().contains("(before the summary)"),
        "a row before the summary reached the model"
    );
    snap("compacted_ling_thread", &w, &call);
    w.assert_all_scripted();
}

/// A run cut off before its reply shows, in the history view, one display
/// row after its last row: `run: "interrupted"` with its calls.
#[tokio::test]
async fn interrupted_run_has_a_display_row() {
    let w = World::start(Setup::default()).await;
    let history = w.api.history(INTERRUPTED, &w.work()).await;
    let rows = history["messages"].as_array().cloned().unwrap_or_default();
    let cut = rows.iter().find(|r| r[0]["run"] == "interrupted");
    let cut = cut.unwrap_or_else(|| panic!("no interrupted row in {history:#}"));
    assert!(
        cut.to_string().contains("Glob"),
        "the row does not carry its call: {cut}"
    );
    let pretty = serde_json::to_string_pretty(&history["messages"]).expect("json");
    snap_text("interrupted_history", &w, &pretty);
    w.assert_all_scripted();
}

/// A message sent with no folder (a page still connecting has no session
/// list to read it from) runs in the session's own folder, not in `~`: the
/// Write lands in the session's cwd.
#[tokio::test]
async fn a_message_without_a_folder_runs_in_the_sessions_own() {
    let w = World::start(Setup::default().script(
        Agent::Ling,
        vec![
            tool("Write", json!({"path": "probe.txt", "content": "hello"})),
            text("Wrote it."),
        ],
    ))
    .await;
    let mark = w.mark();
    w.api
        .chat(COMPACTED, "", "Write probe.txt saying hello.")
        .await;
    let RunEnd::Asked(ask) = w.wait_run(COMPACTED, Agent::Ling, mark).await else {
        panic!("the Write ran without a permission prompt");
    };
    w.api.answer(&ask, "grant:all").await;
    w.finish_run(COMPACTED, Agent::Ling, mark).await;

    let written = std::fs::read_to_string(w.home.work().join("probe.txt"));
    assert_eq!(
        written.ok().as_deref(),
        Some("hello"),
        "not written in the session's folder"
    );
    assert!(!w.home.home().join("probe.txt").exists(), "written in ~");
    w.assert_all_scripted();
}

/// A user session's turns keep the world block in the thread: each turn's
/// model request begins with the whole of the last one's, the blocks stay
/// in order with their `as of` times, and the history view lists none.
#[tokio::test]
async fn kept_world_blocks_let_each_request_begin_with_the_last() {
    let w = World::start(
        Setup::default().script(Agent::Ling, vec![text("one"), text("two"), text("three")]),
    )
    .await;
    let sid = w.api.create_session(json!({"title": "world"})).await;
    for words in ["first", "second", "third"] {
        w.turn(&sid, &w.work(), words, Agent::Ling).await;
    }
    let calls = w.model.calls_of(Agent::Ling);
    assert_eq!(calls.len(), 3);
    for pair in calls.windows(2) {
        let (a, b) = (pair[0].thread(), pair[1].thread());
        assert_eq!(pair[0].body["messages"][0], pair[1].body["messages"][0]);
        assert_eq!(a[..], b[..a.len()], "a request begins with the last one");
    }
    let last = calls[2].thread();
    let kept: Vec<&String> = last
        .iter()
        .filter(|(role, text)| role == "system" && text.starts_with("# World\n"))
        .map(|(_, text)| text)
        .collect();
    assert!(!kept.is_empty(), "{last:?}");
    assert!(kept.iter().all(|t| t.contains("as of ")), "{kept:?}");
    let first_words = last.iter().position(|(_, t)| t == "first").unwrap();
    assert!(last[first_words + 1].1.starts_with("# World\n"), "the first block follows its turn");

    let history = w.api.history(&sid, &w.work()).await;
    assert!(!history["messages"].to_string().contains("# World"), "listed: {history}");
    w.assert_all_scripted();
}

/// A message sent while a turn runs keeps the surface's `client_id` on its
/// row when it leaves the queue, so the surface can follow it to its end.
#[tokio::test]
async fn a_queued_message_keeps_its_client_id() {
    let ask = json!({"questions": [{"question": "Which box?", "header": "Box",
        "options": [{"label": "Red"}, {"label": "Blue"}]}]});
    let setup = Setup::default().script(
        Agent::Ling,
        vec![tool("AskUser", ask), text("Red."), text("Queued one answered.")],
    );
    let w = World::start(setup).await;
    let sid = w.api.create_session(json!({"title": "Queue"})).await;
    let mark = w.mark();
    w.api.chat(&sid, &w.work(), "Pick a box").await;
    let RunEnd::Asked(asked) = w.wait_run(&sid, Agent::Ling, mark).await else {
        panic!("no question");
    };
    let body = json!({"session_id": sid, "project_root": w.work(), "message": "QUEUED words", "client_id": "c-queued-1"});
    let sent = w.api.post("/api/chat", body).await;
    w.api.answer(&asked, "Red").await;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        let rows = w.raw_rows(&sid);
        let row = rows.iter().find(|r| r.to_string().contains("QUEUED words"));
        if let Some(row) = row {
            assert_eq!(row["client_id"], "c-queued-1", "{row} (sent: {sent})");
            break;
        }
        assert!(std::time::Instant::now() < deadline, "the queued message never left the queue: {sent}");
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

/// A page removes only its own app's sessions: one of another app, a plain
/// chat, or a call naming no app is refused, and nothing is removed.
#[tokio::test]
async fn a_page_removes_only_its_own_apps_sessions() {
    let w = World::start(Setup::default().setting_off()).await;
    let dice = w.api.create_session(json!({"title": "Dice", "skill": "dice"})).await;
    let plain = w.api.create_session(json!({"title": "chat"})).await;
    let refused = [
        (json!({"skill": "quiet", "session_id": dice}), "another app's"),
        (json!({"session_id": dice}), "no app named"),
        (json!({"skill": "", "session_id": plain}), "a plain chat, empty name"),
        (json!({"session_id": plain}), "a plain chat, no app named"),
    ];
    for (body, what) in refused {
        assert_eq!(w.api.delete_status("/api/skill-sessions", body).await, 403, "{what}");
    }
    assert!(w.rows_opt(&dice).is_some() && w.rows_opt(&plain).is_some(), "a refused call removed one");
    let own = json!({"skill": "dice", "session_id": dice});
    assert_eq!(w.api.delete_status("/api/skill-sessions", own).await, 200);
    assert!(w.rows_opt(&dice).is_none(), "its own app's session stays");
}

/// The conversation's effort: only a value the answering model offers is
/// kept across a restart. A model change clears it so the new model can use
/// its saved default. The models list tells the slider what is offered.
#[tokio::test]
async fn a_sessions_effort_is_kept_checked_and_survives_a_restart() {
    const ROW: &str = r#"
[[models]]
id = "reasoning"
provider = "openai"
url = "https://api.openai.com/v1"
model = "gpt-5.4"
api_key = "test-not-a-key"
"#;
    let mut w = World::start(Setup { config_append: Some(ROW), ..Setup::default() }).await;
    let offered = |models: &serde_json::Value, id: &str| {
        models.as_array().unwrap().iter().find(|m| m["id"] == id).map(|m| m["effort"].clone())
    };
    let models = w.api.get("/api/models").await;
    assert_eq!(offered(&models, "reasoning"), Some(json!(["low", "medium", "high"])));
    assert_eq!(offered(&models, "fake-ling"), Some(json!([])));

    let sid = w.api.create_session(json!({"title": "Chat"})).await;
    // The standing model offers none, and nothing is kept for it.
    assert_eq!(w.api.patch_status("/api/sessions", json!({"session_id": sid, "effort": "high"})).await, 400);
    assert!(work_meta(&w, &sid).get("effort").is_none());
    // The same request that moves the member to a model with it is fine.
    let both = json!({"session_id": sid, "agent_id": "ling", "model_id": "reasoning", "effort": "high"});
    assert_eq!(w.api.patch_status("/api/sessions", both).await, 200);
    assert_eq!(work_meta(&w, &sid)["effort"], "high");
    // A value no wire carries is refused and the kept one stays.
    assert_eq!(w.api.patch_status("/api/sessions", json!({"session_id": sid, "effort": "xhigh"})).await, 400);
    assert_eq!(work_meta(&w, &sid)["effort"], "high");
    w.engine.restart(&w.home).await;
    assert_eq!(work_meta(&w, &sid)["effort"], "high");
    let other = json!({"session_id": sid, "agent_id": "ling", "model_id": "fake-ling"});
    assert_eq!(w.api.patch_status("/api/sessions", other).await, 200);
    assert!(work_meta(&w, &sid).get("effort").is_none());
    let back = json!({"session_id": sid, "effort": ""});
    assert_eq!(w.api.patch_status("/api/sessions", back).await, 200);
    assert!(work_meta(&w, &sid).get("effort").is_none());
}
