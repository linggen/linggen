//! The built-in models every install carries (the ChatGPT sign-in models), the
//! Linggen Cloud address, and the ids they used to go by.

use super::chatgpt_catalog::{self, Snapshot};
use crate::config::ModelConfig;
use crate::provider::codex_auth;

/// The Linggen Cloud model a person gets first: the list's starter, ticked on
/// a first run with no list read yet (`cloud_tick`, `cloud_models`) and what a fresh config defaults to. A
/// Linggen Cloud model is an ordinary ticked row (`cloud_tick`), routed through
/// the linggen.dev/api/llm relay with the account token resolved fresh per
/// request (sign-in needs no restart); its id is its wire model.
pub const LINGGEN_CLOUD_MODEL_ID: &str = "qwen3.8-flash";

/// Previous ids of the Linggen Cloud model. DeepSeek retired
/// `deepseek-v4-flash` when V4.1 Flash shipped under the plain
/// `deepseek-flash` id (2026-09); that id went when the relay named the model
/// `deepseek-v4.1-flash` and Linggen Cloud's models became ones to tick
/// (2026-11). The relay still accepts the old names, and so does every lookup
/// here — a session, a skill's `model:` pin or a `pet.model` written against
/// an old id resolves to [`LINGGEN_CLOUD_RETIRED_SUCCESSOR`] instead of
/// failing silently into the default. Config::load rewrites persisted
/// defaults and pins to it, and ticks it (`cloud_tick`).
pub const LINGGEN_CLOUD_RETIRED_MODEL_IDS: &[&str] = &["deepseek-v4-flash", "deepseek-flash"];

/// The model a retired Linggen Cloud id became.
pub const LINGGEN_CLOUD_RETIRED_SUCCESSOR: &str = "deepseek-v4.1-flash";

/// The id an exact pin (a session's or mission's `model:`, an agent
/// override) should run on: a retired Linggen Cloud id maps to the current
/// one; a ChatGPT id the backend no longer lists to its family's model now
/// ([`chatgpt_catalog::pin_successor`]); everything else is itself. A pin
/// written against an old generation keeps resolving instead of falling
/// silently into the default. Callers holding the configured models check an
/// exact match first — a user model reusing an old id is theirs.
pub fn canonical_model_id(id: &str) -> String {
    if LINGGEN_CLOUD_RETIRED_MODEL_IDS.contains(&id) {
        return LINGGEN_CLOUD_RETIRED_SUCCESSOR.to_string();
    }
    chatgpt_catalog::pin_successor(id).unwrap_or_else(|| id.to_string())
}

/// The id a reference that follows its family (a room's shared models)
/// should say: like [`canonical_model_id`], but an older ChatGPT id moves to
/// its family's model even while the backend still lists it.
pub fn follow_model_id(id: &str) -> String {
    if LINGGEN_CLOUD_RETIRED_MODEL_IDS.contains(&id) {
        return LINGGEN_CLOUD_RETIRED_SUCCESSOR.to_string();
    }
    chatgpt_catalog::successor(id).unwrap_or_else(|| id.to_string())
}

pub(crate) fn linggen_cloud_url() -> String {
    format!("{}/api/llm", crate::account::site_url())
}

pub(crate) const LINGGEN_CLOUD_PROVIDER: &str = "openai";

/// The ChatGPT built-ins, one per family, as the catalog has them now
/// ([`chatgpt_catalog`](super::chatgpt_catalog) owns which models those
/// are). Unlike the Linggen Cloud built-in, these ALWAYS win: any
/// user-configured entry with one of these ids is replaced, not deferred to,
/// so it's never rendered as a raw editable duplicate — sign in and star one,
/// nothing to configure. A user wanting a different/custom ChatGPT-backed
/// model should give it a different id.
///
/// Also registered: every older model of a built-in family the backend still
/// lists ([`Snapshot::kept_ids`]), so a session or mission pinned to one keeps
/// running on it. Their ids are returned — they are never offered.
pub(super) fn inject_chatgpt_builtin(configs: &mut Vec<ModelConfig>, snap: &Snapshot) -> Vec<String> {
    for id in snap.builtin_ids() {
        configs.retain(|c| c.id != id);
        configs.push(chatgpt_config(&id, snap));
    }
    // A kept id the person configured themselves is theirs, and stays listed.
    let kept: Vec<String> = snap
        .kept_ids()
        .into_iter()
        .filter(|id| !configs.iter().any(|c| &c.id == id))
        .collect();
    configs.extend(kept.iter().map(|id| chatgpt_config(id, snap)));
    kept
}

/// The built-in ids a page does not offer: every one while signed out (the
/// provider has not been added), else the models of the families the person
/// unticked. They stay registered, so a pin to one still runs.
pub(super) fn not_offered(snap: &Snapshot, unticked: &[String], signed_in: bool) -> Vec<String> {
    snap.builtin_ids().into_iter()
        .filter(|id| !signed_in || snap.family_of(id).is_some_and(|f| unticked.contains(&f.name)))
        .collect()
}

fn chatgpt_config(id: &str, snap: &Snapshot) -> ModelConfig {
    ModelConfig {
        id: id.to_string(),
        provider: "chatgpt".to_string(),
        url: codex_auth::CHATGPT_API_BASE.to_string(),
        model: id.to_string(),
        api_key: None,
        keep_alive: None,
        context_window: snap.context_window(id).map(|w| w as usize),
        tags: vec![],
        supports_tools: Some(true),
        auth_mode: Some(crate::config::AuthMode::ChatgptOauth),
        reasoning_effort: None,
        max_output_tokens: None,
        provided_by: Some("ChatGPT".to_string()),
        is_builtin: true,
    }
}
