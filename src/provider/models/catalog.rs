//! The built-in models every install carries: Linggen Cloud and the ChatGPT
//! sign-in models, and the ids they used to go by.

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

/// The id a model lookup should use: retired Linggen Cloud ids map to the
/// current one, everything else is itself.
pub fn canonical_model_id(id: &str) -> &str {
    if LINGGEN_CLOUD_RETIRED_MODEL_IDS.contains(&id) {
        LINGGEN_CLOUD_MODEL_ID
    } else {
        id
    }
}

pub(super) fn inject_linggen_cloud(configs: &mut Vec<ModelConfig>) {
    const CLOUD_MODEL_ID: &str = LINGGEN_CLOUD_MODEL_ID;
    if configs.iter().any(|c| c.id == CLOUD_MODEL_ID) {
        return;
    }
    configs.push(ModelConfig {
        id: CLOUD_MODEL_ID.to_string(),
        provider: "openai".to_string(),
        url: format!("{}/api/llm", crate::account::site_url()),
        model: CLOUD_MODEL_ID.to_string(),
        api_key: None,
        keep_alive: None,
        context_window: Some(1_000_000),
        tags: vec![],
        supports_tools: Some(true),
        auth_mode: Some("linggen_account".to_string()),
        reasoning_effort: None,
        provided_by: Some("Linggen Cloud".to_string()),
        is_builtin: true,
    });
}

/// Primary built-in ChatGPT model — the default routing target and the
/// successor for any retired id without a closer match.
/// Bumping to a newer OpenAI generation: change the ids here AND list each
/// old id with its successor in CHATGPT_RETIRED_MODEL_IDS so persisted
/// configs migrate on load, then release.
pub const CHATGPT_BUILTIN_MODEL_ID: &str = "gpt-6-luna";

/// All built-in ChatGPT models — always present, using the user's own
/// ChatGPT subscription via OAuth (no API key). The GPT-6 family: Sol
/// (flagship, paid plans) and Luna (fast, every plan). Unlike the
/// Linggen Cloud built-in, these ALWAYS win: any user-configured entry
/// with one of these ids is replaced, not deferred to, so it's never
/// rendered as a raw editable duplicate — sign in and star one, nothing to
/// configure. A user wanting a different/custom ChatGPT-backed model
/// should give it a different id.
pub const CHATGPT_BUILTIN_MODEL_IDS: &[&str] = &["gpt-6-sol", "gpt-6-luna"];

/// Previous ChatGPT built-in ids, each with the built-in it moves to.
/// Config::load and the paired-device list migrate these, so a bump never
/// leaves a dangling default, pet pin, or orphaned editable card.
pub const CHATGPT_RETIRED_MODEL_IDS: &[(&str, &str)] = &[
    ("gpt-5.5", CHATGPT_BUILTIN_MODEL_ID),
    ("gpt-5.6-sol", "gpt-6-sol"),
    ("gpt-5.6-terra", CHATGPT_BUILTIN_MODEL_ID),
    ("gpt-5.6-luna", "gpt-6-luna"),
];

/// The current built-in a retired ChatGPT id moves to, if it is retired.
pub fn chatgpt_successor(id: &str) -> Option<&'static str> {
    CHATGPT_RETIRED_MODEL_IDS
        .iter()
        .find(|(old, _)| *old == id)
        .map(|(_, new)| *new)
}

pub(super) fn inject_chatgpt_builtin(configs: &mut Vec<ModelConfig>) {
    for id in CHATGPT_BUILTIN_MODEL_IDS {
        configs.retain(|c| c.id != *id);
        configs.push(ModelConfig {
            id: id.to_string(),
            provider: "chatgpt".to_string(),
            url: codex_auth::CHATGPT_API_BASE.to_string(),
            model: id.to_string(),
            api_key: None,
            keep_alive: None,
            context_window: None,
            tags: vec![],
            supports_tools: Some(true),
            auth_mode: Some("chatgpt_oauth".to_string()),
            reasoning_effort: None,
            provided_by: Some("ChatGPT".to_string()),
            is_builtin: true,
        });
    }
}
