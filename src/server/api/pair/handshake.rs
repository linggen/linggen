//! Pairing a phone: the screen-confirm code, and the QR a phone scans
//! instead.

use super::*;

pub(super) const CODE_TTL: Duration = Duration::from_secs(120);
pub(super) const MAX_ATTEMPTS: u32 = 5;

pub(super) struct PendingPair {
    pair_id: String,
    code: String,
    device_name: String,
    device_id: Option<String>,
    account: Option<AccountRef>,
    created: Instant,
    attempts: u32,
}

/// One pairing attempt at a time — a second request replaces the first.
pub(super) static PENDING: Mutex<Option<PendingPair>> = Mutex::new(None);

pub(super) fn show_code(code: &str, device_name: &str) {
    tracing::info!(
        "[pair] code {code} for device '{device_name}' (valid {}s)",
        CODE_TTL.as_secs()
    );
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
        let _ = std::process::Command::new("osascript")
            .arg("-e")
            .arg(script)
            .spawn();
    }
}

/// This Mac's identity, shown on the phone BEFORE the user commits — in an
/// office full of Linggen Macs, seeing a stranger's name here is the cue to
/// cancel. Display-only info, the same thing the pairing dialog shows.

#[derive(Deserialize)]
pub(crate) struct PairRequest {
    device_name: String,
    #[serde(default)]
    device_id: Option<String>,
    /// Who is signed in on the phone right now. Absent = signed out; pairing
    /// still succeeds, and `identify` fills it in when they sign in later.
    #[serde(default)]
    account: Option<AccountRef>,
}

/// POST /api/pair/request — start a pairing attempt; the code appears on the
/// Mac's screen, never in this response.
pub(crate) async fn post_pair_request(Json(req): Json<PairRequest>) -> impl IntoResponse {
    let code = format!("{:06}", rand::rng().random_range(0..1_000_000u32));
    let pair_id = uuid::Uuid::new_v4().to_string();
    let name = req.device_name.chars().take(64).collect::<String>();
    show_code(&code, &name);
    *PENDING.lock_ok() = Some(PendingPair {
        pair_id: pair_id.clone(),
        code,
        device_name: name,
        device_id: req.device_id,
        account: req.account,
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
    // Everything touching the lock happens in here: the guard is not Send, and
    // minting the relay grant below is an await.
    let claimed = {
        let mut pending = PENDING.lock_ok();
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
            return err(
                StatusCode::TOO_MANY_REQUESTS,
                "too many tries — start again",
            );
        }
        if p.code != req.code.trim() {
            return err(StatusCode::UNAUTHORIZED, "wrong code");
        }
        let claimed = (
            p.device_name.clone(),
            p.device_id.clone(),
            p.account.clone(),
        );
        *pending = None;
        claimed
    };
    let (name, device_id, account) = claimed;
    let device = match commit_device(name, device_id, account) {
        Ok(d) => d,
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, format!("persist: {e}")),
    };
    tracing::info!("[pair] device '{}' paired ({})", device.name, device.id);
    Json(serde_json::json!({
        "device_token": device.secret,
        "device_id": device.id,
        // Same as the QR path: a phone paired here on the LAN still needs a way
        // back to us once it leaves this network.
        "relay_grant": grant_for(&device).await,
    }))
    .into_response()
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

pub(super) struct QrPending {
    pub(super) secret: String,
    /// Last time the surface showing this QR said it was still on screen.
    /// The window closes when that stops — see [`pair_window_watch`].
    pub(super) seen: std::time::Instant,
}

pub(super) static QR_PENDING: Mutex<Option<QrPending>> = Mutex::new(None);

/// How long after the pair surface goes quiet we consider it closed. The
/// surfaces ping every 5s, so this tolerates one missed beat.
pub(super) const QR_IDLE_SHUT: Duration = Duration::from_secs(13);

/// This Mac's relay id, when it has one. Signed out there is no relay presence
/// at all, so the QR is LAN-only and no window can be opened.

#[derive(Deserialize)]
pub(crate) struct QrQuery {
    /// "New code" — deliberately replace what is showing.
    #[serde(default)]
    new: Option<bool>,
}

pub(crate) async fn get_pair_qr(
    State(state): State<std::sync::Arc<crate::server::ServerState>>,
    axum::extract::Query(q): axum::extract::Query<QrQuery>,
) -> impl IntoResponse {
    let (svg, url, host) = if q.new.unwrap_or(false) {
        mint_qr(state.port)
    } else {
        current_qr(state.port)
    };
    Json(serde_json::json!({
        "svg": svg,
        "url": url,
        "host": host,
        "generation": qr_generation(),
    }))
}

/// Mint a fresh QR secret and render it. Shared by the standalone /pair page
/// and the Settings → Phone tab.
///
/// The QR carries the relay instance id as well as the LAN address, so a phone
/// that cannot reach this Mac's network — on a carrier connection, in another
/// city — can still pair by signalling through linggen.dev. The secret is what
/// authorizes that, which is why the window it opens is bound to this QR being
/// on screen: see [`open_pair_window`].
pub(super) fn mint_qr(port: u16) -> (String, String, String) {
    let secret = random_hex(16);
    {
        let mut pending = QR_PENDING.lock_ok();
        *pending = Some(QrPending {
            secret: secret.clone(),
            seen: std::time::Instant::now(),
        });
    }
    QR_GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    open_pair_window(&secret);
    render_qr(port, &secret)
}

/// Draw whatever code is currently on offer, minting one only if there is
/// none.
///
/// Rendering must not mint, or two open surfaces would rotate each other for
/// ever: each would see the other's new code, redraw, and mint again.
pub(super) fn current_qr(port: u16) -> (String, String, String) {
    let existing = QR_PENDING.lock_ok().as_ref().map(|p| p.secret.clone());
    match existing {
        Some(secret) => render_qr(port, &secret),
        None => mint_qr(port),
    }
}

pub(super) fn render_qr(port: u16, secret: &str) -> (String, String, String) {
    let (mac_name, _) = mac_identity();
    let host = format!("{}.local:{}", mac_name.to_lowercase(), port);
    let instance = relay_instance()
        .map(|id| format!("&instance={id}"))
        .unwrap_or_default();
    let url = format!("linggen://pair?host={host}&secret={secret}{instance}");
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

/// Bumped whenever the code changes, so a surface showing the old one knows to
/// redraw without having to compare secrets it should not be handling.
pub(super) static QR_GENERATION: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

pub(super) fn qr_generation() -> u64 {
    QR_GENERATION.load(std::sync::atomic::Ordering::Relaxed)
}

/// Retire the code a phone just used and put a fresh one up.
///
/// A spent code has by definition been seen. Rotating means one left on screen
/// after you have finished pairing cannot quietly pair somebody else's phone —
/// and only on success, so a failed attempt can still be retried with the code
/// the user is looking at.
pub(super) fn rotate_qr(port: u16) {
    let _ = mint_qr(port);
    tracing::info!("[pair] code used — a fresh one is now showing");
}

/// GET /pair — the QR page. Shows whatever code is currently on offer (a
/// reload is not a reason to spend one), tells us it is still open so the
/// window closes when it goes away, and redraws when a phone uses the code.
pub(crate) async fn get_pair_page(
    State(state): State<std::sync::Arc<crate::server::ServerState>>,
) -> impl IntoResponse {
    let (svg, url, host) = current_qr(state.port);
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
         <p style=\"color:#5c6170;font-size:12px\">Scan it from any phone, on any network. The code works while this page is open — close it and it stops.</p>\
         <script>\
         let gen = null;\
         const beat = () => fetch('/api/pair/window/keepalive', {{method:'POST'}})\
           .then(r => r.ok ? r.json() : null)\
           .then(d => {{\
             if (!d) return;\
             if (gen === null) {{ gen = d.generation; return; }}\
             if (d.generation !== gen) location.reload();\
           }})\
           .catch(() => {{}});\
         beat(); setInterval(beat, 5000);\
         </script>"
    ))
}

#[derive(Deserialize)]
pub(crate) struct QrConfirm {
    secret: String,
    device_name: String,
    #[serde(default)]
    device_id: Option<String>,
    #[serde(default)]
    account: Option<AccountRef>,
}

/// POST /api/pair/qr-confirm — trade a scanned QR secret for a device token.
///
/// The secret is reusable *while its QR is on screen*: it is not consumed on
/// use, so one code pairs any number of phones, and re-scanning is idempotent
/// (the same phone's `device_id` replaces its own row via `commit_device`). It
/// dies when the surface showing it goes away, a new one is minted, or the
/// daemon restarts.
///
/// This is reachable two ways, and both mean somebody stood at this Mac: over
/// the LAN, where the QR is loopback-only to display; and over a relay channel
/// that linggen.dev opened because the same secret matched an open pairing
/// window. A relay peer can call nothing else until it gets a token back —
/// see `UserContext::pairing_only`. Every pairing is a revocable row.
pub(crate) async fn post_pair_qr_confirm(
    State(state): State<std::sync::Arc<crate::server::ServerState>>,
    Json(req): Json<QrConfirm>,
) -> impl IntoResponse {
    let matches = QR_PENDING
        .lock_ok()
        .as_ref()
        .is_some_and(|p| p.secret == req.secret);
    if !matches {
        return err(
            StatusCode::UNAUTHORIZED,
            "QR no longer valid — open Settings → Phone on your Mac for the current code",
        );
    }
    let name: String = req.device_name.chars().take(64).collect();
    let device = match commit_device(name, req.device_id, req.account) {
        Ok(d) => d,
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, format!("persist: {e}")),
    };
    tracing::info!(
        "[pair] device '{}' paired via QR ({})",
        device.name,
        device.id
    );
    // Spent, so retire it. Only here, on the success path: a failed attempt
    // leaves the code the user is looking at alive to try again.
    rotate_qr(state.port);
    let (mac_name, account_name) = mac_identity();
    Json(serde_json::json!({
        "device_token": device.secret,
        "device_id": device.id,
        "mac_name": mac_name,
        "account_name": account_name,
        // How to reach us from outside this network with no account of its
        // own. Absent when we are signed out — then there is no relay either.
        "relay_grant": grant_for(&device).await,
    }))
    .into_response()
}
