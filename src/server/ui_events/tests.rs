//! The typed payloads must put on the wire exactly what the old `json!`
//! objects did — the UI reads these keys by name.

use super::map_server_event_to_ui_message as map;
use crate::engine::events::ServerEvent;
use serde_json::json;

fn data(event: ServerEvent) -> serde_json::Value {
    map(event, 7)
        .and_then(|e| e.data)
        .expect("event carries data")
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
        client_id: None,
        via: None,
        via_note: None,
        images: Vec::new(),
    });
    assert_eq!(
        d,
        json!({"from": "user", "to": "ling", "role": "user", "run_id": null, "parent_agent_id": null})
    );
}

#[test]
fn a_persons_message_echoes_its_client_id() {
    let d = data(ServerEvent::Message {
        from: "user".into(),
        to: "yinyue".into(),
        content: "wish me luck".into(),
        session_id: Some("s".into()),
        run_id: None,
        parent_agent_id: None,
        client_id: Some("c-1-ab".into()),
        via: None,
        via_note: None,
        images: Vec::new(),
    });
    assert_eq!(d["client_id"], json!("c-1-ab"));
}

#[test]
fn a_pictured_message_carries_its_picture_urls() {
    let url = "/api/sessions/images?session_id=s&name=ab.png".to_string();
    let d = data(ServerEvent::Message {
        from: "user".into(),
        to: "ling".into(),
        content: "what is this".into(),
        session_id: Some("s".into()),
        run_id: None,
        parent_agent_id: None,
        client_id: None,
        via: None,
        via_note: None,
        images: vec![url.clone()],
    });
    assert_eq!(d["images"], json!([url]));
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
        run_id: None,
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

/// Every event a run sends reaches the surface with its run on it: the key
/// the chat binds the event to that run's bubble by (chat-spec.md § Runs).
#[test]
fn every_run_event_carries_its_run_id() {
    let run = || Some("run-1".to_string());
    let sid = || Some("s".to_string());
    let events = vec![
        ServerEvent::Token {
            agent_id: "ling".into(),
            token: "x".into(),
            done: false,
            thinking: false,
            session_id: sid(),
            run_id: run(),
        },
        ServerEvent::TextSegment {
            agent_id: "ling".into(),
            text: "x".into(),
            parent_id: None,
            session_id: sid(),
            run_id: run(),
        },
        ServerEvent::ContentBlockStart {
            agent_id: "ling".into(),
            block_id: "b".into(),
            block_type: "tool_use".into(),
            tool: Some("Bash".into()),
            args: None,
            parent_id: None,
            session_id: sid(),
            run_id: run(),
            parent_run_id: None,
        },
        ServerEvent::ContentBlockUpdate {
            agent_id: "ling".into(),
            block_id: "b".into(),
            status: Some("done".into()),
            summary: None,
            is_error: None,
            parent_id: None,
            extra: None,
            session_id: sid(),
            run_id: run(),
            parent_run_id: None,
        },
        ServerEvent::ToolProgress {
            agent_id: "ling".into(),
            tool: "Bash".into(),
            line: "l".into(),
            stream: "stdout".into(),
            session_id: sid(),
            run_id: run(),
        },
        ServerEvent::Message {
            from: "ling".into(),
            to: "user".into(),
            content: "Done.".into(),
            session_id: sid(),
            run_id: run(),
            parent_agent_id: None,
            client_id: None,
            via: None,
            via_note: None,
            images: Vec::new(),
        },
        ServerEvent::TurnComplete {
            agent_id: "ling".into(),
            duration_ms: None,
            context_tokens: None,
            parent_id: None,
            session_id: sid(),
            run_id: run(),
            parent_run_id: None,
        },
        ServerEvent::AgentStatus {
            agent_id: "ling".into(),
            status: "idle".into(),
            detail: None,
            status_id: None,
            lifecycle: None,
            parent_agent_id: None,
            session_id: sid(),
            run_id: run(),
            parent_run_id: None,
        },
        ServerEvent::Outcome {
            agent_id: "ling".into(),
            outcome: crate::engine::AgentOutcome::None,
            session_id: sid(),
            run_id: run(),
        },
    ];
    for (i, event) in events.into_iter().enumerate() {
        let what = format!("{event:?}");
        let ui = map(event, i as u64).unwrap_or_else(|| panic!("unmapped: {what}"));
        assert_eq!(
            ui.run_id.as_deref(),
            Some("run-1"),
            "no run_id on {}",
            ui.kind
        );
    }
}

/// A top-level reply names its run on the event, never in `data.run_id`:
/// that key means "a subagent's" to surfaces already shipped (the phone).
#[test]
fn a_top_level_message_keeps_its_run_out_of_the_subagent_key() {
    let msg = |parent: Option<&str>| ServerEvent::Message {
        from: "ling".into(),
        to: "user".into(),
        content: "Done.".into(),
        session_id: Some("s".into()),
        run_id: Some("r1".into()),
        parent_agent_id: parent.map(str::to_string),
        client_id: None,
        via: None,
        via_note: None,
        images: Vec::new(),
    };
    let top = map(msg(None), 1).expect("mapped");
    assert_eq!(top.run_id.as_deref(), Some("r1"));
    assert!(top.data.expect("data")["run_id"].is_null());
    let sub = map(msg(Some("ling")), 2).expect("mapped");
    assert_eq!(sub.data.expect("data")["run_id"], "r1");
}
