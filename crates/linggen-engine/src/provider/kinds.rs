//! What a configured model is for — the one place that decides it.
//!
//! A row in `[[models]]` has no kind field, and none is stored: the kind is
//! worked out here from what the engine declares (`live::PROVIDERS`: a host
//! and the one model it serves) and, for a row at a provider's own approved
//! address, from the public catalog's facts about that exact model
//! (`family_catalog`). It is never guessed from words in a name. Order:
//! a declared live entry wins; then catalog facts describe — an entry that
//! is not a plain text conversation (image or audio out, embedding,
//! transcription) is `NoLane`, a row the engine has no lane to run, so it is
//! never a chat row; a row with no such fact (added by hand, at another
//! address, not in the catalog) is a chat model, as it always was.
//!
//! A live-voice row is kept by `ModelManager` apart from the chat models
//! (`ModelManager::live_rows`), so no list a chat model is picked from can
//! hold one. Code that walks the configured rows for such a list asks
//! `Config::chat_models`; a lock (`src/server/model_locks.rs`) fails when a
//! production file builds a list from the raw rows instead.

use crate::config::ModelConfig;
use crate::provider::models::curated::{self, Bundle};

/// What a model is used for. A closed set: every `match` on it names each
/// case, so a new kind is found by the compiler wherever it was missed.
/// Config rows are `Chat`, `LiveVoice` or `NoLane`; the others are only ever
/// this Mac's own runtime models or a built-in source, never a configured row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Chat,
    LiveVoice,
    Speak,
    Hear,
    Draw,
    /// A configured row the catalog says is no text conversation and the
    /// engine declares no lane for: kept in the config, described, never run.
    NoLane,
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
            Kind::NoLane => "no_lane",
        }
    }
}

/// The kind of a configured model, from the catalog in force.
pub fn kind_of(cfg: &ModelConfig) -> Kind {
    kind_in(cfg, &curated::current())
}

/// [`kind_of`] against one captured snapshot: a caller that decides about
/// many rows captures once, so they cannot disagree with each other.
pub(crate) fn kind_in(cfg: &ModelConfig, facts: &Bundle) -> Kind {
    let declared = crate::credentials::url_host(&cfg.url)
        .is_some_and(|host| crate::live::declares(&host, &cfg.model));
    if declared {
        return Kind::LiveVoice;
    }
    match facts.exact(cfg) {
        Some(entry) if !entry.text_chat() => Kind::NoLane,
        _ => Kind::Chat,
    }
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
        assert_eq!(
            kind_of(&row(QWEN_LIVE, "qwen-audio-3.1-realtime-flash")),
            Kind::LiveVoice
        );
        assert_eq!(
            kind_of(&row(
                "https://generativelanguage.googleapis.com/v1beta/openai",
                "gemini-3.8-live"
            )),
            Kind::LiveVoice
        );
    }

    #[test]
    fn nothing_else_is_live_voice_whatever_its_name_says() {
        // The same host, another model: a chat model.
        assert_eq!(kind_of(&row(QWEN_LIVE, "qwen-plus")), Kind::Chat);
        // The live model's name at another host: a chat model.
        assert_eq!(
            kind_of(&row("https://example.com/v1", "qwen-audio-3.1-realtime-flash")),
            Kind::Chat
        );
        // A name that says "live" or "realtime" decides nothing.
        assert_eq!(kind_of(&row("https://example.com/v1", "my-live-realtime")), Kind::Chat);
        assert_eq!(kind_of(&row("", "")), Kind::Chat);
    }

    #[test]
    fn the_word_a_page_is_told_is_fixed() {
        assert_eq!(Kind::Chat.as_str(), "chat");
        assert_eq!(Kind::LiveVoice.as_str(), "live_voice");
        assert_eq!(Kind::NoLane.as_str(), "no_lane");
    }

    const GEMINI: &str = "https://generativelanguage.googleapis.com/v1beta/openai";

    fn gemini_row(url: &str, model: &str) -> ModelConfig {
        serde_json::from_value(serde_json::json!({
            "id": "m", "provider": "gemini", "url": url, "model": model,
        }))
        .unwrap()
    }

    /// Public facts about Gemini-shaped entries: what each takes and gives.
    fn facts() -> Bundle {
        let entry = |id: &str, task: Option<&str>, input: &[&str], output: &[&str]| {
            serde_json::json!({
                "id": id, "name": id, "stability": "stable", "task": task,
                "modalities": {"input": input, "output": output},
            })
        };
        let rows: Vec<_> = vec![
            entry("hears", None, &["audio", "image", "text"], &["text"]),
            entry("draws", Some("image"), &["image", "text"], &["image", "text"]),
            entry("draws-only", Some("image"), &["text"], &["image"]),
            entry("speaks", Some("speech"), &["text"], &["audio"]),
            entry("talks", None, &["audio", "text"], &["audio", "text"]),
            entry("embeds", Some("embedding"), &["text"], &["text"]),
            entry("transcribes", Some("transcription"), &["audio"], &["text"]),
            entry("chats", Some("chat"), &["text"], &["text"]),
            entry("new-task", Some("rerank"), &["text"], &["text"]),
            // A declared live entry whose public facts say audio out.
            entry("gemini-3.8-live", Some("realtime"), &["audio", "text"], &["audio", "text"]),
        ];
        Bundle::for_tests("2026-11-23", GEMINI, rows.into())
    }

    #[test]
    fn audio_in_text_out_is_chat_and_other_lanes_are_not_offered_to_chat() {
        let facts = facts();
        let kind = |model| kind_in(&gemini_row(GEMINI, model), &facts);
        for chat in ["hears", "chats"] {
            assert_eq!(kind(chat), Kind::Chat, "{chat}");
        }
        for no_lane in ["draws", "draws-only", "speaks", "talks", "embeds", "transcribes", "new-task"] {
            assert_eq!(kind(no_lane), Kind::NoLane, "{no_lane}");
        }
    }

    #[test]
    fn a_declared_live_entry_wins_over_the_catalog_and_the_catalog_never_makes_one() {
        let facts = facts();
        // Declared: live whatever the file says (it says audio out here).
        assert_eq!(kind_in(&gemini_row(GEMINI, "gemini-3.8-live"), &facts), Kind::LiveVoice);
        // The same facts at an undeclared model are only "no lane".
        assert_eq!(kind_in(&gemini_row(GEMINI, "talks"), &facts), Kind::NoLane);
    }

    #[test]
    fn a_row_the_catalog_cannot_describe_is_chat_as_before() {
        let facts = facts();
        // Not in the catalog; and names that sound like another lane decide nothing.
        for model in ["unlisted", "text-embedding-9", "my-image-tts"] {
            assert_eq!(kind_in(&gemini_row(GEMINI, model), &facts), Kind::Chat, "{model}");
        }
        // A custom endpoint under the same provider name inherits no facts.
        for url in ["https://example.com/v1", "https://gateway.example.com/v1beta/openai", ""] {
            assert_eq!(kind_in(&gemini_row(url, "embeds"), &facts), Kind::Chat, "{url}");
        }
        // Another sign-in is not a key at that address.
        let mut account = gemini_row(GEMINI, "embeds");
        account.auth_mode = Some("linggen_account".into());
        assert_eq!(kind_in(&account, &facts), Kind::Chat);
    }

    /// Whatever the shipped copy holds today: an entry that is no text
    /// conversation is never a chat row, and every other one is.
    #[test]
    fn every_entry_of_the_shipped_list_is_kinded_by_its_facts() {
        let shipped = curated::current();
        let mut seen = (0, 0);
        for place in shipped.offered(curated::Edition::International).chain(shipped.offered(curated::Edition::Mainland)) {
            for entry in shipped.facts_at(place.endpoint_key) {
                let row = serde_json::from_value::<ModelConfig>(serde_json::json!({
                    "id": "m", "provider": place.provider.name, "url": place.base, "model": entry.id(),
                }))
                .unwrap();
                let kind = kind_of(&row);
                if kind == Kind::LiveVoice {
                    continue;
                }
                assert_eq!(kind == Kind::Chat, entry.text_chat(), "{} {}", place.endpoint_key, entry.id());
                seen = (seen.0 + (kind == Kind::Chat) as u32, seen.1 + (kind == Kind::NoLane) as u32);
            }
        }
        assert!(seen.0 > 100 && seen.1 > 5, "{seen:?}");
    }
}
