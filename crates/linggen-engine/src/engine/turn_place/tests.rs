use super::{TurnPlace, HOME_KEY};
use crate::state_fs::sessions::SessionMeta;

fn session(skill: Option<&str>, save: Option<&str>) -> SessionMeta {
    let mut meta = SessionMeta {
        id: "sess-1-place".into(),
        skill: skill.map(str::to_string),
        ..Default::default()
    };
    if let Some(save) = save {
        meta.params.insert("save".into(), save.into());
    }
    meta
}

/// A page names its skill: that is the turn's place, with the session's
/// values when the session is that skill's.
#[test]
fn a_page_names_its_place() {
    let game = session(Some("zz-game"), Some("test"));
    let place = TurnPlace::of_request(Some("zz-game"), Some(&game));
    assert_eq!(place.skill.as_deref(), Some("zz-game"));
    assert_eq!(place.params.get("save").map(String::as_str), Some("test"));
    assert!(place.own);
}

/// A page of another skill stands in its own app, and the session's values
/// stay with the session's skill.
#[test]
fn another_apps_page_is_another_place_without_the_sessions_values() {
    let game = session(Some("zz-game"), Some("test"));
    let place = TurnPlace::of_request(Some("zz-ledger"), Some(&game));
    assert_eq!(place.skill.as_deref(), Some("zz-ledger"));
    assert!(place.params.is_empty());
    assert!(!place.own);
    assert!(!place.same_app(&TurnPlace::of_session(Some(&game))));
}

/// The main chat and the phone name no skill: the session's own place.
#[test]
fn a_request_naming_no_skill_stands_in_the_sessions_place() {
    let game = session(Some("zz-game"), None);
    assert_eq!(TurnPlace::of_request(None, Some(&game)), TurnPlace::of_session(Some(&game)));
    assert_eq!(TurnPlace::of_request(Some("  "), Some(&game)).skill.as_deref(), Some("zz-game"));
    let plain = session(None, None);
    assert_eq!(TurnPlace::of_request(None, Some(&plain)), TurnPlace::home());
}

/// A request that makes its session makes it for the page's place.
#[test]
fn a_new_session_is_made_for_the_pages_place() {
    let place = TurnPlace::of_request(Some("zz-game"), None);
    assert_eq!(place.skill.as_deref(), Some("zz-game"));
    assert!(place.own);
    assert_eq!(TurnPlace::of_request(None, None), TurnPlace::home());
}

/// Her own thread, a plain chat, an agent_run session and a mission's
/// session hold no skill: their turns are home.
#[test]
fn a_session_with_no_skill_is_home() {
    for creator in ["user", "agent", "external", "mission"] {
        let meta = SessionMeta { creator: creator.into(), ..session(None, None) };
        let place = TurnPlace::of_session(Some(&meta));
        assert_eq!(place, TurnPlace::home(), "{creator}");
        assert_eq!(place.key(), HOME_KEY);
    }
}

/// Her turn at an app's table, a hand-off made into an app's session and a
/// light turn there stand in that app, with its values.
#[test]
fn a_turn_at_an_apps_table_stands_in_that_app() {
    let table = SessionMeta { creator: "agent".into(), ..session(Some("zz-game"), Some("main")) };
    let place = TurnPlace::of_session(Some(&table));
    assert_eq!(place.key(), "zz-game");
    assert_eq!(place.params.get("save").map(String::as_str), Some("main"));
    assert!(place.own);
}

/// No skill name can be mistaken for home.
#[test]
fn home_has_a_key_no_skill_can_take() {
    assert!(HOME_KEY.contains(':'));
    let place = TurnPlace::of_request(Some("zz-game"), None);
    assert_ne!(place.key(), HOME_KEY);
}
