//! Settings → Phone — the Mac-side management surface for pairing.

use super::*;

/// This Mac's primary LAN address. The UDP-connect trick: no packet is sent,
/// the OS just picks the interface it would route through.
pub(super) fn lan_ip() -> Option<String> {
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
        .map(|d| {
            serde_json::json!({
                "id": d.id,
                "name": d.name,
                "created_at": d.created_at,
                "settings": d.settings,
                "account": d.account,
                // Who to call them on screen. `account` above is the truth and
                // stays absent for a signed-out phone; this is what to print.
                "person": person_label(&d.account),
            })
        })
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

/// Resolve each allow-listed model into "how the phone reaches it" so the phone
/// stays dumb-simple: it picks a model and follows the recipe. The Mac is the
/// single place keys and endpoints live (BYOK syncs from here).
///
/// - `cloud`  → the Linggen Cloud proxy (phone uses its account token)
/// - `oauth`  → ChatGPT/Codex (phone signs in on-device)
/// - `byok`   → a configured provider; the Mac's key + base_url ride along
/// - `local`  → Ollama/local; not reachable off the Mac
pub(super) fn resolve_model_catalog(
    allow: &[String],
    config: &crate::config::Config,
) -> Vec<serde_json::Value> {
    let creds = crate::credentials::Credentials::load_for(
        &crate::credentials::credentials_file(),
        &config.models,
    );
    allow
        .iter()
        .map(|id| {
            if crate::provider::models::CHATGPT_BUILTIN_MODEL_IDS.contains(&id.as_str()) {
                return serde_json::json!({ "id": id, "kind": "oauth" });
            }
            if id == crate::provider::models::LINGGEN_CLOUD_MODEL_ID {
                return serde_json::json!({ "id": id, "kind": "cloud" });
            }
            match config.models.iter().find(|m| &m.id == id) {
                Some(m) if m.provider == "ollama" => {
                    serde_json::json!({ "id": id, "kind": "local" })
                }
                Some(m) => serde_json::json!({
                    "id": id,
                    "kind": "byok",
                    "provider": m.provider,
                    "base_url": m.url,
                    "key": crate::credentials::resolve_api_key(m, &creds),
                }),
                None => serde_json::json!({ "id": id, "kind": "unknown" }),
            }
        })
        .collect()
}

/// GET /api/pair/me — the calling phone's own paired-device row: name, settings,
/// and a resolved model catalog (endpoint + key per allow-listed model). This is
/// the "Sync from Mac" pull, identified by the device token.
pub(crate) async fn get_pair_me(
    State(state): State<std::sync::Arc<crate::server::ServerState>>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    // Two ways to be this phone. The token is the HTTP caller's proof. A
    // request arriving down the WebRTC tunnel carries no token — the peer was
    // authenticated when the channel came up, and the tunnel tags what it
    // proxies with the device it belongs to. Without this second path
    // `/api/pair/me` is unreachable over the tunnel, and since that response
    // is where the phone learns the Mac's relay instance id, a phone off the
    // LAN could never find the relay: it only ever knew the id if it happened
    // to be told at QR-pair time.
    let Some(d) = device_token_from_headers(&headers)
        .and_then(|t| device_by_token(&t))
        .or_else(|| {
            let id = headers.get(ACTOR_DEVICE_HEADER)?.to_str().ok()?;
            load_devices().into_iter().find(|d| d.id == id)
        })
    else {
        return err(StatusCode::UNAUTHORIZED, "unknown device");
    };
    let allow: Vec<String> = d
        .settings
        .get("models")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let config = state.manager.get_config_snapshot().await;
    Json(serde_json::json!({
        "id": d.id,
        "name": d.name,
        "device_id": d.device_id,
        "settings": d.settings,
        "models": resolve_model_catalog(&allow, &config),
        // Where to reach this Mac when the phone is off the LAN: the relay
        // signals per instance. Not a secret — an address. Absent while
        // signed out, since the relay leg needs the account credential.
        "instance_id": crate::account::resolve_token()
            .and(crate::account::instance_id().ok()),
    }))
    .into_response()
}

/// GET /api/pair/qr — the QR as JSON for embedding in Settings → Phone.
/// Mints a fresh single-use secret exactly like the standalone /pair page.

#[derive(Deserialize)]
pub(crate) struct DeviceUpdate {
    #[serde(default)]
    name: Option<String>,
    /// Replaces the device's Mac-owned settings blob (e.g. `{"models": [ids]}`).
    #[serde(default)]
    settings: Option<serde_json::Map<String, serde_json::Value>>,
}

/// PATCH /api/pair/devices/{id} — edit a paired device: rename it and/or set its
/// per-device settings (the models allow-list, etc.). iOS won't hand apps the
/// user-set name without a restricted entitlement, so renaming here is the way
/// to label a device; settings are the Mac→phone pull source.
pub(crate) async fn rename_pair_device(
    Path(id): Path<String>,
    Json(req): Json<DeviceUpdate>,
) -> impl IntoResponse {
    if req.name.is_none() && req.settings.is_none() {
        return err(StatusCode::BAD_REQUEST, "nothing to update");
    }
    let name = match req
        .name
        .map(|n| n.trim().chars().take(64).collect::<String>())
    {
        Some(n) if n.is_empty() => return err(StatusCode::BAD_REQUEST, "name cannot be empty"),
        other => other,
    };
    let found = modify_devices(|devices| {
        let Some(device) = devices.iter_mut().find(|d| d.id == id) else {
            return false;
        };
        if let Some(name) = name {
            device.name = name;
        }
        if let Some(settings) = req.settings {
            device.settings = settings;
        }
        true
    });
    match found {
        Ok(true) => {}
        Ok(false) => return err(StatusCode::NOT_FOUND, "no such device"),
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, format!("persist: {e}")),
    }
    tracing::info!("[pair] device {id} updated");
    Json(serde_json::json!({ "status": "ok" })).into_response()
}

/// DELETE /api/pair/devices/{id} — revoke one device. Its token stops working
/// on the next request; the phone re-pairs with eyes on this Mac.
pub(crate) async fn delete_pair_device(Path(id): Path<String>) -> impl IntoResponse {
    // Take the relay credential with us: dropping the row here only closes the
    // LAN door, and a revoked phone that kept reaching us from anywhere would
    // make Revoke a lie.
    let removed = modify_devices(|devices| {
        let pos = devices.iter().position(|d| d.id == id)?;
        Some(devices.remove(pos).relay_grant)
    });
    let grant = match removed {
        Ok(Some(grant)) => grant,
        Ok(None) => return err(StatusCode::NOT_FOUND, "no such device"),
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, format!("persist: {e}")),
    };
    if let Some(g) = grant {
        revoke_relay_grant(&g).await;
    }
    tracing::info!("[pair] device {id} revoked");
    Json(serde_json::json!({ "status": "ok" })).into_response()
}
