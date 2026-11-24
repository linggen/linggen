//! The settings picker and actual calls share the catalog/wire intersection.

use super::{family_catalog, ModelManager};
use crate::config::{ModelConfig, ProviderKind, SignIn};
use crate::provider::openai::OpenAiClient;

fn choices(row: &ModelConfig) -> Vec<&'static str> {
    if row.sign_in() != SignIn::Key || row.provider_kind() != ProviderKind::OpenAiCompatible
        || !OpenAiClient::model_supports_reasoning(&row.model, OpenAiClient::gemini_wire(&row.url))
    {
        // Subscription capabilities must come from its own backend, not BYOK facts.
        return Vec::new();
    }
    let catalog = family_catalog::current();
    let Some(facts) = catalog.exact(row) else { return Vec::new() };
    OpenAiClient::EFFORT_VALUES.into_iter().filter(|value| facts.effort_values().any(|v| v == *value)).collect()
}

impl ModelManager {
    pub fn display_name(&self, row: &ModelConfig) -> String {
        family_catalog::current().exact(row).map_or_else(|| row.model.clone(), |facts| facts.name().to_string())
    }

    pub fn effort_choices(&self, id: &str) -> Vec<&'static str> {
        self.models.get(self.key(id).as_ref()).map_or_else(Vec::new, |instance| choices(&instance.config))
    }

    /// Resolve after selecting the actual model, including on every fallback.
    /// `auto` is an explicit wire omission; unlike `None` it suppresses legacy row effort.
    pub(crate) fn turn_effort<'a>(&self, id: &str, agent: Option<&'a str>, chat: Option<&'a str>) -> Option<&'a str> {
        agent.or_else(|| chat.map(|value| {
            if self.effort_choices(id).contains(&value) { value } else { "auto" }
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_overlap_of_exact_catalog_values_and_wire_values_is_offered() {
        let mut row: ModelConfig = serde_json::from_value(serde_json::json!({
            "id": "reasoning", "model": "gpt-5.4-pro", "provider": "openai", "url": "https://api.openai.com/v1"
        })).unwrap();
        assert_eq!(choices(&row), ["medium", "high"]);
        row.provider = "ollama".into();
        assert!(choices(&row).is_empty());
        row.provider = "openai".into();
        row.url = "https://gateway.invalid/v1".into();
        assert!(choices(&row).is_empty());
    }

    #[test]
    fn unsupported_defaults_omit_and_legacy_agent_intent_wins() {
        let models = ModelManager::new_with_credentials(vec![], &Default::default());
        assert!(models.effort_choices("gpt-6-luna").is_empty());
        assert_eq!(models.turn_effort("gpt-6-luna", None, Some("high")), Some("auto"));
        assert_eq!(models.turn_effort("gpt-6-luna", Some("low"), Some("high")), Some("low"));
        assert_eq!(models.turn_effort("gpt-6-luna", None, None), None);
    }
}
