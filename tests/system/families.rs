//! Tick a family, follow it, go back: the settings routes over a real engine.
//!
//! The provider's model list and the day come from `test-families.json` in the
//! world's home (`families::Families::new`, a `test-support` build only); no
//! provider is called. The key is a marker, never a real one.

use crate::support::{Setup, World};
use serde_json::{json, Value};

const KEY: &str = "test-only-marker";
const TODAY: &str = "2026-11-23";
/// A followed Opus row on the model before the family's current one.
const BOUND_ROW: &str = r#"
[[models]]
id = "fam-seed"
provider = "anthropic"
url = "https://api.anthropic.com"
model = "claude-opus-5"
[models.family_binding]
family = "claude-opus"
preset = "anthropic"
mode = "follow"
"#;

/// The provider's list; `None`: it cannot be read.
fn lists(ids: Option<&[&str]>) -> String {
    match ids {
        Some(ids) => json!({"today": TODAY, "lists": {"anthropic": ids}}),
        None => json!({"today": TODAY, "lists": {}}),
    }.to_string()
}

const ALL: &[&str] = &["claude-opus-5", "claude-opus-5-5", "claude-sonnet-5-5", "claude-haiku-5-5"];

async fn world(setup: Setup, list: &[&str]) -> World {
    world_with(setup, Some(list)).await
}

async fn world_with(setup: Setup, list: Option<&[&str]>) -> World {
    let list = lists(list);
    World::start_with_home(setup, move |home| {
        std::fs::write(home.linggen_home().join("test-families.json"), list).expect("fixture");
    }).await
}

fn setup(append: Option<&'static str>, default_model: Option<&'static str>) -> Setup {
    Setup { config_append: append, default_model, ..Setup::default() }
}

fn anthropic(body: &Value) -> &Value {
    body["providers"].as_array().unwrap().iter().find(|p| p["label"] == "Anthropic").expect("Anthropic group")
}

fn family<'a>(body: &'a Value, name: &str) -> &'a Value {
    anthropic(body)["families"]["families"].as_array().unwrap().iter().find(|f| f["family"] == name).expect("family")
}

/// Paste the key for the Anthropic group; the starters (if any) are ticked.
async fn connect(w: &World) -> (String, Value) {
    let body = w.api.post("/api/providers", json!({"preset": "Anthropic"})).await;
    let id = anthropic(&body)["id"].as_str().unwrap().to_string();
    let (status, body) = w.api.put_json(&format!("/api/providers/{id}/key"), json!({"api_key": KEY})).await;
    assert_eq!(status, 200);
    (id, body)
}

async fn act(w: &World, id: &str, family: &str, body: Value) -> (u16, String) {
    w.api.put_text(&format!("/api/providers/{id}/families/{family}"), body).await
}

#[tokio::test]
async fn startup_checks_an_existing_key_without_a_tick() {
    let w = World::start_with_home(setup(Some(BOUND_ROW), None), |home| {
        let fixture = json!({"today": TODAY, "delay_ms": 3000, "lists": {"anthropic": ["claude-opus-5-5"]}});
        std::fs::write(home.linggen_home().join("test-families.json"), fixture.to_string()).unwrap();
        let creds = json!({"version": 2, "endpoints": [{"provider":"anthropic", "url":"https://api.anthropic.com", "api_key":KEY}]});
        std::fs::write(home.linggen_home().join("credentials.json"), creds.to_string()).unwrap();
    }).await;
    let body = w.api.get("/api/providers").await;
    let opus = family(&body, "claude-opus");
    assert_eq!(opus["hold"], "checking");
    assert_eq!(opus["next"]["price"]["source"], "catalog");
    for _ in 0..50 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let body = w.api.get("/api/providers").await;
        if family(&body, "claude-opus").get("hold").is_none() {
            assert_eq!(family(&body, "claude-opus")["next"]["model"], "claude-opus-5-5");
            assert_eq!(family(&body, "claude-sonnet")["hold"], "no_access");
            return;
        }
    }
    panic!("the key was never checked");
}

#[tokio::test]
async fn a_key_ticks_the_starter_family_once_and_never_again() {
    let mut w = world(setup(None, None), ALL).await;
    let (id, body) = connect(&w).await;
    let sonnet = family(&body, "claude-sonnet");
    assert_eq!((sonnet["ticked"].clone(), sonnet["mode"].clone()), (true.into(), "follow".into()));
    assert_eq!(sonnet["current"]["model"], "claude-sonnet-5-5");
    assert_eq!(sonnet["current"]["price"]["source"], "catalog");
    assert_eq!(sonnet["label"], "Sonnet", "a family is named by the engine");
    assert_eq!(family(&body, "claude-opus")["ticked"], false);
    assert_eq!(anthropic(&body)["families"]["catalog"]["observed_at"], TODAY);
    assert!(!body.to_string().contains(KEY), "the key is in no answer");
    let row = sonnet["row"].as_str().unwrap().to_string();
    assert!(row.starts_with("fam-") && !row.contains("sonnet"), "{row}");
    assert_eq!(w.api.get("/api/config").await["family_starts"], json!(["anthropic"]));
    // Unticked, and the key saved again: it does not come back, now or after a restart.
    let (status, body) = act(&w, &id, "claude-sonnet", json!({"action": "untick"})).await;
    assert_eq!(status, 200, "{body}");
    w.engine.restart(&w.home).await;
    assert_eq!(w.api.get("/api/config").await["family_starts"], json!(["anthropic"]), "the record survives a restart");
    let (_, body) = w.api.put_json(&format!("/api/providers/{id}/key"), json!({"api_key": "another-test-marker"})).await;
    assert_eq!(family(&body, "claude-sonnet")["ticked"], false);
    assert_eq!(anthropic(&body)["models"], json!([]));
    assert_eq!(anthropic(&body)["key"]["set"], true, "the key stays");
}

#[tokio::test]
async fn a_following_row_moves_with_one_notice_and_each_notice_action_answers_its_move() {
    let w = world(setup(Some(BOUND_ROW), Some("fam-seed")), ALL).await;
    let (id, body) = connect(&w).await;
    let opus = family(&body, "claude-opus");
    assert_eq!((opus["ticked"].clone(), opus["current"]["model"].clone()), (true.into(), "claude-opus-5".into()));
    assert_eq!(opus["next"]["model"], "claude-opus-5-5", "the key's list shows what it would move to");
    assert_eq!(opus["row"], "fam-seed");
    // The synthetic move: follow asks the selector now.
    let (status, text) = act(&w, &id, "claude-opus", json!({"action": "follow"})).await;
    assert_eq!(status, 200, "{text}");
    let body: Value = serde_json::from_str(&text).unwrap();
    let opus = family(&body, "claude-opus");
    assert_eq!(opus["current"]["model"], "claude-opus-5-5");
    assert_eq!((opus["notice"]["move"].clone(), opus["notice"]["kind"].clone(), opus["notice"]["can_go_back"].clone()), (1.into(), "followed".into(), true.into()));
    assert_eq!(opus["row"], "fam-seed", "the same row, so every reference follows");
    let model_of_row = |body: &Value| anthropic(body)["models"].as_array().unwrap().iter().find(|m| m["id"] == "fam-seed").map(|m| m["model"].clone());
    assert_eq!(model_of_row(&body), Some("claude-opus-5-5".into()));
    assert!(!text.contains(KEY));
    // A stale or repeated action does nothing.
    let before = w.api.get("/api/providers").await;
    for stale in [json!({"action": "ack", "move": 7}), json!({"action": "back", "move": 7}), json!({"action": "pin", "move": 7})] {
        assert_eq!(act(&w, &id, "claude-opus", stale).await.0, 200);
        assert_eq!(w.api.get("/api/providers").await, before);
    }
    // 换回 → the previous model, pinned, notice answered; a repeat does nothing.
    assert_eq!(act(&w, &id, "claude-opus", json!({"action": "back", "move": 1})).await.0, 200);
    let after = w.api.get("/api/providers").await;
    let opus = family(&after, "claude-opus");
    assert_eq!((opus["current"]["model"].clone(), opus["mode"].clone()), ("claude-opus-5".into(), "pinned".into()));
    assert!(opus.get("notice").is_none());
    assert_eq!(act(&w, &id, "claude-opus", json!({"action": "back", "move": 1})).await.0, 200);
    assert_eq!(w.api.get("/api/providers").await, after);
    // 恢复自动跟上 → moves again, with the next notice number; 知道了 clears it.
    let (_, text) = act(&w, &id, "claude-opus", json!({"action": "follow"})).await;
    let body: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(family(&body, "claude-opus")["notice"]["move"], 2);
    let (_, text) = act(&w, &id, "claude-opus", json!({"action": "ack", "move": 2})).await;
    let body: Value = serde_json::from_str(&text).unwrap();
    assert!(family(&body, "claude-opus").get("notice").is_none());
    assert_eq!(family(&body, "claude-opus")["current"]["model"], "claude-opus-5-5");
}

#[tokio::test]
async fn a_legacy_settings_save_made_before_a_move_does_not_revert_it() {
    let w = world(setup(Some(BOUND_ROW), None), ALL).await;
    let (id, _) = connect(&w).await;
    let stale = w.api.get("/api/config").await;
    let row = |config: &Value| config["models"].as_array().unwrap().iter().find(|m| m["id"] == "fam-seed").cloned().unwrap();
    assert_eq!(row(&stale)["model"], "claude-opus-5");
    assert_eq!(act(&w, &id, "claude-opus", json!({"action": "follow"})).await.0, 200);
    // The page saves what it read before the move, and sets a binding of its own on a new row.
    let mut posted = stale.clone();
    posted["models"].as_array_mut().unwrap().push(json!({
        "id": "forged", "provider": "anthropic", "url": "https://api.anthropic.com", "model": "claude-opus-5",
        "family_binding": {"family": "claude-opus", "preset": "anthropic", "mode": "follow"},
    }));
    assert_eq!(w.api.post_status("/api/config", posted.clone()).await, 200);
    let config = w.api.get("/api/config").await;
    assert_eq!(row(&config)["model"], "claude-opus-5-5");
    assert_eq!(row(&config)["family_binding"]["last_move"]["to"], "claude-opus-5-5");
    let forged = config["models"].as_array().unwrap().iter().find(|m| m["id"] == "forged").unwrap();
    assert!(forged.get("family_binding").is_none(), "a binding is not accepted from the legacy route");
    // Raced with a real move, in whatever order they land: each round starts
    // from the version before it, and the move survives the stale save.
    for _ in 0..4 {
        assert_eq!(act(&w, &id, "claude-opus", json!({"action": "choose", "model": "claude-opus-5"})).await.0, 200);
        let (_, moved) = tokio::join!(
            w.api.post_status("/api/config", posted.clone()),
            act(&w, &id, "claude-opus", json!({"action": "follow"})),
        );
        assert_eq!(moved.0, 200, "{}", moved.1);
        let config = w.api.get("/api/config").await;
        assert_eq!(row(&config)["model"], "claude-opus-5-5");
        assert_eq!(row(&config)["family_binding"]["mode"], "follow");
        assert_eq!(row(&config)["family_binding"]["last_move"]["to"], "claude-opus-5-5");
    }
}

#[tokio::test]
async fn an_exact_row_is_never_moved_or_answered_as_a_family() {
    const EXACT: &str = r#"
[[models]]
id = "my-opus"
provider = "anthropic"
url = "https://api.anthropic.com"
model = "claude-opus-5"
"#;
    let w = world(setup(Some(EXACT), None), ALL).await;
    let (id, _) = connect(&w).await;
    let before = w.api.get("/api/providers").await;
    assert_eq!(family(&before, "claude-opus")["ticked"], false);
    for body in [json!({"action": "follow"}), json!({"action": "pin"}), json!({"action": "ack", "move": 1}), json!({"action": "back", "move": 1})] {
        assert_eq!(act(&w, &id, "claude-opus", body).await.0, 404);
    }
    assert_eq!(w.api.get("/api/providers").await, before);
    // The family can still be ticked beside it, as its own row; the exact row stays.
    let (status, text) = act(&w, &id, "claude-opus", json!({"action": "tick"})).await;
    assert_eq!(status, 200, "{text}");
    let body: Value = serde_json::from_str(&text).unwrap();
    let rows = anthropic(&body)["models"].as_array().unwrap();
    assert_eq!(rows.iter().find(|m| m["id"] == "my-opus").unwrap()["model"], "claude-opus-5");
    assert_eq!(family(&body, "claude-opus")["current"]["model"], "claude-opus-5-5");
    assert_ne!(family(&body, "claude-opus")["row"], "my-opus");
}

const OWN_OPUS: &str = r#"
[[models]]
id = "my-opus"
provider = "anthropic"
url = "https://api.anthropic.com"
model = "claude-opus-5-5"
"#;

#[tokio::test]
async fn an_explicit_tick_files_its_own_row_and_never_adopts_or_merges_an_ordinary_one() {
    let w = world(setup(Some(OWN_OPUS), Some("my-opus")), ALL).await;
    let (id, body) = connect(&w).await;
    assert_eq!(family(&body, "claude-opus")["ticked"], false);
    assert_eq!(act(&w, &id, "claude-opus", json!({"action":"tick"})).await.0, 200);
    let body = w.api.get("/api/providers").await;
    assert_ne!(family(&body, "claude-opus")["row"], "my-opus", "the ordinary row is not adopted");
    assert!(family(&body, "claude-opus").get("merge_into").is_none(), "no merge offer");
    assert_eq!(anthropic(&body)["models"].as_array().unwrap().len(), 2);
    assert_eq!(w.api.get("/api/config").await["routing"]["default_models"][0], "my-opus");
    // The merge action is gone: a fixed bad request.
    assert_eq!(act(&w, &id, "claude-opus", json!({"action":"merge","model":"my-opus"})).await.0, 400);
}

#[tokio::test]
async fn unticking_refuses_the_first_default_unless_a_replacement_is_named_and_names_its_references() {
    let w = world(setup(Some(BOUND_ROW), Some("fam-seed")), ALL).await;
    let (id, body) = connect(&w).await;
    let places: Vec<_> = family(&body, "claude-opus")["references"].as_array().unwrap().iter().map(|r| r["place"].as_str().unwrap()).collect();
    assert_eq!(places, ["default_chat"]);
    let before = w.api.get("/api/providers").await;
    assert_eq!(act(&w, &id, "claude-opus", json!({"action": "untick"})).await, (409, "first_default".into()));
    assert_eq!(act(&w, &id, "claude-opus", json!({"action": "untick", "model": "no-such-model"})).await.0, 400);
    assert_eq!(w.api.get("/api/providers").await, before);
    // The exact-row delete route does not slip past the same guard.
    assert_eq!(w.api.delete_status(&format!("/api/providers/{id}/models/fam-seed"), json!({})).await, 409);
    assert_eq!(w.api.get("/api/providers").await, before);
    let (status, text) = act(&w, &id, "claude-opus", json!({"action": "untick", "model": "fake-ling"})).await;
    assert_eq!(status, 200, "{text}");
    let body: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(family(&body, "claude-opus")["ticked"], false);
    assert_eq!(w.api.get("/api/config").await["routing"]["default_models"][0], "fake-ling");
    assert_eq!(anthropic(&body)["key"]["set"], true);
}

#[tokio::test]
async fn failures_have_fixed_codes_and_no_ticking_without_a_key_a_list_or_the_owner() {
    let w = world(setup(None, None), ALL).await;
    let body = w.api.post("/api/providers", json!({"preset": "Anthropic"})).await;
    let id = anthropic(&body)["id"].as_str().unwrap().to_string();
    // No key yet.
    assert_eq!(act(&w, &id, "claude-opus", json!({"action": "tick"})).await, (409, "no_key".into()));
    let (_, _) = w.api.put_json(&format!("/api/providers/{id}/key"), json!({"api_key": KEY})).await;
    assert_eq!(act(&w, &id, "never-seen", json!({"action": "tick"})).await.0, 404);
    assert_eq!(act(&w, &id, "claude-opus", json!({"action": "choose"})).await.0, 400);
    assert_eq!(act(&w, &id, "claude-opus", json!({"action": "tick", "extra": 1})).await.0, 400);
    assert_eq!(act(&w, &id, "claude-opus", json!({"action": "choose", "model": "claude-opus-4-8"})).await, (409, "no_access".into()));
    assert_eq!(act(&w, "0000", "claude-opus", json!({"action": "tick"})).await.0, 404);
    // Another person on the tunnel is not the owner.
    let refused = reqwest::Client::new()
        .put(format!("{}/api/providers/{id}/families/claude-opus", w.engine.base_url()))
        .header("x-linggen-tunnel", "guest")
        .json(&json!({"action": "tick"}))
        .send()
        .await
        .unwrap();
    assert_eq!(refused.status().as_u16(), 403);
    assert_eq!(family(&w.api.get("/api/providers").await, "claude-opus")["ticked"], false);
}

#[tokio::test]
async fn a_provider_without_a_list_gives_a_choice_not_a_tick() {
    let w = world_with(setup(None, None), None).await;
    let (id, body) = connect(&w).await;
    // The list could not be read: no starter is ticked and no tick is made on a guess.
    assert_eq!(anthropic(&body)["models"], json!([]));
    assert_eq!(w.api.get("/api/config").await.get("family_starts"), None, "a key not proven records nothing");
    assert_eq!(act(&w, &id, "claude-opus", json!({"action": "tick"})).await, (409, "needs_choice".into()));
    assert_eq!(family(&w.api.get("/api/providers").await, "claude-opus")["hold"], "list_failed");
}
