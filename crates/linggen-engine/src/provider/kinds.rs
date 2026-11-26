//! What a configured model is for — the one place that decides it.
//!
//! A row in `[[models]]` has no kind field, and none is stored: the kind is
//! worked out here. A row is live voice when the engine declares its model at
//! its host (`live::PROVIDERS`) or its model id says `realtime`; every other
//! row is a chat model. No model list is read: a wrong pick (a tts or an
//! embedding id) fails on first use with the provider's own error and is
//! removed by the person.
//!
//! A live-voice row is kept by `ModelManager` apart from the chat models
//! (`ModelManager::live_rows`), so no list a chat model is picked from can
//! hold one. Code that walks the configured rows for such a list asks
//! `Config::chat_models`; a lock (`src/server/model_locks.rs`) fails when a
//! production file builds a list from the raw rows instead.

use crate::config::ModelConfig;

/// What a model is used for. A closed set: every `match` on it names each
/// case, so a new kind is found by the compiler wherever it was missed.
/// Config rows are `Chat` or `LiveVoice`; the others are only ever this
/// Mac's own runtime models or a built-in source, never a configured row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Chat,
    LiveVoice,
    Speak,
    Hear,
    Draw,
}

impl Kind {
    /// The word a page is told.
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Chat => "chat",
            Kind::LiveVoice => "live_voice",
            Kind::Speak => "speak",
            Kind::Hear => "hear",
            Kind::Draw => "draw",
        }
    }
}

/// The kind of a configured model.
pub fn kind_of(cfg: &ModelConfig) -> Kind {
    let declared = crate::credentials::url_host(&cfg.url)
        .is_some_and(|host| crate::live::declares(&host, &cfg.model));
    if declared || cfg.model.to_ascii_lowercase().contains("realtime") {
        return Kind::LiveVoice;
    }
    Kind::Chat
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(url: &str, model: &str) -> ModelConfig {
        serde_json::from_value(serde_json::json!({
            "id": "m", "provider": "qwen", "url": url, "model": model,
        }))
        .unwrap()
    }

    const QWEN_LIVE: &str = "https://ap-southeast-1.maas.qwencloudapi.com/compatible-mode/v1";

    #[test]
    fn a_declared_live_model_at_its_host_is_live_voice() {
        assert_eq!(kind_of(&row(QWEN_LIVE, "qwen-audio-3.1-realtime-flash")), Kind::LiveVoice);
        assert_eq!(
            kind_of(&row("https://generativelanguage.googleapis.com/v1beta/openai", "gemini-3.8-live")),
            Kind::LiveVoice
        );
    }

    #[test]
    fn a_realtime_id_is_live_voice_anywhere_and_everything_else_is_chat() {
        assert_eq!(kind_of(&row("https://example.com/v1", "gpt-realtime-2")), Kind::LiveVoice);
        assert_eq!(kind_of(&row(QWEN_LIVE, "qwen-plus")), Kind::Chat);
        // "live" alone, away from its declared host, decides nothing.
        assert_eq!(kind_of(&row("https://example.com/v1", "gemini-3.8-live")), Kind::Chat);
        assert_eq!(kind_of(&row("https://example.com/v1", "text-embedding-3")), Kind::Chat);
        assert_eq!(kind_of(&row("", "")), Kind::Chat);
    }

    #[test]
    fn the_word_a_page_is_told_is_fixed() {
        assert_eq!(Kind::Chat.as_str(), "chat");
        assert_eq!(Kind::LiveVoice.as_str(), "live_voice");
    }
}
