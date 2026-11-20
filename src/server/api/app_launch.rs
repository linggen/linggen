//! `POST /api/apps/launch` — bring a skill's web app up on this Mac's
//! surfaces: the same `AppLaunched` the `RunApp` tool sends, for a skill
//! script that has no model turn to call `RunApp` from (a play asked for from
//! the phone, an errand that needs its page in front).
//!
//! What a surface does with it is the surface's own (`ui` eventHandlers): the
//! desktop app's window opens the app's tab and comes up. Nothing here
//! opens a browser — a launch nobody hears is just unheard, and the caller
//! learns whether its page came up from the page itself.

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use serde::Deserialize;
use std::sync::Arc;

use crate::server::{ServerEvent, ServerState};

#[derive(Deserialize)]
pub(crate) struct LaunchBody {
    skill: String,
    /// One of the page's views to land on (a tapped offer's `open`), passed
    /// to the page as `?view=`. What a view means is the page's own.
    #[serde(default)]
    view: Option<String>,
}

pub(crate) async fn launch(
    State(state): State<Arc<ServerState>>,
    Json(body): Json<LaunchBody>,
) -> impl IntoResponse {
    let refuse =
        |code: StatusCode, error: String| (code, Json(serde_json::json!({ "error": error })));
    let Some(skill) = state.manager.skills.get_skill(&body.skill).await else {
        return refuse(StatusCode::NOT_FOUND, format!("no skill '{}'", body.skill));
    };
    let Some(app) = skill.app.as_ref().filter(|a| a.launcher == "web") else {
        return refuse(
            StatusCode::BAD_REQUEST,
            format!("'{}' has no web app", skill.name),
        );
    };
    let url = match body.view.as_deref().map(str::trim).filter(|v| is_view(v)) {
        Some(v) => format!("/apps/{}/{}?view={v}", skill.name, app.entry),
        None => format!("/apps/{}/{}", skill.name, app.entry),
    };
    tracing::info!("[apps] launch {} over HTTP", skill.name);
    let _ = state.events_tx.send(ServerEvent::AppLaunched {
        skill: skill.name.clone(),
        launcher: "web".to_string(),
        url: url.clone(),
        title: skill.description.clone(),
        width: app.width,
        height: app.height,
        session_id: None,
    });
    (
        StatusCode::OK,
        Json(serde_json::json!({ "ok": true, "url": url })),
    )
}

/// A view name fit for a URL: a short slug.
fn is_view(v: &str) -> bool {
    (1..=32).contains(&v.len()) && v.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_view_is_a_short_slug() {
        assert!(super::is_view("files"));
        assert!(!super::is_view("a&b=c"));
        assert!(!super::is_view(""));
    }
}
