//! The providers the "Add provider" picker offers: one table, in the engine.
//!
//! Each preset is a name the config knows, an address and an API style.
//! Any other provider is typed by the person (name, address, style, key).
//! Nobody maintains a model list here: a key provider's models are what the
//! person adds, suggested from the provider's own `/models` (`suggest`).
//! A new preset is one row below, shipped with an engine release.

use crate::config::providers::Way;
use crate::credentials::same_endpoint;

/// How the engine speaks to a provider: the two wires a key provider can use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiStyle {
    OpenAiCompatible,
    Claude,
}

impl ApiStyle {
    /// The word a page is told and sends back.
    pub fn as_str(self) -> &'static str {
        match self {
            ApiStyle::OpenAiCompatible => "openai",
            ApiStyle::Claude => "claude",
        }
    }

    pub fn parse(word: &str) -> Option<Self> {
        [Self::OpenAiCompatible, Self::Claude].into_iter().find(|s| s.as_str() == word)
    }
}

pub struct Preset {
    /// The provider name a row is filed under (`providers::known_names`).
    pub name: &'static str,
    pub label: &'static str,
    pub url: &'static str,
    pub style: ApiStyle,
    pub way: Way,
}

const fn key(name: &'static str, label: &'static str, url: &'static str, style: ApiStyle) -> Preset {
    Preset { name, label, url, style, way: Way::Key }
}

/// THE table. Order is the order the picker shows.
pub const PRESETS: &[Preset] = &[
    key("anthropic", "Anthropic", "https://api.anthropic.com", ApiStyle::Claude),
    key("openai", "OpenAI", "https://api.openai.com/v1", ApiStyle::OpenAiCompatible),
    key("gemini", "Google Gemini", "https://generativelanguage.googleapis.com/v1beta/openai", ApiStyle::OpenAiCompatible),
    key("deepseek", "DeepSeek", "https://api.deepseek.com/v1", ApiStyle::OpenAiCompatible),
    key("qwen", "Qwen Cloud", "https://ap-southeast-1.maas.qwencloudapi.com/compatible-mode/v1", ApiStyle::OpenAiCompatible),
    key("openrouter", "OpenRouter", "https://openrouter.ai/api/v1", ApiStyle::OpenAiCompatible),
    key("groq", "Groq", "https://api.groq.com/openai/v1", ApiStyle::OpenAiCompatible),
    Preset { name: "ollama", label: "Ollama", url: "http://127.0.0.1:11434", style: ApiStyle::OpenAiCompatible, way: Way::Local },
];

/// The preset at this address, whatever name a row there is filed under.
pub fn at(url: &str) -> Option<&'static Preset> {
    PRESETS.iter().find(|p| p.way == Way::Key && same_endpoint("", p.url, "", url))
}

/// The preset a person picked, by label or name.
pub fn named(selector: &str) -> Option<&'static Preset> {
    PRESETS.iter().find(|p| p.label == selector || p.name == selector)
}

/// A provider's label: the preset's at this address; else the address's
/// host (a provider the person typed is filed under its API style's name,
/// which would mislabel it); else the preset's of this name, or the name.
pub fn label_for(name: &str, url: &str) -> String {
    if let Some(p) = PRESETS.iter().find(|p| same_endpoint("", p.url, "", url)) {
        return p.label.to_string();
    }
    if let Some(host) = crate::credentials::url_host(url) {
        return host;
    }
    PRESETS.iter().find(|p| p.name.eq_ignore_ascii_case(name.trim()))
        .map_or_else(|| name.trim().to_string(), |p| p.label.to_string())
}

/// The provider name a typed provider of `style` is filed under: Claude's
/// wire is the `anthropic` name; any other speaks OpenAI-compatible.
pub fn name_for_style(style: ApiStyle) -> &'static str {
    match style {
        ApiStyle::Claude => "anthropic",
        ApiStyle::OpenAiCompatible => "openai",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_preset_is_a_known_name_at_its_own_address() {
        let known = crate::config::providers::known_names();
        for p in PRESETS {
            assert!(known.contains(&p.name), "{}", p.name);
            assert!(url::Url::parse(p.url).is_ok(), "{}", p.url);
        }
        let labels: std::collections::BTreeSet<_> = PRESETS.iter().map(|p| p.label).collect();
        assert_eq!(labels.len(), PRESETS.len(), "one row per provider");
    }

    #[test]
    fn a_label_is_found_by_address_then_name() {
        assert_eq!(label_for("openai", "https://api.deepseek.com/v1/"), "DeepSeek");
        assert_eq!(label_for("openai", "https://gw.example.com/v1"), "gw.example.com");
        assert_eq!(label_for("groq", ""), "Groq");
        assert_eq!(label_for("mine", ""), "mine");
        assert_eq!(named("Qwen Cloud").map(|p| p.name), Some("qwen"));
        assert_eq!(ApiStyle::parse("claude"), Some(ApiStyle::Claude));
    }
}
