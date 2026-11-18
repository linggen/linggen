//! Every route into rooms, and the one gate in front of them: with the
//! switch off (`server::rooms_switch`) each answers the same fact and
//! reaches nothing behind it. No room route is declared anywhere else; a
//! test (below) fails when one is.

use super::Routes;
use crate::server::api::rooms::{
    connect_proxy_room_api, disconnect_proxy_room_api, get_room_config, proxy_rooms,
    proxy_status_api, token_usage_api, update_room_config,
};
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::middleware::{from_fn_with_state, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get, post};
use axum::{Json, Router};

/// The room routes, open only when `on`.
pub(super) fn routes(on: bool) -> Routes {
    with_gate(
        Routes::new()
            .route("/api/rooms", any(proxy_rooms))
            .route("/api/rooms/", any(proxy_rooms))
            .route("/api/rooms/{*path}", any(proxy_rooms))
            .route("/api/proxy/connect", post(connect_proxy_room_api))
            .route("/api/proxy/disconnect", post(disconnect_proxy_room_api))
            .route("/api/proxy/status", get(proxy_status_api))
            .route("/api/token-usage", get(token_usage_api))
            .route(
                "/api/room-config",
                get(get_room_config).post(update_room_config),
            ),
        on,
    )
}

/// `router` answering [`crate::server::rooms_switch::OFF`] to everything
/// while `on` is false.
fn with_gate<S: Clone + Send + Sync + 'static>(router: Router<S>, on: bool) -> Router<S> {
    router.layer(from_fn_with_state(on, gate))
}

async fn gate(State(on): State<bool>, req: Request, next: Next) -> Response {
    if on {
        return next.run(req).await;
    }
    (
        StatusCode::FORBIDDEN,
        Json(serde_json::json!({ "error": crate::server::rooms_switch::OFF })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower::ServiceExt;

    async fn status_of(on: bool, method: &str, path: &str) -> (u16, String) {
        let app: Router = with_gate(
            Router::new().route("/api/rooms/{*path}", any(|| async { "reached" })),
            on,
        );
        let req = Request::builder()
            .method(method)
            .uri(path)
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let status = resp.status().as_u16();
        let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        (status, String::from_utf8_lossy(&body).to_string())
    }

    /// Off: the one fact, for any method, and the route behind is not reached.
    #[tokio::test]
    async fn off_every_method_gets_the_one_fact() {
        for method in ["GET", "POST", "PATCH", "DELETE"] {
            let (status, body) = status_of(false, method, "/api/rooms/public").await;
            assert_eq!(status, 403, "{method}");
            assert!(body.contains("rooms are off"), "{method}: {body}");
            assert!(!body.contains("reached"));
        }
    }

    /// On: the route is reached, as before.
    #[tokio::test]
    async fn on_the_route_is_reached() {
        assert_eq!(status_of(true, "GET", "/api/rooms/public").await, (200, "reached".into()));
    }

    /// Room routes are declared in this file only: no other routes file
    /// names a room handler or a room path.
    #[test]
    fn no_room_route_is_declared_elsewhere() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/server/routes");
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name == "rooms.rs" || name == "classes.rs" {
                continue;
            }
            let text = std::fs::read_to_string(entry.path()).unwrap();
            let code = text.split("#[cfg(test)]").next().unwrap_or(&text);
            for word in ["api::rooms", "/api/rooms", "/api/proxy/", "/api/room-config", "/api/token-usage"] {
                assert!(!code.contains(word), "{name}: `{word}` belongs in routes/rooms.rs");
            }
        }
    }
}
