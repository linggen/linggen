//! /api/account endpoints — billing sign-in + entitlement for the gate UIs
//! (app shells, web UI, CLI). The daemon owns the token; callers never see
//! it. See linggensite/doc/entitlement-spec.md.

use crate::account;
use crate::server::ServerState;
use axum::{
    extract::{Json, Query, State},
    http::StatusCode,
    response::{Html, IntoResponse},
};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

struct PendingLogin {
    csrf: String,
    created: Instant,
}

/// One sign-in attempt at a time; the CSRF state is single-use.
static PENDING_LOGIN: Mutex<Option<PendingLogin>> = Mutex::new(None);
const LOGIN_WINDOW: Duration = Duration::from_secs(300);

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// GET /api/account?app=<app> — signed-in state, entitlement + trial maps,
/// and (with `app`) a resolved gate verdict so callers stay dumb.
pub(crate) async fn get_account(
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let Some((token, source)) = account::resolve_token() else {
        return Json(serde_json::json!({ "signed_in": false }));
    };
    let acc = account::load_account();
    let (entitlement, offline) = match account::entitlement_cached(&token).await {
        Some((v, off)) => (Some(v), off),
        None => (None, true),
    };
    let mut body = serde_json::json!({
        "signed_in": true,
        "source": source,
        "user_name": acc.as_ref().and_then(|a| a.user_name.clone())
            .or_else(account::resolved_user_name),
        "avatar_url": acc.as_ref().and_then(|a| a.avatar_url.clone()),
        "offline": offline,
        "entitlement": entitlement,
    });
    if let Some(app) = params.get("app") {
        body["gate"] = gate_for(&body["entitlement"], app);
    }
    Json(body)
}

/// Gate verdict for one app: `allowed = entitled || trial.active`.
fn gate_for(ent: &serde_json::Value, app: &str) -> serde_json::Value {
    if ent.is_null() {
        return serde_json::json!({ "app": app, "entitled": false, "trial": null, "allowed": false });
    }
    let entitled = account::app_entitled(ent, app, unix_now());
    let trial = ent
        .get("trial")
        .and_then(|t| t.get(app))
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let trial_active = trial.get("active").and_then(|a| a.as_bool()).unwrap_or(false);
    serde_json::json!({
        "app": app,
        "entitled": entitled,
        "trial": trial,
        "allowed": entitled || trial_active,
    })
}

/// POST /api/account/login — open the system browser to linggen.dev sign-in;
/// the token returns to GET /api/account/callback on this daemon. Callers
/// poll GET /api/account until signed_in flips.
pub(crate) async fn post_account_login(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    let csrf = uuid::Uuid::new_v4().to_string();
    *PENDING_LOGIN.lock().unwrap() = Some(PendingLogin {
        csrf: csrf.clone(),
        created: Instant::now(),
    });
    let callback = format!("http://127.0.0.1:{}/api/account/callback", state.port);
    let url = format!(
        "{}/auth/link?callback={}&state={}",
        account::site_url(),
        urlencoding::encode(&callback),
        urlencoding::encode(&csrf),
    );
    let opened = open::that(&url).is_ok();
    Json(serde_json::json!({ "ok": true, "opened": opened, "url": url }))
}

/// GET /api/account/callback — browser redirect target. Verifies the
/// single-use CSRF state, then stores the token in ~/.linggen/account.toml.
pub(crate) async fn get_account_callback(
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let state_ok = {
        let mut guard = PENDING_LOGIN.lock().unwrap();
        match guard.take() {
            Some(p) => {
                p.created.elapsed() < LOGIN_WINDOW
                    && params.get("state").map(|s| s.as_str()) == Some(p.csrf.as_str())
            }
            None => false,
        }
    };
    if !state_ok {
        return fail_page("Security check failed (state mismatch or expired). Please try again.");
    }
    let token = match params.get("token") {
        Some(t) if t.starts_with("usr_") => t.clone(),
        _ => return fail_page("No valid token received."),
    };
    // Best-effort profile fetch; sign-in succeeds even if it fails.
    let me = account::fetch_me(&token).await.unwrap_or_default();
    if let Err(e) = account::save_account(&account::config_from_me(token, &me)) {
        return fail_page(&format!("Could not save account: {e}"));
    }
    Html(
        "<html><body><h2>Signed in!</h2><p>You can close this tab and return to the app.</p>\
         <script>window.close()</script></body></html>"
            .to_string(),
    )
}

fn fail_page(reason: &str) -> Html<String> {
    Html(format!(
        "<html><body><h2>Sign-in failed</h2><p>{reason}</p></body></html>"
    ))
}

/// POST /api/account/logout — remove account.toml. A remote.toml link (if
/// any) is left untouched and still provides a billing token.
pub(crate) async fn post_account_logout() -> impl IntoResponse {
    let removed = account::delete_account().unwrap_or(false);
    let remote_link_active = crate::cli::login::load_remote_config().is_some();
    Json(serde_json::json!({ "ok": true, "removed": removed, "remote_link_active": remote_link_active }))
}

#[derive(serde::Deserialize)]
pub(crate) struct CheckoutReq {
    app: String,
}

/// POST /api/account/checkout {app} — returns the Stripe Checkout URL for
/// the caller to open in the system browser.
pub(crate) async fn post_account_checkout(Json(req): Json<CheckoutReq>) -> impl IntoResponse {
    let Some((token, _)) = account::resolve_token() else {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": "Not signed in" })),
        )
            .into_response();
    };
    match account::create_checkout(&token, &req.app).await {
        Ok(url) => Json(serde_json::json!({ "url": url })).into_response(),
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}
