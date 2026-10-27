//! The phone's door (over the WebRTC tunnel, which stamps who it is) and the
//! Mac's own (loopback): register, ack what was told, read the state, raise
//! a declared line.
//!
//!   POST   /api/push/device  {token, env, key, language, fallback, morning, lines}
//!   DELETE /api/push/device
//!   POST   /api/push/told    {told: [...]}
//!   GET    /api/push/status
//!   POST   /api/push/notice  {app, line, key, facts, route?, until?}   (loopback only)

use axum::{
    extract::{Json, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::sync::Arc;

use crate::server::api::pair::{caller_device, TUNNEL_HEADER};
use crate::server::ServerState;

use super::raise::{self, Raised};
use super::store::{self, Phone};

const MAX_TOLD: usize = 64;
const MAX_LINES: usize = 64;

fn err(code: StatusCode, msg: &str) -> Response {
    (code, Json(serde_json::json!({ "error": msg }))).into_response()
}

fn device_or_401(headers: &HeaderMap) -> Result<String, Response> {
    caller_device(headers).ok_or_else(|| err(StatusCode::UNAUTHORIZED, "a paired phone only"))
}

#[derive(Deserialize)]
pub(crate) struct DeviceBody {
    token: String,
    env: String,
    key: String,
    #[serde(default)]
    language: String,
    #[serde(default)]
    fallback: String,
    #[serde(default)]
    morning: String,
    #[serde(default)]
    lines: BTreeMap<String, bool>,
}

/// The registration as the store keeps it, or why not.
pub(crate) fn phone_of(b: DeviceBody, now: u64) -> Result<Phone, &'static str> {
    let token = b.token.trim().to_ascii_lowercase();
    if !(64..=200).contains(&token.len()) || !token.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err("token must be the APNs device token in hex");
    }
    if b.env != "sandbox" && b.env != "production" {
        return Err("env must be sandbox or production");
    }
    if super::seal::key_of(&b.key).is_none() {
        return Err("key must be base64 of 32 bytes");
    }
    if b.lines.len() > MAX_LINES || b.lines.keys().any(|k| k.len() > 80) {
        return Err("too many lines");
    }
    let fallback: String = b.fallback.trim().chars().take(60).collect();
    Ok(Phone {
        token,
        env: b.env,
        key: b.key.trim().to_string(),
        language: if b.language.trim().is_empty() {
            crate::language::effective()
        } else {
            b.language.trim().chars().take(20).collect()
        },
        fallback: if fallback.is_empty() {
            "Linggen".into()
        } else {
            fallback
        },
        morning: super::timing::morning_time(&b.morning)
            .format("%H:%M")
            .to_string(),
        lines: b.lines,
        registered_at: now,
        ..Default::default()
    })
}

pub(crate) async fn post_device(headers: HeaderMap, Json(b): Json<DeviceBody>) -> Response {
    let device = match device_or_401(&headers) {
        Ok(d) => d,
        Err(r) => return r,
    };
    let now = crate::util::now_ts_secs();
    let fresh = match phone_of(b, now) {
        Ok(p) => p,
        Err(e) => return err(StatusCode::BAD_REQUEST, e),
    };
    store::modify(|s| {
        // What it told and what it spent survive a re-registration.
        let kept = s.phones.remove(&device).unwrap_or_default();
        s.phones.insert(
            device.clone(),
            Phone {
                told: kept.told,
                spent: kept.spent,
                last: kept.last,
                ..fresh
            },
        );
    });
    tracing::info!("[push] {device} registered");
    Json(serde_json::json!({ "ok": true })).into_response()
}

pub(crate) async fn delete_device(headers: HeaderMap) -> Response {
    let device = match device_or_401(&headers) {
        Ok(d) => d,
        Err(r) => return r,
    };
    store::modify(|s| s.phones.remove(&device));
    Json(serde_json::json!({ "ok": true })).into_response()
}

#[derive(Deserialize)]
pub(crate) struct ToldBody {
    told: Vec<String>,
}

pub(crate) async fn post_told(headers: HeaderMap, Json(b): Json<ToldBody>) -> Response {
    let device = match device_or_401(&headers) {
        Ok(d) => d,
        Err(r) => return r,
    };
    if b.told.len() > MAX_TOLD || b.told.iter().any(|t| t.is_empty() || t.len() > 200) {
        return err(
            StatusCode::BAD_REQUEST,
            "told: at most 64 keys of 200 characters",
        );
    }
    let now = crate::util::now_ts_secs();
    let known = store::modify(|s| {
        let Some(phone) = s.phones.get_mut(&device) else {
            return false;
        };
        for t in b.told {
            phone.told.entry(t).or_insert(now);
        }
        store::prune(phone, now);
        true
    });
    // An ack before the phone ever registered is still worth nothing to keep.
    Json(serde_json::json!({ "ok": true, "registered": known })).into_response()
}

/// The calling phone's own state; from the Mac, every phone's.
pub(crate) async fn get_status(headers: HeaderMap) -> Response {
    let state = store::load();
    let summary = |p: &Phone| {
        serde_json::json!({
            "registered": true,
            "env": p.env,
            "registered_at": p.registered_at,
            "last": p.last,
        })
    };
    if let Some(device) = caller_device(&headers) {
        let body = state
            .phones
            .get(&device)
            .map(summary)
            .unwrap_or_else(|| serde_json::json!({ "registered": false }));
        return Json(body).into_response();
    }
    if headers.contains_key(TUNNEL_HEADER) {
        return err(StatusCode::UNAUTHORIZED, "a paired phone only");
    }
    let phones: BTreeMap<_, _> = state
        .phones
        .iter()
        .map(|(d, p)| (d.clone(), summary(p)))
        .collect();
    Json(serde_json::json!({ "phones": phones, "waiting": state.queue.len() })).into_response()
}

#[derive(Deserialize)]
pub(crate) struct NoticeBody {
    app: String,
    line: String,
    #[serde(flatten)]
    rest: serde_json::Value,
}

/// Raise a declared line from code on this Mac. Never from a phone: what a
/// phone is told is the Mac's to decide.
pub(crate) async fn post_notice(
    State(state): State<Arc<ServerState>>,
    headers: HeaderMap,
    Json(b): Json<NoticeBody>,
) -> Response {
    if headers.contains_key(TUNNEL_HEADER) || caller_device(&headers).is_some() {
        return err(StatusCode::FORBIDDEN, "raised on this Mac only");
    }
    let Some(skill) = state.skills.get_skill(&b.app).await else {
        return err(StatusCode::NOT_FOUND, "no such skill");
    };
    let Some(decl) = skill.notices.iter().find(|n| n.line == b.line) else {
        return err(StatusCode::NOT_FOUND, "that skill declares no such line");
    };
    let Some(raised): Option<Raised> = raise::parse(&b.rest.to_string()) else {
        return err(StatusCode::BAD_REQUEST, "needs key and facts");
    };
    let added = raise::enqueue(&skill.name, decl, raised, crate::util::now_ts_secs());
    Json(serde_json::json!({ "ok": true, "queued": added })).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{engine::general_purpose::STANDARD, Engine};

    fn body(token: &str, env: &str, key: &str) -> DeviceBody {
        DeviceBody {
            token: token.into(),
            env: env.into(),
            key: key.into(),
            language: "zh-Hans".into(),
            fallback: "  银月有话说 ".into(),
            morning: "6:45".into(),
            lines: [("cfo.watch".to_string(), true)].into_iter().collect(),
        }
    }

    #[test]
    fn a_good_registration_is_kept_clean() {
        let key = STANDARD.encode([3u8; 32]);
        let p = phone_of(body(&"AB".repeat(32), "sandbox", &key), 9).unwrap();
        assert_eq!(p.token, "ab".repeat(32));
        assert_eq!(p.fallback, "银月有话说");
        assert_eq!(p.morning, "06:45");
        assert_eq!(p.registered_at, 9);
        assert_eq!(p.lines.get("cfo.watch"), Some(&true));
    }

    #[test]
    fn a_bad_token_env_or_key_is_refused() {
        let key = STANDARD.encode([3u8; 32]);
        assert!(phone_of(body("abc", "sandbox", &key), 0).is_err());
        assert!(phone_of(body(&"ab".repeat(32), "dev", &key), 0).is_err());
        assert!(phone_of(body(&"ab".repeat(32), "sandbox", "short"), 0).is_err());
    }
}
