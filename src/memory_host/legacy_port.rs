//! The old daemon's address, `127.0.0.1:9528`, kept answering: plugin
//! configs and hooks of sessions already running still say 9528, so while
//! nothing else holds the port the engine serves memory's own router there,
//! the same paths the old daemon served (`/api/memory/*`, `/mcp`,
//! `/api/health`, …). Loopback only — never `0.0.0.0`. A taken port costs one
//! log line.
//!
//! This goes away one release after the plugins move to the engine's port.

use std::net::SocketAddr;

/// The old daemon's port.
pub(crate) const DEFAULT_PORT: u16 = 9528;

/// Another port for tests (never the real 9528 there); `0` turns it off.
const PORT_ENV: &str = "LINGGEN_LEGACY_MEMORY_PORT";

/// The port to listen on, `None` when turned off.
pub(super) fn port() -> Option<u16> {
    let Ok(raw) = std::env::var(PORT_ENV) else {
        return Some(DEFAULT_PORT);
    };
    match raw.trim().parse::<u16>() {
        Ok(0) => None,
        Ok(p) => Some(p),
        Err(_) => {
            tracing::warn!("memory: {PORT_ENV}={raw:?} is not a port; using {DEFAULT_PORT}");
            Some(DEFAULT_PORT)
        }
    }
}

/// Listen on `127.0.0.1:<port>` with `router`, on memory's runtime.
pub(super) fn serve(runtime: &tokio::runtime::Runtime, router: axum::Router, port: u16) {
    runtime.spawn(async move {
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let listener = match tokio::net::TcpListener::bind(addr).await {
            Ok(l) => l,
            Err(e) => {
                tracing::info!("memory: the old address {addr} is not served ({e})");
                return;
            }
        };
        tracing::info!("memory: also answering at the old address http://{addr}");
        let app = router.into_make_service_with_connect_info::<SocketAddr>();
        if let Err(e) = axum::serve(listener, app).await {
            tracing::warn!("memory: the old address {addr} stopped: {e}");
        }
    });
}
