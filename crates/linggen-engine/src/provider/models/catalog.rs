//! The built-in models every install carries: Linggen Cloud and the ChatGPT
//! sign-in models, and the ids they used to go by.

use super::chatgpt_catalog::{self, Snapshot};
use crate::config::ModelConfig;
use crate::provider::codex_auth;

/// Built-in Linggen Cloud model — present in every install, routed through
/// the linggen.dev/api/llm proxy with the account token resolved fresh per
/// request (sign-in needs no restart). A user-defined model with the same id
/// wins (BYOK-first), and injection never touches the default model choice.
pub const LINGGEN_CLOUD_MODEL_ID: &str = "deepseek-flash";

/// Previous ids of the Linggen Cloud model. DeepSeek retired
/// `deepseek-v4-flash` when V4.1 Flash shipped under the plain
/// `deepseek-flash` id (2026-09); the proxy still accepts the old name, and
/// so does every lookup here — a session, a skill's `model:` pin or a
/// `pet.model` written against the old id resolves to the cloud model
/// instead of failing silently into the default. Config::load rewrites
/// persisted defaults and pins to the current id.
pub const LINGGEN_CLOUD_RETIRED_MODEL_IDS: &[&str] = &["deepseek-v4-flash"];

/// The id an exact pin (a session's or mission's `model:`, an agent
/// override) should run on: a retired Linggen Cloud id maps to the current
/// one; a ChatGPT id the backend no longer lists to its family's model now
/// ([`chatgpt_catalog::pin_successor`]); everything else is itself. A pin
/// written against an old generation keeps resolving instead of falling
/// silently into the default. Callers holding the configured models check an
/// exact match first — a user model reusing an old id is theirs.
pub fn canonical_model_id(id: &str) -> String {
    if LINGGEN_CLOUD_RETIRED_MODEL_IDS.contains(&id) {
        return LINGGEN_CLOUD_MODEL_ID.to_string();
    }
    chatgpt_catalog::pin_successor(id).unwrap_or_else(|| id.to_string())
}

/// The id a reference that follows its family (a room's shared models)
/// should say: like [`canonical_model_id`], but an older ChatGPT id moves to
/// its family's model even while the backend still lists it.
pub fn follow_model_id(id: &str) -> String {
    if LINGGEN_CLOUD_RETIRED_MODEL_IDS.contains(&id) {
        return LINGGEN_CLOUD_MODEL_ID.to_string();
    }
    chatgpt_catalog::successor(id).unwrap_or_else(|| id.to_string())
}

pub(super) fn linggen_cloud_url() -> String {
    format!("{}/api/llm", crate::account::site_url())
}

pub(super) const LINGGEN_CLOUD_PROVIDER: &str = "openai";

pub(super) fn inject_linggen_cloud(configs: &mut Vec<ModelConfig>) {
    const CLOUD_MODEL_ID: &str = LINGGEN_CLOUD_MODEL_ID;
    if configs.iter().any(|c| c.id == CLOUD_MODEL_ID) {
        return;
    }
    configs.push(ModelConfig {
        id: CLOUD_MODEL_ID.to_string(),
        provider: LINGGEN_CLOUD_PROVIDER.to_string(),
        url: linggen_cloud_url(),
        model: CLOUD_MODEL_ID.to_string(),
        api_key: None,
        keep_alive: None,
        context_window: Some(1_000_000),
        tags: vec![],
        supports_tools: Some(true),
        auth_mode: Some(crate::config::AuthMode::LinggenAccount),
        reasoning_effort: None,
        max_output_tokens: None,
        provided_by: Some("Linggen Cloud".to_string()),
        is_builtin: true,
            price: None,
    });
}

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
            price: None,
    }
}
