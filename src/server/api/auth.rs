//! Auth + user-profile endpoints — proxies to linggen.dev.

use crate::server::ServerState;
use axum::{
    extract::{Json, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use std::collections::HashMap;
use std::sync::Arc;

/// GET /api/user/me — fetch the authenticated user's profile from linggen.dev.
/// Reads the API token from `~/.linggen/remote.toml` and proxies to the relay.
pub(crate) async fn get_user_me() -> impl IntoResponse {
    let config = match crate::cli::login::load_remote_config() {
        Some(c) => c,
        None => return (StatusCode::NOT_FOUND, "Not logged in").into_response(),
    };

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap_or_default();

    let resp = client
        .get(format!("{}/api/auth/me", config.relay_url))
        .header("Authorization", format!("Bearer {}", config.api_token))
        .send()
        .await;

    match resp {
        Ok(r) if r.status().is_success() => match r.json::<serde_json::Value>().await {
            Ok(body) => {
                // Update remote.toml with fresh user info so UserContext stays current
                let new_name = body
                    .get("display_name")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let new_avatar = body
                    .get("avatar_url")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                if new_name != config.user_name || new_avatar != config.avatar_url {
                    let mut updated = config;
                    updated.user_name = new_name;
                    updated.avatar_url = new_avatar;
                    if let Ok(toml_str) = toml::to_string_pretty(&updated) {
                        let path = crate::paths::linggen_home().join("remote.toml");
                        let _ = std::fs::write(&path, &toml_str);
                    }
                }
                Json(body).into_response()
            }
            Err(_) => (StatusCode::BAD_GATEWAY, "Invalid response").into_response(),
        },
        Ok(r) => (
            StatusCode::from_u16(r.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY),
            "Auth failed",
        )
            .into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, format!("Relay error: {}", e)).into_response(),
    }
}

/// GET /api/auth/login — redirect to linggen.dev OAuth with callback to this server.
pub(crate) async fn auth_login(
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let host = params.get("host").cloned().unwrap_or_else(|| {
        let port = params
            .get("port")
            .and_then(|p| p.parse::<u16>().ok())
            .unwrap_or(crate::config::DEFAULT_PORT);
        format!("localhost:{}", port)
    });
    let callback = format!("http://{}/api/auth/callback", host);
    let state = uuid::Uuid::new_v4().to_string();
    let prompt = params.get("prompt").cloned().unwrap_or_default();
    let url = format!(
        "https://linggen.dev/auth/link?callback={}&state={}&prompt={}",
        urlencoding::encode(&callback),
        urlencoding::encode(&state),
        urlencoding::encode(&prompt),
    );
    axum::response::Redirect::temporary(&url).into_response()
}

/// GET /api/auth/callback — receives token from linggen.dev OAuth redirect.
pub(crate) async fn auth_callback(
    State(state): State<Arc<ServerState>>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let token = match params.get("token") {
        Some(t) if t.starts_with("usr_") => t.clone(),
        _ => {
            return axum::response::Html(
                "<html><body><h2>Authentication failed</h2><p>No valid token received.</p></body></html>"
                    .to_string(),
            )
            .into_response()
        }
    };

    let instance_id =
        crate::cli::login::get_or_create_instance_id().unwrap_or_else(|_| "unknown".into());
    let instance_name = gethostname::gethostname().to_string_lossy().to_string();

    // Register instance with linggen.dev
    let client = reqwest::Client::new();
    let user_id = match client
        .post("https://linggen.dev/api/instances")
        .bearer_auth(&token)
        .json(&serde_json::json!({
            "instance_id": instance_id,
            "name": instance_name,
        }))
        .send()
        .await
    {
        Ok(resp) => resp.json::<serde_json::Value>().await.ok().and_then(|v| {
            v.get("user_id")
                .and_then(|u| u.as_str())
                .map(|s| s.to_string())
        }),
        Err(_) => None,
    };

    let config = crate::cli::login::RemoteConfig {
        relay_url: "https://linggen.dev".to_string(),
        api_token: token,
        instance_name,
        instance_id,
        user_id,
        user_name: None,
        avatar_url: None,
    };
    let path = crate::paths::linggen_home().join("remote.toml");
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let toml_str = toml::to_string_pretty(&config).unwrap_or_default();
    let _ = std::fs::write(&path, &toml_str);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }

    // Restart relay to pick up the new config
    let _ = state
        .events_tx
        .send(crate::server::ServerEvent::StateUpdated);

    axum::response::Html(
        r#"<html><body><h2>Authenticated!</h2><p>You can close this tab.</p><script>window.opener&&window.opener.postMessage({type:'linggen-auth-done'},'*');window.close()</script></body></html>"#.to_string()
    ).into_response()
}

/// POST /api/auth/logout — remove remote.toml to log out.
pub(crate) async fn auth_logout() -> impl IntoResponse {
    let path = crate::paths::linggen_home().join("remote.toml");
    if path.exists() {
        let _ = std::fs::remove_file(&path);
        Json(serde_json::json!({ "ok": true })).into_response()
    } else {
        Json(serde_json::json!({ "ok": true, "message": "Not logged in" })).into_response()
    }
}
