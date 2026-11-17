//! The person a request into a session is for, as the engine is reached
//! from off this machine: the routes that take a `user_id` and how a
//! request is sent as its person. The channel's tunnel and the LAN gate
//! both send their requests through [`send_as`], with the person resolved
//! from the device through memory's own seam ([`of_device`]). It is the one
//! seam a session id from off this machine passes: a request naming a day
//! its person may not enter (`session_id` or `session`, in the query or at
//! the top of a JSON body, any method) is refused there.

/// The ways into the day a page reads by URL, and a person's notes: each is
/// answered for the person on this channel, never for one the page names.
pub(crate) const PERSON_PATHS: &[&str] = &[
    "/api/sessions/day",
    "/api/skill-sessions",
    "/api/skill-sessions/state",
    "/api/workspace/state",
    "/api/notes",
    "/api/pending-ask-user",
];

/// The ways into a session a page sends a body to: each body is sent as
/// this channel's person, whoever the page names (the control messages
/// `chat`, `plan_*`, … reach the same routes and are sent the same way).
pub(crate) const PERSON_BODIES: &[&str] = &[
    "/api/sessions",
    "/api/chat",
    "/api/chat/clear",
    "/api/chat/compact",
    "/api/plan/approve",
    "/api/plan/reject",
    "/api/plan/edit",
    "/api/ask-user-response",
    "/api/agent-cancel",
    "/api/queue/steer",
    "/api/yinyue/chat",
    "/api/yinyue/event",
    "/api/presence",
    "/api/topic/publish",
];

/// The names a request gives a session by: [`send_as`] looks for them in
/// the query and at the top of a JSON body. A route that reads a session
/// from anywhere else fails the lock test below until it is brought under
/// the seam.
pub(crate) const SESSION_KEYS: &[&str] = &["session_id", "session"];

/// A request refused at the seam: it names `0`, a day its person may not
/// enter.
#[derive(Debug, PartialEq)]
pub(crate) struct NotTheirs(pub(crate) String);

impl NotTheirs {
    /// The words a refused caller is answered with.
    pub(crate) fn reason(&self) -> &'static str {
        "that day is another person's"
    }
}

/// Why the seam refused a request.
#[derive(Debug, PartialEq)]
pub(crate) enum Refused {
    /// It names a day its person may not enter.
    NotTheirs(NotTheirs),
    /// Its route's class (`routes::classes`) is above the caller's level.
    BeyondLevel {
        level: PermissionMode,
        needs: PermissionMode,
    },
    /// Its path is not the one it would be sent as
    /// (`routes::classes::sent_unchanged`).
    BadPath,
}

impl Refused {
    /// The fact a refused caller is answered with, as JSON: the day rule's
    /// words, or `{"error":"beyond_level","level":…,"needs":…}` — the
    /// companion words that one; code never does.
    pub(crate) fn body(&self) -> serde_json::Value {
        match self {
            Refused::NotTheirs(n) => serde_json::json!({ "error": n.reason() }),
            Refused::BadPath => serde_json::json!({ "error": "Invalid URL path" }),
            Refused::BeyondLevel { level, needs } => serde_json::json!({
                "error": "beyond_level",
                "level": level.to_string(),
                "needs": needs.to_string(),
            }),
        }
    }
}

/// A request from off this machine judged by its caller's standing, then
/// sent as its person ([`send_as`]): refused ([`Refused::BeyondLevel`]) when
/// its route's class is above the standing's level. The one seam the
/// channel's requests and frames and the LAN gate's requests pass.
pub(crate) fn send_judged(
    method: &str,
    url_path: &str,
    body: &mut serde_json::Value,
    standing: &Standing,
    user_type: &str,
) -> Result<String, Refused> {
    if !crate::server::routes::classes::sent_unchanged(url_path) {
        return Err(Refused::BadPath);
    }
    let path = url_path.split_once('?').map_or(url_path, |(p, _)| p);
    let needs = crate::server::routes::classes::class_of(method, path).needs();
    if needs > standing.level() {
        return Err(Refused::BeyondLevel {
            level: standing.level(),
            needs,
        });
    }
    send_as(method, url_path, body, (standing.person(), user_type)).map_err(Refused::NotTheirs)
}

/// `body` sent as `person`, of `user_type`: what the page put there is
/// replaced.
pub(crate) fn as_person(body: &mut serde_json::Value, person: &str, user_type: &str) {
    if !body.is_object() {
        *body = serde_json::json!({});
    }
    body["user_id"] = serde_json::Value::String(person.to_string());
    body["user_type"] = serde_json::Value::String(user_type.to_string());
}

/// `url_path` asking for this channel's person when it is a way into the
/// day: any `user_id` the page put there is replaced by the channel's own.
pub(crate) fn with_person(url_path: &str, user_id: &str) -> String {
    let (path, query) = url_path.split_once('?').unwrap_or((url_path, ""));
    if !PERSON_PATHS.contains(&path) {
        return url_path.to_string();
    }
    let mut pairs: Vec<&str> = query
        .split('&')
        .filter(|p| !p.is_empty() && !names_user_id(p))
        .collect();
    let person = format!("user_id={}", urlencoding::encode(user_id));
    pairs.push(&person);
    format!("{path}?{}", pairs.join("&"))
}

/// Whether a query pair is a `user_id`, however its key is written.
fn names_user_id(pair: &str) -> bool {
    let key = pair.split('=').next().unwrap_or(pair);
    urlencoding::decode(key).is_ok_and(|k| k == "user_id")
}

/// A request into a session sent as `person`, of `user_type`: refused
/// ([`NotTheirs`]) when it names a day that person may not enter; else a
/// way into the day by URL asks for that person, a way in by body (any
/// method but GET) carries them, and any `user_id` the caller put in a
/// body is theirs. Returns the URL to send.
pub(crate) fn send_as(
    method: &str,
    url_path: &str,
    body: &mut serde_json::Value,
    (person, user_type): (&str, &str),
) -> Result<String, NotTheirs> {
    let (path, query) = url_path.split_once('?').unwrap_or((url_path, ""));
    if let Some(day) = named_sessions(query, body).find(|sid| !crate::server::day_session::may_enter(sid, Some(person))) {
        return Err(NotTheirs(day));
    }
    let takes = takes_body(method, path);
    let url = with_person(url_path, person);
    if takes {
        as_person(body, person, user_type);
    } else if body.get("user_id").is_some() {
        body["user_id"] = serde_json::Value::String(person.to_string());
    }
    Ok(url)
}

/// The sessions a request names, under [`SESSION_KEYS`]: in its query, and
/// at the top of its body.
fn named_sessions<'a>(query: &'a str, body: &'a serde_json::Value) -> impl Iterator<Item = String> + 'a {
    let in_query = query.split('&').filter_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        let key = urlencoding::decode(key).ok()?;
        SESSION_KEYS.contains(&key.as_ref()).then(|| urlencoding::decode(value).ok().map(|v| v.into_owned()))?
    });
    let in_body = SESSION_KEYS
        .iter()
        .filter_map(|key| body.get(*key).and_then(|v| v.as_str()).map(str::to_string));
    in_query.chain(in_body)
}

/// Whether a request to `path` carries its person in its body.
pub(crate) fn takes_body(method: &str, path: &str) -> bool {
    method != "GET" && PERSON_BODIES.contains(&path)
}

use crate::engine::permission::PermissionMode;

/// Who a request from off this machine is for, and the level it stands at:
/// the one answer every check point takes (a request, a frame, the media
/// channel, a session stream). Made only here ([`standing_of`],
/// [`unnamed`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Standing {
    person: String,
    level: PermissionMode,
}

impl Standing {
    /// The person: the owner's id, another account, a device stamp, or no
    /// one (`rtc::NO_ONE`).
    pub(crate) fn person(&self) -> &str {
        &self.person
    }

    /// The most it may do.
    pub(crate) fn level(&self) -> PermissionMode {
        self.level
    }

    /// The owner's standing.
    pub(crate) fn is_owner(&self) -> bool {
        self.level == PermissionMode::Admin
    }
}

/// The most a device that is not the owner's may be raised to on this Mac.
/// Admin — settings, the shell, everything — is the owner's alone.
pub(crate) const MOST_FOR_ANOTHER: PermissionMode = PermissionMode::Edit;

/// A device's level when nobody has set one on this Mac.
pub(crate) const DEVICE_DEFAULT: PermissionMode = PermissionMode::Read;

/// The standing of paired device `device`: the owner's device (an approved
/// machine, or a phone whose claim of this Mac's account is verified,
/// `PairedDevice::is_owners`) is the owner at admin; any other device is its
/// person (memory's own seam, `api::memory::person_behind_for`) at the level
/// set for it on this Mac, read when none is; a device not paired is no one
/// at chat. A phone out of step with this Mac (`rtc::contract::behind`)
/// stands at chat at most, whoever it is.
pub(crate) fn standing_of(device: &str) -> Standing {
    let mac = crate::account::load_account().and_then(|a| a.user_id);
    let devices = crate::state_fs::devices::load_devices();
    let owner = crate::server::rtc::UserContext::owner(mac.clone()).user_id;
    standing_in(devices.iter().find(|d| d.id == device), mac.as_deref(), &owner)
}

/// [`standing_of`] for a row found (or not), with this Mac's account
/// (`mac`) and the owner's person id (`owner`).
fn standing_in(
    device: Option<&crate::state_fs::devices::PairedDevice>,
    mac: Option<&str>,
    owner: &str,
) -> Standing {
    let Some(device) = device else {
        return unnamed(false, "");
    };
    let (person, level) = if device.is_owners(mac) {
        (owner.to_string(), PermissionMode::Admin)
    } else {
        let person = crate::server::api::memory::person_behind_for(device, mac)
            .unwrap_or_else(|| crate::server::rtc::NO_ONE.to_string());
        (person, device.permission.unwrap_or(DEVICE_DEFAULT).min(MOST_FOR_ANOTHER))
    };
    Standing {
        person,
        level: level.min(most_for_contract(device)),
    }
}

/// The most a device may do for the contract it speaks: chat for a phone
/// out of step with this Mac; no bound for a machine, which speaks none.
fn most_for_contract(device: &crate::state_fs::devices::PairedDevice) -> PermissionMode {
    let phone = device.kind() == crate::state_fs::devices::DeviceKind::Phone;
    if phone && crate::server::rtc::contract::behind(device.contract.unwrap_or(0)) {
        return PermissionMode::Chat;
    }
    PermissionMode::Admin
}

/// The standing of a peer that has named no device: its channel's user at
/// admin when the tunnel knows it as the owner (`owner_known`: this
/// machine's page, the owner's account on the relay), else no one at chat.
pub(crate) fn unnamed(owner_known: bool, user_id: &str) -> Standing {
    if owner_known {
        return Standing {
            person: user_id.to_string(),
            level: PermissionMode::Admin,
        };
    }
    Standing {
        person: crate::server::rtc::NO_ONE.to_string(),
        level: PermissionMode::Chat,
    }
}

/// The most of a body the LAN gate reads to send a request as its person:
/// the framework's own limit for a route's body (axum's `DefaultBodyLimit`,
/// which no route here changes).
const LAN_BODY_LIMIT: usize = 2 * 1024 * 1024;

/// The LAN gate's request from paired device `device`, sent as its person
/// (as the channel sends a peer's): the URL's `user_id` and a JSON body's
/// replaced, and refused (403) when it names another person's day. A body
/// that is not a JSON object is passed through untouched — it names no
/// session and carries no person. A body read under the framework's limit
/// and over it is answered as the framework answers it (413); one that says
/// it is longer goes on unread, for its route to read under its own limit.
pub(crate) async fn lan_request_as(
    req: axum::extract::Request,
    device: &str,
) -> Result<axum::extract::Request, axum::response::Response> {
    use axum::extract::FromRequest;
    use axum::http::header::CONTENT_LENGTH;
    use axum::response::IntoResponse;
    let standing = standing_of(device);
    let user_type = crate::server::rtc::PeerKind::Paired.as_str();
    let (mut parts, body) = req.into_parts();
    let url_path = parts.uri.path_and_query().map_or("/", |pq| pq.as_str()).to_string();
    let mut none = serde_json::Value::Null;
    if declared_length(&parts.headers).is_some_and(|n| n > LAN_BODY_LIMIT) {
        let url = send_judged(parts.method.as_str(), &url_path, &mut none, &standing, user_type)
            .map_err(refused)?;
        set_uri(&mut parts, &url);
        stamp_level(&mut parts, standing.level());
        return Ok(axum::extract::Request::from_parts(parts, body));
    }
    let read = axum::extract::Request::from_parts(parts.clone(), body);
    let bytes = axum::body::Bytes::from_request(read, &())
        .await
        .map_err(IntoResponse::into_response)?;
    let mut value = match serde_json::from_slice::<serde_json::Value>(&bytes) {
        Ok(v) if v.is_object() => Some(v),
        _ => None,
    };
    let url = send_judged(
        parts.method.as_str(),
        &url_path,
        value.as_mut().unwrap_or(&mut none),
        &standing,
        user_type,
    )
    .map_err(refused)?;
    set_uri(&mut parts, &url);
    stamp_level(&mut parts, standing.level());
    let bytes = match value {
        Some(v) => serde_json::to_vec(&v).map(axum::body::Bytes::from).unwrap_or(bytes),
        None => bytes,
    };
    parts.headers.insert(CONTENT_LENGTH, bytes.len().into());
    Ok(axum::extract::Request::from_parts(parts, axum::body::Body::from(bytes)))
}

/// The length a request says its body is.
fn declared_length(headers: &axum::http::HeaderMap) -> Option<usize> {
    headers
        .get(axum::http::header::CONTENT_LENGTH)?
        .to_str()
        .ok()?
        .parse()
        .ok()
}

/// The level a request was judged at, over whatever the caller sent
/// (`devices::LEVEL_HEADER`).
fn stamp_level(parts: &mut axum::http::request::Parts, level: PermissionMode) {
    if let Ok(value) = level.to_string().parse() {
        parts.headers.insert(crate::server::api::pair::LEVEL_HEADER, value);
    }
}

fn set_uri(parts: &mut axum::http::request::Parts, url: &str) {
    if let Ok(uri) = url.parse() {
        parts.uri = uri;
    }
}

/// A request refused at the seam, answered 403 as the channel answers it.
fn refused(refused: Refused) -> axum::response::Response {
    use axum::response::IntoResponse;
    (axum::http::StatusCode::FORBIDDEN, axum::Json(refused.body())).into_response()
}

#[cfg(test)]
mod tests {
    /// A request is sent as its person: a `user_id` written another way is
    /// dropped too, a body is the person's on a way in by body, and any
    /// `user_id` a body names is theirs.
    #[test]
    fn a_request_is_sent_as_its_person() {
        let mut body = serde_json::json!({"user_id": "u-owner", "message": "hi"});
        let url = super::send_as("POST", "/api/chat", &mut body, ("u1", "paired"));
        assert_eq!(url.as_deref(), Ok("/api/chat"));
        assert_eq!(body["user_id"], "u1");
        assert_eq!(body["user_type"], "paired");
        let mut other = serde_json::json!({"user_id": "x"});
        super::send_as("POST", "/api/config", &mut other, ("u1", "paired")).unwrap();
        assert_eq!(other["user_id"], "u1");
        let mut plain = serde_json::json!({"a": 1});
        super::send_as("POST", "/api/config", &mut plain, ("u1", "paired")).unwrap();
        assert!(plain.get("user_id").is_none());
        assert_eq!(
            super::with_person("/api/sessions/day?user%5Fid=u-owner", "u1"),
            "/api/sessions/day?user_id=u1"
        );
    }

    /// A day its person may not enter is refused wherever the request names
    /// it — the query or the top of the body, any method, any route; their
    /// own day and other sessions pass.
    #[test]
    fn a_day_not_theirs_is_refused_at_the_seam() {
        let day = "sess-day-2026-11-16";
        let refused = |method: &str, url: &str, mut body: serde_json::Value| {
            super::send_as(method, url, &mut body, ("u-friend", "paired")).is_err()
        };
        assert!(refused("GET", &format!("/api/sessions/images?session_id={day}&name=x"), serde_json::Value::Null));
        assert!(refused("GET", "/api/chat/system-prompt?session%5Fid=sess-day-2026-11-16", serde_json::Value::Null));
        assert!(refused("DELETE", "/api/sessions/all", serde_json::json!({"session_id": day})));
        assert!(refused("POST", "/api/yinyue/event", serde_json::json!({"session": day})));
        assert!(!refused("POST", "/api/chat", serde_json::json!({"session_id": "sess-1-a"})));
        // Their own day: `u-friend` in hex.
        let theirs = "sess-day-2026-11-16-p-752d667269656e64";
        assert!(!refused("POST", "/api/chat/clear", serde_json::json!({"session_id": theirs})));
    }

    fn rust_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                rust_files(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }

    /// The routes that find their session from another id, each asking the
    /// day rule itself (`day_session::may_enter`) with the person the seam
    /// put in its body.
    const BY_ANOTHER_ID: &[(&str, &str)] = &[
        ("src/server/chat/admin.rs", "a question's id: may_answer"),
        ("src/server/api/agents.rs", "a run's id: cancel_agent_run"),
        ("src/server/claude/mod.rs", "a queued message's id: steer_queued"),
    ];

    /// A session id from off this machine reaches a route only where the
    /// seam looks ([`super::SESSION_KEYS`], top level). A request field
    /// that names a session any other way, a route path with a session in
    /// it, or a header read for one fails here until it is brought under
    /// the seam (or, finding its session from another id, is listed above).
    #[test]
    fn a_session_id_reaches_a_route_only_where_the_seam_looks() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut files = Vec::new();
        rust_files(&root.join("src/server"), &mut files);
        let field = regex::Regex::new(r"(?m)^\s*(?:pub(?:\([a-z]+\))?\s+)?([a-z_]*session[a-z_]*)\s*:").unwrap();
        let route = regex::Regex::new(r#"\.route\(\s*"[^"]*\{[^}]*session"#).unwrap();
        let header = regex::Regex::new(r#"headers?\.get\(\s*"[^"]*session"#).unwrap();
        let mut wrong = Vec::new();
        for file in files {
            let rel = file.strip_prefix(root).unwrap().to_string_lossy().to_string();
            let text = std::fs::read_to_string(&file).unwrap();
            for (start, body) in request_structs(&text) {
                for cap in field.captures_iter(body) {
                    let name = &cap[1];
                    if !super::SESSION_KEYS.contains(&name) {
                        wrong.push(format!("{rel}:{start} field `{name}`"));
                    }
                }
            }
            if route.is_match(&text) || header.is_match(&text) {
                wrong.push(format!("{rel}: a session in a route path or a header"));
            }
        }
        for (file, _) in BY_ANOTHER_ID {
            assert!(root.join(file).exists(), "{file}");
        }
        assert!(wrong.is_empty(), "bring these under person::send_as: {wrong:#?}");
    }

    /// Each `#[derive(.. Deserialize ..)] struct { .. }` in `text`: its line
    /// and its body.
    fn request_structs(text: &str) -> Vec<(usize, &str)> {
        let derive = regex::Regex::new(r"#\[derive\([^)]*Deserialize[^)]*\)\]").unwrap();
        let mut out = Vec::new();
        for m in derive.find_iter(text) {
            let rest = &text[m.end()..];
            let Some(open) = rest.find(|c| c == '{' || c == ';') else { continue };
            let head = &rest[..open];
            if !head.contains("struct ") || rest.as_bytes()[open] != b'{' {
                continue;
            }
            let Some(close) = rest[open..].find("\n}") else { continue };
            let line = text[..m.start()].lines().count() + 1;
            out.push((line, &rest[open..open + close]));
        }
        out
    }

    /// A page reads the day for the person on its channel: a `user_id` it
    /// names is replaced, and other paths are left alone.
    fn phone(account: Option<&str>) -> crate::state_fs::devices::PairedDevice {
        crate::state_fs::devices::PairedDevice {
            id: "dev-1".into(),
            kind: Some(crate::state_fs::devices::DeviceKind::Phone),
            account: account.map(|id| crate::state_fs::devices::AccountRef {
                id: id.into(),
                email: None,
                name: None,
                at: None,
            }),
            contract: Some(crate::server::rtc::contract::CONTRACT),
            ..Default::default()
        }
    }

    /// A phone below this Mac's oldest contract (one that never said, too)
    /// stands at chat at most, owner's or not; a machine is not a phone.
    #[test]
    fn a_phone_out_of_step_stands_at_chat() {
        use crate::engine::permission::PermissionMode::*;
        let old = crate::state_fs::devices::PairedDevice {
            contract: None,
            ..phone(Some("u-friend"))
        };
        assert_eq!(super::standing_in(Some(&old), Some("u-owner"), "u-owner").level(), Chat);
        let owners_old = crate::state_fs::devices::PairedDevice {
            contract: Some(0),
            verified_account: Some("u-owner".into()),
            ..phone(Some("u-owner"))
        };
        let s = super::standing_in(Some(&owners_old), Some("u-owner"), "u-owner");
        assert_eq!((s.person(), s.level()), ("u-owner", Chat));
        let machine = crate::state_fs::devices::PairedDevice {
            kind: Some(crate::state_fs::devices::DeviceKind::Machine),
            contract: None,
            ..phone(None)
        };
        assert_eq!(super::standing_in(Some(&machine), None, "__local__").level(), Admin);
    }

    /// The owner's device is the owner at admin: an approved machine, and a
    /// phone whose claim of this Mac's account is verified.
    #[test]
    fn the_owners_device_stands_as_the_owner() {
        use crate::engine::permission::PermissionMode::*;
        let verified = crate::state_fs::devices::PairedDevice {
            verified_account: Some("u-owner".into()),
            ..phone(Some("u-owner"))
        };
        let s = super::standing_in(Some(&verified), Some("u-owner"), "u-owner");
        assert_eq!((s.person(), s.level()), ("u-owner", Admin));
        let machine = crate::state_fs::devices::PairedDevice {
            kind: Some(crate::state_fs::devices::DeviceKind::Machine),
            ..phone(None)
        };
        assert_eq!(super::standing_in(Some(&machine), None, "__local__").level(), Admin);
    }

    /// Any other device stands at the level set for it, read when none is,
    /// never above edit; an unverified claim of the owner's account is its
    /// device; a device not paired is no one at chat.
    #[test]
    fn another_device_stands_at_its_level() {
        use crate::engine::permission::PermissionMode::*;
        let claimed = super::standing_in(Some(&phone(Some("u-owner"))), Some("u-owner"), "u-owner");
        assert_eq!((claimed.person(), claimed.level()), ("device:dev-1", Read));
        assert!(!crate::server::day_session::is_builder(Some(claimed.person())), "not the builder's day");
        let friend = super::standing_in(Some(&phone(Some("u-friend"))), Some("u-owner"), "u-owner");
        assert_eq!((friend.person(), friend.level()), ("u-friend", Read));
        let raised = crate::state_fs::devices::PairedDevice {
            permission: Some(Edit),
            ..phone(Some("u-friend"))
        };
        assert_eq!(super::standing_in(Some(&raised), Some("u-owner"), "u-owner").level(), Edit);
        let over = crate::state_fs::devices::PairedDevice {
            permission: Some(Admin),
            ..phone(Some("u-friend"))
        };
        assert_eq!(super::standing_in(Some(&over), Some("u-owner"), "u-owner").level(), Edit);
        let lowered = crate::state_fs::devices::PairedDevice {
            permission: Some(Chat),
            ..phone(None)
        };
        assert_eq!(super::standing_in(Some(&lowered), Some("u-owner"), "u-owner").level(), Chat);
        let nobody = super::standing_in(None, Some("u-owner"), "u-owner");
        assert_eq!((nobody.person(), nobody.level()), (crate::server::rtc::NO_ONE, Chat));
    }

    /// A peer that named no device is the owner only when the tunnel knows it.
    #[test]
    fn an_unnamed_peer_is_the_owner_only_when_known() {
        use crate::engine::permission::PermissionMode::*;
        assert_eq!(super::unnamed(true, "u-owner").level(), Admin);
        let stranger = super::unnamed(false, "u-owner");
        assert_eq!((stranger.person(), stranger.level()), (crate::server::rtc::NO_ONE, Chat));
    }

    fn at(level: crate::engine::permission::PermissionMode) -> super::Standing {
        super::Standing {
            person: "u-friend".into(),
            level,
        }
    }

    /// A device at read: a read route passes; an edit route, the raw shell
    /// and a `clear` or plan's route are refused with its level and what the
    /// route needs; raised to edit, the edit route passes.
    #[test]
    fn a_route_above_the_level_is_refused() {
        use crate::engine::permission::PermissionMode::*;
        let judged = |method: &str, url: &str, level| {
            super::send_judged(method, url, &mut serde_json::json!({}), &at(level), "paired")
        };
        assert!(judged("GET", "/api/topic/latest?topic=x", Read).is_ok());
        assert!(judged("POST", "/api/chat", Read).is_ok());
        let refused = judged("POST", "/api/media/backup", Read).unwrap_err();
        assert_eq!(
            refused.body(),
            serde_json::json!({"error": "beyond_level", "level": "read", "needs": "edit"})
        );
        assert_eq!(
            judged("POST", "/api/bash", Read).unwrap_err().body(),
            serde_json::json!({"error": "beyond_level", "level": "read", "needs": "admin"})
        );
        for frame_route in ["/api/chat/clear", "/api/plan/approve", "/api/plan/reject", "/api/plan/edit"] {
            assert!(judged("POST", frame_route, Read).is_err(), "{frame_route}");
        }
        assert!(judged("POST", "/api/media/backup", Edit).is_ok());
        assert!(judged("POST", "/api/bash", Edit).is_err());
        assert!(judged("POST", "/api/memory/add", Chat).is_ok(), "its own records at any level");
        assert!(judged("GET", "/api/topic/latest", Chat).is_err());
        assert_eq!(judged("GET", "/assets/%2e%2e/api/credentials", Admin), Err(super::Refused::BadPath));
    }

    /// The day rule still holds under the level.
    #[test]
    fn a_day_not_theirs_is_refused_whatever_the_level() {
        let mut body = serde_json::json!({"session_id": "sess-day-2026-11-16"});
        let got = super::send_judged(
            "POST",
            "/api/chat",
            &mut body,
            &at(crate::engine::permission::PermissionMode::Edit),
            "paired",
        );
        assert!(matches!(got, Err(super::Refused::NotTheirs(_))));
    }

    #[test]
    fn a_page_reads_the_day_of_its_channels_person() {
        assert_eq!(super::with_person("/api/sessions/day", "u 1"), "/api/sessions/day?user_id=u%201");
        assert_eq!(
            super::with_person("/api/skill-sessions?skill=dice&user_id=other", "u1"),
            "/api/skill-sessions?skill=dice&user_id=u1"
        );
        assert_eq!(super::with_person("/api/sessions?x=1", "u1"), "/api/sessions?x=1");
    }
}
