//! Add an Ollama model that is not downloaded yet (`doc/models.md`): the engine asks the
//! Ollama at the row's address to pull it (`POST {address}/api/pull`, streamed), keeps the
//! progress for the page to read (`GET /api/providers/{id}/pull`, polled while it runs), and
//! when Ollama is done files the model like a typed one (`providers::add_model`). The page
//! never talks to Ollama itself. A failure adds nothing and is read once as one fixed word.

use std::collections::HashMap;
use std::sync::Arc;

use axum::{body::Bytes, extract::{Path, State}, http::HeaderMap, response::{IntoResponse, Response}, Json};
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{json, Value};

use super::providers::{endpoint, owner_refusal, parse, Failure, BAD};
use crate::credentials::{self, Credentials};
use crate::server::ServerState;

/// One download as the page reads it.
pub(crate) struct Pull {
    model: String,
    phase: PullState,
    /// Stops the download (cancel); the stream is dropped with the task.
    abort: Option<tokio::task::AbortHandle>,
}

#[derive(Clone)]
enum PullState {
    Pulling { completed: u64, total: u64 },
    /// Read once, then forgotten: `not_running` (nothing answered) or `failed` (Ollama refused the name or the pull).
    Failed(&'static str),
    Done,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Ask {
    model: String,
}

const CONFLICT: Failure = (axum::http::StatusCode::CONFLICT, "A download is already running.");

/// `POST /api/providers/{id}/pull {model}`: start downloading. Answers at once.
pub(crate) async fn start(State(state): State<Arc<ServerState>>, headers: HeaderMap, Path(id): Path<String>, body: Bytes) -> Response {
    if let Some(refused) = owner_refusal(&headers) { return refused; }
    match begin(&state, &id, &body).await {
        Ok(()) => Json(json!({"state": "pulling"})).into_response(),
        Err(failure) => failure.into_response(),
    }
}

async fn begin(state: &Arc<ServerState>, id: &str, body: &[u8]) -> Result<(), Failure> {
    let model = parse::<Ask>(body)?.model.trim().to_string();
    if model.is_empty() || model.contains(char::is_whitespace) { return Err(BAD); }
    let config = state.manager.get_config_snapshot().await;
    let creds = Credentials::load_with(&credentials::credentials_file(), &config.models);
    let (name, url) = endpoint(state, &config, &creds, id).await?;
    if name != "ollama" { return Err(BAD); }
    let mut pulls = state.pulls.lock().await;
    if matches!(pulls.get(id), Some(Pull { phase: PullState::Pulling { .. }, .. })) { return Err(CONFLICT); }
    let task = tokio::spawn(run(state.clone(), id.to_string(), model.clone(), url));
    pulls.insert(id.to_string(), Pull { model, phase: PullState::Pulling { completed: 0, total: 0 }, abort: Some(task.abort_handle()) });
    Ok(())
}

/// The download itself: read Ollama's lines, keep the progress, file the model when it is whole.
async fn run(state: Arc<ServerState>, id: String, model: String, base: String) {
    let outcome = download(&state, &id, &model, &base).await;
    let next = match outcome {
        Ok(()) => match super::providers::add_model(&state, &id, &model).await {
            Ok(()) => PullState::Done,
            Err(_) => PullState::Failed("failed"),
        },
        Err(why) => PullState::Failed(why),
    };
    if let Some(pull) = state.pulls.lock().await.get_mut(&id) { pull.phase = next; pull.abort = None; }
}

async fn download(state: &Arc<ServerState>, id: &str, model: &str, base: &str) -> Result<(), &'static str> {
    // No overall timeout: a model is gigabytes. No redirect, as for every provider call.
    let client = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build().map_err(|_| "failed")?;
    let url = format!("{}/api/pull", base.trim_end_matches('/'));
    let response = client.post(url).json(&json!({"model": model, "stream": true})).send().await.map_err(|_| "not_running")?;
    if !response.status().is_success() { return Err("failed"); }
    let mut layers: HashMap<String, (u64, u64)> = HashMap::new();
    let mut stream = response.bytes_stream();
    let mut pending = Vec::new();
    let mut finished = false;
    while let Some(chunk) = stream.next().await {
        pending.extend_from_slice(&chunk.map_err(|_| "failed")?);
        while let Some(end) = pending.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = pending.drain(..=end).collect();
            let Ok(line) = serde_json::from_slice::<Value>(&line) else { continue };
            if line.get("error").is_some() { return Err("failed"); }
            if line["status"] == "success" { finished = true; }
            if let (Some(digest), Some(total)) = (line["digest"].as_str(), line["total"].as_u64()) {
                layers.insert(digest.to_string(), (line["completed"].as_u64().unwrap_or(0), total));
            }
            let (completed, total) = layers.values().fold((0, 0), |(c, t), (lc, lt)| (c + lc, t + lt));
            if let Some(pull) = state.pulls.lock().await.get_mut(id) { pull.phase = PullState::Pulling { completed, total }; }
        }
    }
    finished.then_some(()).ok_or("failed")
}

/// `GET /api/providers/{id}/pull`: `{state: "none"}`, `{state: "pulling", model, completed, total}`,
/// `{state: "done", model}` or `{state: "failed", model, why}`. Done and failed are read once.
pub(crate) async fn status(State(state): State<Arc<ServerState>>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    if let Some(refused) = owner_refusal(&headers) { return refused; }
    let mut pulls = state.pulls.lock().await;
    let Some(pull) = pulls.get(&id) else { return Json(json!({"state": "none"})).into_response() };
    let model = pull.model.clone();
    let answer = match &pull.phase {
        PullState::Pulling { completed, total } => json!({"state": "pulling", "model": model, "completed": completed, "total": total}),
        PullState::Failed(why) => json!({"state": "failed", "model": model, "why": why}),
        PullState::Done => json!({"state": "done", "model": model}),
    };
    if !matches!(pull.phase, PullState::Pulling { .. }) { pulls.remove(&id); }
    Json(answer).into_response()
}

/// `DELETE /api/providers/{id}/pull`: stop a running download.
pub(crate) async fn cancel(State(state): State<Arc<ServerState>>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    if let Some(refused) = owner_refusal(&headers) { return refused; }
    if let Some(pull) = state.pulls.lock().await.remove(&id) {
        if let Some(handle) = pull.abort { handle.abort(); }
    }
    Json(json!({"state": "none"})).into_response()
}
