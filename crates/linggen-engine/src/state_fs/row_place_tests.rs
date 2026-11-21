//! Rows carry the place they were written in; rows from before places read
//! as the session's own.

use super::sessions::{ChatMsg, SessionMeta, SessionStore};
use super::turn_place::{TurnPlace, HOME_KEY};

fn row(content: &str) -> ChatMsg {
    ChatMsg {
        agent_id: "ling".into(),
        from_id: "user".into(),
        to_id: "ling".into(),
        content: content.into(),
        timestamp: 1000,
        is_observation: false,
        client_id: None,
        failed: false,
        via: None,
        via_note: None,
        images: Vec::new(),
        run_id: None,
        place: None,
    }
}

fn game_session() -> SessionMeta {
    SessionMeta {
        id: "sess-1-rows".into(),
        title: "game".into(),
        skill: Some("zz-game".into()),
        creator: "skill".into(),
        ..Default::default()
    }
}

/// A row saved while a turn runs carries that turn's place; one saved with
/// no turn running carries none.
#[test]
fn a_row_saved_in_a_turn_carries_its_place() {
    let dir = tempfile::tempdir().unwrap();
    let store = SessionStore::with_sessions_dir(dir.path().join("sessions"));
    let meta = game_session();
    store.add_session(&meta).unwrap();

    store.add_chat_message(&meta.id, &row("before any turn")).unwrap();
    let ledger = TurnPlace::of_request(Some("zz-ledger"), Some(&meta));
    let mark = store.running_places.mark(&meta.id, &ledger).unwrap();
    store.add_chat_message(&meta.id, &row("in the ledger's turn")).unwrap();
    drop(mark);
    let mark = store.running_places.mark(&meta.id, &TurnPlace::of_session(Some(&meta))).unwrap();
    store.add_chat_message(&meta.id, &row("in the game's turn")).unwrap();
    drop(mark);

    let rows = store.get_chat_history(&meta.id).unwrap();
    let places: Vec<Option<&str>> = rows.iter().map(|r| r.place.as_deref()).collect();
    assert_eq!(places, [None, Some("zz-ledger"), Some("zz-game")]);
    let read: Vec<String> = rows.iter().map(|r| r.place_in(&meta)).collect();
    assert_eq!(read, ["zz-game", "zz-ledger", "zz-game"], "an untagged row is the session's own");
}

/// A row written before places parses, and reads as the session's own place
/// — the skill's, or home for a plain chat. A tagged row round-trips.
#[test]
fn an_old_row_reads_as_the_sessions_own_place() {
    let old = r#"{"agent_id":"ling","from_id":"user","to_id":"ling","content":"hi","timestamp":1,"is_observation":false}"#;
    let parsed: ChatMsg = serde_json::from_str(old).unwrap();
    assert_eq!(parsed.place, None);
    assert_eq!(parsed.place_in(&game_session()), "zz-game");
    assert_eq!(parsed.place_in(&SessionMeta::default()), HOME_KEY);

    let tagged = ChatMsg { place: Some(HOME_KEY.into()), ..parsed };
    let line = serde_json::to_string(&tagged).unwrap();
    assert!(line.contains(r#""place":":home""#));
    let back: ChatMsg = serde_json::from_str(&line).unwrap();
    assert_eq!(back.place_in(&game_session()), HOME_KEY);
    // An untagged row is written as it always was.
    assert!(!serde_json::to_string(&row("x")).unwrap().contains("place"));
}

/// A rewrite (compaction, a clear) keeps each row's place.
#[test]
fn a_rewrite_keeps_each_rows_place() {
    let dir = tempfile::tempdir().unwrap();
    let store = SessionStore::with_sessions_dir(dir.path().join("sessions"));
    let meta = game_session();
    store.add_session(&meta).unwrap();
    let rows = vec![row("old"), ChatMsg { place: Some("zz-ledger".into()), ..row("new") }];
    store.rewrite_chat_history(&meta.id, &rows).unwrap();
    let back = store.get_chat_history(&meta.id).unwrap();
    assert_eq!(back, rows);
}
