//! The person a request into a session is for, as the engine is reached
//! from off this machine: the routes that take a `user_id` and how a
//! request is sent as its person.

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
        .filter(|p| !p.is_empty() && !p.starts_with("user_id="))
        .collect();
    let person = format!("user_id={}", urlencoding::encode(user_id));
    pairs.push(&person);
    format!("{path}?{}", pairs.join("&"))
}

#[cfg(test)]
mod tests {
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
