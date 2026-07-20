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

use axum::{extract::Json, http::StatusCode, response::IntoResponse};
use rand::RngExt;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const CODE_TTL: Duration = Duration::from_secs(120);
const MAX_ATTEMPTS: u32 = 5;

struct PendingPair {
    pair_id: String,
    code: String,
    device_name: String,
    created: Instant,
    attempts: u32,
}

/// One pairing attempt at a time — a second request replaces the first.
static PENDING: Mutex<Option<PendingPair>> = Mutex::new(None);

#[derive(Serialize, Deserialize, Clone)]
pub struct PairedDevice {
    pub id: String,
    pub name: String,
    pub secret: String,
    pub created_at: i64,
}

fn devices_path() -> PathBuf {
    crate::paths::linggen_home().join("paired-devices.json")
}

pub fn load_devices() -> Vec<PairedDevice> {
    let Ok(text) = std::fs::read_to_string(devices_path()) else {
        return Vec::new();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

fn save_devices(devices: &[PairedDevice]) -> std::io::Result<()> {
    std::fs::write(devices_path(), serde_json::to_string_pretty(devices)?)
}

/// The LAN gate's check: does any paired device own this token?
pub fn is_valid_device_token(token: &str) -> bool {
    !token.is_empty() && load_devices().iter().any(|d| d.secret == token)
}

fn random_hex(bytes: usize) -> String {
    let mut rng = rand::rng();
    (0..bytes).map(|_| format!("{:02x}", rng.random::<u8>())).collect()
}

/// Show the code on THIS Mac — the screen-confirm half of the handshake.
/// Best-effort: the dialog needs macOS; the daemon log always carries it.
fn show_code(code: &str, device_name: &str) {
    tracing::info!("[pair] code {code} for device '{device_name}' (valid {}s)", CODE_TTL.as_secs());
    #[cfg(target_os = "macos")]
    {
        let text = format!(
            "\"{device_name}\" wants to pair.\n\nCode: {code}\n\nEnter it on the device to allow access.",
        );
        let script = format!(
            "display dialog \"{}\" with title \"Linggen\" buttons {{\"OK\"}} default button 1 giving up after {}",
            text.replace('"', "'"),
            CODE_TTL.as_secs(),
        );
        let _ = std::process::Command::new("osascript").arg("-e").arg(script).spawn();
    }
}

#[derive(Deserialize)]
pub(crate) struct PairRequest {
    device_name: String,
}

/// POST /api/pair/request — start a pairing attempt; the code appears on the
/// Mac's screen, never in this response.
pub(crate) async fn post_pair_request(Json(req): Json<PairRequest>) -> impl IntoResponse {
    let code = format!("{:06}", rand::rng().random_range(0..1_000_000u32));
    let pair_id = uuid::Uuid::new_v4().to_string();
    let name = req.device_name.chars().take(64).collect::<String>();
    show_code(&code, &name);
    *PENDING.lock().unwrap() = Some(PendingPair {
        pair_id: pair_id.clone(),
        code,
        device_name: name,
        created: Instant::now(),
        attempts: 0,
    });
    Json(serde_json::json!({ "pair_id": pair_id, "expires_in": CODE_TTL.as_secs() }))
}

#[derive(Deserialize)]
pub(crate) struct PairConfirm {
    pair_id: String,
    code: String,
}

/// POST /api/pair/confirm — trade the on-screen code for a device token.
pub(crate) async fn post_pair_confirm(Json(req): Json<PairConfirm>) -> impl IntoResponse {
    let mut pending = PENDING.lock().unwrap();
    let Some(p) = pending.as_mut() else {
        return err(StatusCode::NOT_FOUND, "no pairing in progress");
    };
    if p.pair_id != req.pair_id || p.created.elapsed() > CODE_TTL {
        *pending = None;
        return err(StatusCode::GONE, "pairing expired — start again");
    }
    p.attempts += 1;
    if p.attempts > MAX_ATTEMPTS {
        *pending = None;
        return err(StatusCode::TOO_MANY_REQUESTS, "too many tries — start again");
    }
    if p.code != req.code.trim() {
        return err(StatusCode::UNAUTHORIZED, "wrong code");
    }
    let device = PairedDevice {
        id: uuid::Uuid::new_v4().to_string(),
        name: p.device_name.clone(),
        secret: random_hex(24),
        created_at: chrono::Utc::now().timestamp(),
    };
    *pending = None;
    drop(pending);
    let mut devices = load_devices();
    devices.push(device.clone());
    if let Err(e) = save_devices(&devices) {
        return err(StatusCode::INTERNAL_SERVER_ERROR, format!("persist: {e}"));
    }
    tracing::info!("[pair] device '{}' paired ({})", device.name, device.id);
    Json(serde_json::json!({ "device_token": device.secret, "device_id": device.id }))
        .into_response()
}

fn err(code: StatusCode, msg: impl Into<String>) -> axum::response::Response {
    (code, Json(serde_json::json!({ "error": msg.into() }))).into_response()
}
