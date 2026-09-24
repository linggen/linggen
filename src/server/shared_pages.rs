//! `/shared/*` — page helpers every skill app uses (the chat bridge, the API
//! client, app-mode), served from the engine so skills stop carrying copies
//! that drift apart. Embedded at build time from the crate's `shared/` dir:
//! one version per engine, matching the embed chat it talks to.

use axum::{extract::Path, http::StatusCode, response::Response};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "shared/"]
struct SharedAssets;

/// GET /shared/{file}
pub(crate) async fn serve(Path(file): Path<String>) -> Response {
    let Some(asset) = SharedAssets::get(&file) else {
        return status(StatusCode::NOT_FOUND);
    };
    let mime = mime_guess::from_path(&file).first_or_octet_stream();
    Response::builder()
        .header("Content-Type", mime.as_ref())
        // Revalidate every load: a page must never pair an old bridge with a
        // newer embed after an engine update.
        .header("Cache-Control", "no-cache")
        .body(axum::body::Body::from(asset.data.into_owned()))
        .unwrap_or_else(|_| status(StatusCode::INTERNAL_SERVER_ERROR))
}

fn status(code: StatusCode) -> Response {
    Response::builder()
        .status(code)
        .body(axum::body::Body::empty())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_helpers_are_served_as_javascript() {
        for file in ["chat-bridge.js", "api.js", "app-mode.js"] {
            let res = serve(Path(file.to_string())).await;
            assert_eq!(res.status(), StatusCode::OK, "{file}");
            let ct = res.headers()["content-type"].to_str().unwrap().to_string();
            assert!(ct.contains("javascript"), "{file}: {ct}");
        }
        let missing = serve(Path("nope.js".to_string())).await;
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    }
}
