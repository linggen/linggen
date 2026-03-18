use crate::server::{ServerEvent, ServerState};
use crate::skills::marketplace::{self, SkillScope};
use crate::skills;
use axum::{
    extract::{Json, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Query / request types
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub(crate) struct SearchQuery {
    q: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct InstallRequest {
    name: String,
    repo_url: Option<String>,
    git_ref: Option<String>,
    scope: Option<SkillScope>,
    project_root: Option<String>,
    #[serde(default)]
    force: bool,
    source: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct UninstallRequest {
    name: String,
    scope: Option<SkillScope>,
    project_root: Option<String>,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

pub(crate) async fn community_search(
    Query(query): Query<SearchQuery>,
) -> impl IntoResponse {
    let q = query.q.unwrap_or_default();
    if q.is_empty() {
        return (StatusCode::BAD_REQUEST, "Missing query parameter 'q'").into_response();
    }

    match marketplace::search_community(&q).await {
        Ok(skills) => axum::Json(skills).into_response(),
        Err(e) => {
            tracing::error!(err = %e, "Community skills search failed");
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
        }
    }
}

pub(crate) async fn marketplace_install(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<InstallRequest>,
) -> impl IntoResponse {
    let scope = req.scope.unwrap_or_default();
    let project_root_path = req.project_root.as_deref().map(Path::new);

    let target_dir = match marketplace::skill_target_dir(&req.name, scope, project_root_path) {
        Ok(d) => d,
        Err(e) => return (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    };

    match marketplace::install_skill(
        &req.name,
        req.repo_url.as_deref(),
        req.git_ref.as_deref(),
        &target_dir,
        req.force,
        req.source.as_deref(),
    )
    .await
    {
        Ok(msg) => {
            let _ = state.skill_manager.load_all(project_root_path).await;
            let _ = state.events_tx.send(ServerEvent::StateUpdated);
            axum::Json(serde_json::json!({ "ok": true, "message": msg })).into_response()
        }
        Err(e) => {
            tracing::error!(err = %e, skill = %req.name, "Marketplace install failed");
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
        }
    }
}

pub(crate) async fn marketplace_uninstall(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<UninstallRequest>,
) -> impl IntoResponse {
    let project_root_path = req.project_root.as_deref().map(Path::new);

    // Look up the skill's actual source to resolve the correct directory.
    // This handles Compat (Codex/Claude) skills that aren't in ~/.linggen/skills/.
    let target_dir = if let Some(skill) = state.skill_manager.get_skill(&req.name).await {
        match marketplace::skill_dir_for_source(&req.name, &skill.source, project_root_path) {
            Ok(d) => d,
            Err(e) => return (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
        }
    } else {
        // Skill not loaded — fall back to scope-based resolution.
        let scope = req.scope.unwrap_or_default();
        match marketplace::skill_target_dir(&req.name, scope, project_root_path) {
            Ok(d) => d,
            Err(e) => return (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
        }
    };

    match marketplace::delete_skill(&req.name, &target_dir) {
        Ok(msg) => {
            let _ = state.skill_manager.load_all(project_root_path).await;
            let _ = state.events_tx.send(ServerEvent::StateUpdated);
            axum::Json(serde_json::json!({ "ok": true, "message": msg })).into_response()
        }
        Err(e) => {
            tracing::error!(err = %e, skill = %req.name, "Marketplace uninstall failed");
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
        }
    }
}

// ---------------------------------------------------------------------------
// Move to global
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub(crate) struct MoveToGlobalRequest {
    name: String,
    project_root: Option<String>,
}

pub(crate) async fn marketplace_move_to_global(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<MoveToGlobalRequest>,
) -> impl IntoResponse {
    let project_root_path = req.project_root.as_deref().map(Path::new);

    let skill = match state.skill_manager.get_skill(&req.name).await {
        Some(s) => s,
        None => return (StatusCode::NOT_FOUND, "Skill not found").into_response(),
    };

    match marketplace::move_skill_to_global(&req.name, &skill.source, project_root_path) {
        Ok(msg) => {
            let _ = state.skill_manager.load_all(project_root_path).await;
            let _ = state.events_tx.send(ServerEvent::StateUpdated);
            axum::Json(serde_json::json!({ "ok": true, "message": msg })).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

// ---------------------------------------------------------------------------
// Built-in skills
// ---------------------------------------------------------------------------

pub(crate) async fn builtin_skills_list(
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    if params.get("refresh").is_some_and(|v| v == "true" || v == "1") {
        skills::clear_builtin_cache().await;
    }
    axum::Json(skills::fetch_builtin_skills().await)
}

#[derive(Deserialize)]
pub(crate) struct BuiltInInstallRequest {
    name: String,
}

pub(crate) async fn builtin_skills_install(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<BuiltInInstallRequest>,
) -> impl IntoResponse {
    let target_dir = match marketplace::skill_target_dir(&req.name, SkillScope::Global, None) {
        Ok(d) => d,
        Err(e) => return (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    };

    // Install from linggen/skills repo
    match marketplace::install_skill(
        &req.name,
        Some("https://github.com/linggen/skills"),
        Some("main"),
        &target_dir,
        true, // force overwrite to get latest version
        None,
    )
    .await
    {
        Ok(msg) => {
            let _ = state.skill_manager.load_all(None).await;
            let _ = state.events_tx.send(ServerEvent::StateUpdated);
            axum::Json(serde_json::json!({ "ok": true, "message": msg })).into_response()
        }
        Err(e) => {
            tracing::error!(err = %e, skill = %req.name, "Built-in skill install failed");
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
        }
    }
}

pub(crate) async fn builtin_skills_install_all(
    State(state): State<Arc<ServerState>>,
) -> impl IntoResponse {
    let target = crate::paths::global_skills_dir();
    let zip_url = marketplace::build_github_zip_url("linggen", "skills", "main");
    let client = match marketplace::http_client() {
        Ok(c) => c,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };
    let temp_zip = match marketplace::download_to_temp(&client, &zip_url).await {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(err = %e, "Failed to download skills repo");
            return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
        }
    };

    let result = marketplace::extract_all_skills_from_zip(&temp_zip, &target);
    let _ = std::fs::remove_file(&temp_zip);

    match result {
        Ok(installed) => {
            let _ = state.skill_manager.load_all(None).await;
            let _ = state.events_tx.send(ServerEvent::StateUpdated);
            axum::Json(serde_json::json!({
                "ok": true,
                "installed": installed,
                "message": format!("Installed {} skills", installed.len()),
            }))
            .into_response()
        }
        Err(e) => {
            tracing::error!(err = %e, "Failed to extract skills from ZIP");
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
        }
    }
}

// ---------------------------------------------------------------------------
// ClawHub scan
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub(crate) struct ClawHubScanQuery {
    slug: Option<String>,
}

pub(crate) async fn clawhub_scan(
    Query(query): Query<ClawHubScanQuery>,
) -> impl IntoResponse {
    let slug = match query.slug {
        Some(s) if !s.is_empty() => s,
        _ => return (StatusCode::BAD_REQUEST, "Missing query parameter 'slug'").into_response(),
    };

    match marketplace::fetch_clawhub_scan(&slug).await {
        Ok(scan) => axum::Json(scan).into_response(),
        Err(e) => {
            tracing::error!(err = %e, slug = %slug, "ClawHub scan fetch failed");
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
        }
    }
}
