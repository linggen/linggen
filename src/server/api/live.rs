//! Live voice for an app's page (`GET /api/live`, `POST /api/live/token`).
//!
//! The page opens the provider's live socket itself and must never hold the
//! provider's key; this engine holds the keys, so it mints a token for ONE
//! live session and hands the page only that (`crate::live`). The engine
//! names no app: any page may ask.
//!
//! Owner only, as hearing is (`/api/stt`): both routes are classed at chat
//! (`routes/classes.rs`) and the handlers keep them to the owner with
//! `person::owners_only` — the one rule that decides who may ask. A caller
//! that is not the owner is told the refusal every owner-only route gives
//! (403 `beyond_level`); `GET /api/live` lists `not_owner` as the reason
//! instead, so a page can say why without failing.
//!
//! Nothing here writes the key or a token anywhere: no log line, event,
//! session row or error text carries either (`crate::live::Secret`; the
//! system test `live_token` reads the engine's log for both). A refusal is a
//! fixed code.

use std::sync::Arc;
use std::time::Instant;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};

use crate::live::{self, Provider, Refusal};
use crate::server::ServerState;

/// Why a provider is not ready for this caller, as a fixed word; `None`: it is.
fn why_not_ready(owner: bool, key_found: impl FnOnce() -> bool) -> Option<&'static str> {
    if !owner {
        return Some("not_owner");
    }
    (!key_found()).then_some("no_key")
}

/// GET /api/live — which live providers this Mac can use now. Starts
/// nothing, calls no provider, and says nothing of any key but whether one
/// is stored.
pub(crate) async fn status_handler(
    State(state): State<Arc<ServerState>>,
    headers: HeaderMap,
) -> Response {
    let level = crate::server::api::pair::caller_level(&headers);
    let owner = crate::server::person::owners_only(level).is_none();
    let configured = state.manager.get_config_snapshot().await.models;
    let providers: Vec<Value> = live::PROVIDERS
        .iter()
        .map(
            |p| match why_not_ready(owner, || live::find_key(p, &configured).is_some()) {
                Some(why) => json!({"id": p.id, "ready": false, "why": why}),
                None => json!({"id": p.id, "ready": true}),
            },
        )
        .collect();
    Json(json!({ "providers": providers })).into_response()
}

/// POST /api/live/token `{provider}` — a token for one new live session.
pub(crate) async fn token_handler(
    State(state): State<Arc<ServerState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let level = crate::server::api::pair::caller_level(&headers);
    if let Some(refused) = crate::server::person::owners_only(level) {
        return refused;
    }
    let configured = state.manager.get_config_snapshot().await.models;
    let asked = serde_json::from_slice::<Value>(&body)
        .ok()
        .and_then(|v| v.get("provider")?.as_str().and_then(live::provider_named));
    answer(asked, &configured, &state.live_rate).await
}

/// The decision once the owner is known: the provider named, its key, the
/// rate, then the one request to the provider.
async fn answer(
    asked: Option<&'static Provider>,
    configured: &[crate::config::ModelConfig],
    rate: &live::TokenRate,
) -> Response {
    let Some(p) = asked else {
        return refuse(Refusal::UnknownProvider, "?");
    };
    let Some(key) = live::find_key(p, configured) else {
        return refuse(Refusal::NoKey, p.id);
    };
    if !rate.allow(Instant::now()) {
        return refuse(Refusal::SlowDown, p.id);
    }
    match live::mint(p, &live::http_base(p), &key, chrono::Utc::now()).await {
        Ok(minted) => {
            tracing::info!("live token made for {}", minted.provider);
            let mut res = Json(json!({
                "provider": minted.provider,
                "model": minted.model,
                "ws_url": minted.ws_url.expose(),
                "token": minted.token.expose(),
                "expires": minted.expires,
            }))
            .into_response();
            // A grant is for one page, once: nothing keeps a copy.
            res.headers_mut().insert(
                header::CACHE_CONTROL,
                "no-store".parse().expect("header value"),
            );
            res
        }
        Err(why) => refuse(why, p.id),
    }
}

/// A refusal: its status and `{error: <fixed code>}`, and one log line that
/// names the provider and the code only.
fn refuse(why: Refusal, provider: &str) -> Response {
    tracing::info!("live token refused for {provider}: {}", why.code());
    let status = StatusCode::from_u16(why.status()).expect("a refusal's status is valid");
    (status, Json(json!({ "error": why.code() }))).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::permission::PermissionMode;
    use crate::server::routes::classes::{class_of, CLASSES};

    #[test]
    fn readiness_names_the_first_thing_in_the_way() {
        assert_eq!(why_not_ready(false, || true), Some("not_owner"));
        assert_eq!(why_not_ready(false, || false), Some("not_owner"));
        assert_eq!(why_not_ready(true, || false), Some("no_key"));
        assert_eq!(why_not_ready(true, || true), None);
    }

    /// Lock: the live routes are exactly these two, both classed; a third
    /// fails here until it is looked at (the owner check, the redaction).
    #[test]
    fn the_live_routes_are_classed_and_there_are_two() {
        let named: Vec<(&str, &str)> = CLASSES
            .iter()
            .filter(|(_, path, _)| path.starts_with("/api/live"))
            .map(|(m, p, _)| (*m, *p))
            .collect();
        assert_eq!(
            named,
            vec![("GET", "/api/live"), ("POST", "/api/live/token")],
            "a live route was added: class it, keep it to the owner, update this lock"
        );
        for (method, path) in named {
            assert_eq!(
                class_of(method, path).needs(),
                PermissionMode::Chat,
                "{method} {path}"
            );
        }
    }

    /// Lock: the scratch engines' key knob and the test-only address knob are
    /// named in `live.rs` alone — nothing else reads either.
    #[test]
    fn the_live_knobs_are_read_in_one_file() {
        let mut named_in: Vec<String> =
            crate::server::lock_scan::production_code(&["src", "crates/linggen-engine/src"])
                .into_iter()
                .filter(|(_, code)| code.contains("LINGGEN_LIVE_"))
                .map(|(rel, _)| rel)
                .collect();
        named_in.sort();
        assert_eq!(
            named_in,
            vec!["crates/linggen-engine/src/live.rs".to_string()],
            "another file reads a LINGGEN_LIVE_ knob"
        );
    }

    /// Lock: each handler asks `owners_only`, the key is looked up by these
    /// two handlers alone, only the page's grant opens a secret, and no log
    /// line names one.
    #[test]
    fn both_routes_keep_to_the_owner_and_nothing_logs_a_secret() {
        let code = include_str!("live.rs");
        let production = &code[..code.find("#[cfg(test)]").unwrap()];
        assert_eq!(
            production.matches("person::owners_only(").count(),
            2,
            "each route asks the one owner rule"
        );
        assert_eq!(
            production.matches("live::find_key(").count(),
            2,
            "the key is looked up by the status and the token only"
        );
        assert_eq!(
            production.matches(".expose()").count(),
            2,
            "only the page's grant opens a secret here"
        );
        for line in production.lines().filter(|l| l.contains("tracing::")) {
            for word in ["key", "token}", "ws_url", "minted.token", "expose"] {
                assert!(
                    !line.contains(word),
                    "a log line names a secret ({word}): {line}"
                );
            }
        }
    }
}
