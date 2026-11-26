//! The lock of kinds (`provider::kinds`) at the readers a unit test reaches:
//! a live-voice row is in no list a chat model is picked from.

use super::*;
use crate::engine::model_choice::{live, resolve, Asker, Outcome, Rule, Why};

fn row(id: &str, url: &str, model: &str) -> ModelConfig {
    serde_json::from_value(serde_json::json!({
        "id": id, "provider": "qwen", "url": url, "model": model,
    }))
    .unwrap()
}

fn live_row() -> ModelConfig {
    row(
        "voice",
        "https://ap-southeast-1.maas.qwencloudapi.com/compatible-mode/v1",
        "qwen-audio-3.1-realtime-flash",
    )
}

fn chat_row() -> ModelConfig {
    row("chat", "https://example.com/v1", "plain-chat")
}

fn manager() -> ModelManager {
    ModelManager::new_with_credentials(
        vec![chat_row(), live_row()],
        &crate::credentials::Credentials::default(),
    )
}

#[test]
fn a_live_voice_row_is_in_no_list_a_chat_model_is_picked_from() {
    let manager = manager();
    assert!(manager.list_models().iter().any(|m| m.id == "chat"));
    assert!(!manager.list_models().iter().any(|m| m.id == "voice"));
    assert!(manager.model_ids().contains(&"chat".to_string()));
    assert!(!manager.model_ids().contains(&"voice".to_string()));
    assert!(manager.has_model("chat") && !manager.has_model("voice"));
    assert_eq!(manager.resolve_id("voice"), None);
    assert!(manager.readiness("voice").is_none());
    assert!(!manager.readiness_list(None).to_string().contains("\"voice\""));
    assert!(!manager.ready_in_order().iter().any(|m| m.id == "voice"));
    assert_eq!(live(&manager, Asker::Owner, "voice"), Err(Why::NotHere));
    let shared = vec!["voice".to_string()];
    assert_eq!(live(&manager, Asker::Guest(&shared), "voice"), Err(Why::NotHere));
    let kept: Vec<&str> = manager.live_rows().iter().map(|m| m.id.as_str()).collect();
    assert_eq!(kept, vec!["voice"]);
}

/// An asked or pinned live-voice id is passed over as a missing model is.
#[test]
fn a_turn_that_asks_for_a_live_voice_row_runs_the_standing_model() {
    let rule = Rule { asker: Asker::Owner, may_switch: true };
    let choice = resolve(&manager(), Some("voice"), "chat", rule);
    assert_eq!(choice.model(), "chat");
    assert_eq!(choice.outcome(), Outcome::Replaced(Why::NotHere));
}
