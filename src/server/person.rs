//! The person a request into a session is for, as the engine is reached
//! from off this machine: the routes that take a `user_id` and how a
//! request is sent as its person. The channel's tunnel and the LAN gate
//! both send their requests through [`send_as`], with the person resolved
//! from the device through memory's own seam ([`of_device`]). It is the one
//! seam a session id from off this machine passes: a request naming a
//! session its person may not enter (`day_session::may_enter`: another
//! person's day or session; `session_id` or `session`, in the query or at
//! the top of a JSON body, any method) is refused there. [`send_as`] is
//! private: a device's request enters through [`send_judged`], a room
//! guest's through [`send_as_guest`], and no other way sends a request as
//! its person.

/// The ways into the day a page reads by URL, a person's notes and the
/// retained readings: each is answered for the person on this channel, never
/// for one the page names.
pub(crate) const PERSON_PATHS: &[&str] = &[
    "/api/sessions/day",
    "/api/skill-sessions",
    "/api/skill-sessions/state",
    "/api/workspace/state",
    "/api/notes",
    "/api/pending-ask-user",
    "/api/topic/latest",
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

/// A request refused at the seam: it names `0`, a session its person may
/// not enter.
#[derive(Debug, PartialEq)]
pub(crate) struct NotTheirs(pub(crate) String);

impl NotTheirs {
    /// The words a refused caller is answered with.
    pub(crate) fn reason(&self) -> &'static str {
        "that session is another person's"
    }
}

/// Why the seam refused a request.
#[derive(Debug, PartialEq)]
pub(crate) enum Refused {
    /// It names a session its person may not enter.
    NotTheirs(NotTheirs),
    /// It is for an app its person may not use (`people::may_use_app`).
    NotYourApp(String),
    /// Its route's class (`routes::classes`) is above the caller's level.
    BeyondLevel {
        level: PermissionMode,
        needs: PermissionMode,
    },
    /// Its path is not the one it would be sent as
    /// (`routes::classes::sent_unchanged`).
    BadPath,
    /// Its peer has named no device (no one) and the route is not the way
    /// in (`routes::classes::is_way_in`).
    Unnamed,
    /// Its JSON body is an array, a number or a boolean: a route's body type
    /// can be filled from an array by position, naming a session where the
    /// seam does not look.
    BadBody,
}

impl Refused {
    /// The fact a refused caller is answered with, as JSON: the day rule's
    /// words, or `{"error":"beyond_level","level":…,"needs":…}` — the
    /// companion words that one; code never does.
    pub(crate) fn body(&self) -> serde_json::Value {
        match self {
            Refused::NotTheirs(n) => serde_json::json!({ "error": n.reason() }),
            Refused::NotYourApp(app) => serde_json::json!({ "error": "not_your_app", "app": app }),
            Refused::BadPath => serde_json::json!({ "error": "Invalid URL path" }),
            Refused::Unnamed => serde_json::json!({ "error": "unnamed" }),
            Refused::BadBody => serde_json::json!({ "error": "a JSON request body must be an object" }),
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
/// its route's class is above the standing's level, and — for a peer that
/// has named no device — anywhere but the way in ([`Refused::Unnamed`]). The one seam the
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
    if standing.person().is_no_one() && !crate::server::routes::classes::is_way_in(method, path) {
        return Err(Refused::Unnamed);
    }
    let needs = crate::server::routes::classes::class_of(method, path).needs();
    if needs > standing.level() {
        return Err(Refused::BeyondLevel {
            level: standing.level(),
            needs,
        });
    }
    if crate::server::routes::classes::app_named(method, url_path, body).is_some() {
        judge_app(&crate::server::people::current(), (method, url_path), body, standing.person())?;
    }
    send_as(method, url_path, body, (standing.person(), user_type))
}

/// A request for an app (`routes::classes::app_named`) from `person`:
/// refused ([`Refused::NotYourApp`]) when `people` do not let them use it.
/// The one way into an app by a request — its page files, its tools, its
/// sync, its save, its chat, its launch — asks [`People::may_use_app`] here.
fn judge_app(
    people: &crate::server::people::People,
    (method, url_path): (&str, &str),
    body: &serde_json::Value,
    person: &Person,
) -> Result<(), Refused> {
    match crate::server::routes::classes::app_named(method, url_path, body) {
        Some(app) if !people.may_use_app(person, &app) => Err(Refused::NotYourApp(app)),
        _ => Ok(()),
    }
}

/// A room guest's request sent as its person: its path as it is sent
/// ([`Refused::BadPath`]), then the same [`send_as`] a device's passes — so a
/// guest, like a device, names only sessions that are theirs. Its route was
/// admitted by the room gate (`routes::classes::tunnel_allows`) before it
/// came here; the room's levels are not the route classes' (`ROOM`).
pub(crate) fn send_as_guest(
    method: &str,
    url_path: &str,
    body: &mut serde_json::Value,
    standing: &Standing,
    user_type: &str,
) -> Result<String, Refused> {
    if !crate::server::routes::classes::sent_unchanged(url_path) {
        return Err(Refused::BadPath);
    }
    send_as(method, url_path, body, (standing.person(), user_type))
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
/// ([`Refused::BadBody`]) when its body is an array or a bare number or
/// boolean, as the LAN gate refuses it too ([`lan_body`]); refused
/// ([`Refused::NotTheirs`]) when it names a session that person may not
/// enter; else a
/// way into the day by URL asks for that person, a way in by body (any
/// method but GET) carries them, and any `user_id` the caller put in a
/// body is theirs. Returns the URL to send.
fn send_as(
    method: &str,
    url_path: &str,
    body: &mut serde_json::Value,
    (person, user_type): (&Person, &str),
) -> Result<String, Refused> {
    if body.is_array() || body.is_number() || body.is_boolean() {
        return Err(Refused::BadBody);
    }
    let (path, query) = url_path.split_once('?').unwrap_or((url_path, ""));
    if let Some(day) = named_sessions(query, body).find(|sid| !crate::server::day_session::may_enter(sid, person)) {
        return Err(Refused::NotTheirs(NotTheirs(day)));
    }
    let id = person.wire();
    let takes = takes_body(method, path);
    let url = with_person(url_path, &id);
    if takes {
        as_person(body, &id, user_type);
    } else if body.get("user_id").is_some() {
        body["user_id"] = serde_json::Value::String(id);
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

mod who;
pub(crate) use who::{mac_account, pairing_wire, scan_owner_ids, stamp_of, Person};

/// Who a request from off this machine is for, and the level it stands at:
/// the one answer every check point takes (a request, a frame, the media
/// channel, a session stream). Made only here ([`standing_of`],
/// [`consumer`], [`unnamed`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Standing {
    person: Person,
    level: PermissionMode,
}

impl Standing {
    /// The person: the owner, another account, a device, or no one.
    pub(crate) fn person(&self) -> &Person {
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

/// The owner: this machine, and what the engine starts itself.
pub(crate) fn owner() -> Person {
    Person::owner()
}

/// No one: a peer that has not said which device it is.
pub(crate) fn no_one() -> Person {
    Person::no_one()
}

/// The standing of paired device `device`: the owner's device (an approved
/// machine, or a phone whose claim of this Mac's account is verified,
/// `PairedDevice::is_owners`) is the owner at admin; any other device is its
/// person ([`of_device`], the answer memory stamps its rows with) at the level
/// set for it on this Mac, read when none is; a device not paired is no one
/// at chat. A phone out of step with this Mac (`rtc::contract::behind`)
/// stands at chat at most, whoever it is.
pub(crate) fn standing_of(device: &str) -> Standing {
    let mac = mac_account();
    let devices = crate::state_fs::devices::load_devices();
    standing_in(devices.iter().find(|d| d.id == device), mac.as_deref(), &crate::server::people::current())
}

/// Who paired device row `device` is now (`of_device` with this Mac's account
/// as it is).
pub(crate) fn of_row(device: &crate::state_fs::devices::PairedDevice) -> Person {
    of_device(device, mac_account().as_deref())
}

/// Who the device holding `token` is now; `None` for a token no row holds.
pub(crate) fn of_token(token: &str) -> Option<Person> {
    crate::state_fs::devices::device_by_token(token).map(|d| of_row(&d))
}

/// Who a request that reached a handler (over loopback, from the channel
/// or the LAN gate) is from: the paired device it names, else — by what the
/// tunnel marked it with — the owner when it is this machine's own page or
/// the owner's relay login (`TUNNEL_LOCAL`, `TUNNEL_OWNER`), else no one. A
/// request with no mark and no level is straight to loopback: the owner's.
/// The channel stamps a level on every request, the owner's page included,
/// so the level says nothing of who asks. For a handler that answers by
/// person (a list of apps, of models).
pub(crate) fn of_headers(headers: &axum::http::HeaderMap) -> Person {
    use crate::server::api::pair::{LEVEL_HEADER, TUNNEL_HEADER, TUNNEL_LOCAL, TUNNEL_OWNER};
    if let Some(device) = crate::server::api::pair::caller_device(headers) {
        return standing_of(&device).person().clone();
    }
    let mark = headers.get(TUNNEL_HEADER).and_then(|v| v.to_str().ok());
    match mark {
        Some(TUNNEL_LOCAL | TUNNEL_OWNER) => owner(),
        Some(_) => no_one(),
        None if headers.contains_key(LEVEL_HEADER) => no_one(),
        None => owner(),
    }
}

/// Who a request's `user_id` names: that person, and no one when it names
/// none — a request from off this machine always carries one
/// ([`send_judged`]), so a missing id is not the owner's.
pub(crate) fn asking(user_id: Option<&str>) -> Person {
    match user_id.map(str::trim).filter(|id| !id.is_empty()) {
        Some(id) => Person::from_wire(Some(id)),
        None => no_one(),
    }
}

/// What the relay has not yet vouched for in a phone's claim (what the
/// greeting tells it, so it can dial the relay once; the account itself is
/// never told).
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) enum Unvouched {
    /// Nothing: not a phone, signed out, or the claim is vouched for.
    No,
    /// It claims an account the relay has not vouched for.
    Claim,
    /// It claims this Mac's own account, not vouched for.
    OwnersClaim,
}

/// The [`Unvouched`] of paired device `device`.
pub(crate) fn unvouched_claim(device: &str) -> Unvouched {
    let mac = mac_account();
    let devices = crate::state_fs::devices::load_devices();
    devices
        .iter()
        .find(|d| d.id == device)
        .map_or(Unvouched::No, |d| unvouched_claim_in(d, mac.as_deref()))
}

/// [`unvouched_claim`] for a row, with this Mac's account `mac`. A claim that
/// is not an account's own id ([`names_an_account`]) is no claim.
fn unvouched_claim_in(device: &crate::state_fs::devices::PairedDevice, mac: Option<&str>) -> Unvouched {
    let phone = device.kind() == crate::state_fs::devices::DeviceKind::Phone;
    let Some(claim) = device.account.as_ref().map(|a| a.id.as_str()) else {
        return Unvouched::No;
    };
    if !phone || device.verified_claim().is_some() {
        return Unvouched::No;
    }
    if mac.is_some_and(|m| !m.is_empty() && m == claim) {
        return Unvouched::OwnersClaim;
    }
    match names_an_account(claim, mac) {
        true => Unvouched::Claim,
        false => Unvouched::No,
    }
}

/// Whose paired device `device` is, as Settings shows it, with this Mac's
/// account `mac`: `owner` (an approved machine, or a phone whose claim of
/// this Mac's account is vouched for), `unverified` (it claims an account
/// the relay has not vouched for), `other` (another account, vouched for),
/// `signed_out` (no account of its own, [`of_device`]).
pub(crate) fn whose_device(device: &crate::state_fs::devices::PairedDevice, mac: Option<&str>) -> &'static str {
    let person = of_device(device, mac);
    if person.is_owner() {
        return "owner";
    }
    if unvouched_claim_in(device, mac) != Unvouched::No {
        return "unverified";
    }
    match person.is_device() {
        true => "signed_out",
        false => "other",
    }
}

/// Whether paired device row `device` stands as the owner's with this Mac's
/// account `mac` (what [`standing_of`] answers for it).
pub(crate) fn is_owners_device(device: &crate::state_fs::devices::PairedDevice, mac: Option<&str>) -> bool {
    // The owner's standing never asks the people file: no settings are read.
    standing_in(Some(device), mac, &crate::server::people::People::default()).is_owner()
}

/// Who paired device `device` is, with this Mac's account `mac`: the owner
/// for the owner's device (an approved machine, or a phone whose claim of
/// this Mac's account the relay vouched for, `PairedDevice::is_owners`); else
/// the account the relay vouched for (`PairedDevice::verified_claim`); else
/// its device — signed out, or claiming an account the relay has not vouched
/// for, or an id that is not an account's own ([`names_an_account`]). The one
/// answer a device's standing, its day and memory's stamp take.
pub(crate) fn of_device(device: &crate::state_fs::devices::PairedDevice, mac: Option<&str>) -> Person {
    if device.is_owners(mac) {
        return Person::owner();
    }
    match device.verified_claim() {
        Some(id) if names_an_account(id, mac) => Person::account(id),
        _ => Person::device(&device.id),
    }
}

/// Whether `id` is kept as another account's own id with this Mac's account
/// `mac` — what a phone's claim must be to be that account.
pub(crate) fn names_an_account(id: &str, mac: Option<&str>) -> bool {
    Person::names_an_account(id, mac)
}

/// [`standing_of`] for a row found (or not), with this Mac's account
/// (`mac`) and the household's settings (`people`): the level is the
/// person's, so two phones of one account stand at one.
fn standing_in(
    device: Option<&crate::state_fs::devices::PairedDevice>,
    mac: Option<&str>,
    people: &crate::server::people::People,
) -> Standing {
    let Some(device) = device else {
        return unnamed(false);
    };
    let person = of_device(device, mac);
    let level = match person.is_owner() {
        true => PermissionMode::Admin,
        false => people.level_of(&person),
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

/// The standing of a proxy room's consumer: its own account, at chat (its
/// RTC HTTP and media requests use `routes::classes::tunnel_allows`); no one
/// when its id is not an account's own ([`names_an_account`]: none, this
/// Mac's owner's, no one's, a device's).
pub(crate) fn consumer(user_id: &str) -> Standing {
    consumer_in(user_id, mac_account().as_deref())
}

/// [`consumer`] with this Mac's account `mac`.
fn consumer_in(user_id: &str, mac: Option<&str>) -> Standing {
    let person = match names_an_account(user_id, mac) {
        true => Person::account(user_id),
        false => Person::no_one(),
    };
    Standing {
        person,
        level: PermissionMode::Chat,
    }
}

/// A session kept in the process's store for `user` (`None`: the builder),
/// for tests elsewhere that name sessions.
#[cfg(test)]
pub(crate) fn keep_session_for_test(id: &str, user: Option<&str>) {
    let mut meta = crate::state_fs::sessions::SessionMeta::default();
    meta.id = id.into();
    meta.user_id = user.map(str::to_string);
    crate::state_fs::SessionStore::global().add_session(&meta).unwrap();
}

/// A standing of `person` at `level`, for tests elsewhere.
#[cfg(test)]
pub(crate) fn for_test(person: &str, level: PermissionMode) -> Standing {
    Standing {
        person: Person::for_test(Some(person)),
        level,
    }
}

/// The standing of a peer that has named no device: the owner at admin
/// when the tunnel knows it as the owner (`owner_known`: this machine's
/// page, the owner's account on the relay), else no one at chat.
pub(crate) fn unnamed(owner_known: bool) -> Standing {
    if owner_known {
        return Standing {
            person: Person::owner(),
            level: PermissionMode::Admin,
        };
    }
    Standing {
        person: Person::no_one(),
        level: PermissionMode::Chat,
    }
}

/// The most of a body the LAN gate reads to send a request as its person:
/// the framework's own limit for a route's body (axum's `DefaultBodyLimit`),
/// which one route raises — `/api/stt`, a minute of audio — and that body
/// goes on unread, for its route to read under its own limit.
const LAN_BODY_LIMIT: usize = 2 * 1024 * 1024;

/// The LAN gate's request from paired device `device`, sent as its person
/// (as the channel sends a peer's): the URL's `user_id` and a JSON body's
/// replaced, and refused (403) when it names another person's day. A body
/// that is not JSON is passed through untouched — it names no session and
/// carries no person; JSON that is not an object is refused (400), see
/// [`lan_body`]. A body read under the framework's limit
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
    let mut value = match lan_body(&bytes) {
        Ok(value) => value,
        Err(()) => {
            return Err((
                axum::http::StatusCode::BAD_REQUEST,
                "a JSON request body must be an object",
            )
                .into_response())
        }
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

/// A LAN request's body as the gate may send it on (what a body may be is
/// decided in [`send_as`]; this answers 400 before the body is sent): a JSON object, to be
/// sent as its person (`Some`); bytes that are not JSON, passed through
/// untouched (`None`); JSON of any other kind, refused — a route's body
/// type can be filled from an array by position, and such a body would go
/// on carrying no person.
fn lan_body(bytes: &[u8]) -> Result<Option<serde_json::Value>, ()> {
    match serde_json::from_slice::<serde_json::Value>(bytes) {
        Ok(v) if v.is_object() => Ok(Some(v)),
        Ok(_) => Err(()),
        Err(_) => Ok(None),
    }
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

/// What only the owner's own turns reach — a session another runtime hosts
/// (its tools run under that runtime's own permissions, which no ceiling
/// reaches), the companion's own thread: refused (403, `beyond_level`, needs
/// admin) for a request judged at `level` below admin.
pub(crate) fn owners_only(level: PermissionMode) -> Option<axum::response::Response> {
    (level < PermissionMode::Admin).then(|| {
        refused(Refused::BeyondLevel {
            level,
            needs: PermissionMode::Admin,
        })
    })
}

/// A request refused at the seam, answered 403 as the channel answers it.
fn refused(refused: Refused) -> axum::response::Response {
    use axum::response::IntoResponse;
    (axum::http::StatusCode::FORBIDDEN, axum::Json(refused.body())).into_response()
}

#[cfg(test)]
mod tests {

    /// The gate sends on an object (as its person) and bytes that are not
    /// JSON; an array or a bare value is refused, since a body type can be
    /// filled from an array by position and would carry no person.
    #[test]
    fn a_lan_json_body_is_an_object_or_refused() {
        assert!(matches!(super::lan_body(br#"{"message":"hi"}"#), Ok(Some(v)) if v.is_object()));
        assert_eq!(super::lan_body(b"\x89PNG not json"), Ok(None));
        assert_eq!(super::lan_body(b""), Ok(None));
        for body in [&br#"["hi", null]"#[..], b"null", b"3", br#""text""#] {
            assert_eq!(super::lan_body(body), Err(()), "{}", String::from_utf8_lossy(body));
        }
    }
    /// A request is sent as its person: a `user_id` written another way is
    /// dropped too, a body is the person's on a way in by body, and any
    /// `user_id` a body names is theirs.
    #[test]
    fn a_request_is_sent_as_its_person() {
        let mut body = serde_json::json!({"user_id": "u-owner", "message": "hi"});
        let u1 = super::Person::for_test(Some("u1"));
        let url = super::send_as("POST", "/api/chat", &mut body, (&u1, "paired"));
        assert_eq!(url.as_deref(), Ok("/api/chat"));
        assert_eq!(body["user_id"], "u1");
        assert_eq!(body["user_type"], "paired");
        let mut other = serde_json::json!({"user_id": "x"});
        super::send_as("POST", "/api/config", &mut other, (&u1, "paired")).unwrap();
        assert_eq!(other["user_id"], "u1");
        let mut plain = serde_json::json!({"a": 1});
        super::send_as("POST", "/api/config", &mut plain, (&u1, "paired")).unwrap();
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
            super::send_as(method, url, &mut body, (&super::Person::for_test(Some("u-friend")), "paired")).is_err()
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
        ("src/server/api/agents.rs", "a run's id: cancel_agent_run; a queued message's id: steer_queued"),
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

    /// [`super::standing_in`] when nobody set anything (`people`).
    fn standing_in(
        device: Option<&crate::state_fs::devices::PairedDevice>,
        mac: Option<&str>,
    ) -> super::Standing {
        super::standing_in(device, mac, &crate::server::people::People::default())
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

    /// A phone signed in to `account`, which the relay vouched for.
    fn vouched(account: &str) -> crate::state_fs::devices::PairedDevice {
        crate::state_fs::devices::PairedDevice {
            verified_account: Some(account.into()),
            ..phone(Some(account))
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
        assert_eq!(standing_in(Some(&old), Some("u-owner")).level(), Chat);
        let owners_old = crate::state_fs::devices::PairedDevice {
            contract: Some(0),
            verified_account: Some("u-owner".into()),
            ..phone(Some("u-owner"))
        };
        let s = standing_in(Some(&owners_old), Some("u-owner"));
        assert_eq!((s.person(), s.level()), (&super::Person::owner(), Chat));
        let machine = crate::state_fs::devices::PairedDevice {
            kind: Some(crate::state_fs::devices::DeviceKind::Machine),
            contract: None,
            ..phone(None)
        };
        assert_eq!(standing_in(Some(&machine), None).level(), Admin);
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
        let s = standing_in(Some(&verified), Some("u-owner"));
        assert_eq!((s.person(), s.level()), (&super::Person::owner(), Admin));
        let machine = crate::state_fs::devices::PairedDevice {
            kind: Some(crate::state_fs::devices::DeviceKind::Machine),
            ..phone(None)
        };
        assert_eq!(standing_in(Some(&machine), None).level(), Admin);
    }

    /// Any other device stands at the level set for it, read when none is,
    /// never above edit; an unverified claim of the owner's account is its
    /// device; a device not paired is no one at chat.
    #[test]
    fn another_device_stands_at_its_level() {
        use crate::engine::permission::PermissionMode::*;
        let claimed = standing_in(Some(&phone(Some("u-owner"))), Some("u-owner"));
        assert_eq!((claimed.person().wire(), claimed.level()), ("device:dev-1".to_string(), Read));
        assert!(!crate::server::day_session::is_builder(claimed.person()), "not the builder's day");
        let friend = standing_in(Some(&vouched("u-friend")), Some("u-owner"));
        assert_eq!((friend.person().wire(), friend.level()), ("u-friend".to_string(), Read));
        let unvouched = standing_in(Some(&phone(Some("u-friend"))), Some("u-owner"));
        assert_eq!(unvouched.person(), &super::Person::device("dev-1"), "an account not vouched for");
        // The level is the person's: set for them, it holds for every phone
        // of theirs, whatever a row once said; admin is not one to give.
        let set = |person: &str, level: &str| {
            let mut people = crate::server::people::People::default();
            let patch = serde_json::json!({ "level": level });
            people.set(&super::Person::for_test(Some(person)), patch.as_object().unwrap()).map(|_| people)
        };
        let at = |people: &crate::server::people::People, d: &crate::state_fs::devices::PairedDevice| {
            super::standing_in(Some(d), Some("u-owner"), people).level()
        };
        let raised = set("u-friend", "edit").unwrap();
        assert_eq!(at(&raised, &vouched("u-friend")), Edit);
        assert!(set("u-friend", "admin").is_err(), "admin is the owner's alone");
        let lowered = set(&crate::server::person::stamp_of("dev-1"), "chat").unwrap();
        assert_eq!(at(&lowered, &phone(None)), Chat);
        assert_eq!(at(&raised, &phone(None)), Read, "another person's setting is not theirs");
        let nobody = standing_in(None, Some("u-owner"));
        assert_eq!((nobody.person(), nobody.level()), (&super::Person::no_one(), Chat));
    }

    /// A claimed account id that reads as another kind of person — none,
    /// the owner's, no one's, a device's, or padded — is no account: the
    /// phone is its device, signed in to this Mac or signed out, in its
    /// standing and in Settings (memory's stamp and the rows it wrote:
    /// `api::memory`'s own test).
    #[test]
    fn a_claim_that_reads_as_another_person_is_its_device() {
        let local = ["__", "local__"].concat();
        let paired = ["__", "paired__"].concat();
        let claims = ["", "  ", local.as_str(), paired.as_str(), "device:dev-2", " u-friend"];
        for mac in [Some("u-owner"), None] {
            for claim in claims {
                let d = phone(Some(claim));
                let s = standing_in(Some(&d), mac);
                assert_eq!(s.person(), &super::Person::device("dev-1"), "{claim:?} with {mac:?}");
                assert_eq!(super::whose_device(&d, mac), "signed_out", "{claim:?}");
            }
        }
        let friend = vouched("u-friend");
        assert_eq!(standing_in(Some(&friend), None).person(), &super::Person::account("u-friend"));
        // Vouched or not, an id that is no account's own is its device.
        for claim in claims {
            let d = crate::state_fs::devices::PairedDevice {
                verified_account: Some(claim.into()),
                ..phone(Some(claim))
            };
            assert_eq!(super::of_device(&d, Some("u-owner")), super::Person::device("dev-1"), "vouched {claim:?}");
        }
    }

    /// A claim of any account is that account only when the relay vouched
    /// for it: unvouched, the phone is its device, whichever account it
    /// names; Settings says its account is not verified.
    #[test]
    fn a_claim_is_an_account_only_when_the_relay_vouched_for_it() {
        for mac in [Some("u-owner"), None] {
            let claimed = phone(Some("u-friend"));
            assert_eq!(super::of_device(&claimed, mac), super::Person::device("dev-1"), "{mac:?}");
            assert_eq!(super::whose_device(&claimed, mac), "unverified");
            let vouched = crate::state_fs::devices::PairedDevice {
                verified_account: Some("u-friend".into()),
                ..phone(Some("u-friend"))
            };
            assert_eq!(super::of_device(&vouched, mac), super::Person::account("u-friend"), "{mac:?}");
            assert_eq!(super::whose_device(&vouched, mac), "other");
            assert!(!super::is_owners_device(&vouched, mac), "vouched is not the owner's");
        }
        // A vouch recorded for one account does not stand for a claim of another.
        let stale = crate::state_fs::devices::PairedDevice {
            verified_account: Some("u-friend".into()),
            ..phone(Some("u-pal"))
        };
        assert_eq!(super::of_device(&stale, Some("u-owner")), super::Person::device("dev-1"));
    }

    /// Any phone whose claim the relay has not vouched for is told so, to
    /// dial it once; only one claiming this Mac's account is told it as
    /// the owner's.
    #[test]
    fn a_phone_is_told_when_its_claim_is_not_vouched_for() {
        use super::Unvouched::*;
        let told = |d: &crate::state_fs::devices::PairedDevice, mac| super::unvouched_claim_in(d, mac);
        assert_eq!(told(&phone(Some("u-friend")), Some("u-owner")), Claim);
        assert_eq!(told(&phone(Some("u-friend")), None), Claim);
        assert_eq!(told(&phone(Some("u-owner")), Some("u-owner")), OwnersClaim);
        assert_eq!(told(&phone(None), Some("u-owner")), No);
        let vouched = crate::state_fs::devices::PairedDevice {
            verified_account: Some("u-friend".into()),
            ..phone(Some("u-friend"))
        };
        assert_eq!(told(&vouched, Some("u-owner")), No);
        let machine = crate::state_fs::devices::PairedDevice {
            kind: Some(crate::state_fs::devices::DeviceKind::Machine),
            ..phone(Some("u-friend"))
        };
        assert_eq!(told(&machine, Some("u-owner")), No);
    }

    /// Settings is told whose each device is.
    #[test]
    fn whose_a_device_is() {
        let mac = Some("u-owner");
        let verified = crate::state_fs::devices::PairedDevice {
            verified_account: Some("u-owner".into()),
            ..phone(Some("u-owner"))
        };
        assert_eq!(super::whose_device(&verified, mac), "owner");
        assert_eq!(super::whose_device(&phone(Some("u-owner")), mac), "unverified");
        let vouched = crate::state_fs::devices::PairedDevice {
            verified_account: Some("u-friend".into()),
            ..phone(Some("u-friend"))
        };
        assert_eq!(super::whose_device(&vouched, mac), "other");
        assert_eq!(super::whose_device(&phone(Some("u-friend")), mac), "unverified");
        assert_eq!(super::whose_device(&phone(None), mac), "signed_out");
        let machine = crate::state_fs::devices::PairedDevice {
            kind: Some(crate::state_fs::devices::DeviceKind::Machine),
            ..phone(None)
        };
        assert_eq!(super::whose_device(&machine, mac), "owner");
    }

    /// A room consumer is its own account, at chat; an id that is not an
    /// account's own (none, the owner's, no one's, a device's) is no one.
    #[test]
    fn a_room_consumer_is_its_own_account_or_no_one() {
        use crate::engine::permission::PermissionMode::*;
        let room = super::consumer("u-room");
        assert_eq!((room.person(), room.level()), (&super::Person::account("u-room"), Chat));
        let local = ["__", "local__"].concat();
        for id in ["", local.as_str(), "device:dev-1", " u-room"] {
            let s = super::consumer(id);
            assert_eq!((s.person(), s.level()), (&super::Person::no_one(), Chat), "{id:?}");
        }
        let owners = super::consumer_in("u-owner", Some("u-owner"));
        assert_eq!(owners.person(), &super::Person::no_one(), "not the owner through a room");
        assert_eq!(super::consumer_in("u-room", Some("u-owner")).person(), &super::Person::account("u-room"));
    }

    /// A list a person is shown (models, apps) is answered for who is asking.
    /// The channel stamps a level on every request it re-issues, the owner's
    /// own page included: a request this machine's page or the owner's relay
    /// login sends is the owner's (it showed the owner no built-in models), and
    /// one the tunnel does not know as the owner's is no one's.
    #[test]
    fn a_request_over_the_channel_is_the_owners_when_the_tunnel_knows_it() {
        use crate::server::api::pair::{LEVEL_HEADER, TUNNEL_HEADER, TUNNEL_LOCAL, TUNNEL_OWNER};
        let from = |pairs: &[(&'static str, &str)]| {
            let mut h = axum::http::HeaderMap::new();
            for (k, v) in pairs {
                h.insert(*k, v.parse().unwrap());
            }
            super::of_headers(&h)
        };
        assert_eq!(from(&[]), super::Person::owner(), "this machine, straight to loopback");
        assert_eq!(from(&[(TUNNEL_HEADER, TUNNEL_LOCAL), (LEVEL_HEADER, "admin")]), super::Person::owner(), "this machine's page");
        assert_eq!(from(&[(TUNNEL_HEADER, TUNNEL_OWNER), (LEVEL_HEADER, "admin")]), super::Person::owner(), "the owner on the relay");
        assert_eq!(from(&[(TUNNEL_HEADER, "1"), (LEVEL_HEADER, "chat")]), super::Person::no_one(), "a peer that named no device");
        assert_eq!(from(&[(LEVEL_HEADER, "admin")]), super::Person::no_one(), "a level with no tunnel mark");
    }

    /// A peer that named no device is the owner only when the tunnel knows it.
    #[test]
    fn an_unnamed_peer_is_the_owner_only_when_known() {
        use crate::engine::permission::PermissionMode::*;
        let known = super::unnamed(true);
        assert_eq!((known.person(), known.level()), (&super::Person::owner(), Admin));
        let stranger = super::unnamed(false);
        assert_eq!((stranger.person(), stranger.level()), (&super::Person::no_one(), Chat));
    }

    fn at(level: crate::engine::permission::PermissionMode) -> super::Standing {
        super::Standing {
            person: super::Person::for_test(Some("u-friend")),
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

    /// A peer that has named no device reaches the way in and nothing else:
    /// not a chat, not its own-records routes, not the read doors.
    #[test]
    fn an_unnamed_peer_reaches_only_the_way_in() {
        let nobody = super::unnamed(false);
        let judged = |method: &str, url: &str| {
            super::send_judged(method, url, &mut serde_json::json!({}), &nobody, "paired")
        };
        for (method, url) in [
            ("GET", "/api/health"),
            ("POST", "/api/pair/request"),
            ("POST", "/api/pair/qr-confirm"),
            ("GET", "/index.html"),
            ("GET", "/shared/channel.js"),
        ] {
            assert!(judged(method, url).is_ok(), "{method} {url}");
        }
        for (method, url) in [
            ("POST", "/api/chat"),
            ("POST", "/api/sessions"),
            ("POST", "/api/presence"),
            ("POST", "/api/memory/search"),
            ("POST", "/api/push/device"),
            ("POST", "/api/tts"),
            ("POST", "/api/stt"),
            ("GET", "/api/rtc/token"),
            ("POST", "/api/rtc/whip"),
            ("POST", "/api/agent-cancel"),
        ] {
            assert_eq!(judged(method, url), Err(super::Refused::Unnamed), "{method} {url}");
        }
        assert!(super::send_judged("POST", "/api/chat", &mut serde_json::json!({}), &at(crate::engine::permission::PermissionMode::Chat), "paired").is_ok());
    }

    /// A body that is an array (a route's body type is filled from it by
    /// position, naming a session where the seam does not look), a number or
    /// a boolean is refused, for a device and for a guest alike; an object,
    /// no body and a text body pass.
    #[test]
    fn a_body_that_is_not_an_object_is_refused() {
        use crate::engine::permission::PermissionMode::Chat;
        let device = at(Chat);
        let guest = super::consumer("u-friend");
        for bad in [serde_json::json!(["sess-1-builder"]), serde_json::json!(3), serde_json::json!(true)] {
            let by_device = super::send_judged("POST", "/api/chat/compact", &mut bad.clone(), &device, "paired");
            assert_eq!(by_device, Err(super::Refused::BadBody), "{bad}");
            let by_guest = super::send_as_guest("DELETE", "/api/sessions/all", &mut bad.clone(), &guest, "consumer");
            assert_eq!(by_guest, Err(super::Refused::BadBody), "{bad}");
        }
        for ok in [serde_json::json!({"session_id": "sess-9-new"}), serde_json::Value::Null, serde_json::json!("text")] {
            assert!(super::send_as_guest("DELETE", "/api/sessions/all", &mut ok.clone(), &guest, "consumer").is_ok(), "{ok}");
        }
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

    /// Two phones of one account are one person: one level, whatever each
    /// row once held, and a level set for the account holds for both.
    #[test]
    fn two_phones_of_one_account_stand_at_one_level() {
        use crate::engine::permission::PermissionMode::{Chat, Edit};
        let row = |id: &str, level| crate::state_fs::devices::PairedDevice {
            id: id.into(),
            legacy_permission: Some(level),
            ..vouched("u-friend")
        };
        let mut people = crate::server::people::People::default();
        let at = |people: &crate::server::people::People, d: &crate::state_fs::devices::PairedDevice| {
            super::standing_in(Some(d), Some("u-owner"), people).level()
        };
        let (a, b) = (row("a", Edit), row("b", Chat));
        assert_eq!(at(&people, &a), at(&people, &b), "what a row once held decides nothing");
        let patch = serde_json::json!({ "level": "chat" });
        people.set(&super::Person::for_test(Some("u-friend")), patch.as_object().unwrap()).unwrap();
        assert_eq!((at(&people, &a), at(&people, &b)), (Chat, Chat));
    }

    /// An app is for a person only when they may use it: every way a request
    /// names one — its page files, its tools, its sync, a chat said in it, a
    /// launch, its sessions — is refused for another, whatever the level,
    /// and passes for the one it is theirs and for the owner.
    #[test]
    fn a_request_for_an_app_the_person_may_not_use_is_refused_at_the_seam() {
        use serde_json::json;
        let mut people = crate::server::people::People::default();
        people.keep_for_owner(vec!["cfo".into()]);
        let friend = super::Person::for_test(Some("u-friend"));
        let patch = json!({"apps": ["music", "cfo"]});
        people.set(&friend, patch.as_object().unwrap()).unwrap();
        let mut ways: Vec<(&str, &str, serde_json::Value)> = Vec::new();
        for app in ["cfo", "dj"] {
            ways.push(("GET", Box::leak(format!("/apps/{app}/index.html").into_boxed_str()), json!(null)));
            ways.push(("POST", Box::leak(format!("/api/skills/{app}/tools/Snapshot").into_boxed_str()), json!({})));
            ways.push(("GET", Box::leak(format!("/api/skill-sync/{app}/items").into_boxed_str()), json!(null)));
            ways.push(("GET", Box::leak(format!("/api/skill-save/{app}").into_boxed_str()), json!(null)));
            ways.push(("GET", Box::leak(format!("/api/skill-sessions/state?skill={app}&x=1").into_boxed_str()), json!(null)));
            ways.push(("POST", "/api/chat", json!({"skill_name": app, "message": "hi"})));
            ways.push(("POST", "/api/apps/launch", json!({"skill": app})));
        }
        for (method, url, body) in &ways {
            let kept = super::judge_app(&people, (method, url), body, &friend);
            assert!(matches!(kept, Err(super::Refused::NotYourApp(_))), "{method} {url}: {kept:?}");
            assert_eq!(super::judge_app(&people, (method, url), body, &super::Person::test_owner()), Ok(()), "{method} {url}");
        }
        for (method, url, body) in [
            ("GET", "/apps/music/index.html", json!(null)),
            ("POST", "/api/chat", json!({"skill_name": "music"})),
            ("POST", "/api/chat", json!({"message": "no app"})),
            ("GET", "/api/status", json!(null)),
        ] {
            assert_eq!(super::judge_app(&people, (method, url), &body, &friend), Ok(()), "{method} {url}");
        }
        // The path is read as the server reads it: an app written another way.
        let escaped = super::judge_app(&people, ("GET", "/apps/%63fo/index.html"), &json!(null), &friend);
        assert!(matches!(escaped, Err(super::Refused::NotYourApp(_))), "{escaped:?}");
    }
}
