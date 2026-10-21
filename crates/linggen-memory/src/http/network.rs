//! `/api/network` — how this daemon listens: shared on the LAN (the default)
//! or this machine only.
//!
//! The choice is the `network` field of `.config.json`, beside the other
//! knobs, and `serve` resolves it at bind time: an explicit `--host` wins, then
//! this setting, then the LAN. Every starter — the plugin hook, the engine, a
//! bare `ling-mem restart` — passes no `--host`, so the setting is what binds.
//!
//! Only this machine may change it. The gate already admits paired devices
//! from the LAN, and a device must never decide how the Mac listens — so both
//! writes here pass `local::require_local` themselves, and `PUT /api/config`
//! keeps whatever is on disk (see `config::put_config`).
//!
//! A new bind needs a new listener, so a change applies on restart:
//! `POST /api/network/restart` spawns `ling-mem restart`, which stops this
//! daemon and starts the next one on the saved choice.

use axum::{
    extract::{ConnectInfo, State},
    response::Response,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::http::config;
use crate::http::envelope::{ok, ApiError};
use crate::http::gate;
use crate::http::local;
use crate::http::state::SharedState;
use axum::http::HeaderMap;

/// Where the daemon listens. LAN by default: one store shared by this
/// machine and the others on the network, with the gate requiring a paired
/// device from anything off-machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Network {
    #[default]
    Lan,
    Local,
}

impl Network {
    pub fn host(self) -> IpAddr {
        match self {
            Network::Lan => IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            Network::Local => IpAddr::V4(Ipv4Addr::LOCALHOST),
        }
    }
}

/// The address `serve` binds: the flag someone typed, else the saved choice.
pub fn resolve_bind(flag: Option<IpAddr>, setting: Network) -> IpAddr {
    flag.unwrap_or_else(|| setting.host())
}

#[derive(Deserialize)]
struct SetNetwork {
    network: Network,
}

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/api/network", get(get_network).put(put_network))
        .route("/api/network/restart", post(restart))
}

/// The saved choice beside the address actually bound — facts the console
/// shows as-is. `applied` is false until a restart picks up a change.
fn view(saved: Network, bound: SocketAddr, data_dir: &Path) -> serde_json::Value {
    json!({
        "network": saved,
        "bound": bound.to_string(),
        "applied": bound.ip() == saved.host(),
        "paired_devices": gate::has_paired_devices_file(data_dir),
    })
}

async fn get_network(State(state): State<SharedState>) -> Result<Response, ApiError> {
    let saved = config::load(&state.data_dir).await.network;
    Ok(ok(view(saved, state.bound, &state.data_dir)))
}

async fn put_network(
    State(state): State<SharedState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<SetNetwork>,
) -> Result<Response, ApiError> {
    local::require_local(peer.ip(), &headers, state.port)?;
    let mut cfg = config::load(&state.data_dir).await;
    cfg.network = body.network;
    config::save(&state.data_dir, &cfg)
        .await
        .map_err(|e| ApiError::internal(anyhow::anyhow!(e)))?;
    Ok(ok(view(cfg.network, state.bound, &state.data_dir)))
}

async fn restart(
    State(state): State<SharedState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    local::require_local(peer.ip(), &headers, state.port)?;
    schedule_restart(state.data_dir.clone(), state.port);
    Ok(ok(json!({ "restarting": true })))
}

/// Restart from outside: a child `ling-mem restart` stops this daemon and
/// starts the next. Delayed a beat so the response reaches the console first.
fn schedule_restart(data_dir: PathBuf, port: u16) {
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        if let Err(e) = spawn_restart(&data_dir, port) {
            tracing::warn!("restart for a network change failed to spawn: {e}");
        }
    });
}

fn spawn_restart(data_dir: &Path, port: u16) -> std::io::Result<()> {
    let exe = std::env::current_exe()?;
    std::process::Command::new(exe)
        .arg("restart")
        .arg("--port")
        .arg(port.to_string())
        .env("LINGGEN_DATA_DIR", data_dir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lan_is_the_default() {
        assert_eq!(Network::default(), Network::Lan);
        assert_eq!(
            resolve_bind(None, Network::default()),
            "0.0.0.0".parse::<IpAddr>().unwrap()
        );
    }

    #[test]
    fn the_saved_choice_binds_unless_a_flag_overrides_it() {
        let local: IpAddr = "127.0.0.1".parse().unwrap();
        assert_eq!(resolve_bind(None, Network::Local), local);
        let flag: IpAddr = "192.168.1.5".parse().unwrap();
        assert_eq!(resolve_bind(Some(flag), Network::Local), flag);
        assert_eq!(resolve_bind(Some(local), Network::Lan), local);
    }

    #[test]
    fn the_setting_round_trips_as_a_word() {
        let v = serde_json::to_value(Network::Local).unwrap();
        assert_eq!(v, json!("local"));
        let n: Network = serde_json::from_value(json!("lan")).unwrap();
        assert_eq!(n, Network::Lan);
    }

    #[test]
    fn the_view_says_whether_the_saved_choice_is_live() {
        let dir = tempfile::tempdir().unwrap();
        let wide: SocketAddr = "0.0.0.0:9528".parse().unwrap();
        assert_eq!(view(Network::Lan, wide, dir.path())["applied"], true);
        assert_eq!(view(Network::Local, wide, dir.path())["applied"], false);
        assert_eq!(
            view(Network::Lan, wide, dir.path())["bound"],
            "0.0.0.0:9528"
        );
    }
}
