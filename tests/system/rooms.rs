//! Rooms are one switch (`src/server/rooms_switch.rs`), off unless the
//! machine's `room_config.toml` says `rooms_on = true`. Off, every room
//! route answers the one fact; on, they answer as they always did.

use crate::support::{Setup, World};

/// Every path `src/server/routes/rooms.rs` routes, a wildcard filled in.
fn room_paths() -> Vec<String> {
    let text = include_str!("../../src/server/routes/rooms.rs");
    let code = text.split("#[cfg(test)]").next().unwrap();
    let paths: Vec<String> = code
        .split(".route(")
        .skip(1)
        .filter_map(|call| {
            let rest = call.trim_start().strip_prefix('"')?;
            Some(rest[..rest.find('"')?].replace("{*path}", "anything"))
        })
        .collect();
    assert!(paths.len() >= 8, "read the room routes: {paths:?}");
    paths
}

#[tokio::test]
async fn off_by_default_every_room_route_says_so() {
    let w = World::start(Setup::default()).await;
    for path in room_paths() {
        for method in ["GET", "POST", "PATCH", "DELETE"] {
            let (status, body) = w.api.call(method, &path).await;
            assert_eq!(status, 403, "{method} {path}");
            assert_eq!(body["error"], "rooms are off", "{method} {path}");
        }
    }
}

#[tokio::test]
async fn on_the_room_routes_answer_as_before() {
    let w = World::start(Setup { rooms_on: true, ..Setup::default() }).await;
    let (status, body) = w.api.call("GET", "/api/room-config").await;
    assert_eq!(status, 200);
    assert!(body.get("shared_models").is_some(), "{body}");
    let (status, body) = w.api.call("GET", "/api/proxy/status").await;
    assert_eq!(status, 200);
    assert!(body.get("connections").is_some(), "{body}");
    assert_eq!(w.api.call("GET", "/api/token-usage").await.0, 200);
}
