//! The LAN gate — who may reach this store from off-machine.
//!
//! The daemon listens on the LAN by default (`0.0.0.0`; see `network.rs` for
//! the setting, and "this machine only" in the console). The case it exists
//! for: a second machine on the LAN whose agents should share ONE memory,
//! rather than growing a second binary, a second daemon, and a second store
//! that forks the user's biography.
//!
//! One rule makes the open port safe: **a non-loopback request must carry a
//! paired device's token.** Loopback stays open — the CLI, the engine, and the
//! local app all talk to `127.0.0.1`, and a process already on this machine is
//! inside the fence. A machine that has never paired anything has no tokens,
//! so everything off-machine is refused; the port is open, the store is not.
//!
//! The auth is **not new**. Both daemons live under the same `~/.linggen`, so
//! this reads the very file the engine's pairing flow writes — pair once, on
//! the Mac's screen, and both daemons accept the device. There is no second
//! enrolment, no second secret, and nothing here mints one: this file only ever
//! reads.

use super::envelope::ApiError;
use super::state::SharedState;
use axum::extract::{ConnectInfo, Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};

/// The header a paired device presents. Same name the engine's gate reads —
/// one device, one token, either daemon.
const DEVICE_HEADER: &str = "x-linggen-device";

/// Written by the engine's pairing flow (`server/api/pair.rs`). Only the
/// secret matters here; the rest of each row is the Mac's business.
#[derive(Deserialize)]
struct PairedDevice {
    #[serde(default)]
    secret: String,
}

pub fn devices_path(data_dir: &Path) -> PathBuf {
    data_dir.join("paired-devices.json")
}

/// Has this machine ever paired a device? A fact the console shows: with no
/// file, every request from off-machine is refused.
pub fn has_paired_devices_file(data_dir: &Path) -> bool {
    devices_path(data_dir).is_file()
}

/// Does any paired device own this token?
///
/// Read per request rather than cached: the file is tiny, and a device revoked
/// on the Mac must stop working now, not at the next daemon restart.
fn is_paired_token(data_dir: &Path, token: &str) -> bool {
    if token.is_empty() {
        return false;
    }
    let Ok(text) = std::fs::read_to_string(devices_path(data_dir)) else {
        return false;
    };
    let devices: Vec<PairedDevice> = serde_json::from_str(&text).unwrap_or_default();
    devices.iter().any(|d| d.secret == token)
}

/// The admission rule, apart from axum so it can be tested as-is: loopback, the
/// health probe and the two routes a machine uses to ask for a token
/// (`connect.rs`) always; anything else only with a paired device's token.
fn admits(peer: IpAddr, path: &str, token: &str, data_dir: &Path) -> bool {
    peer.is_loopback() || is_open_path(path) || is_paired_token(data_dir, token)
}

#[cfg(test)]
pub(crate) fn admits_for_test(peer: IpAddr, path: &str, token: &str, data_dir: &Path) -> bool {
    admits(peer, path, token, data_dir)
}

fn is_open_path(path: &str) -> bool {
    path == "/api/health" || super::connect::is_open_path(path)
}

/// Everything a caller off this machine must pass. Loopback goes straight
/// through — it is the CLI, the engine, and the local app, all of which are
/// already inside the fence.
///
/// `/api/health` is deliberately open: it is a liveness probe that returns a
/// version and nothing about the user, and a pairing UI needs to see the daemon
/// before it has a token to show it.
pub async fn lan_gate(
    State(state): State<SharedState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    req: Request,
    next: Next,
) -> Response {
    let token = req
        .headers()
        .get(DEVICE_HEADER)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    if admits(peer.ip(), req.uri().path(), token, &state.data_dir) {
        return next.run(req).await;
    }
    tracing::warn!(
        "refused {} from {} — no paired device token",
        req.uri().path(),
        peer.ip()
    );
    ApiError::unauthorized(format!(
        "this store is reachable from your machine but not open to it: pair with \
         Linggen and send the device token as `{DEVICE_HEADER}`"
    ))
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    /// The LAN is the default bind, so a fresh install — no paired devices,
    /// no file — must still refuse everything from off-machine.
    #[test]
    fn with_nothing_paired_only_this_machine_gets_in() {
        let dir = tempfile::tempdir().unwrap();
        let lan = ip("192.168.20.31");
        assert!(!admits(lan, "/api/memory/search", "", dir.path()));
        assert!(!admits(lan, "/mcp", "guess", dir.path()));
        assert!(admits(
            ip("127.0.0.1"),
            "/api/memory/search",
            "",
            dir.path()
        ));
        assert!(admits(ip("::1"), "/mcp", "", dir.path()));
        assert!(admits(lan, "/api/health", "", dir.path()));
        assert!(admits(lan, "/api/connect/request", "", dir.path()));
        assert!(admits(lan, "/api/connect/status/abc", "", dir.path()));
        assert!(!admits(lan, "/api/connect/pending", "", dir.path()));
        assert!(!admits(lan, "/api/connect/abc/allow", "", dir.path()));
    }

    /// An empty header from loopback — what the plugin sends when
    /// `LING_MEM_TOKEN` is unset — is still this machine.
    #[test]
    fn loopback_never_needs_a_token() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            devices_path(dir.path()),
            r#"[{"id":"a","name":"DS242","secret":"cafe1234","created_at":0}]"#,
        )
        .unwrap();
        assert!(admits(ip("127.0.0.1"), "/mcp", "", dir.path()));
        assert!(admits(ip("192.168.20.31"), "/mcp", "cafe1234", dir.path()));
        assert!(!admits(ip("192.168.20.31"), "/mcp", "beef0000", dir.path()));
        assert!(!admits(ip("192.168.20.31"), "/api/network", "", dir.path()));
    }

    /// The file the engine's pairing flow writes, read as-is. Nothing here
    /// mints a token; a row this daemon cannot parse grants nothing.
    #[test]
    fn a_token_is_paired_only_if_the_engines_file_says_so() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            devices_path(dir.path()),
            r#"[{"id":"a","name":"DS242","secret":"cafe1234","created_at":0}]"#,
        )
        .unwrap();
        assert!(is_paired_token(dir.path(), "cafe1234"));
        assert!(!is_paired_token(dir.path(), "beef0000"));
        assert!(!is_paired_token(dir.path(), ""));

        std::fs::write(devices_path(dir.path()), "not json").unwrap();
        assert!(!is_paired_token(dir.path(), "cafe1234"));
    }

    #[test]
    fn a_missing_file_grants_nothing() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!has_paired_devices_file(dir.path()));
        assert!(!is_paired_token(dir.path(), "anything"));
    }
}
