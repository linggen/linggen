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
/// (as the channel sends a peer's): the URL's `user_id` and a JSON body's
/// replaced, and refused when it names another person's day. A body that
/// is not a JSON object is passed through untouched — it names no session
/// and carries no person.
pub(crate) async fn lan_request_as(
    req: axum::extract::Request,
    device: &str,
) -> Result<axum::extract::Request, NotTheirs> {
    use axum::http::header::CONTENT_LENGTH;
    let person = of_device(device);
    let user_type = crate::server::rtc::PeerKind::Paired.as_str();
    let (mut parts, body) = req.into_parts();
    let url_path = parts.uri.path_and_query().map_or("/", |pq| pq.as_str()).to_string();
    let bytes = axum::body::to_bytes(body, usize::MAX).await.unwrap_or_default();
    let mut value = match serde_json::from_slice::<serde_json::Value>(&bytes) {
        Ok(v) if v.is_object() => Some(v),
        _ => None,
    };
    let mut none = serde_json::Value::Null;
    let url = send_as(parts.method.as_str(), &url_path, value.as_mut().unwrap_or(&mut none), (&person, user_type))?;
    if let Ok(uri) = url.parse() {
        parts.uri = uri;
    }
    let bytes = match value {
        Some(v) => serde_json::to_vec(&v).map(axum::body::Bytes::from).unwrap_or(bytes),
        None => bytes,
    };
    parts.headers.insert(CONTENT_LENGTH, bytes.len().into());
    Ok(axum::extract::Request::from_parts(parts, axum::body::Body::from(bytes)))
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
