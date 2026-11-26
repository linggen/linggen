//! Owner-only family settings: tick a family, follow it, go back.
//!
//! `PUT /api/providers/{id}/families/{family}` carries
//! `{action, model?, move?}`; the answer is the fresh provider list. The rules
//! live in the engine (`provider::models::families`); this file takes the
//! settings write lock around them, asks the provider for its model list
//! outside that lock, and saves a change in one config write. The same lock
//! is taken by the background pass that moves following rows and by the
//! go-back of a refused move, so a legacy save cannot interleave with either.

use std::sync::Arc;

use axum::{body::Bytes, extract::{Path, State}, http::{HeaderMap, StatusCode}, response::{IntoResponse, Response}};
use serde::Deserialize;

use super::providers::{answer, endpoint, owner_refusal, parse, save_config, Failure, BAD, MISSING};
use crate::credentials::{self, Credentials};
use crate::provider::models::families::{Action, Refusal};
use crate::server::ServerState;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    action: String,
    model: Option<String>,
    #[serde(rename = "move")]
    moved: Option<u64>,
}

impl Request {
    /// What was asked, with its fixed words: anything else is a bad request.
    fn action(self) -> Result<Action, Failure> {
        let Request { action, model, moved } = self;
        match (action.as_str(), model, moved) {
            ("tick", None, None) => Ok(Action::Tick),
            ("untick", replacement, None) => Ok(Action::Untick { replacement }),
            ("follow", None, None) => Ok(Action::Follow),
            ("choose", Some(model), None) => Ok(Action::Choose { model }),
            ("ack", None, Some(moved)) => Ok(Action::Ack { moved }),
            ("back", None, Some(moved)) => Ok(Action::Back { moved }),
            _ => Err(BAD),
        }
    }
}

fn refused(why: Refusal) -> Failure {
    match why {
        Refusal::Unsupported => BAD,
        Refusal::Missing => MISSING,
        other => (StatusCode::CONFLICT, other.code()),
    }
}

pub(crate) async fn put(State(state): State<Arc<ServerState>>, headers: HeaderMap, Path((id, family)): Path<(String, String)>, body: Bytes) -> Response {
    if let Some(refused) = owner_refusal(&headers) { return refused; }
    match act(&state, &id, &family, body).await {
        Ok((config, creds)) => answer(&state, &config, &creds).await,
        Err(failure) => failure.into_response(),
    }
}

async fn act(state: &Arc<ServerState>, id: &str, family: &str, body: Bytes) -> Result<(crate::config::Config, Credentials), Failure> {
    let action = parse::<Request>(&body)?.action()?;
    if let Action::Untick { replacement: Some(replacement) } = &action {
        // The model that takes the first place must be one that can run.
        let models = state.manager.models.read().await;
        if !models.list_models().iter().any(|m| m.id == *replacement) { return Err(BAD); }
    }
    let path = credentials::credentials_file();
    // Asking the provider takes as long as it takes: outside the write lock.
    let config = state.manager.get_config_snapshot().await;
    let creds = Credentials::load_with(&path, &config.models);
    let (name, url) = endpoint(state, &config, &creds, id).await?;
    if action.needs_list() {
        state.manager.families.gather(&config, &creds, Some((&name, &url))).await;
    }

    let _write = state.settings_write.lock().await;
    let mut config = state.manager.get_config_snapshot().await;
    let creds = Credentials::load_with(&path, &config.models);
    let changed = state.manager.families.act(&mut config, &creds, (&name, &url), family, action).map_err(refused)?;
    if changed {
        save_config(state, &config).await?;
        state.provider_drafts.lock().await.remove(id);
    }
    Ok((config, creds))
}

/// How often following rows are looked at: the lists they move on are held
/// for hours, and the catalog changes weekly.
const FOLLOW_EVERY: std::time::Duration = std::time::Duration::from_secs(60 * 60);
const FOLLOW_FIRST: std::time::Duration = std::time::Duration::from_secs(60);

/// The two background jobs: move following rows when the catalog and the
/// providers' lists say so, and answer a refused first call on a moved row.
pub(crate) fn spawn(state: &Arc<ServerState>) {
    let followers = state.clone();
    tokio::spawn(async move {
        let config = followers.manager.get_config_snapshot().await;
        let creds = Credentials::load_with(&credentials::credentials_file(), &config.models);
        followers.manager.families.gather(&config, &creds, None).await;
        tokio::time::sleep(FOLLOW_FIRST).await;
        loop {
            follow_once(&followers).await;
            tokio::time::sleep(FOLLOW_EVERY).await;
        }
    });
    let refusals = state.clone();
    tokio::spawn(async move {
        loop {
            refusals.manager.families.refusal_queued().await;
            go_back_once(&refusals).await;
        }
    });
}

/// One pass: ask the providers outside the lock, then move under it.
pub(crate) async fn follow_once(state: &Arc<ServerState>) {
    let config = state.manager.get_config_snapshot().await;
    let creds = Credentials::load_with(&credentials::credentials_file(), &config.models);
    state.manager.families.gather(&config, &creds, None).await;
    let _write = state.settings_write.lock().await;
    let mut config = state.manager.get_config_snapshot().await;
    let creds = Credentials::load_with(&credentials::credentials_file(), &config.models);
    if state.manager.families.follow(&mut config, &creds) {
        if save_config(state, &config).await.is_err() {
            tracing::warn!("A family moved, but the move could not be saved");
        }
    }
}

pub(crate) async fn go_back_once(state: &Arc<ServerState>) {
    let _write = state.settings_write.lock().await;
    let mut config = state.manager.get_config_snapshot().await;
    if state.manager.families.go_back_refused(&mut config) {
        if save_config(state, &config).await.is_err() {
            tracing::warn!("A refused move was sent back, but that could not be saved");
        }
    }
}
