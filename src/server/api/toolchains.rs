//! Settings → the owner's list of build tools (`permission::toolchains`):
//! the programs a session may be granted by name.
//!
//! - `GET /api/toolchains` — the list, and the default build tools found on
//!   the engine's path that are not on it (offered, never added by itself).
//! - `PUT /api/toolchains {tools: [{name, path}]}` — replace the list.
//!
//! Both only for someone at this Mac (`is_at_this_mac`); the PUT also needs
//! a page Origin of this engine. That Origin is a speed bump, not a lock: a
//! program on this Mac can send any header, and a delegate with admin on its
//! folder can already reach loopback routes — it stops a plain request with
//! no page behind it, nothing more.

use axum::{
    extract::ConnectInfo,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::json;
use std::net::SocketAddr;

use crate::engine::permission::toolchains::{self, OwnerTools, Tool};
use crate::server::api::ling_mem_connect::{is_at_this_mac, refused};
use crate::server::loopback_guard;

/// `GET /api/toolchains`
pub async fn get_toolchains(
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Response {
    if !is_at_this_mac(peer, &headers) {
        return refused();
    }
    let search = crate::util::shell_path();
    let list = toolchains::owner_list(search);
    let found = offered(&list, search);
    Json(json!({ "seeded": list.seeded, "tools": list.tools, "found": found })).into_response()
}

#[derive(Deserialize)]
pub struct PutBody {
    tools: Vec<Tool>,
}

/// `PUT /api/toolchains`
pub async fn put_toolchains(
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<PutBody>,
) -> Response {
    if !may_change(peer, &headers) {
        return refused();
    }
    if let Err(error) = checked(&body.tools) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "ok": false, "error": error })),
        )
            .into_response();
    }
    let file = toolchains::owner_file();
    let list = OwnerTools {
        seeded: toolchains::load(&file).seeded,
        tools: body.tools,
    };
    match toolchains::save(&file, &list) {
        Ok(()) => Json(json!({ "ok": true, "tools": list.tools })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "ok": false, "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// Someone at this Mac, from a page of this engine (see the module note).
fn may_change(peer: SocketAddr, headers: &HeaderMap) -> bool {
    let origin = headers.get("origin").and_then(|v| v.to_str().ok());
    is_at_this_mac(peer, headers)
        && origin
            .is_some_and(|o| loopback_guard::origin_is_loopback_page(o) || o == "tauri://localhost")
}

/// Every entry a plain name on an executable file, each name once.
fn checked(tools: &[Tool]) -> Result<(), String> {
    for (i, tool) in tools.iter().enumerate() {
        toolchains::check_entry(tool)?;
        if tools[..i].iter().any(|t| t.name == tool.name) {
            return Err(format!("{} is listed twice", tool.name));
        }
    }
    Ok(())
}

/// The default build tools found on `search` and not on the list.
fn offered(list: &OwnerTools, search: &str) -> Vec<Tool> {
    toolchains::DEFAULT_NAMES
        .iter()
        .filter(|name| !list.tools.iter().any(|t| t.name == **name))
        .filter_map(|name| {
            toolchains::find(name, search).map(|path| Tool {
                name: name.to_string(),
                path,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::api::pair::{TUNNEL_HEADER, TUNNEL_LOCAL};

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(*k, v.parse().unwrap());
        }
        h
    }

    fn lo() -> SocketAddr {
        "127.0.0.1:50000".parse().unwrap()
    }

    #[test]
    fn a_page_of_this_engine_at_this_mac_may_change_the_list() {
        let page = headers(&[
            ("host", "127.0.0.1:9527"),
            ("origin", "http://127.0.0.1:9527"),
        ]);
        assert!(may_change(lo(), &page));
        let shell = headers(&[("host", "localhost:9527"), ("origin", "tauri://localhost")]);
        assert!(may_change(lo(), &shell));
    }

    #[test]
    fn a_change_not_from_this_mac_is_refused() {
        let lan: SocketAddr = "192.168.20.31:50000".parse().unwrap();
        let page = headers(&[
            ("host", "127.0.0.1:9527"),
            ("origin", "http://127.0.0.1:9527"),
        ]);
        assert!(!may_change(lan, &page));
        // A paired device's channel, re-issued from loopback.
        let device = headers(&[
            ("host", "127.0.0.1:9527"),
            ("origin", "http://127.0.0.1:9527"),
            (TUNNEL_HEADER, "1"),
        ]);
        assert!(!may_change(lo(), &device));
        let site = headers(&[
            ("host", "127.0.0.1:9527"),
            ("origin", "https://evil.example"),
        ]);
        assert!(!may_change(lo(), &site));
    }

    #[test]
    fn a_change_with_no_origin_is_refused() {
        let curl = headers(&[("host", "127.0.0.1:9527")]);
        assert!(is_at_this_mac(lo(), &curl), "it may read the list");
        assert!(!may_change(lo(), &curl));
        // This Mac's own channel carries no Origin either.
        let channel = headers(&[("host", "127.0.0.1:9527"), (TUNNEL_HEADER, TUNNEL_LOCAL)]);
        assert!(!may_change(lo(), &channel));
    }

    #[test]
    fn a_list_is_plain_names_each_once_and_offers_what_it_lacks() {
        let dir = tempfile::tempdir().unwrap();
        let cargo = dir.path().join("cargo");
        std::fs::write(&cargo, "").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&cargo, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let tool = Tool {
            name: "cargo".into(),
            path: cargo.clone(),
        };
        assert!(checked(&[tool.clone()]).is_ok());
        assert!(checked(&[tool.clone(), tool.clone()]).is_err());
        let search = dir.path().to_string_lossy().to_string();
        assert_eq!(offered(&OwnerTools::default(), &search), [tool.clone()]);
        let listed = OwnerTools {
            seeded: None,
            tools: vec![tool],
        };
        assert!(offered(&listed, &search).is_empty());
    }
}
