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
    let mac_account = crate::server::person::mac_account();
    let people = crate::server::people::current();
    let devices: Vec<serde_json::Value> = load_devices()
        .iter()
        .map(|d| {
            serde_json::json!({
                "id": d.id,
                "name": d.name,
                "created_at": d.created_at,
                // phone | machine: a computer sees all the owner's memory.
                "kind": d.kind(),
                "settings": d.settings,
                "account": d.account,
                // What it stands at: the owner's device is admin; any other
                // the level set here (read when none is).
                "level": crate::server::person::standing_of(&d.id).level(),
                "verified": d.verified_claim().is_some(),
                // Whose it is: owner, unverified (claims this Mac's account),
                // other (another account), signed_out.
                "whose": crate::server::person::whose_device(d, mac_account.as_deref()),
                // The level set here for its person when it is not the owner's
                // (what the choice shows), read when none is.
                "permission": people.level_of(&crate::server::person::of_row(d)),
                // A phone out of step with this Mac's contract: told to update.
                "needs_update": d.kind() == crate::state_fs::devices::DeviceKind::Phone
                    && crate::server::rtc::contract::behind(d.contract.unwrap_or(0)),
                // Who to call them on screen. `account` above is the truth and
                // stays absent for a signed-out phone; this is what to print.
                "person": person_label(&d.account),
            })
        })
        .collect();
    Json(serde_json::json!({
        "lan_live": lan_live,
        // The host the config asks for, from `url` or the legacy pair.
        "config_host": config.server.addr().0,
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
/// - `oauth`  → ChatGPT/Codex (phone signs in on-device): a built-in, or a
///   model the person added that uses the ChatGPT sign-in
/// - `byok`   → a configured provider; the Mac's key + base_url ride along
/// - `local`  → Ollama/local; not reachable off the Mac
///
/// A model the person configured also carries `model`, the provider's own
/// name for it — the id is only what this Mac calls it.
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
        // A live-voice row is not a chat model: the phone is not offered it.
        .filter(|id| !config.is_live_voice_row(id))
        .map(|id| {
            if crate::provider::models::chatgpt_catalog::is_builtin(id) {
                return serde_json::json!({ "id": id, "kind": "oauth" });
            }
            if id == crate::provider::models::LINGGEN_CLOUD_MODEL_ID {
                return serde_json::json!({ "id": id, "kind": "cloud" });
            }
            match config.chat_models().find(|m| &m.id == id) {
                Some(m) if m.provider_kind() == crate::config::ProviderKind::Ollama => {
                    serde_json::json!({ "id": id, "kind": "local" })
                }
                Some(m) if m.signs_in_with_chatgpt() => {
                    serde_json::json!({ "id": id, "kind": "oauth", "model": m.model })
                }
                Some(m) => serde_json::json!({
                    "id": id,
                    "kind": "byok",
                    "model": m.model,
                    "provider": m.provider,
                    "base_url": m.url,
                    "key": crate::credentials::resolve_api_key(m, &creds),
                }),
                None => serde_json::json!({ "id": id, "kind": "unknown" }),
            }
        })
        .collect()
}

/// Not asked through `model_choice::resolve`: this is a display list for a
/// phone from the settings alone, with no turn, session or asker to resolve for.
///
/// Which model each agent runs on here, by agent id — what a phone with no
/// pick of its own follows. The lead agent uses the routing default chain's first entry;
/// the companion (`[pet] pet`) her `[pet] model`, with "auto" resolved the way her turns resolve it
/// and falling back to the routing default when it names none. An agent with
/// no answer is left out.
fn agent_models(config: &crate::config::Config) -> serde_json::Value {
    let routing = config.routing.default_models.first().cloned();
    let mut out = serde_json::Map::new();
    if let Some(m) = &routing {
        out.insert(crate::engine::agent::LEAD_AGENT_ID.into(), m.clone().into());
    }
    if let Some(m) = crate::server::resident::resolve_pet_model(&config.pet.model).or(routing) {
        out.insert(config.pet.pet.clone(), m.into());
    }
    out.into()
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
    let config = state.manager.get_config_snapshot().await;
    let mac = crate::server::person::mac_account();
    let owner = crate::server::person::is_owners_device(&d, mac.as_deref());
    let person = crate::server::person::of_device(&d, mac.as_deref());
    let allow = if person.is_owner() {
        device_models(&d)
    } else {
        // Use the same model list (including successors) as a person's turn.
        crate::server::people::current()
            .ceiling_of(&person, &[])
            .models
    };
    let mut body = me_body(&d, &config, owner, &allow);
    // Where to reach this Mac when the phone is off the LAN: the relay
    // signals per instance. Not a secret — an address. Absent while
    // signed out, since the relay leg needs the account credential.
    body["instance_id"] = crate::account::resolve_token()
        .and(crate::account::instance_id().ok())
        .into();
    Json(body).into_response()
}

/// The owner's device keeps its own catalog selection, not a people row.
fn device_models(d: &crate::state_fs::devices::PairedDevice) -> Vec<String> {
    d.settings
        .get("models")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

/// Keep the phone's response shape and only send Mac credentials to the owner.
fn me_body(
    d: &crate::state_fs::devices::PairedDevice,
    config: &crate::config::Config,
    owner: bool,
    allow: &[String],
) -> serde_json::Value {
    let mut models = resolve_model_catalog(allow, config);
    if !owner {
        models.retain(|m| matches!(m["kind"].as_str(), Some("cloud" | "oauth")));
    }
    serde_json::json!({
        "id": d.id,
        "name": d.name,
        "device_id": d.device_id,
        "settings": d.settings,
        "models": models,
        "agent_models": agent_models(config),
        "chatgpt": crate::provider::models::chatgpt_catalog::current().for_phone(),
    })
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
/// on the next request; the phone re-pairs with eyes on this Mac. A device
/// below the owner's standing revokes only itself (`may_revoke`).
pub(crate) async fn delete_pair_device(
    Path(id): Path<String>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    if !may_revoke(&headers, &id) {
        return err(StatusCode::FORBIDDEN, "a device revokes only itself");
    }
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

#[derive(Deserialize)]
pub(crate) struct LevelUpdate {
    permission: crate::engine::permission::PermissionMode,
}

/// PUT /api/pair/devices/{id}/permission — the level the person behind a
/// device that is not the owner's stands at (`server::person::standing_of`):
/// chat, read or edit. The same writer as `PUT /api/people/{id}`
/// (`server::people`), kept for Settings → Phone until the People page
/// replaces it. Set only by someone at this Mac (`is_at_this_mac`): never from
/// a paired device, nor from another machine. The owner's own device stands
/// at admin whatever is set.
pub(crate) async fn set_device_level(
    axum::extract::ConnectInfo(peer): axum::extract::ConnectInfo<std::net::SocketAddr>,
    headers: axum::http::HeaderMap,
    Path(id): Path<String>,
    Json(req): Json<LevelUpdate>,
) -> axum::response::Response {
    if !crate::server::api::ling_mem_connect::is_at_this_mac(peer, &headers) {
        return err(StatusCode::FORBIDDEN, "only at this Mac");
    }
    let Some(person) = load_devices().iter().find(|d| d.id == id).map(crate::server::person::of_row) else {
        return err(StatusCode::BAD_REQUEST, "no such device");
    };
    let patch = serde_json::json!({ "level": req.permission });
    let set = crate::server::people::modify(|people| people.set(&person, patch.as_object().expect("an object")));
    match set {
        Ok(Ok(())) => {}
        Ok(Err(why)) => return err(StatusCode::BAD_REQUEST, why),
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, format!("persist: {e}")),
    }
    tracing::info!("[pair] device {id} set to {}", req.permission);
    Json(serde_json::json!({ "status": "ok", "permission": req.permission })).into_response()
}

/// Whether the caller may revoke device `id`: at admin any device; below
/// it, only the device asking.
fn may_revoke(headers: &axum::http::HeaderMap, id: &str) -> bool {
    use crate::engine::permission::PermissionMode;
    caller_level(headers) == PermissionMode::Admin || caller_device(headers).as_deref() == Some(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn configured(id: &str, provider: &str, model: &str, auth_mode: Option<&str>) -> crate::config::ModelConfig {
        let toml = format!("id = \"{id}\"\nprovider = \"{provider}\"\nurl = \"https://example.test/v1\"\nmodel = \"{model}\"\n");
        let mut m: crate::config::ModelConfig = toml::from_str(&toml).unwrap();
        m.auth_mode = auth_mode.map(Into::into);
        m
    }

    /// A model the person added is told to the phone with the provider's
    /// own name for it, and one that uses the ChatGPT sign-in is `oauth`
    /// like the built-ins — not a key-based model with no key.
    #[test]
    fn a_model_the_person_added_reaches_the_phone_as_what_it_is() {
        let mut cfg = crate::config::Config::default();
        cfg.models = vec![
            configured("my-sol", "chatgpt", "gpt-9-sol", Some("chatgpt_oauth")),
            configured("my-claude", "anthropic", "claude-next", None),
        ];
        let allow = ["my-sol".to_string(), "my-claude".to_string(), "gpt-6-luna".to_string()];
        let got = resolve_model_catalog(&allow, &cfg);
        assert_eq!(got[0], serde_json::json!({"id": "my-sol", "kind": "oauth", "model": "gpt-9-sol"}));
        assert_eq!(got[1]["kind"], "byok");
        assert_eq!(got[1]["model"], "claude-next");
        assert_eq!(got[2], serde_json::json!({"id": "gpt-6-luna", "kind": "oauth"}));
    }

    /// A `chatgpt` provider signs in with ChatGPT whatever its auth mode
    /// says (`ModelConfig::sign_in`): the phone is told `oauth`, never a
    /// key-based model with no key.
    #[test]
    fn a_chatgpt_provider_without_an_auth_mode_reaches_the_phone_as_oauth() {
        let mut cfg = crate::config::Config::default();
        cfg.models = vec![configured("plain-sol", "chatgpt", "gpt-9-sol", None)];
        let got = resolve_model_catalog(&["plain-sol".to_string()], &cfg);
        assert_eq!(got[0], serde_json::json!({"id": "plain-sol", "kind": "oauth", "model": "gpt-9-sol"}));
    }

    /// The phone takes the ChatGPT client version and what every older id
    /// now means from the Mac, instead of keeping its own copies.
    #[test]
    fn the_phone_is_told_the_chatgpt_version_and_what_old_ids_mean() {
        let cfg = crate::config::Config::default();
        let d = crate::state_fs::devices::PairedDevice::default();
        let body = me_body(&d, &cfg, false, &[]);
        let catalog = crate::provider::models::chatgpt_catalog::current();
        assert_eq!(body["chatgpt"]["client_version"], catalog.client_version());
        assert_eq!(body["chatgpt"]["successors"]["gpt-6.1-sol"], "gpt-6.2-sol");
        assert_eq!(body["chatgpt"]["successors"]["gpt-5.5"], catalog.primary_id());
        assert_eq!(body["chatgpt"]["builtins"], serde_json::json!(catalog.builtin_ids()));
    }

    /// The Mac's provider keys are the owner's: the owner's device is given
    /// every allow-listed model with its key; any other device only the
    /// models it reaches with its own credentials, and no key anywhere in
    /// the answer.
    #[test]
    fn a_key_never_crosses_to_a_device_not_the_owners() {
        let mut cfg = crate::config::Config::default();
        let mut byok = configured("my-claude", "anthropic", "claude-next", None);
        byok.api_key = Some("sk-THE-MACS-KEY".into());
        cfg.models = vec![byok, configured("my-sol", "chatgpt", "gpt-9-sol", Some("chatgpt_oauth"))];
        let mut d = crate::state_fs::devices::PairedDevice::default();
        d.settings.insert(
            "models".into(),
            serde_json::json!([
                "my-claude",
                "my-sol",
                crate::provider::models::LINGGEN_CLOUD_MODEL_ID
            ]),
        );
        let owners = me_body(&d, &cfg, true, &device_models(&d));
        assert!(owners.to_string().contains("sk-THE-MACS-KEY"));
        assert_eq!(owners["models"].as_array().map(Vec::len), Some(3));
        let other = me_body(&d, &cfg, false, &device_models(&d));
        assert!(!other.to_string().contains("sk-THE-MACS-KEY"), "{other}");
        let kinds: Vec<_> = other["models"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["kind"].clone())
            .collect();
        assert_eq!(
            kinds,
            vec![serde_json::json!("oauth"), serde_json::json!("cloud")]
        );

        // The same phone row: claimed, then verified at its next sync, then
        // signed in to another account.
        let mac = Some("u-owner");
        let keyed = |d: &crate::state_fs::devices::PairedDevice| {
            me_body(
                d,
                &cfg,
                crate::server::person::is_owners_device(d, mac),
                &device_models(d),
            )
            .to_string()
            .contains("sk-THE-MACS-KEY")
        };
        let claim = |id: &str| crate::state_fs::devices::AccountRef { id: id.into(), email: None, name: None, at: None };
        d.kind = Some(crate::state_fs::devices::DeviceKind::Phone);
        d.contract = Some(crate::server::rtc::contract::CONTRACT);
        d.account = Some(claim("u-owner"));
        assert!(!keyed(&d), "a claim alone");
        d.verified_account = Some("u-owner".into());
        assert!(keyed(&d), "verified: the owner's");
        d.account = Some(claim("u-other"));
        assert!(!keyed(&d), "no longer the owner's");
        d.verified_account = Some("u-other".into());
        assert!(!keyed(&d), "another account the relay vouched for is not the owner's");
    }

    /// The level is set only at this Mac: a paired device's request through
    /// the tunnel, a relay peer's, and another machine's are refused.
    #[tokio::test]
    async fn a_level_is_set_only_at_this_mac() {
        use crate::engine::permission::PermissionMode;
        use axum::extract::ConnectInfo;
        let h = |pairs: &[(&'static str, &'static str)]| {
            let mut m = axum::http::HeaderMap::new();
            for (k, v) in pairs {
                m.insert(*k, v.parse().unwrap());
            }
            m
        };
        let lo: std::net::SocketAddr = "127.0.0.1:5000".parse().unwrap();
        let lan: std::net::SocketAddr = "192.168.1.5:5000".parse().unwrap();
        let tries = [
            (
                lo,
                h(&[
                    ("host", "127.0.0.1:9527"),
                    (ACTOR_DEVICE_HEADER, "phone-1"),
                    (TUNNEL_HEADER, "1"),
                ]),
            ),
            (
                lo,
                h(&[("host", "127.0.0.1:9527"), (TUNNEL_HEADER, TUNNEL_OWNER)]),
            ),
            (lan, h(&[("host", "192.168.1.2:9527")])),
        ];
        for (peer, headers) in tries {
            let body = Json(LevelUpdate {
                permission: PermissionMode::Edit,
            });
            let res =
                set_device_level(ConnectInfo(peer), headers, Path("phone-1".into()), body).await;
            assert_eq!(res.status(), StatusCode::FORBIDDEN);
        }
    }

    /// Below admin a device revokes itself and no other; this machine and
    /// the owner revoke any.
    #[test]
    fn a_device_below_the_owner_revokes_only_itself() {
        let h = |pairs: &[(&'static str, &'static str)]| {
            let mut m = axum::http::HeaderMap::new();
            for (k, v) in pairs {
                m.insert(*k, v.parse().unwrap());
            }
            m
        };
        assert!(may_revoke(&h(&[]), "other"));
        assert!(may_revoke(&h(&[(LEVEL_HEADER, "admin")]), "other"));
        assert!(!may_revoke(&h(&[(LEVEL_HEADER, "read")]), "other"));
        assert!(!may_revoke(&h(&[(LEVEL_HEADER, "bogus")]), "other"));
        // Its own row: the tunnel names the device asking.
        let own = h(&[(LEVEL_HEADER, "read"), (ACTOR_DEVICE_HEADER, "not-paired")]);
        assert!(!may_revoke(&own, "other"), "a device not paired is no one");
    }

    #[test]
    fn each_agent_reports_the_model_it_runs_on() {
        let mut cfg = crate::config::Config::default();
        cfg.routing.default_models = vec!["m-main".into(), "m-next".into()];
        cfg.pet.model = "m-pet".into();
        let m = agent_models(&cfg);
        assert_eq!(m["ling"], "m-main");
        assert_eq!(m["yinyue"], "m-pet");
    }

    #[test]
    fn an_empty_chain_leaves_the_agents_out() {
        let mut cfg = crate::config::Config::default();
        cfg.routing.default_models.clear();
        cfg.pet.model = "m-pet".into();
        let m = agent_models(&cfg);
        assert!(m.get("ling").is_none());
        assert_eq!(m["yinyue"], "m-pet");
    }
}
