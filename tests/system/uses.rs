use crate::support::{Setup, World};
use serde_json::json;

#[tokio::test]
async fn defaults_round_trip_without_adding_a_fallback_or_exposing_live_as_chat() {
    const ROW: &str = r#"
[[models]]
id = "reasoning"
provider = "openai"
url = "https://api.openai.com/v1"
model = "gpt-5.4"
api_key = "test-not-a-key"
"#;
    let w = World::start(Setup { config_append: Some(ROW), ..Setup::default() }).await;
    let before = w.api.get("/api/uses").await;
    assert_eq!(before["chain"], json!(["fake-ling"]));
    assert!(before.get("live").is_none());
    assert_eq!(w.api.put_json("/api/uses", json!({"use":"live","choice":"google"})).await.0, 400);
    assert!(before["speak"]["choice"].is_null());
    assert!(before["draw"]["choice"].is_null());
    assert_eq!(before["hear"]["options"][0]["id"], "local-stt");
    for kind in ["chat", "speak", "voice", "draw", "hear"] {
        let options = before[kind]["options"].as_array().unwrap();
        assert!(!options.is_empty(), "{kind}");
        for option in options {
            let name = option["name"].as_str().expect("readable name");
            let source = &option["source"];
            assert!(matches!(source["way"].as_str(), Some("key" | "subscription" | "account" | "local")), "{option}");
            assert!(!name.trim().is_empty() && !name.contains('/'), "{option}");
            assert!(!["local", "this mac", "本机", "subscription", "订阅", "key", "密钥", "account", "账户", "chatgpt", "alibaba", "google", "system"].contains(&name.to_lowercase().as_str()), "{option}");
            assert_ne!(option["name"], source["provider"], "{option}");
        }
    }
    let option = |kind: &str, id: &str| before[kind]["options"].as_array().unwrap().iter().find(|o| o["id"] == id).unwrap();
    let flux = option("draw", "local");
    assert_eq!(flux["name"], "FLUX.2 klein 4B");
    assert_eq!(flux["source"], json!({"way":"local","size_gb":4.3,"free":true}));
    assert_eq!(flux["model_id"], "ar9av/FLUX.2-klein-4B-mflux-4bit");
    assert_eq!(option("draw", "chatgpt")["name"], "ChatGPT drawing");
    assert_eq!(option("speak", "qwen")["name"], "Qwen3-TTS 0.6B");
    assert_eq!(option("speak", "system")["name"], "macOS system voices");
    assert_eq!(option("hear", "local-stt")["name"], "Whisper large-v3 turbo");
    let live = w.api.get("/api/live").await;
    assert_eq!(live["providers"].as_array().unwrap().len(), 1);
    assert_eq!(live["providers"][0]["id"], "alibaba");
    for edit in [
        json!({"use":"chat","choice":"reasoning"}),
        json!({"use":"effort","model":"reasoning","choice":"medium"}),
        json!({"use":"speak","choice":"system"}),
        json!({"use":"voice","choice":"serena"}),
        json!({"use":"draw","choice":"chatgpt"}),
    ] {
        let (code, answer) = w.api.put_json("/api/uses", edit.clone()).await;
        assert_eq!(code, 200, "{edit}: {answer}");
        assert_eq!(answer, w.api.get("/api/uses").await);
    }
    let after = w.api.get("/api/uses").await;
    assert_eq!(after["chain"], json!(["reasoning"]));
    assert_eq!(after["effort"]["reasoning"]["choice"], "medium");
    let effort = after["effort"]["reasoning"]["options"].as_array().unwrap();
    assert!(!effort.is_empty());
    for option in effort {
        assert!(matches!(option["name"].as_str(), Some("low" | "medium" | "high")), "{option}");
        assert_eq!(option["source"]["way"], "key");
    }
    assert_eq!(after["speak"]["effective"], "system");
    assert_eq!(after["draw"]["effective"], "chatgpt");
    assert_eq!(w.api.put_json("/api/uses", json!({"use":"effort","model":"reasoning","choice":"xhigh"})).await.0, 400);
    assert_eq!(w.api.put_json("/api/uses", json!({"use":"app","id":"model-lesson","choice":"live:google"})).await.0, 400);
    assert_eq!(w.api.get("/api/uses").await, after);
    let saved = std::fs::read_to_string(w.home.config_path()).unwrap();
    let config = linggen_engine::config::Config::from_toml(&saved).unwrap();
    assert_eq!(config.uses.model_effort["reasoning"], "medium");
    assert_eq!(config.pet.voice, "serena");
}

#[tokio::test]
async fn chat_fallbacks_are_manual_ordered_and_survive_other_settings_saves() {
    const NO_KEY: &str = r#"
[[models]]
id = "needs-key"
provider = "openai"
url = "https://api.example.com/v1"
model = "test"
"#;
    let w = World::start(Setup { config_append: Some(NO_KEY), ..Setup::default() }).await;
    let stale_config = w.api.get("/api/config").await;
    for (edit, chain) in [
        (json!({"use":"chat_fallbacks","choices":["fake-eye","fake-sol"]}), json!(["fake-ling","fake-eye","fake-sol"])),
        (json!({"use":"chat_fallbacks","choices":["fake-sol","fake-eye"]}), json!(["fake-ling","fake-sol","fake-eye"])),
        (json!({"use":"chat","choice":"fake-eye"}), json!(["fake-eye","fake-sol"])),
        (json!({"use":"chat","choice":"fake-ling"}), json!(["fake-ling","fake-sol"])),
        (json!({"use":"model_fallback","enabled":false}), json!(["fake-ling","fake-sol"])),
    ] {
        let (code, answer) = w.api.put_json("/api/uses", edit.clone()).await;
        assert_eq!(code, 200, "{edit}: {answer}");
        assert_eq!(answer["chain"], chain);
    }
    assert_eq!(w.api.post_status("/api/config", stale_config).await, 200);
    let saved = w.api.get("/api/uses").await;
    assert_eq!(saved["chain"], json!(["fake-ling","fake-sol"]));
    assert_eq!(saved["model_fallback"], false);
    for invalid in [json!(["fake-ling"]), json!(["fake-sol","fake-sol"]), json!(["needs-key"]), json!(["missing"])] {
        assert_eq!(w.api.put_json("/api/uses", json!({"use":"chat_fallbacks","choices":invalid})).await.0, 400);
        assert_eq!(w.api.get("/api/uses").await, saved);
    }
    let (code, answer) = w.api.put_json("/api/uses", json!({"use":"chat_fallbacks","choices":[]})).await;
    assert_eq!(code, 200);
    assert_eq!(answer["chain"], json!(["fake-ling"]));
    let config = linggen_engine::config::Config::from_toml(&std::fs::read_to_string(w.home.config_path()).unwrap()).unwrap();
    assert_eq!(config.routing.default_models, ["fake-ling"]);
    assert!(!config.routing.model_fallback);
}

#[test]
fn old_use_settings_migrate_effort_and_drop_app_and_live_choices() {
    use linggen_engine::config::Config;
    let mut base = Config::default();
    base.routing.default_models = vec!["reasoning".into()];
    let old = toml::to_string(&base).unwrap() + "\n[uses]\nspeak = 'system'\nlive = 'google'\nchat_effort = 'high'\n[uses.apps]\nmodel-lesson = 'fake-eye'\n";
    let config = Config::from_toml(&old).unwrap();
    assert_eq!(config.uses.model_effort["reasoning"], "high");
    assert_eq!(config.uses.speak.as_deref(), Some("system"));
    let saved = toml::to_string(&config).unwrap();
    assert!(!saved.contains("[uses.apps]"));
    assert!(!saved.contains("chat_effort"));
    assert!(!saved.contains("live ="));
    assert_eq!(linggen_engine::config::Config::from_toml(&saved).unwrap().uses, config.uses);
}

/// 2026-11-25: a save through this route left `[memory]` out because its
/// values equalled the running build's default, and a later build with
/// another default changed the person's memory. What the person set stays
/// in the file, even when it equals the default; what they never set stays out.
#[tokio::test]
async fn a_settings_save_keeps_chosen_values_that_equal_the_default() {
    const CHOSEN: &str = "\n[memory]\noutside_agents = \"v1\"\nlinggen = \"v1\"\n\n[model_catalog]\nedition = \"auto\"\n";
    let w = World::start(Setup { config_append: Some(CHOSEN), ..Setup::default() }).await;
    let (code, answer) = w.api.put_json("/api/uses", json!({"use":"speak","choice":"system"})).await;
    assert_eq!(code, 200, "{answer}");
    let saved = std::fs::read_to_string(w.home.config_path()).unwrap();
    assert!(saved.contains("outside_agents = \"v1\"") && saved.contains("linggen = \"v1\""), "{saved}");
    assert!(saved.contains("edition = \"auto\""), "{saved}");

    // Never set (the world is not pinned to v1 here): nothing is written.
    let w = World::start(Setup { memory_defaults: true, ..Setup::default() }).await;
    let (code, answer) = w.api.put_json("/api/uses", json!({"use":"speak","choice":"system"})).await;
    assert_eq!(code, 200, "{answer}");
    let saved = std::fs::read_to_string(w.home.config_path()).unwrap();
    assert!(!saved.contains("[memory]") && !saved.contains("[model_catalog]"), "{saved}");
}
