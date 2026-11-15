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
//! The same daemon also serves the Data Browser UI: `GET /` returns the
//! bundled `index.html` and `GET /assets/*` fans out to the rest of
//! `static/`. See `doc/ui-spec.md`.

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
mod ui;

use crate::telemetry::Telemetry;
use axum::extract::Request;
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::routing::get;
use axum::Router;
use state::SharedState;

/// Compose the full router for the daemon.
///
/// `state` carries the shared MemoryStore and Embedder — opened once at
/// daemon startup and reused across all requests. `telemetry` is the
/// anonymous-usage telemetry handle (no-op when opted out / feature
/// disabled); it's wrapped around the Memory subrouter so every Memory.*
/// call records a `command` event.
pub fn build_router(state: SharedState, telemetry: Telemetry) -> Router {
    Router::new()
        .route("/api/health", get(health::handler))
        .merge(
            memory::router()
                .merge(days::router())
                .merge(chains::router())
                .merge(issues::router())
                .merge(stats::router())
                .merge(session::router())
                .layer(middleware::from_fn_with_state(
                    telemetry,
                    command_telemetry_layer,
                )),
        )
        .merge(mcp::router())
        .merge(config::router())
        .merge(network::router())
        .merge(connect::router())
        .merge(sync::router())
        .merge(primary::router())
        .merge(lease::router())
        .merge(migration::router())
        .merge(ui::router())
        // On a replica, writes go to the primary (or the outbox) — inside the
        // gate, so only admitted callers reach it. See `replica::forward`.
        .layer(middleware::from_fn_with_state(
            state.clone(),
            crate::replica::forward::replica_writes,
        ))
        // Outermost, so it runs before any handler: a caller off this machine
        // must be a paired device. Loopback — the CLI, the engine, the local
        // app — goes straight through, and `/api/health` plus the two routes a
        // machine uses to ask for a token (`connect.rs`) stay open.
        // See `gate.rs`. With no paired devices, nothing off-machine gets in.
        .layer(middleware::from_fn_with_state(
            state.clone(),
            gate::lan_gate,
        ))
        // Outside even the gate: a refused request is still traffic, and
        // background maintenance must not rewrite tables while anything at
        // all is knocking.
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

/// Middleware: emit a `command` telemetry event for every `/api/memory/*`
/// request. The verb is parsed from the URI path (the segment after
/// `/api/memory/`); requests with unexpected paths are skipped.
async fn command_telemetry_layer(
    axum::extract::State(telemetry): axum::extract::State<Telemetry>,
    request: Request,
    next: Next,
) -> Response {
    // Sanitize to the digest key charset: the segment comes off the URL, and
    // an arbitrary path must never become a count key.
    let verb: Option<String> = request
        .uri()
        .path()
        .strip_prefix("/api/memory/")
        .map(|v| {
            v.chars()
                .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
                .take(24)
                .collect()
        })
        .filter(|v: &String| !v.is_empty());

    let response = next.run(request).await;

    if let Some(verb) = verb {
        telemetry.command(&format!("memory.{verb}"));
        // Digest counts split by outcome; the daily `command` row above keeps
        // the DAU contract unchanged.
        let status = response.status();
        if status.is_success() {
            telemetry.bump(&format!("memory.{verb}"));
        } else {
            let code = match status.as_u16() {
                401 | 403 => "auth_required",
                429 => "quota",
                s if s >= 500 => "server",
                _ => "request",
            };
            telemetry.error("memory", code);
        }
    }
    response
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
