//! `GET`/`PUT /api/config` — the user-tunable memory config. The console
//! and the dream pass read it here; the console writes it. The file and its
//! schema are `ops::config`.

use axum::{
    extract::State,
    response::Response,
    routing::{get, put},
    Json, Router,
};

use crate::http::envelope::{ok, ApiError};
use crate::http::state::SharedState;
use crate::ops;
use crate::ops::config::Config;

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/api/config", get(get_config))
        .route("/api/config", put(put_config))
}

async fn get_config(State(state): State<SharedState>) -> Result<Response, ApiError> {
    Ok(ok(ops::config::get_config(&state).await?))
}

async fn put_config(
    State(state): State<SharedState>,
    Json(cfg): Json<Config>,
) -> Result<Response, ApiError> {
    Ok(ok(ops::config::put_config(&state, cfg).await?))
}

