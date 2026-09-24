//! The typed payloads must put on the wire exactly what the old `json!`
//! objects did — the UI reads these keys by name.

use super::map_server_event_to_ui_message as map;
use crate::engine::events::ServerEvent;
use serde_json::json;

fn data(event: ServerEvent) -> serde_json::Value {
    map(event, 7).and_then(|e| e.data).expect("event carries data")
}

#[test]
fn a_message_names_its_role_and_keeps_null_routing_keys() {
    let d = data(ServerEvent::Message {
        from: "user".into(),
        to: "ling".into(),
        content: "hi".into(),
        session_id: Some("s".into()),
        run_id: None,
        parent_agent_id: None,
    });
    assert_eq!(
        d,
        json!({"from": "user", "to": "ling", "role": "user", "run_id": null, "parent_agent_id": null})
    );
}

#[test]
fn a_block_update_carries_its_extras_flat_and_they_win() {
    let d = data(ServerEvent::ContentBlockUpdate {
        agent_id: "ling".into(),
        block_id: "b1".into(),
        status: Some("done".into()),
        summary: None,
        is_error: Some(false),
        parent_id: None,
        extra: Some(json!({"diff": "+a", "status": "overridden"})),
        session_id: None,
        run_id: None,
        parent_run_id: None,
    });
    assert_eq!(
        d,
        json!({
            "block_id": "b1", "status": "overridden", "summary": null, "is_error": false,
            "parent_id": null, "run_id": null, "parent_run_id": null, "diff": "+a",
        })
    );
}

#[test]
fn a_thinking_token_says_so_and_a_plain_one_carries_nothing() {
    let token = |thinking| ServerEvent::Token {
        agent_id: "ling".into(),
        token: "x".into(),
        done: false,
        thinking,
        session_id: None,
    };
    assert_eq!(data(token(true)), json!({"thinking": true}));
    assert!(map(token(false), 1).unwrap().data.is_none());
}

#[test]
fn a_new_session_notification_is_tagged_session_created() {
    let d = data(ServerEvent::SessionCreated {
        session_id: "s".into(),
        title: "t".into(),
        creator: "user".into(),
        project: None,
        project_name: None,
        skill: Some("cfo".into()),
        mission_id: None,
    });
    assert_eq!(d["kind"], "session_created");
    assert_eq!(d["skill"], "cfo");
    assert!(d["project"].is_null());
}
