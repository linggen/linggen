//! Presence per surface (doc/yinyue-companion-spec.md): every surface keeps
//! its own reading, and the person is the most present live one — a focused
//! but idle tab no longer overwrites the one they are working in.

use crate::support::model::{Agent, Call};
use crate::support::{text, Setup, World};
use serde_json::json;

async fn beat(w: &World, surface: &str, idle_ms: u64) {
    let body = json!({"surface": surface, "focused": true, "typing": false, "idle_ms": idle_ms});
    w.api.post("/api/presence", body).await;
}

/// Talk to her on her own thread; the Right now block her call carried.
async fn her_right_now(w: &World) -> String {
    let mark = w.mark();
    w.api
        .post("/api/yinyue/chat", json!({"text": "Are you there?"}))
        .await;
    w.wait_agent(Agent::Yinyue, mark).await;
    let call: Call = w.model.calls_of(Agent::Yinyue).pop().expect("her call");
    let them = call.thread().into_iter().find_map(|(_, t)| {
        t.lines()
            .find(|l| l.starts_with("- Them: "))
            .map(str::to_string)
    });
    them.unwrap_or_else(|| panic!("no Right now block in her call: {:?}", call.thread()))
}

/// Her own thread, with the setting off; in the day:
/// `day_session::in_the_day_her_call_reads_the_room`.
#[tokio::test]
async fn active_and_idle_focused_surfaces_read_present() {
    let w = World::start(
        Setup::default()
            .setting_off()
            .script(Agent::Yinyue, vec![text("Here."), text("Still here.")]),
    )
    .await;

    // A focused tab idle for ten minutes, alone: away.
    beat(&w, "skill-page", 600_000).await;
    let alone = her_right_now(&w).await;
    assert!(alone.starts_with("- Them: away"), "{alone}");

    // The person reads in the main UI (input 30 s ago: reading, never typing
    // however slow the run); the idle tab beats after it.
    beat(&w, "main-ui", 30_000).await;
    beat(&w, "skill-page", 600_000).await;
    let both = her_right_now(&w).await;
    assert!(
        both.starts_with("- Them: present_reading"),
        "the idle tab's beat hid the active one: {both}"
    );
    w.assert_all_scripted();
}

/// An app that keeps Yinyue away until its own state says she joined.
async fn install_gated(w: &World) {
    let dir = w.home.linggen_home().join("skills/gated");
    std::fs::create_dir_all(&dir).expect("gated dir");
    std::fs::write(
        dir.join("SKILL.md"),
        "---\nname: gated\ndescription: A test app that keeps her away.\nallowed-tools: []\n\
         members: [ling, yinyue]\nplace:\n  yinyue:\n    absent_until: {file: state.json, path: companion.joined}\n\
         ---\n\nOne line.\n",
    )
    .expect("SKILL.md");
    w.api.post("/api/skills/reload", json!({})).await;
}

/// The app's presence rule holds for a first message too: with no session
/// yet, the place is the request's app, and she is refused there before any
/// session is made or any model is called.
#[tokio::test]
async fn a_first_message_meets_the_apps_presence_rule() {
    let w = World::start(Setup::default().setting_off()).await;
    install_gated(&w).await;
    let root = w.work();
    let first = json!({"project_root": root, "message": "hi", "agent_id": "yinyue", "skill_name": "gated"});
    let refused = w.api.post("/api/chat", first).await;
    assert_eq!(refused["status"], "absent", "{refused}");
    assert!(w.model.calls_of(Agent::Yinyue).is_empty(), "a model call ran for her");
}
