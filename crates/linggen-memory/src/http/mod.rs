//! HTTP surface served by the daemon.
//!
//! Phase 3 ships the seven `Memory.*` methods at `/api/memory/<method>`
//! plus the lifecycle probe at `/api/health`. All endpoints return the
//! standard `{ok, data}` envelope (see `envelope.rs`).
//!
//! Per the spec (see `../../linggen/doc/memory-spec.md`), tool name maps
//! directly to endpoint path — `Memory.search` → `POST /api/memory/search`.
//! Linggen's dispatcher translates nothing; JSON args pass through.
//!
//! The Data Browser UI (`static/`) stayed in the linggen-memory repo when this
//! code moved into the engine's workspace; it is not served from here.

mod chains;
pub(crate) mod config;
pub(crate) mod connect;
pub(crate) mod days;
pub mod envelope;
pub mod gate;
mod health;
pub(crate) mod issues;
pub(crate) mod lease;
pub(crate) mod local;
mod mcp;
mod memory;
mod migration;
pub(crate) mod network;
pub mod presence;
mod primary;
pub(crate) mod session;
pub mod state;
mod stats;
pub(crate) mod sync;

use axum::extract::Request;
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::routing::get;
use axum::Router;
use state::SharedState;

/// The router for memory hosted inside another process (`crate::hosted`):
/// the daemon's routes without its LAN gate (the host decides who reaches
/// them), and without its telemetry layer. Replica write-forwarding and the
/// activity stamp that background upkeep waits on stay.
pub fn build_hosted_router(state: SharedState) -> Router {
    Router::new()
        .route("/api/health", get(health::handler))
        .merge(memory::router())
        .merge(days::router())
        .merge(chains::router())
        .merge(issues::router())
        .merge(stats::router())
        .merge(session::router())
        .merge(mcp::router())
        .merge(config::router())
        .merge(network::router())
        .merge(connect::router())
        .merge(sync::router())
        .merge(primary::router())
        .merge(lease::router())
        .merge(migration::router())
        .layer(middleware::from_fn_with_state(
            state.clone(),
            crate::replica::forward::replica_writes,
        ))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            mark_activity_layer,
        ))
        .with_state(state)
}

/// Middleware: stamp "the daemon is being used" on every request, so the
/// background maintenance loop can wait for a genuinely quiet window.
async fn mark_activity_layer(
    axum::extract::State(state): axum::extract::State<SharedState>,
    request: Request,
    next: Next,
) -> Response {
    state.mark_request();
    next.run(request).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::{Memory, MemoryType, Origin};
    use axum::extract::ConnectInfo;
    use std::net::SocketAddr;

    /// A write a replica forwards (or replays from its outbox) arrives here
    /// from the LAN with the replica's token, and meets the same user-voice
    /// guard as any local write: replacing a from=user row without
    /// `user_directed` is refused ON THE PRIMARY, and the row stays.
    #[tokio::test]
    async fn the_primary_enforces_the_user_voice_guard_on_forwarded_writes() {
        let dir = tempfile::tempdir().unwrap();
        let state = state::scratch_state(dir.path(), "0.0.0.0:9528".parse().unwrap()).await;
        std::fs::write(
            gate::devices_path(dir.path()),
            r#"[{"id":"r","name":"DS242","secret":"cafe1234","created_at":0}]"#,
        )
        .unwrap();
        let voice = Memory::new("Alex prefers tea", MemoryType::Preference, Origin::User);
        let id = voice.id.clone();
        state.store.insert(&[voice]).await.unwrap();

        let app = memory::router()
            .layer(middleware::from_fn_with_state(
                state.clone(),
                gate::lan_gate,
            ))
            .with_state(state.clone());
        let body = serde_json::json!({ "content": "Alex prefers coffee", "replace_ids": [id] });
        let mut req = axum::http::Request::post("/api/memory/add")
            .header("content-type", "application/json")
            .header("x-linggen-device", "cafe1234")
            .body(axum::body::Body::from(body.to_string()))
            .unwrap();
        let replica: SocketAddr = "192.168.20.242:50000".parse().unwrap();
        req.extensions_mut().insert(ConnectInfo(replica));
        let res = tower::ServiceExt::oneshot(app, req).await.unwrap();

        assert!(res.status().is_client_error(), "{}", res.status());
        let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
            .await
            .unwrap();
        assert!(String::from_utf8_lossy(&bytes).contains("user-voice"));
        let kept = state.store.get(&id).await.unwrap().unwrap();
        assert!(kept.expired_at.is_none(), "the user's row is untouched");
    }
}
