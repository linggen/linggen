//! Device pairing — the trust layer for a non-loopback daemon.
//!
//! A phone on the same Wi-Fi is not a trusted caller: before `[server] host`
//! opens beyond loopback, every LAN request must present a device token
//! minted here. The handshake is screen-confirm (AirPlay-style): the phone
//! asks to pair, this Mac shows a 6-digit code, the user types it on the
//! phone — proving they can see this Mac's screen, which is exactly what a
//! stranger on the network cannot. Tokens are per-device, revocable by
//! deleting their row in `~/.linggen/paired-devices.json`, and IP-agnostic
//! (DHCP churn doesn't unpair).

pub use crate::state_fs::devices::{
    actor_for_token, device_name, is_valid_device_token, load_devices, person_label,
    set_device_account, AccountRef, Actor, PairedDevice,
};
use crate::state_fs::devices::{commit_device, device_by_token, modify_devices, random_hex};
use crate::util::LockExt;
use axum::{
    extract::{Json, Path, State},
    http::StatusCode,
    response::{Html, IntoResponse},
};
use rand::RngExt;
use serde::Deserialize;
use std::sync::Mutex;
use std::time::{Duration, Instant};

mod handshake;
mod identity;
mod manage;
mod relay_window;

pub(crate) use handshake::*;
pub use identity::*;
pub(crate) use manage::*;
pub use relay_window::*;

fn err(code: StatusCode, msg: impl Into<String>) -> axum::response::Response {
    (code, Json(serde_json::json!({ "error": msg.into() }))).into_response()
}
