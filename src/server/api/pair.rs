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

use axum::{
    extract::{Json, Path, State},
    http::StatusCode,
    response::{Html, IntoResponse},
};
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
    device_id: Option<String>,
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
    /// Stable per-phone install id. Re-pairing the same phone replaces its row
    /// (matched on this) rather than stacking duplicates. Optional so rows
    /// written before this field, and older apps that don't send one, still load.
    #[serde(default)]
    pub device_id: Option<String>,
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

/// Mint a token for a freshly-confirmed device and persist it. A phone that
/// sends a stable `device_id` replaces its own prior row (re-pairing refreshes
/// the token/name in place instead of stacking duplicates); without one — older
/// apps — it appends, as before.
fn commit_device(name: String, device_id: Option<String>) -> std::io::Result<PairedDevice> {
    let device = PairedDevice {
        id: uuid::Uuid::new_v4().to_string(),
        name,
        secret: random_hex(24),
        created_at: chrono::Utc::now().timestamp(),
        device_id: device_id.clone(),
    };
    let mut devices = load_devices();
    if let Some(did) = device_id {
        devices.retain(|d| d.device_id.as_deref() != Some(did.as_str()));
    }
    devices.push(device.clone());
    save_devices(&devices)?;
    Ok(device)
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

/// This Mac's identity, shown on the phone BEFORE the user commits — in an
/// office full of Linggen Macs, seeing a stranger's name here is the cue to
/// cancel. Display-only info, the same thing the pairing dialog shows.
fn mac_identity() -> (String, Option<String>) {
    let mac_name = local_host_name().unwrap_or_else(|| "Mac".to_string());
    let account = crate::account::load_account().and_then(|a| a.user_name);
    (mac_name, account)
}

fn local_host_name() -> Option<String> {
    let out = std::process::Command::new("scutil")
        .args(["--get", "LocalHostName"])
        .output()
        .ok()?;
    let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!name.is_empty()).then_some(name)
}

#[derive(Deserialize)]
pub(crate) struct PairRequest {
    device_name: String,
    #[serde(default)]
    device_id: Option<String>,
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
        device_id: req.device_id,
        created: Instant::now(),
        attempts: 0,
    });
    let (mac_name, account_name) = mac_identity();
    Json(serde_json::json!({
        "pair_id": pair_id,
        "expires_in": CODE_TTL.as_secs(),
        "mac_name": mac_name,
        "account_name": account_name,
    }))
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
    let name = p.device_name.clone();
    let device_id = p.device_id.clone();
    *pending = None;
    drop(pending);
    let device = match commit_device(name, device_id) {
        Ok(d) => d,
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, format!("persist: {e}")),
    };
    tracing::info!("[pair] device '{}' paired ({})", device.name, device.id);
    Json(serde_json::json!({ "device_token": device.secret, "device_id": device.id }))
        .into_response()
}

fn err(code: StatusCode, msg: impl Into<String>) -> axum::response::Response {
    (code, Json(serde_json::json!({ "error": msg.into() }))).into_response()
}

// ---------------------------------------------------------------------------
// Settings → Phone — the Mac-side management surface for pairing.
// ---------------------------------------------------------------------------

/// This Mac's primary LAN address. The UDP-connect trick: no packet is sent,
/// the OS just picks the interface it would route through.
fn lan_ip() -> Option<String> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("1.1.1.1:80").ok()?;
    Some(socket.local_addr().ok()?.ip().to_string())
}

/// GET /api/pair/info — everything the Phone settings tab shows: live bind
/// state, addresses, and the paired-device list (names only, never secrets).
pub(crate) async fn get_pair_info(
    State(state): State<std::sync::Arc<crate::server::ServerState>>,
) -> impl IntoResponse {
    let config = state.manager.get_config_snapshot().await;
    let lan_live = state.bound_host != "127.0.0.1" && state.bound_host != "localhost";
    let (mac_name, account_name) = mac_identity();
    let devices: Vec<serde_json::Value> = load_devices()
        .iter()
        .map(|d| serde_json::json!({ "id": d.id, "name": d.name, "created_at": d.created_at }))
        .collect();
    Json(serde_json::json!({
        "lan_live": lan_live,
        "config_host": config.server.host,
        "port": state.port,
        "lan_ip": lan_ip(),
        "mdns_host": format!("{}.local", mac_name.to_lowercase()),
        "mac_name": mac_name,
        "account_name": account_name,
        "devices": devices,
    }))
}

/// GET /api/pair/qr — the QR as JSON for embedding in Settings → Phone.
/// Mints a fresh single-use secret exactly like the standalone /pair page.
pub(crate) async fn get_pair_qr(
    State(state): State<std::sync::Arc<crate::server::ServerState>>,
) -> impl IntoResponse {
    let (svg, url, host) = mint_qr(state.port);
    Json(serde_json::json!({ "svg": svg, "url": url, "host": host }))
}

/// DELETE /api/pair/devices/{id} — revoke one device. Its token stops working
/// on the next request; the phone re-pairs with eyes on this Mac.
pub(crate) async fn delete_pair_device(Path(id): Path<String>) -> impl IntoResponse {
    let mut devices = load_devices();
    let before = devices.len();
    devices.retain(|d| d.id != id);
    if devices.len() == before {
        return err(StatusCode::NOT_FOUND, "no such device");
    }
    if let Err(e) = save_devices(&devices) {
        return err(StatusCode::INTERNAL_SERVER_ERROR, format!("persist: {e}"));
    }
    tracing::info!("[pair] device {id} revoked");
    Json(serde_json::json!({ "status": "ok" })).into_response()
}

// ---------------------------------------------------------------------------
// Bonjour — the daemon announces itself so phones list nearby Macs by name.
// ---------------------------------------------------------------------------

/// Advertise `_linggen._tcp` on the LAN. The phone's pair sheet browses for
/// this and shows "This-Mac · linggen" entries — no addresses typed, and in
/// an office of Linggens each entry is named. The responder lives in the
/// daemon process, so it dies (and the record expires) with it. Loopback-only
/// binds skip it: nothing to discover that the LAN can reach.
pub fn advertise(port: u16, lan_bound: bool) {
    if !lan_bound {
        return;
    }
    let (mac_name, account) = mac_identity();
    let result = mdns_sd::ServiceDaemon::new().and_then(|daemon| {
        // The SRV target must be a host label WE own, never the Mac's own
        // `.local` name. enable_addr_auto() publishes A records for whatever
        // host we name here; naming it the OS hostname makes macOS see a
        // second responder claiming its own name and defensively rename the
        // computer (This-Mac → This-Mac-2 → …) on every daemon start. A
        // linggen-scoped label carries our addresses without touching the name
        // the OS owns. Resolvers read the IPs straight from the service info,
        // so the label never needs to be human-meaningful.
        let host = format!("linggen-{}.local.", mac_name.to_lowercase());
        let mut info = mdns_sd::ServiceInfo::new(
            "_linggen._tcp.local.",
            &mac_name,
            &host,
            (),
            port,
            &[("name", mac_name.as_str()), ("account", account.as_deref().unwrap_or(""))][..],
        )?
        .enable_addr_auto();
        info.set_requires_probe(false);
        daemon.register(info)?;
        // Leak the daemon handle — it must outlive this fn (daemon lifetime).
        std::mem::forget(daemon);
        Ok(())
    });
    match result {
        Ok(()) => tracing::info!("[bonjour] advertising _linggen._tcp as '{mac_name}' on port {port}"),
        Err(e) => tracing::warn!("[bonjour] advertise failed: {e}"),
    }
}

// ---------------------------------------------------------------------------
// QR pairing — scan the Mac's screen instead of typing anything.
// ---------------------------------------------------------------------------
//
// GET /pair (loopback-only via the LAN gate: only someone AT this Mac can see
// it) renders a QR encoding `linggen://pair?host=<name>:<port>&secret=…`. The
// phone scans it and trades the secret for a device token at
// POST /api/pair/qr-confirm. Scanning a screen you're standing in front of is
// an even stronger version of the code confirm — wrong-Mac pairing becomes
// physically impossible.

const QR_TTL: Duration = Duration::from_secs(600);

struct QrPending {
    secret: String,
    created: Instant,
}

static QR_PENDING: Mutex<Option<QrPending>> = Mutex::new(None);

/// Mint a fresh single-use QR secret and render it. Shared by the standalone
/// /pair page and the Settings → Phone tab.
fn mint_qr(port: u16) -> (String, String, String) {
    let secret = random_hex(16);
    *QR_PENDING.lock().unwrap() = Some(QrPending { secret: secret.clone(), created: Instant::now() });
    let (mac_name, _) = mac_identity();
    let host = format!("{}.local:{}", mac_name.to_lowercase(), port);
    let url = format!("linggen://pair?host={host}&secret={secret}");
    let svg = qrcode::QrCode::new(url.as_bytes())
        .map(|qr| {
            qr.render::<qrcode::render::svg::Color>()
                .min_dimensions(260, 260)
                .quiet_zone(true)
                .build()
        })
        .unwrap_or_default();
    (svg, url, host)
}

/// GET /pair — the QR page. Each load mints a fresh single-use secret.
pub(crate) async fn get_pair_page(
    State(state): State<std::sync::Arc<crate::server::ServerState>>,
) -> impl IntoResponse {
    let (svg, url, host) = mint_qr(state.port);
    let (mac_name, account) = mac_identity();
    let who = account.map(|a| format!(" · {a}")).unwrap_or_default();
    Html(format!(
        "<!doctype html><meta charset=utf-8><title>Pair with {mac_name}</title>\
         <body style=\"font-family:-apple-system,sans-serif;display:flex;flex-direction:column;\
         align-items:center;justify-content:center;min-height:90vh;background:#12151D;color:#E8E6DF\">\
         <h2 style=\"font-weight:600\">Pair your phone</h2>\
         <p style=\"color:#8F94A3;margin:0 0 18px\">Scan with Linggen on your phone — pairing with <b>{mac_name}</b>{who}</p>\
         <div style=\"background:#fff;padding:14px;border-radius:12px\">{svg}</div>\
         <p style=\"color:#8F94A3;margin-top:18px;font-size:13px\">Can't scan? Type <code>{host}</code> in the app and confirm the on-screen code.</p>\
         <p style=\"color:#5c6170;font-size:11px;word-break:break-all\">{url}</p>\
         <p style=\"color:#5c6170;font-size:12px\">This QR is single-use and expires in 10 minutes. Reload for a fresh one.</p>"
    ))
}

#[derive(Deserialize)]
pub(crate) struct QrConfirm {
    secret: String,
    device_name: String,
    #[serde(default)]
    device_id: Option<String>,
}

/// POST /api/pair/qr-confirm — trade a scanned QR secret for a device token.
pub(crate) async fn post_pair_qr_confirm(Json(req): Json<QrConfirm>) -> impl IntoResponse {
    let mut pending = QR_PENDING.lock().unwrap();
    let valid = pending
        .as_ref()
        .is_some_and(|p| p.secret == req.secret && p.created.elapsed() <= QR_TTL);
    if !valid {
        return err(StatusCode::UNAUTHORIZED, "QR expired or already used — reload the page on your Mac");
    }
    *pending = None;
    drop(pending);
    let name: String = req.device_name.chars().take(64).collect();
    let device = match commit_device(name, req.device_id) {
        Ok(d) => d,
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, format!("persist: {e}")),
    };
    tracing::info!("[pair] device '{}' paired via QR ({})", device.name, device.id);
    let (mac_name, account_name) = mac_identity();
    Json(serde_json::json!({
        "device_token": device.secret,
        "device_id": device.id,
        "mac_name": mac_name,
        "account_name": account_name,
    }))
    .into_response()
}
