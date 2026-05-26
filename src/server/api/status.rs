//! Status + models listing.

use crate::server::ServerState;
use axum::{
    extract::{Json, Query, State},
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

pub(crate) async fn list_models_api(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    let models_guard = state.manager.models.read().await;
    let models: Vec<_> = models_guard.list_models().into_iter().cloned().collect();
    drop(models_guard);
    Json(models).into_response()
}

#[derive(Deserialize)]
pub(crate) struct StatusQuery {
    project_root: String,
    session_id: Option<String>,
}

#[derive(Serialize)]
pub(crate) struct StatusResponse {
    pub version: String,
    pub sessions: usize,
    pub total_runs: usize,
    pub completed_runs: usize,
    pub failed_runs: usize,
    pub cancelled_runs: usize,
    pub active_days: usize,
    pub first_run_at: Option<u64>,
    pub last_run_at: Option<u64>,
    pub model_usage: Vec<(String, usize)>,
    pub default_model: Option<String>,
    pub models: Vec<StatusModelInfo>,
    /// Accumulated prompt tokens this session (in-memory).
    pub session_prompt_tokens: usize,
    /// Accumulated completion tokens this session (in-memory).
    pub session_completion_tokens: usize,
}

#[derive(Serialize)]
pub(crate) struct StatusModelInfo {
    pub id: String,
    pub provider: String,
    pub model: String,
}

pub(crate) async fn get_status_api(
    State(state): State<Arc<ServerState>>,
    Query(query): Query<StatusQuery>,
) -> impl IntoResponse {
    let root = PathBuf::from(&query.project_root);

    let runs = state
        .manager
        .list_agent_runs(&root, None)
        .await
        .unwrap_or_default();

    let total_runs = runs.len();
    let mut completed = 0usize;
    let mut failed = 0usize;
    let mut cancelled = 0usize;
    let mut day_set = std::collections::HashSet::new();
    let mut first_run_at: Option<u64> = None;
    let mut last_run_at: Option<u64> = None;
    let mut model_count: HashMap<String, usize> = HashMap::new();

    for r in &runs {
        match r.status {
            crate::engine::agent::AgentRunStatus::Completed => completed += 1,
            crate::engine::agent::AgentRunStatus::Failed => failed += 1,
            crate::engine::agent::AgentRunStatus::Cancelled => cancelled += 1,
            _ => {}
        }
        let secs = r.started_at;
        let day = secs / 86400;
        day_set.insert(day);
        if first_run_at.is_none() || secs < first_run_at.unwrap() {
            first_run_at = Some(secs);
        }
        if last_run_at.is_none() || secs > last_run_at.unwrap() {
            last_run_at = Some(secs);
        }
        if let Some(kind) = &r.agent_kind {
            *model_count.entry(kind.clone()).or_default() += 1;
        }
    }

    let mut model_usage: Vec<(String, usize)> = model_count.into_iter().collect();
    model_usage.sort_by(|a, b| b.1.cmp(&a.1));

    let sessions = state.manager.global_sessions.count_sessions();

    let config = state.manager.get_config_snapshot().await;
    let default_model = config.routing.default_models.first().cloned();

    let models_guard = state.manager.models.read().await;
    let models: Vec<StatusModelInfo> = models_guard
        .list_models()
        .iter()
        .map(|m| StatusModelInfo {
            id: m.id.clone(),
            provider: m.provider.clone(),
            model: m.model.clone(),
        })
        .collect();
    drop(models_guard);

    let (session_prompt_tokens, session_completion_tokens) = {
        let tokens = state.session_tokens.lock().await;
        if let Some(sid) = &query.session_id {
            tokens.get(sid).copied().unwrap_or((0, 0))
        } else {
            tokens.values().fold((0, 0), |acc, v| (acc.0 + v.0, acc.1 + v.1))
        }
    };

    Json(StatusResponse {
        version: env!("CARGO_PKG_VERSION").to_string(),
        sessions,
        total_runs,
        completed_runs: completed,
        failed_runs: failed,
        cancelled_runs: cancelled,
        active_days: day_set.len(),
        first_run_at,
        last_run_at,
        model_usage,
        default_model,
        models,
        session_prompt_tokens,
        session_completion_tokens,
    })
    .into_response()
}
