use crate::provider::codex_auth;
use crate::config::Config;
use crate::credentials::{self, Credentials};
use crate::server::ServerState;
use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use std::sync::Arc;

pub(crate) async fn get_config_api(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    let mut config = state.manager.get_config_snapshot().await;
    // Strip API keys from config response — credentials are served separately.
    for model in &mut config.models {
        model.api_key = None;
    }
    Json(config).into_response()
}

pub(crate) async fn update_config_api(
    State(state): State<Arc<ServerState>>,
    Json(mut new_config): Json<Config>,
) -> impl IntoResponse {
    // Strip API keys from config — they should be saved via /api/credentials instead.
    // For backward compat: if the UI sends a non-redacted, non-empty key, migrate it
    // to credentials.json.
    let creds_file = credentials::credentials_file();
    let mut creds = Credentials::load(&creds_file);
    let mut creds_changed = false;

    for model in &mut new_config.models {
        if let Some(ref key) = model.api_key {
            if key != "***" && !key.is_empty() {
                // Migrate to credentials.json
                creds.set_api_key(&model.id, Some(key.clone()));
                creds_changed = true;
            }
        }
        // Always strip from TOML
        model.api_key = None;
    }

    if creds_changed {
        if let Err(e) = creds.save(&creds_file) {
            tracing::warn!("Failed to save credentials.json: {}", e);
        }
    }

    if let Err(e) = new_config.validate() {
        return (StatusCode::BAD_REQUEST, e.to_string()).into_response();
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

// ---------------------------------------------------------------------------
// Model health API — in-memory health status for all configured models
// ---------------------------------------------------------------------------

pub(crate) async fn get_models_health(
    State(state): State<Arc<ServerState>>,
) -> impl IntoResponse {
    use crate::provider::models::ModelHealthStatus;

    let models_guard = state.manager.models.read().await;
    let health_records = models_guard.health.get_all().await;

    // Build a map of model_id → health record for easy lookup
    let health_map: std::collections::HashMap<String, _> =
        health_records.into_iter().collect();

    // Return health for all configured models
    let config = state.manager.get_config_snapshot().await;
    let models_ref = &*models_guard;

    let mut result: Vec<serde_json::Value> = Vec::new();
    for m in &config.models {
        let cw = models_ref.context_window(&m.id).await.ok().flatten();
        let mut entry = if let Some(rec) = health_map.get(&m.id) {
            serde_json::json!({
                "id": m.id,
                "health": rec.status,
                "last_error": rec.last_error,
                "since_secs": rec.since_secs,
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
            entry.as_object_mut().unwrap().insert("context_window".to_string(), serde_json::json!(cw));
        }
        result.push(entry);
    }

    Json(result).into_response()
}

// ---------------------------------------------------------------------------
// Credentials API — reads/writes ~/.linggen/credentials.json
// ---------------------------------------------------------------------------

pub(crate) async fn get_credentials_api() -> impl IntoResponse {
    let creds = Credentials::load(&credentials::credentials_file());
    Json(creds.redacted()).into_response()
}

#[derive(serde::Deserialize)]
pub(crate) struct UpdateCredentialsRequest {
    /// Model ID → API key. Send null/empty to remove.
    #[serde(flatten)]
    entries: std::collections::HashMap<String, serde_json::Value>,
}

pub(crate) async fn update_credentials_api(
    Json(body): Json<UpdateCredentialsRequest>,
) -> impl IntoResponse {
    let creds_file = credentials::credentials_file();
    let mut creds = Credentials::load(&creds_file);

    for (model_id, value) in &body.entries {
        match value {
            serde_json::Value::Object(obj) => {
                let api_key = obj
                    .get("api_key")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                // Skip if value is the redacted placeholder
                if api_key.as_deref() == Some("***") {
                    continue;
                }
                creds.set_api_key(model_id, api_key);
            }
            serde_json::Value::Null => {
                creds.set_api_key(model_id, None);
            }
            _ => {}
        }
    }

    match creds.save(&creds_file) {
        Ok(()) => Json(serde_json::json!({ "status": "ok" })).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
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

/// Start ChatGPT OAuth browser login flow.
///
/// Aborts any prior in-flight login task before spawning a new one. Without
/// this, two `browser_login()` flows can race after a failed first attempt:
/// each holds an independent state + callback server, and the browser's
/// cached redirect from one attempt lands at the other's server, producing
/// a "State mismatch. Please try again." error. The Mac Shifu settings
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

    let manager = state.manager.clone();
    let state_for_task = state.clone();
    let handle = tokio::spawn(async move {
        match codex_auth::browser_login().await {
            Ok(_) => {
                tracing::info!("ChatGPT OAuth login completed via Web UI");
                // Rebuild ModelManager so it picks up the fresh token
                let config = manager.get_config_snapshot().await;
                let new_models = std::sync::Arc::new(
                    crate::provider::models::ModelManager::new(config.models.clone()),
                );
                *manager.models.write().await = new_models;
                // Clear all session engines so they use the new models
                manager.session_engines.lock().await.clear();
                // Restore proxy models that the rebuild above dropped.
                crate::server::rtc::proxy_room::reapply_proxy_models(&state_for_task).await;
                tracing::info!("Reloaded models after ChatGPT OAuth login");
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
