//! What a model row may be filed under, and how a person gets in.
//!
//! The list of providers a page offers is NOT here: it is the curated model
//! list (`provider::models::curated`), so adding a provider or a model to that
//! list needs no code. What stays in code is the closed set of provider
//! *names* a stored row may carry. A new name is a stored name the released
//! binary refuses to save, so a name is added only with a release that knows
//! it, and the list may use only names written here.

/// How a person gets into a provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Way {
    /// An API key.
    Key,
    /// A server on this Mac.
    Local,
    /// A Linggen account (Linggen Cloud).
    Account,
    /// A subscription sign-in (ChatGPT).
    Subscription,
}

impl Way {
    /// The word a page is told.
    pub fn as_str(self) -> &'static str {
        match self {
            Way::Key => "key",
            Way::Local => "local",
            Way::Account => "account",
            Way::Subscription => "subscription",
        }
    }
}

/// The provider names the 2.6.1 binary accepts, fixed: what `Config::validate`
/// checks and the curated list may use. Not derived from what a page offers.
const KNOWN_NAMES: [&str; 13] = [
    "ollama", "chatgpt", "anthropic", "openai", "gemini", "groq", "deepseek", "openrouter",
    "github", "qwen", "moonshot", "zhipu", "siliconflow",
];

pub fn known_names() -> Vec<&'static str> {
    KNOWN_NAMES.to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_known_names_are_the_fixed_2_6_1_allowlist() {
        let names = known_names();
        assert_eq!(names.len(), 13, "{names:?}");
        for old in [
            "ollama", "openai", "chatgpt", "anthropic", "gemini", "groq", "deepseek",
            "openrouter", "github", "qwen", "moonshot", "zhipu", "siliconflow",
        ] {
            assert!(names.contains(&old), "{old} is no longer a known name");
        }
    }

    /// A config and a credentials file as the released binary wrote them
    /// load and save back the same in meaning: nothing new is stored.
    #[test]
    fn an_old_shaped_config_and_credentials_file_save_back_unchanged_in_meaning() {
        // The required tables and agent limit match the fresh system fixture.
        let old_config = r#"
[server]
url = "127.0.0.1:9527"

[agent]
max_iters = 12

[routing]
default_models = ["a", "b"]

[[models]]
id = "a"
provider = "openai"
url = "https://api.openai.com/v1"
model = "gpt-4o"
tags = ["vision"]

[[models]]
id = "voice"
provider = "qwen"
url = "https://ap-southeast-1.maas.qwencloudapi.com/compatible-mode/v1"
model = "qwen-audio-3.1-realtime-flash"

[[models]]
id = "b"
provider = "ollama"
url = "http://127.0.0.1:11434"
model = "qwen3:32b"
keep_alive = "10m"
"#;
        let old_credentials = serde_json::json!({
            "version": 2,
            "endpoints": [
                {"provider": "openai", "url": "https://api.openai.com/v1", "api_key": "test-not-a-key-1"},
                {"provider": "qwen", "url": "https://ap-southeast-1.maas.qwencloudapi.com/compatible-mode/v1", "api_key": "test-not-a-key-2"}
            ],
            "models": {"b": {"api_key": "test-not-a-key-3"}},
            "services": {"search": {"api_key": "test-not-a-key-4"}}
        });
        let dir = tempfile::tempdir().unwrap();

        let loaded = crate::config::Config::from_toml(old_config).unwrap();
        loaded.validate().unwrap();
        loaded.save_runtime(Some(dir.path())).unwrap();
        let saved = std::fs::read_to_string(dir.path().join("linggen.runtime.toml")).unwrap();
        let again = crate::config::Config::from_toml(&saved).unwrap();
        assert_eq!(
            toml::Value::try_from(&loaded).unwrap(),
            toml::Value::try_from(&again).unwrap()
        );
        let ids: Vec<&str> = again.models.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["a", "voice", "b"], "no row added, dropped or renamed");

        let file = dir.path().join("credentials.json");
        std::fs::write(&file, old_credentials.to_string()).unwrap();
        let creds = crate::credentials::Credentials::load_for(&file, &again.models);
        creds.save(&file).unwrap();
        let back: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(back, old_credentials);
    }
}
