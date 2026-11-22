//! Which private apps agents outside Linggen may use (`Config::outside_apps`),
//! as the owner sets it in Settings → Tools & MCP:
//!
//!   GET /api/mcp/outside-apps    the private apps that offer an action anywhere, each open or not
//!   PUT /api/mcp/outside-apps    `{"apps": [..]}` — the apps now open; set only at this Mac
//!
//! Admin class; a setting is made only by someone at this Mac
//! (`ling_mem_connect::is_at_this_mac`), never from a paired device or
//! another machine — as the household's (`people`). The list is the
//! engine's (`skill::outside::can_be_opened`), never the page's.

use axum::{
    extract::{ConnectInfo, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde_json::{json, Value};
use std::sync::Arc;

use crate::engine::skill::outside::can_be_opened;
use crate::server::ServerState;

fn err(code: StatusCode, msg: &str) -> Response {
    (code, Json(json!({ "error": msg }))).into_response()
}

/// GET /api/mcp/outside-apps — each app the owner may open to outside
/// agents, whether it is open, and whether the asker is at this Mac, where
/// alone it is set.
pub(crate) async fn list(
    State(state): State<Arc<ServerState>>,
    ConnectInfo(peer): ConnectInfo<std::net::SocketAddr>,
    headers: HeaderMap,
) -> Response {
    let opened = state.manager.get_config_snapshot().await.outside_apps;
    let apps: Vec<Value> = state
        .manager
        .skills
        .list_skills()
        .await
        .iter()
        .filter(|skill| can_be_opened(skill))
        .map(|skill| json!({ "name": skill.name, "description": skill.description, "open": opened.contains(&skill.name) }))
        .collect();
    let at_this_mac = crate::server::api::ling_mem_connect::is_at_this_mac(peer, &headers);
    Json(json!({ "apps": apps, "at_this_mac": at_this_mac })).into_response()
}

/// PUT /api/mcp/outside-apps — `{"apps": [names]}`: exactly these are open
/// now. A name that is not an app the owner may open changes nothing.
pub(crate) async fn set(
    State(state): State<Arc<ServerState>>,
    ConnectInfo(peer): ConnectInfo<std::net::SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if !crate::server::api::ling_mem_connect::is_at_this_mac(peer, &headers) {
        return err(StatusCode::FORBIDDEN, "only at this Mac");
    }
    let named: Option<Vec<String>> = body
        .get("apps")
        .and_then(|v| serde_json::from_value(v.clone()).ok());
    let Some(named) = named else {
        return err(StatusCode::BAD_REQUEST, "apps is a list of app names");
    };
    let installed = state.manager.skills.list_skills().await;
    let mut apps: Vec<String> = Vec::new();
    for name in named {
        let openable = installed
            .iter()
            .any(|skill| skill.name == name && can_be_opened(skill));
        if !openable {
            return err(
                StatusCode::BAD_REQUEST,
                &format!("'{name}' is not a private app that offers an action anywhere"),
            );
        }
        if !apps.contains(&name) {
            apps.push(name);
        }
    }
    if let Err(e) = state.manager.set_outside_apps(apps.clone()).await {
        return err(StatusCode::INTERNAL_SERVER_ERROR, &format!("persist: {e}"));
    }
    tracing::info!("[mcp] apps open to outside agents: {}", apps.len());
    Json(json!({ "status": "ok", "apps": apps })).into_response()
}
