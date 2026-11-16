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
use crate::ops::config::{load, save, Config};

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/api/config", get(get_config))
        .route("/api/config", put(put_config))
}

async fn get_config(State(state): State<SharedState>) -> Result<Response, ApiError> {
    let cfg = load(&state.data_dir).await;
    Ok(ok(serde_json::to_value(cfg).expect("Config serializes")))
}

async fn put_config(
    State(state): State<SharedState>,
    Json(mut cfg): Json<Config>,
) -> Result<Response, ApiError> {
    // The network choice is not this route's to change — see `network.rs`.
    cfg.network = load(&state.data_dir).await.network;
    // Light range check — the dream pass treats a 0 TTL as "evict
    // everything," which is rarely what the user wants. Allow it but
    // pin the lower bound; cap is generous to keep this future-proof.
    if cfg.episodic_ttl_days > 3650 {
        return Err(ApiError::bad_request(
            "episodic_ttl_days must be <= 3650 (10 years)",
        ));
    }
    if !(0.0..=1.0).contains(&cfg.recall_min_score) {
        return Err(ApiError::bad_request(
            "recall_min_score must be between 0.0 and 1.0",
        ));
    }
    save(&state.data_dir, &cfg)
        .await
        .map_err(|e| ApiError::internal(anyhow::anyhow!(e)))?;
    Ok(ok(serde_json::to_value(cfg).expect("Config serializes")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::network::Network;

    async fn scratch_state(dir: &std::path::Path) -> SharedState {
        crate::http::state::scratch_state(dir, std::net::SocketAddr::from(([0, 0, 0, 0], 0))).await
    }

    /// `PUT /api/config` is open to paired devices; the network choice is
    /// not, so this route keeps whatever is on disk.
    #[tokio::test]
    async fn the_config_route_never_changes_the_network() {
        let dir = tempfile::tempdir().unwrap();
        let state = scratch_state(dir.path()).await;
        let cfg = Config {
            network: Network::Local,
            ..Config::default()
        };
        save(dir.path(), &cfg).await.unwrap();

        let sent = Config {
            network: Network::Lan,
            episodic_ttl_days: 9,
            ..Config::default()
        };
        put_config(State(state), Json(sent)).await.unwrap();

        let stored = load(dir.path()).await;
        assert_eq!(stored.network, Network::Local);
        assert_eq!(stored.episodic_ttl_days, 9);
    }
}
