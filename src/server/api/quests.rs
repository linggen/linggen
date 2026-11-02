//! One app's quest file, read: `GET /api/quests/{app}` answers
//! `~/.linggen/quests/<app>.json` as it is (`{"app":…,"quests":[…]}`), or
//! an empty list when the app has none. A paired phone reads the Mac's
//! `linggen` file over the WebRTC channel, so its own play of Lingjing lists
//! what the Mac saw done (开府). Only ids, words and times are in it.

use axum::{extract::Path, http::StatusCode, response::IntoResponse, Json};
use serde_json::{json, Value};

pub(crate) async fn get_quests(Path(app): Path<String>) -> impl IntoResponse {
    if !valid_app(&app) {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "bad app" })));
    }
    let path = crate::paths::quests_dir().join(format!("{app}.json"));
    let doc = tokio::fs::read_to_string(&path)
        .await
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .filter(|d| d["quests"].is_array())
        .unwrap_or_else(|| json!({ "app": app, "quests": [] }));
    (StatusCode::OK, Json(doc))
}

fn valid_app(app: &str) -> bool {
    !app.is_empty()
        && app.len() <= 48
        && app
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

#[cfg(test)]
mod tests {
    #[test]
    fn app_names_are_plain() {
        assert!(super::valid_app("linggen"));
        assert!(super::valid_app("apple-shifu"));
        for bad in ["", "../x", "a.json", "A", "a/b"] {
            assert!(!super::valid_app(bad), "{bad}");
        }
    }
}
