//! Linggen Cloud's models: ticked rows like any provider's, with the account
//! for a key. Two moments tick them with no person's click — a connection's
//! first start (the list's starter) and an old id that became another model —
//! and both run when the config is read (`Config::from_toml`), before anything
//! is served. A person's tick and untick go through `act`.

use super::binding::{self, Mode};
use super::super::catalog::{LINGGEN_CLOUD_RETIRED_MODEL_IDS, LINGGEN_CLOUD_RETIRED_SUCCESSOR};
use super::super::curated::{self, Bundle, EntryType, Place};
use super::super::family_catalog::Target;
use crate::config::Config;

/// What Linggen Cloud's chat models run with: the relay serves each of them
/// with this much context (no facts are joined to a connection with no address).
pub(super) const CONTEXT_WINDOW: usize = 1_000_000;

/// Tick one entry of the account's place as a pinned row, unless it is ticked.
fn tick_entry(config: &mut Config, list: &Bundle, place: &Place<'_>, family: &str) -> bool {
    let Some(entry) = place.entry_for(family) else { return false };
    if binding::row_of(list, config, place.endpoint_key, family).is_some() {
        return false;
    }
    let target = Target { id: &entry.model, facts: list.facts_for(place.endpoint_key, &entry.model) };
    // An exact entry is a pin from the start (as in `act`).
    let mode = if entry.kind == EntryType::Exact { Mode::Pinned } else { Mode::Follow };
    binding::tick(config, place, family, target, mode);
    true
}

/// The family key of the entry that runs `model`, at the account's place.
fn family_running<'a>(place: &Place<'a>, model: &str) -> Option<&'a str> {
    place.tickable().find(|(_, e)| e.model == model).map(|(family, _)| family)
}

/// The config as it is read: an id Linggen Cloud retired becomes the model it
/// became (in the default chain and Yinyue's pin) and that model is ticked;
/// the list's starters are ticked once. True when the config changed.
pub(crate) fn settle(config: &mut Config) -> bool {
    let list = curated::current();
    let Some(place) = list.account_place(config.model_catalog.offered()) else { return false };
    let mut changed = false;
    let retired = |id: &str| LINGGEN_CLOUD_RETIRED_MODEL_IDS.contains(&id) && !config.models.iter().any(|m| m.id == id);
    let moves = config.routing.default_models.iter().chain([&config.pet.model]).any(|id| retired(id));
    if moves {
        let was = std::mem::take(&mut config.routing.default_models);
        let mut seen = std::collections::HashSet::new();
        config.routing.default_models = was.into_iter()
            .map(|id| if retired(&id) { LINGGEN_CLOUD_RETIRED_SUCCESSOR.to_string() } else { id })
            .filter(|id| seen.insert(id.clone()))
            .collect();
        if retired(&config.pet.model) {
            config.pet.model = LINGGEN_CLOUD_RETIRED_SUCCESSOR.to_string();
        }
        if let Some(family) = family_running(&place, LINGGEN_CLOUD_RETIRED_SUCCESSOR) {
            tick_entry(config, &list, &place, family);
        }
        changed = true;
    }
    if !config.family_starts.contains(place.endpoint_key) {
        for (family, _) in place.tickable().filter(|(_, e)| e.starter) {
            tick_entry(config, &list, &place, family);
        }
        binding::mark_started(config, place.endpoint_key);
        changed = true;
    }
    changed
}
