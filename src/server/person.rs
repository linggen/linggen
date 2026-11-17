//! The person a request into a session is for, as the engine is reached
//! from off this machine: the routes that take a `user_id` and how a
//! request is sent as its person. The channel's tunnel and the LAN gate
//! both send their requests through [`send_as`], with the person resolved
//! from the device through memory's own seam ([`of_device`]).

/// The ways into the day a page reads by URL: each is answered for the
/// person on this channel, never for one the page names.
pub(crate) const PERSON_PATHS: &[&str] = &[
    "/api/sessions/day",
    "/api/skill-sessions",
    "/api/skill-sessions/state",
    "/api/workspace/state",
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
];

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

/// A request into a session sent as `person`, of `user_type`: a way into
/// the day by URL asks for that person, and a way in by body (any method
/// but GET) carries them, whatever the caller put there. Returns the URL
/// to send.
pub(crate) fn send_as(
    method: &str,
    url_path: &str,
    body: &mut serde_json::Value,
    (person, user_type): (&str, &str),
) -> String {
    let url = with_person(url_path, person);
    if takes_body(method, url.split_once('?').map_or(url.as_str(), |(p, _)| p)) {
        as_person(body, person, user_type);
    }
    url
}

/// Whether a request to `path` carries its person in its body.
pub(crate) fn takes_body(method: &str, path: &str) -> bool {
    method != "GET" && PERSON_BODIES.contains(&path)
}

/// The person behind paired device `device`, through memory's own seam
/// (`api::memory::person_behind`): the owner's own device is the owner; a
/// phone signed in to another account is that account, a signed-out phone
/// its device; a device not paired is no one (`rtc::NO_ONE`).
pub(crate) fn of_device(device: &str) -> String {
    match crate::server::api::memory::person_behind(device) {
        Some(Some(person)) => person,
        Some(None) => crate::server::rtc::UserContext::owner(None).user_id,
        None => crate::server::rtc::NO_ONE.to_string(),
    }
}

/// The LAN gate's request from paired device `device`, sent as its person
/// (as the channel sends a peer's): the URL's `user_id` and a body's
/// replaced. A body it cannot read is sent as an empty one with the person.
pub(crate) async fn lan_request_as(req: axum::extract::Request, device: &str) -> axum::extract::Request {
    use axum::http::header::{CONTENT_LENGTH, CONTENT_TYPE};
    let person = of_device(device);
    let user_type = crate::server::rtc::PeerKind::Paired.as_str();
    let (mut parts, body) = req.into_parts();
    let url_path = parts.uri.path_and_query().map_or("/", |pq| pq.as_str()).to_string();
    let rewrites_body = takes_body(parts.method.as_str(), parts.uri.path());
    let mut value = serde_json::Value::Null;
    let body = match rewrites_body {
        true => {
            let bytes = axum::body::to_bytes(body, LAN_BODY_LIMIT).await.unwrap_or_default();
            value = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
            None
        }
        false => Some(body),
    };
    let url = send_as(parts.method.as_str(), &url_path, &mut value, (&person, user_type));
    if let Ok(uri) = url.parse() {
        parts.uri = uri;
    }
    let body = body.unwrap_or_else(|| {
        let bytes = serde_json::to_vec(&value).unwrap_or_default();
        parts.headers.insert(CONTENT_LENGTH, bytes.len().into());
        parts.headers.insert(CONTENT_TYPE, axum::http::HeaderValue::from_static("application/json"));
        axum::body::Body::from(bytes)
    });
    axum::extract::Request::from_parts(parts, body)
}

/// The most a LAN body sent as its person is read up to (a chat with its
/// pictures inline).
const LAN_BODY_LIMIT: usize = 64 * 1024 * 1024;

#[cfg(test)]
mod tests {
    /// A request is sent as its person: a `user_id` written another way is
    /// dropped too, and a body is the person's only on a way in by body.
    #[test]
    fn a_request_is_sent_as_its_person() {
        let mut body = serde_json::json!({"user_id": "u-owner", "message": "hi"});
        let url = super::send_as("POST", "/api/chat", &mut body, ("u1", "paired"));
        assert_eq!(url, "/api/chat");
        assert_eq!(body["user_id"], "u1");
        assert_eq!(body["user_type"], "paired");
        let mut other = serde_json::json!({"user_id": "x"});
        super::send_as("POST", "/api/config", &mut other, ("u1", "paired"));
        assert_eq!(other["user_id"], "x");
        assert_eq!(
            super::with_person("/api/sessions/day?user%5Fid=u-owner", "u1"),
            "/api/sessions/day?user_id=u1"
        );
    }

    /// A page reads the day for the person on its channel: a `user_id` it
    /// names is replaced, and other paths are left alone.
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
