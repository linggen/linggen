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

#[tokio::test]
async fn active_and_idle_focused_surfaces_read_present() {
    let w = World::start(
        Setup::default().script(Agent::Yinyue, vec![text("Here."), text("Still here.")]),
    )
    .await;

    // A focused tab idle for ten minutes, alone: away.
    beat(&w, "skill-page", 600_000).await;
    let alone = her_right_now(&w).await;
    assert!(alone.starts_with("- Them: away"), "{alone}");

    // The person works in the main UI; the idle tab beats after it.
    beat(&w, "main-ui", 0).await;
    beat(&w, "skill-page", 600_000).await;
    let both = her_right_now(&w).await;
    assert!(
        !both.starts_with("- Them: away"),
        "the idle tab's beat hid the active one: {both}"
    );
    w.assert_all_scripted();
}
