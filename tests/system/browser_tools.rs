//! A session is offered the browser's three tools (`browser_open`,
//! `browser_read`, `browser_act`), not the ten it used to be; an app that
//! still names an old one in its `allowed-tools` gets the tool that now
//! holds it (doc/tool-spec.md § Browser tools).
//!
//! Every app here is a made-up fixture installed by its test.

use crate::day_session::{day_world_at, install_app, say};
use crate::support::model::Agent;
use crate::support::{text, tool, Setup};
use serde_json::json;

/// The name an app lists, the call its model makes, and what it is told: an
/// app naming `Browser_click` reaches `browser_act` (and, with no extension
/// in this world, is told the browser is not connected); an app naming only
/// `Read` is told the tool is not allowed for it.
#[tokio::test]
async fn an_app_naming_an_old_browser_tool_reaches_the_tool_that_holds_it() {
    let click = || tool("browser_act", json!({"op": "click", "ref": "n1"}));
    let setup = Setup::default().script(
        Agent::Ling,
        vec![click(), text("Done."), click(), text("Done.")],
    );
    let (w, _) = day_world_at(12, setup).await;
    install_app(&w, "webby", "Browser_click", false).await;
    install_app(&w, "closed", "Read", false).await;

    let mine = w.api.create_session(json!({"title": "Webby", "skill": "webby"})).await;
    let mark = w.mark();
    say(&w, &mine, Some("webby"), "Click it").await;
    w.finish_run(&mine, Agent::Ling, mark).await;
    let theirs = w.api.create_session(json!({"title": "Closed", "skill": "closed"})).await;
    let mark = w.mark();
    say(&w, &theirs, Some("closed"), "Click it").await;
    w.finish_run(&theirs, Agent::Ling, mark).await;

    let calls = w.model.calls_of(Agent::Ling);
    assert_eq!(calls.len(), 4, "two calls an app");
    let offered = calls[0].tool_names();
    assert!(offered.contains(&"browser_act".to_string()), "{offered:?}");
    assert!(!offered.iter().any(|n| n.starts_with("Browser_")), "{offered:?}");
    assert!(calls[1].text().contains("browser not connected"), "{}", calls[1].text());
    let refused = calls[3].text();
    assert!(refused.contains("Tool 'browser_act' is not allowed for this agent"), "{refused}");
    w.assert_all_scripted();
}

/// A plain chat is offered the three, in the door's words, and none of the
/// ten.
#[tokio::test]
async fn a_plain_chat_is_offered_the_three_and_none_of_the_ten() {
    let w = crate::support::World::start(Setup::default().script(Agent::Ling, vec![text("Hi.")])).await;
    let sid = w.api.create_session(json!({"title": "Chat"})).await;
    w.turn(&sid, &w.work(), "Hello.", Agent::Ling).await;
    let calls = w.model.calls_of(Agent::Ling);
    let offered = calls[0].tool_names();
    for name in ["browser_open", "browser_read", "browser_act"] {
        assert!(offered.contains(&name.to_string()), "{name} in {offered:?}");
    }
    let old: Vec<&String> = offered.iter().filter(|n| n.starts_with("Browser_")).collect();
    assert!(old.is_empty(), "{old:?}");
    w.assert_all_scripted();
}
