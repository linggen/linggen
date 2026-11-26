//! Linggen Cloud's models as ticked rows: an ordinary `[[models]]` row each,
//! with the account for a key. This file is the one maker of such a row.
//!
//! Two moments tick one with no person's click, both when the config is read
//! (`Config::load_with_path`), before anything is served: the starter, once
//! per install (`Config::cloud_started`); and the model a retired Linggen
//! Cloud id became. A person's tick and untick are the provider routes.

use super::catalog::{linggen_cloud_url, LINGGEN_CLOUD_PROVIDER, LINGGEN_CLOUD_RETIRED_MODEL_IDS, LINGGEN_CLOUD_RETIRED_SUCCESSOR};
use crate::config::{AuthMode, Config, ModelConfig, SignIn};

/// The context the relay serves each of its chat models with.
const CONTEXT_WINDOW: usize = 1_000_000;

/// The row Linggen Cloud's `model` runs as: named by its wire model, so an
/// old pin to that id lands on it.
pub fn row(config: &Config, model: &str) -> ModelConfig {
    let id = if config.models.iter().any(|r| r.id == model) { config.fresh_model_id("cloud") } else { model.to_string() };
    ModelConfig {
        id,
        provider: LINGGEN_CLOUD_PROVIDER.to_string(),
        url: linggen_cloud_url(),
        model: model.to_string(),
        api_key: None,
        keep_alive: None,
        context_window: Some(CONTEXT_WINDOW),
        tags: Vec::new(),
        // The relay serves tools on every model it lists.
        supports_tools: Some(true),
        auth_mode: Some(AuthMode::LinggenAccount),
        reasoning_effort: None,
        max_output_tokens: None,
        provided_by: Some("Linggen Cloud".to_string()),
        is_builtin: false,
    }
}

/// The row that runs Linggen Cloud's `model`, if it is ticked.
pub fn ticked<'a>(config: &'a Config, model: &str) -> Option<&'a ModelConfig> {
    config.chat_models().find(|r| r.sign_in() == SignIn::LinggenAccount && r.model == model)
}

/// Tick `model` unless it is ticked. True when the config changed.
pub fn tick(config: &mut Config, model: &str) -> bool {
    if ticked(config, model).is_some() {
        return false;
    }
    let row = row(config, model);
    config.models.push(row);
    true
}

/// The config as it is read: a retired id becomes the model it became (in
/// the default chain and Yinyue's pin) and that model is ticked; the
/// starter is ticked once. True when the config changed.
pub fn settle(config: &mut Config) -> bool {
    let mut changed = false;
    let retired = |config: &Config, id: &str| LINGGEN_CLOUD_RETIRED_MODEL_IDS.contains(&id) && !config.models.iter().any(|m| m.id == id);
    let moves = config.routing.default_models.iter().chain([&config.pet.model]).any(|id| retired(config, id));
    if moves {
        let was = std::mem::take(&mut config.routing.default_models);
        let mut seen = std::collections::HashSet::new();
        let now: Vec<String> = was.into_iter()
            .map(|id| if retired(config, &id) { LINGGEN_CLOUD_RETIRED_SUCCESSOR.to_string() } else { id })
            .filter(|id| seen.insert(id.clone()))
            .collect();
        config.routing.default_models = now;
        if retired(config, &config.pet.model) {
            config.pet.model = LINGGEN_CLOUD_RETIRED_SUCCESSOR.to_string();
        }
        tick(config, LINGGEN_CLOUD_RETIRED_SUCCESSOR);
        changed = true;
    }
    if !config.cloud_started {
        tick(config, &super::cloud_models::current().starter);
        config.cloud_started = true;
        changed = true;
    }
    changed
}
