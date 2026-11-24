//! The providers a person can add, written once.
//!
//! [`PRESETS`] is the one list: a provider's name, its label, its address, how
//! a person gets in, and what to suggest as a first model. The names the
//! config accepts ([`known_names`], what `Config::validate` checks) are read
//! from it, and so is what a page offers (`GET /api/providers`). A new preset
//! under an existing name changes no stored shape; a new *name* is a stored
//! name the released binary refuses to save, so a name is added only with a
//! release that knows it.
//!
//! A preset is one name at one address: a name may have several (Qwen at
//! DashScope and at Qwen Cloud). Which live-voice models a preset serves is
//! not written here — it is joined by host to the declared live providers
//! ([`crate::live::PROVIDERS`]) in [`Preset::live_models`].

use crate::credentials::url_host;

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

/// Public data approved for this preset. The preset URL, not the public
/// catalog's API field, is the address the engine speaks to.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CatalogBinding {
    pub source_id: &'static str,
    pub starter_families: &'static [&'static str],
    /// Candidates only: this endpoint must list them itself and inherits no rates.
    pub borrowed: bool,
}

/// One provider a person can add: a name the config knows, at an address.
#[derive(Debug, Clone, Copy)]
pub struct Preset {
    /// The provider name a model row is filed under.
    pub name: &'static str,
    pub label: &'static str,
    pub url: &'static str,
    pub way: Way,
    pub(crate) key: &'static str,
    pub(crate) catalog: Option<CatalogBinding>,
    /// Legacy route suggestion; family starters are declared separately.
    /// A first model to suggest; empty when there is no good default.
    pub default_model: &'static str,
    pub placeholder: &'static str,
}

impl Preset {
    /// The live-voice models served at this preset's host: the declared live
    /// providers whose key host is this one.
    pub fn live_models(&self) -> Vec<&'static str> {
        crate::live::PROVIDERS.iter().filter(|p| self.serves_live(p)).map(|p| p.model).collect()
    }

    pub fn offered_live_models(&self) -> Vec<&'static str> {
        crate::live::offered_to_pages().filter(|p| self.serves_live(p)).map(|p| p.model).into_iter().collect()
    }

    pub(crate) fn serves_live(&self, provider: &crate::live::Provider) -> bool {
        url_host(self.url).is_some_and(|host| provider.key_host().eq_ignore_ascii_case(&host))
    }
}

/// Every preset, in the order a page offers them. An open set: a new one is
/// one entry (under a name already in use, until a release knows a new name).
pub const PRESETS: &[Preset] = &[
    Preset {
        name: "ollama",
        label: "Ollama",
        url: "http://127.0.0.1:11434",
        way: Way::Local,
        key: "ollama",
        catalog: None,
        default_model: "",
        placeholder: "e.g. qwen3:32b",
    },
    // No default model: the ChatGPT built-ins (the common case) come with the
    // sign-in; this is for a genuinely different ChatGPT-backed model id.
    Preset {
        name: "chatgpt",
        label: "ChatGPT 订阅",
        url: crate::provider::codex_auth::CHATGPT_API_BASE,
        way: Way::Subscription,
        key: "chatgpt",
        catalog: None,
        default_model: "",
        placeholder: "",
    },
    Preset {
        name: "anthropic",
        label: "Anthropic",
        url: "https://api.anthropic.com",
        way: Way::Key,
        key: "anthropic",
        catalog: Some(CatalogBinding { source_id: "anthropic", starter_families: &["claude-sonnet"], borrowed: false }),
        default_model: "claude-sonnet-5-5",
        placeholder: "e.g. claude-sonnet-5-5, claude-haiku-5-5, claude-opus-5-5",
    },
    Preset {
        name: "openai",
        label: "OpenAI",
        url: "https://api.openai.com/v1",
        way: Way::Key,
        key: "openai",
        catalog: Some(CatalogBinding { source_id: "openai", starter_families: &["gpt"], borrowed: false }),
        default_model: "",
        placeholder: "",
    },
    Preset {
        name: "gemini",
        label: "Google Gemini",
        url: "https://generativelanguage.googleapis.com/v1beta/openai",
        way: Way::Key,
        key: "gemini",
        catalog: Some(CatalogBinding { source_id: "google", starter_families: &["gemini-flash"], borrowed: false }),
        default_model: "gemini-2.5-flash",
        placeholder: "e.g. gemini-2.5-flash, gemini-3-flash-preview",
    },
    Preset {
        name: "groq",
        label: "Groq",
        url: "https://api.groq.com/openai/v1",
        way: Way::Key,
        key: "groq",
        catalog: Some(CatalogBinding { source_id: "groq", starter_families: &[], borrowed: false }),
        default_model: "llama-3.3-70b-versatile",
        placeholder: "e.g. llama-3.3-70b-versatile",
    },
    Preset {
        name: "deepseek",
        label: "DeepSeek",
        url: "https://api.deepseek.com/v1",
        way: Way::Key,
        key: "deepseek",
        catalog: Some(CatalogBinding { source_id: "deepseek", starter_families: &["deepseek-flash"], borrowed: false }),
        default_model: "deepseek-chat",
        placeholder: "e.g. deepseek-chat",
    },
    Preset {
        name: "openrouter",
        label: "OpenRouter",
        url: "https://openrouter.ai/api/v1",
        way: Way::Key,
        key: "openrouter",
        catalog: Some(CatalogBinding {
            source_id: "openrouter",
            starter_families: &[],
            borrowed: false,
        }),
        default_model: "",
        placeholder: "e.g. google/gemini-2.5-pro",
    },
    Preset {
        name: "github",
        label: "GitHub Models",
        url: "https://models.inference.ai.azure.com",
        way: Way::Key,
        key: "github-models",
        // The source lists Copilot, not this endpoint.
        catalog: None,
        default_model: "",
        placeholder: "",
    },
    // Mainland-China providers: OpenAI-compatible, each with its own key.
    Preset {
        name: "qwen",
        label: "通义千问 (DashScope)",
        url: "https://dashscope.aliyuncs.com/compatible-mode/v1",
        way: Way::Key,
        key: "dashscope-cn",
        catalog: Some(CatalogBinding { source_id: "alibaba-cn", starter_families: &[], borrowed: false }),
        default_model: "qwen-plus",
        placeholder: "e.g. qwen-plus, qwen3.8-max",
    },
    // Qwen's international service, under the same name: its live-voice model
    // is served here, and a key filed at this address is the live key.
    Preset {
        name: "qwen",
        label: "Qwen Cloud",
        url: "https://ap-southeast-1.maas.qwencloudapi.com/compatible-mode/v1",
        way: Way::Key,
        key: "qwen-cloud",
        catalog: Some(CatalogBinding { source_id: "alibaba", starter_families: &[], borrowed: true }),
        default_model: "",
        placeholder: "e.g. qwen-plus",
    },
    Preset {
        name: "moonshot",
        label: "Moonshot (Kimi)",
        url: "https://api.moonshot.cn/v1",
        way: Way::Key,
        key: "moonshot-cn",
        catalog: Some(CatalogBinding { source_id: "moonshotai-cn", starter_families: &[], borrowed: false }),
        default_model: "kimi-k2.6",
        placeholder: "e.g. kimi-k2.6, kimi-k3",
    },
    Preset {
        name: "zhipu",
        label: "智谱 (GLM)",
        url: "https://open.bigmodel.cn/api/paas/v4",
        way: Way::Key,
        key: "zhipu-cn",
        catalog: Some(CatalogBinding { source_id: "zhipuai", starter_families: &[], borrowed: false }),
        default_model: "glm-5.3",
        placeholder: "e.g. glm-5.3, glm-5.3-flash",
    },
    Preset {
        name: "siliconflow",
        label: "SiliconFlow",
        url: "https://api.siliconflow.cn/v1",
        way: Way::Key,
        key: "siliconflow-cn",
        catalog: Some(CatalogBinding { source_id: "siliconflow-cn", starter_families: &[], borrowed: false }),
        default_model: "Pro/deepseek-ai/DeepSeek-R1",
        placeholder: "e.g. Pro/deepseek-ai/DeepSeek-R1",
    },
];

/// The provider names a model may be filed under, each once, in preset
/// order — what `Config::validate` accepts.
pub fn known_names() -> Vec<&'static str> {
    let mut names: Vec<&'static str> = Vec::new();
    for preset in PRESETS {
        if !names.contains(&preset.name) {
            names.push(preset.name);
        }
    }
    names
}

/// An address as a preset is found by it: host and port, scheme and path
/// aside (a local server is told apart by its port).
fn place_of(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url.trim()).ok()?;
    let host = parsed.host_str()?.to_ascii_lowercase();
    Some(match parsed.port_or_known_default() {
        Some(port) => format!("{host}:{port}"),
        None => host,
    })
}

/// The preset at `url`'s host and port, whatever name it is filed under.
fn preset_at(url: &str) -> Option<&'static Preset> {
    let place = place_of(url)?;
    PRESETS.iter().find(|p| place_of(p.url).as_deref() == Some(place.as_str()))
}

/// A provider's label. Found by its ADDRESS first, so rows filed under a
/// generic name at a preset's address show that preset's label; then by the
/// name alone (the first preset of that name); else the name as written.
pub fn label_for(name: &str, url: &str) -> String {
    if let Some(preset) = preset_at(url) {
        return preset.label.to_string();
    }
    PRESETS
        .iter()
        .find(|p| p.name.eq_ignore_ascii_case(name.trim()))
        .map_or_else(|| name.trim().to_string(), |p| p.label.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_known_names_are_the_presets_names_each_once() {
        let names = known_names();
        assert_eq!(names.len(), 13, "{names:?}");
        for old in [
            "ollama", "openai", "chatgpt", "anthropic", "gemini", "groq", "deepseek",
            "openrouter", "github", "qwen", "moonshot", "zhipu", "siliconflow",
        ] {
            assert!(names.contains(&old), "{old} is no longer a known name");
        }
        // Step 1 adds no provider name: the released binary refuses one it does not know.
        assert!(PRESETS.iter().all(|p| names.contains(&p.name)));
    }

    #[test]
    fn every_live_provider_host_belongs_to_a_preset() {
        for p in crate::live::PROVIDERS {
            let served: Vec<&str> = PRESETS
                .iter()
                .filter(|preset| preset.live_models().contains(&p.model))
                .map(|preset| preset.label)
                .collect();
            assert_eq!(served.len(), 1, "live provider {} is served by presets {served:?}", p.id);
        }
    }

    #[test]
    fn no_two_presets_share_an_address() {
        let mut seen = Vec::new();
        for p in PRESETS {
            let place = place_of(p.url).unwrap_or_else(|| panic!("{} has no address", p.label));
            assert!(!seen.contains(&place), "{} repeats an address", p.label);
            seen.push(place);
        }
    }

    #[test]
    fn every_preset_passes_the_config_check() {
        for preset in PRESETS {
            let row: crate::config::ModelConfig = serde_json::from_value(serde_json::json!({
                "id": "m",
                "provider": preset.name,
                "url": preset.url,
                "model": if preset.default_model.is_empty() { "some-model" } else { preset.default_model },
            }))
            .unwrap();
            let mut cfg = crate::config::Config::default();
            cfg.models.push(row);
            cfg.validate().unwrap_or_else(|e| panic!("{}: {e}", preset.label));
        }
    }

    #[test]
    fn a_label_is_found_by_address_before_name() {
        // A generic name at Qwen Cloud's address shows Qwen Cloud.
        assert_eq!(
            label_for("openai", "https://ap-southeast-1.maas.qwencloudapi.com/compatible-mode/v1/"),
            "Qwen Cloud"
        );
        // The name alone: the first preset of that name.
        assert_eq!(label_for("qwen", "https://my-gateway.example/v1"), "通义千问 (DashScope)");
        // Another local port is not Ollama; fall back to the OpenAI preset by name.
        assert_eq!(label_for("openai", "http://127.0.0.1:1234/v1"), "OpenAI");
        assert_eq!(label_for("ollama", "http://127.0.0.1:11434"), "Ollama");
    }

    #[test]
    fn the_live_models_of_a_preset_come_from_the_declared_providers() {
        let cloud = PRESETS.iter().find(|p| p.label == "Qwen Cloud").unwrap();
        assert_eq!(cloud.live_models(), vec!["qwen-audio-3.1-realtime-flash"]);
        let gemini = PRESETS.iter().find(|p| p.name == "gemini").unwrap();
        assert_eq!(gemini.live_models(), vec!["gemini-3.8-live"]);
        let groq = PRESETS.iter().find(|p| p.name == "groq").unwrap();
        assert!(groq.live_models().is_empty());
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
