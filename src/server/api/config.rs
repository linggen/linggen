use crate::config::Config;
use crate::credentials::{self, Credentials};
use crate::provider::codex_auth;
use crate::server::ServerState;
use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use std::sync::Arc;

pub(crate) async fn get_config_api(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    let mut config = state.manager.get_config_snapshot().await;
    // Strip API keys from config response — credentials are served separately.
    for model in &mut config.models {
        model.api_key = None;
    }
    #[derive(serde::Serialize)]
    struct ConfigView {
        #[serde(flatten)]
        config: Config,
        // Keep the default separate so saving other settings makes no choice.
        daily_session_default: bool,
    }
    Json(ConfigView {
        config,
        daily_session_default: crate::config::DAILY_SESSION_DEFAULT,
    })
}

pub(crate) async fn update_config_api(
    State(state): State<Arc<ServerState>>,
    Json(mut new_config): Json<Config>,
) -> impl IntoResponse {
    let _write = state.settings_write.lock().await;
    // Which private apps outside agents may use is set at this Mac alone
    // (`outside_apps::set`): a saved config, from any page, keeps it.
    let current = state.manager.get_config_snapshot().await;
    new_config.outside_apps = current.outside_apps.clone();
    // Which edition of the model list is offered is the owner's own route (`providers::set_edition`).
    new_config.model_catalog = current.model_catalog.clone();
    // Only /api/uses edits these choices; a stale settings page keeps them.
    new_config.routing.default_models = current.routing.default_models.clone();
    new_config.keep_model_fallback(&current);
    new_config.uses = current.uses.clone();
    new_config.pet.voice = current.pet.voice.clone();
    // Nor does it undo a family's move, or set a binding: the writer is `families`.
    crate::provider::models::families::Families::keep_bindings(&mut new_config, &current);
    if let Err(e) = take_credentials(&mut new_config, &credentials::credentials_file()) {
        return (StatusCode::BAD_REQUEST, e).into_response();
    }
    match state.manager.apply_config(new_config).await {
        Ok(()) => {
            // apply_config rebuilt ModelManager from static config and dropped
            // any dynamically-registered proxy models. Restore them so chats
            // requesting `proxy:*` ids don't fall back to local defaults.
            crate::server::rtc::proxy_room::reapply_proxy_models(&state).await;
            Json(serde_json::json!({ "status": "ok" })).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// Take the keys out of a posted config into `creds_file` and prune the
/// per-model overrides of rows it no longer has.
fn take_credentials(new_config: &mut Config, creds_file: &std::path::Path) -> Result<(), String> {
    // A page opened before a ChatGPT family moved (or a Stay was pressed)
    // posts the old ids back: they follow the catalog again here.
    new_config.follow_chatgpt_catalog(&crate::provider::models::chatgpt_catalog::current());
    // A config the save refuses writes nothing: checked before the
    // credentials file is even read (reading may migrate it).
    new_config.validate().map_err(|e| e.to_string())?;
    // Strip API keys from config — they should be saved via /api/credentials instead.
    // For backward compat: if the UI sends a non-redacted, non-empty key, migrate it
    // to credentials.json.
    let mut creds = Credentials::load_for(creds_file, &new_config.models);
    let mut creds_changed = false;

    for model in &mut new_config.models {
        if let Some(ref key) = model.api_key {
            if key != "***" && !key.is_empty() {
                // Migrate to credentials.json — a key belongs to the endpoint.
                creds.set_endpoint_key(&model.provider, &model.url, Some(key.clone()));
                creds_changed = true;
            }
        }
        // Always strip from TOML
        model.api_key = None;
    }
    // An override belongs to a model row; a deleted row takes it along.
    if creds.prune_overrides(&new_config.models) > 0 {
        creds_changed = true;
    }

    if creds_changed {
        if let Err(e) = creds.save(creds_file) {
            tracing::warn!("Failed to save credentials.json: {}", e);
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Model health API — in-memory health status for all configured models
// ---------------------------------------------------------------------------

pub(crate) async fn get_models_health(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    use crate::provider::models::ModelHealthStatus;

    let models_guard = state.manager.models.read().await;
    let health_records = crate::provider::models::health_snapshot();

    // Build a map of model_id → health record for easy lookup
    let health_map: std::collections::HashMap<String, _> = health_records.into_iter().collect();

    // Return health for all configured models
    let models_ref = &*models_guard;

    let mut result: Vec<serde_json::Value> = Vec::new();
    for m in models_ref.list_models() {
        let cw = models_ref.context_window(&m.id).await.ok().flatten();
        let mut entry = if let Some(rec) = health_map.get(&m.id) {
            serde_json::json!({
                "id": m.id,
                "health": rec.status,
                "kind": rec.kind,
                "last_error": rec.last_error,
                "since_secs": rec.since_secs,
                "retry_in_secs": rec.retry_in_secs,
            })
        } else {
            serde_json::json!({
                "id": m.id,
                "health": ModelHealthStatus::Healthy,
                "last_error": null,
                "since_secs": null,
            })
        };
        if let Some(cw) = cw {
            entry
                .as_object_mut()
                .unwrap()
                .insert("context_window".to_string(), serde_json::json!(cw));
        }
        result.push(entry);
    }

    Json(result).into_response()
}

// ---------------------------------------------------------------------------
// Credentials API — reads/writes ~/.linggen/credentials.json
// ---------------------------------------------------------------------------

pub(crate) async fn get_credentials_api(
    State(state): State<std::sync::Arc<crate::server::ServerState>>,
) -> impl IntoResponse {
    let config = state.manager.get_config_snapshot().await;
    let creds = Credentials::load_for(&credentials::credentials_file(), &config.models);
    Json(creds.redacted()).into_response()
}

/// Two body shapes are accepted. The version-2 shape names what a key is
/// for: `{"endpoints": [{provider, url, api_key}], "models": {"<id>":
/// {api_key}}, "services": {"<name>": {api_key}}}` (endpoints may also be
/// the older map keyed by id; a null or empty api_key removes). The
/// settings page's older flat shape, model id → `{api_key, provider, url}`,
/// means "the key for this model's endpoint" (or the model's own override
/// when no endpoint is given). `null` or an empty key removes; `"***"` is
/// the redacted placeholder and changes nothing.
#[derive(serde::Deserialize)]
pub(crate) struct UpdateCredentialsRequest {
    #[serde(flatten)]
    entries: std::collections::HashMap<String, serde_json::Value>,
}

fn key_of(v: &serde_json::Value) -> Option<Option<String>> {
    match v {
        serde_json::Value::Null => Some(None),
        serde_json::Value::Object(obj) => match obj.get("api_key") {
            Some(serde_json::Value::String(s)) if s == "***" => None,
            Some(serde_json::Value::String(s)) => Some(Some(s.clone())),
            _ => Some(None),
        },
        _ => None,
    }
}

fn endpoint_of(v: &serde_json::Value) -> Option<(String, String)> {
    let obj = v.as_object()?;
    match (
        obj.get("provider").and_then(|p| p.as_str()),
        obj.get("url").and_then(|u| u.as_str()),
    ) {
        (Some(p), Some(u)) if !p.is_empty() && !u.is_empty() => {
            Some((p.to_string(), u.to_string()))
        }
        _ => None,
    }
}

pub(crate) async fn update_credentials_api(
    State(state): State<std::sync::Arc<crate::server::ServerState>>,
    Json(body): Json<UpdateCredentialsRequest>,
) -> impl IntoResponse {
    let _write = state.settings_write.lock().await;
    let creds_file = credentials::credentials_file();
    let config = state.manager.get_config_snapshot().await;
    let mut creds = Credentials::load_for(&creds_file, &config.models);

    let sectioned = ["endpoints", "models", "services"]
        .iter()
        .any(|k| body.entries.contains_key(*k));
    if sectioned {
        let listed: Vec<&serde_json::Value> = match body.entries.get("endpoints") {
            Some(serde_json::Value::Array(items)) => items.iter().collect(),
            Some(serde_json::Value::Object(map)) => map.values().collect(),
            _ => Vec::new(),
        };
        for v in listed {
            let Some(key) = key_of(v) else { continue };
            if let Some((p, u)) = endpoint_of(v) {
                creds.set_endpoint_key(&p, &u, key);
            }
        }
        for (id, v) in body
            .entries
            .get("models")
            .and_then(|e| e.as_object())
            .into_iter()
            .flatten()
        {
            if let Some(key) = key_of(v) {
                creds.set_model_override(id, key);
            }
        }
        for (id, v) in body
            .entries
            .get("services")
            .and_then(|e| e.as_object())
            .into_iter()
            .flatten()
        {
            if let Some(key) = key_of(v) {
                creds.set_service_key(id, key);
            }
        }
    } else {
        for (model_id, v) in &body.entries {
            let Some(key) = key_of(v) else { continue };
            let endpoint = endpoint_of(v).or_else(|| {
                config
                    .models
                    .iter()
                    .find(|m| &m.id == model_id)
                    .map(|m| (m.provider.clone(), m.url.clone()))
            });
            match endpoint {
                Some((p, u)) => {
                    // The field on a model row edits the endpoint's key; a
                    // stale override on that model would hide the change.
                    creds.set_model_override(model_id, None);
                    creds.set_endpoint_key(&p, &u, key);
                }
                None => creds.set_model_override(model_id, key),
            }
        }
    }

    if let Err(e) = creds.save(&creds_file) {
        return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
    }

    // A saved key takes effect now. The model manager resolves keys when it
    // is built, and the settings page saves config before credentials — so
    // without this rebuild a freshly entered key stayed dead until the next
    // config save or restart.
    reload_models(&state).await;
    tracing::info!("Reloaded models after credentials update");

    Json(serde_json::json!({ "status": "ok" })).into_response()
}

#[derive(serde::Deserialize)]
pub(crate) struct StayRequest {
    family: String,
    stay: bool,
}

/// POST /api/models/chatgpt/stay — stay on a ChatGPT family's previous model
/// (while the backend lists it), or follow the family again. Kept in the
/// catalog, so it survives refreshes; what follows the family moves with it.
pub(crate) async fn set_chatgpt_stay(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<StayRequest>,
) -> impl IntoResponse {
    let catalog = match crate::provider::models::chatgpt_catalog::set_stay(&req.family, req.stay) {
        Ok(c) => c,
        Err(why) => return (StatusCode::BAD_REQUEST, why).into_response(),
    };
    crate::server::background::follow_chatgpt_catalog(&state, &catalog).await;
    reload_models(&state).await;
    Json(serde_json::json!({ "status": "ok", "builtins": catalog.builtin_ids() })).into_response()
}

/// Rebuild the model manager from the config in force (fresh keys, tokens,
/// catalog) and restore the proxy models the rebuild drops. Session engines
/// are kept: each reads the new models at its next turn
/// (`AgentManager::mark_settings_changed`).
pub(crate) async fn reload_models(state: &std::sync::Arc<crate::server::ServerState>) {
    let config = state.manager.get_config_snapshot().await;
    let new_models = std::sync::Arc::new(crate::provider::models::ModelManager::new(
        config.models.clone(),
    ));
    *state.manager.models.write().await = new_models;
    crate::server::rtc::proxy_room::reapply_proxy_models(state).await;
    state.manager.mark_settings_changed();
}

/// A sign-in was made: the default model follows it when it could not run
/// (the rule is `model_choice::after_sign_in`; every sign-in's end calls
/// this). The models are rebuilt by the config change, so a room's proxy
/// models are put back.
pub(crate) async fn adopt_default_after_sign_in(
    state: &std::sync::Arc<crate::server::ServerState>,
    signed_in: crate::config::SignIn,
) {
    let _write = state.settings_write.lock().await;
    if state.manager.adopt_default_after_sign_in(signed_in).await.is_some() {
        crate::server::rtc::proxy_room::reapply_proxy_models(state).await;
    }
}

/// How long a page waits, after a ChatGPT sign-in shows, for its end to
/// finish choosing the default.
const SIGN_IN_SETTLE: std::time::Duration = std::time::Duration::from_secs(3);

/// GET /api/models/signed-in-default — the model the latest sign-in made
/// the default, with the one line to tell it and the chain to adopt; null
/// when it made none (or the default has moved since).
pub(crate) async fn get_signed_in_default(
    State(state): State<std::sync::Arc<crate::server::ServerState>>,
) -> impl IntoResponse {
    let waiting_since = std::time::Instant::now();
    while state.codex_login_task.lock().await.is_some() && waiting_since.elapsed() < SIGN_IN_SETTLE {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let Some(model) = state.manager.signed_in_default().await else {
        return Json(serde_json::Value::Null);
    };
    let chain = state.manager.get_config_snapshot().await.routing.default_models;
    Json(serde_json::json!({
        "model": model,
        "line": crate::engine::model_choice::made_default_line(&model),
        "default_models": chain,
    }))
}

// ---------------------------------------------------------------------------
// ChatGPT OAuth API — login, status, logout
// ---------------------------------------------------------------------------

/// Get ChatGPT OAuth status.
pub(crate) async fn get_codex_auth_status() -> impl IntoResponse {
    let tokens = codex_auth::CodexAuthTokens::load(&codex_auth::codex_auth_file());
    Json(serde_json::json!({
        "authenticated": tokens.is_valid(),
        "account_id": tokens.account_id,
        "needs_refresh": tokens.needs_refresh(),
        "last_refresh": tokens.last_refresh,
    }))
    .into_response()
}

/// Account-level Codex quota; credentials never leave the engine.
pub(crate) async fn get_codex_usage() -> impl IntoResponse {
    if !codex_auth::chatgpt_signed_in() {
        return (StatusCode::UNAUTHORIZED, "Not signed in").into_response();
    }
    let tokens = codex_auth::CodexAuthTokens::load(&codex_auth::codex_auth_file());
    let client = crate::provider::openai::OpenAiClient::new_chatgpt_oauth(
        codex_auth::CHATGPT_API_BASE.to_string(), tokens.access_token.unwrap_or_default(), tokens.account_id,
    );
    match client.codex_metadata("../wham/usage").await {
        Ok(value) => Json(serde_json::json!({
            "rate_limit": value.get("rate_limit"),
            "credits": value.get("credits"),
        })).into_response(),
        Err(_) => (StatusCode::BAD_GATEWAY, "Usage unavailable").into_response(),
    }
}

/// Start ChatGPT OAuth browser login flow.
///
/// Aborts any prior in-flight login task before spawning a new one. Without
/// this, two `browser_login()` flows can race after a failed first attempt:
/// each holds an independent state + callback server, and the browser's
/// cached redirect from one attempt lands at the other's server, producing
/// a "State mismatch. Please try again." error. The Apple Shifu settings
/// page is the typical reproducer (signin → fail → signout → signin).
pub(crate) async fn start_codex_auth_login(
    State(state): State<std::sync::Arc<crate::server::ServerState>>,
) -> impl IntoResponse {
    // Cancel any prior in-flight flow before starting a new one.
    {
        let mut guard = state.codex_login_task.lock().await;
        if let Some(prior) = guard.take() {
            prior.abort();
            tracing::info!("Aborted prior ChatGPT OAuth login task to start a fresh one");
        }
    }

    let state_for_task = state.clone();
    let handle = tokio::spawn(async move {
        match codex_auth::browser_login().await {
            Ok(_) => {
                tracing::info!("ChatGPT OAuth login completed via Web UI");
                // Rebuild ModelManager so it picks up the fresh token
                reload_models(&state_for_task).await;
                tracing::info!("Reloaded models after ChatGPT OAuth login");
                adopt_default_after_sign_in(&state_for_task, crate::config::SignIn::ChatGpt).await;
                // A fresh sign-in reads the catalog now, in the background.
                crate::server::background::refresh_chatgpt_catalog(state_for_task.clone(), true);
            }
            Err(e) => tracing::warn!("ChatGPT OAuth login failed: {}", e),
        }
        // Self-clear: drop our slot so logout/restart can detect "no flow alive".
        let mut guard = state_for_task.codex_login_task.lock().await;
        *guard = None;
    });
    *state.codex_login_task.lock().await = Some(handle);

    Json(serde_json::json!({
        "status": "login_started",
        "message": "Browser opened for ChatGPT login. Complete sign-in in the browser.",
    }))
    .into_response()
}

/// Logout from ChatGPT OAuth.
///
/// Also aborts any in-flight login task so a sign-out mid-attempt fully
/// clears server-side state — the next sign-in starts from a clean slate.
pub(crate) async fn codex_auth_logout(
    State(state): State<std::sync::Arc<crate::server::ServerState>>,
) -> impl IntoResponse {
    {
        let mut guard = state.codex_login_task.lock().await;
        if let Some(prior) = guard.take() {
            prior.abort();
            tracing::info!("Aborted in-flight ChatGPT OAuth login task on logout");
        }
    }

    let manager = codex_auth::CodexAuthManager::new();
    match manager.logout().await {
        Ok(()) => Json(serde_json::json!({ "status": "ok" })).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

// ---------------------------------------------------------------------------
// Claude Code OAuth API — status only (no login/logout)
// ---------------------------------------------------------------------------
//
// Unlike ChatGPT, Claude Code OAuth tokens are managed by the `claude` CLI —
// sign-in and refresh happen there, Linggen just reads the OS-native store
// on every inference call. So we expose a status endpoint but no login /
// logout routes; the UI instead tells the user to run `claude`.

/// Get Claude Code OAuth status. Never returns the actual access token —
/// the UI only needs metadata (subscription, scopes, expiry) for display.
pub(crate) async fn get_claude_auth_status() -> impl IntoResponse {
    match crate::provider::claude_auth::load() {
        Ok(tokens) => Json(serde_json::json!({
            "authenticated": !tokens.is_expired(),
            "expires_at": tokens.expires_at,
            "expired": tokens.is_expired(),
            "subscription_type": tokens.subscription_type,
            "rate_limit_tier": tokens.rate_limit_tier,
            "scopes": tokens.scopes,
            "can_do_inference": tokens.can_do_inference(),
        }))
        .into_response(),
        Err(e) => Json(serde_json::json!({
            "authenticated": false,
            "error": e.to_string(),
        }))
        .into_response(),
    }
}

// ---------------------------------------------------------------------------
// MCP servers — what the Settings tab renders
// ---------------------------------------------------------------------------

/// `GET /api/mcp` — every configured MCP server and what became of it.
///
/// Deliberately includes servers that FAILED, with the reason. A server the
/// engine tried and could not reach must be visible: absent reads as "not
/// configured", which is a different and misleading thing. Same rule as the
/// rest of the UI — show the server's state, never a prettier version of it.
///
/// `gated` rides along because a tab that lets you add a server without saying
/// what its tools may do is a phantom: real capability, invisible.
pub(crate) async fn list_mcp_api() -> impl IntoResponse {
    let servers: Vec<serde_json::Value> = crate::mcp_client::registry()
        .status()
        .into_iter()
        .map(|s| {
            let builtin = s.name == crate::mcp_client::BUILTIN_MEMORY;
            serde_json::json!({
                "name": s.name,
                "target": s.target,
                "enabled": s.enabled,
                "connected": s.connected,
                "error": s.error,
                "tools": s.tools,
                // What the gate actually does with this server's tools —
                // read from the entry, not inferred here, so the UI cannot
                // disagree with what the engine enforces.
                "gated": s.gated,
                "tier": if s.gated { "admin" } else { "ungated" },
                // Provenance is presentation only. The permission decision is
                // `gated` above; this just lets the tab explain why the
                // shipped entry differs, and why removing it is a config edit
                // rather than a mistake.
                "builtin": builtin,
            })
        })
        .collect();
    axum::Json(serde_json::json!({ "servers": servers }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(id: &str, name: &str) -> crate::config::ModelConfig {
        serde_json::from_value(serde_json::json!({
            "id": id, "provider": "openai", "url": "https://api.example.com/v1", "model": name
        }))
        .unwrap()
    }

    /// A config the save refuses writes nothing: the key it carried is not
    /// stored and no override of a row it left out is pruned.
    #[test]
    fn a_refused_config_writes_no_credentials() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("credentials.json");
        let mut creds = Credentials::default();
        creds.set_model_override("kept", Some("sk-kept".into()));
        creds.save(&file).unwrap();
        let before = std::fs::read(&file).unwrap();

        let mut posted = Config::default();
        let mut bad = model("new", "");
        bad.api_key = Some("sk-new".into());
        posted.models = vec![bad];
        assert!(take_credentials(&mut posted, &file).is_err());
        assert_eq!(std::fs::read(&file).unwrap(), before);

        // The same post, made valid, is taken.
        posted.models[0].model = "gpt".into();
        posted.models[0].api_key = Some("sk-new".into());
        take_credentials(&mut posted, &file).unwrap();
        assert!(posted.models[0].api_key.is_none());
        let after = Credentials::load(&file);
        assert!(after.model_override("kept").is_none());
        assert_eq!(
            after.endpoint_key("openai", "https://api.example.com/v1"),
            Some("sk-new")
        );
    }
}

#[cfg(test)]
mod keeps_engines {
    /// A reload of models tells live engines to read the new list
    /// (`AgentManager::mark_settings_changed`); it never drops them (engine
    /// side: `a_change_of_config_keeps_every_live_engine`).
    #[test]
    fn reload_models_keeps_every_live_engine() {
        let text = include_str!("config.rs");
        let start = text.find("pub(crate) async fn reload_models(").expect("reload_models");
        let body = &text[start..];
        let body = &body[..body.find("\n}\n").expect("its end")];
        assert!(
            body.contains(["mark_settings_changed", "()"].concat().as_str()),
            "reload_models must mark the change"
        );
        assert!(
            !body.contains(["session_", "engines"].concat().as_str()),
            "reload_models must not drop the session engines"
        );
    }
}
