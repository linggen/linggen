//! The section a session's work adds to its prompt (`SessionSpec::work`,
//! `doc/session-spec.md` § The session spec). `code` adds how to work on
//! code: one text for every model, then the few lines its model's family
//! needs. The prompt is not all a work changes: a coding session also runs
//! programs in its folder without asking (`AgentEngine::is_coding_session`).

use crate::engine::session_spec::Work;
use crate::engine::AgentEngine;
use crate::message::ChatMessage;
use crate::prompts::keys;

/// The model families that get lines of their own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Family {
    OpenAi,
    Gemini,
    DeepSeek,
}

impl Family {
    fn key(self) -> &'static str {
        match self {
            Self::OpenAi => keys::WORK_CODE_OPENAI,
            Self::Gemini => keys::WORK_CODE_GEMINI,
            Self::DeepSeek => keys::WORK_CODE_DEEPSEEK,
        }
    }
}

/// What a model id says of its family, whoever serves it (a proxy, a cloud
/// account, a local runner): first match wins.
const BY_ID: &[(&str, Family)] = &[
    ("deepseek", Family::DeepSeek),
    ("gemini", Family::Gemini),
    ("gpt", Family::OpenAi),
    ("codex", Family::OpenAi),
];

/// The one place that says which family a model is: its id when the id names
/// one, else its provider. `None` for a family with no lines of its own.
pub(crate) fn family_of(provider: Option<&str>, model_id: &str) -> Option<Family> {
    let id = model_id.to_ascii_lowercase();
    if let Some((_, family)) = BY_ID.iter().find(|(word, _)| id.contains(word)) {
        return Some(*family);
    }
    match provider? {
        "openai" | "chatgpt" => Some(Family::OpenAi),
        "gemini" => Some(Family::Gemini),
        "deepseek" => Some(Family::DeepSeek),
        _ => None,
    }
}

impl AgentEngine {
    /// The section this session's work adds for the model it runs on.
    pub(super) fn work_section(&self) -> Option<String> {
        self.work_section_for(&self.model_id)
    }

    /// The section for `model_id`: the text for every model, then its
    /// family's lines.
    fn work_section_for(&self, model_id: &str) -> Option<String> {
        match self.session_spec().work? {
            Work::Code => {}
        }
        let mut section = self.prompt_store.render_or_fallback(keys::WORK_CODE_CORE, &[]);
        let provider = self.model_manager.provider_name(model_id);
        if let Some(family) = family_of(provider, model_id) {
            section.push_str("\n\n");
            section.push_str(&self.prompt_store.render_or_fallback(family.key(), &[]));
        }
        Some(section)
    }

    /// `messages` as `model_id` should get them, when a call falls back to a
    /// model of another family than the one the prompt was built for: its
    /// family's lines in place of the first model's. `None` when nothing
    /// differs (the usual case).
    pub(crate) fn messages_for_model(
        &self,
        model_id: &str,
        messages: &[ChatMessage],
    ) -> Option<Vec<ChatMessage>> {
        let built = self.work_section()?;
        let wanted = self.work_section_for(model_id)?;
        if built == wanted {
            return None;
        }
        let mut out = messages.to_vec();
        let system = out.first_mut()?;
        if !system.content.contains(&built) {
            return None;
        }
        system.content = system.content.replacen(&built, &wanted, 1);
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_models_family_comes_from_its_id_then_its_provider() {
        let table = [
            (Some("openai"), "anything", Some(Family::OpenAi)),
            (Some("chatgpt"), "anything", Some(Family::OpenAi)),
            (Some("gemini"), "anything", Some(Family::Gemini)),
            (Some("deepseek"), "anything", Some(Family::DeepSeek)),
            // Served by someone else: the id says it.
            (Some("proxy"), "deepseek-flash", Some(Family::DeepSeek)),
            (Some("openrouter"), "google/gemini-3-pro", Some(Family::Gemini)),
            (Some("proxy"), "GPT-6.1-terra", Some(Family::OpenAi)),
            (None, "gpt-5.2-codex", Some(Family::OpenAi)),
            // No lines of their own.
            (Some("anthropic"), "claude-opus-5-5", None),
            (Some("ollama"), "qwen3:32b", None),
            (Some("proxy"), "some-model", None),
            (None, "some-model", None),
        ];
        for (provider, id, expected) in table {
            assert_eq!(family_of(provider, id), expected, "{provider:?} {id}");
        }
    }
}
