//! The pages the engine serves: the web UI's embedded bundle, each web
//! app skill's files, and the Memory_* capability a skill page may call.

use super::ServerState;
use axum::{
    extract::State,
    http::Uri,
    response::{IntoResponse, Response},
};
use rust_embed::RustEmbed;
use std::sync::Arc;

#[derive(RustEmbed)]
#[folder = "ui/dist/"]
struct Assets;

/// Dispatch a Memory_* tool on behalf of a skill's webpage.
/// Route: POST /apps/{skill_name}/capability/{tool_name}
///
/// Memory is engine-built-in (HTTP to `ling-mem`); this endpoint is a
/// thin wrapper around `tools::memory_http::call_memory_http` so skill
/// webpages can call Memory_* without hardcoding the daemon URL. The
/// call also rides the existing WebRTC fetch proxy in remote mode
/// (control channel forwards `/apps/*` to the host's linggen server).
///
/// The pre-PR2 implementation gated this on the skill's frontmatter
/// `provides: [memory]`. With memory now engine-built-in there is no
/// `provides` list to consult — every installed skill can invoke
/// Memory_* via this endpoint. Skills the user doesn't trust shouldn't
/// be installed in the first place; the route name + skill_name still
/// shows up in audit logs.
pub(super) async fn capability_dispatch(
    State(state): State<Arc<ServerState>>,
    axum::extract::Path((skill_name, tool_name)): axum::extract::Path<(String, String)>,
    body: Option<axum::Json<serde_json::Value>>,
) -> Response {
    use axum::http::StatusCode;

    let args = body
        .map(|axum::Json(v)| v)
        .unwrap_or(serde_json::Value::Object(Default::default()));

    let writes_allowed = match tool_name.as_str() {
        "Memory_query" => false,
        "Memory_write" => true,
        _ => {
            return (
                StatusCode::NOT_FOUND,
                format!(
                    "Unknown built-in tool '{}' on /apps/*/capability/*",
                    tool_name
                ),
            )
                .into_response();
        }
    };

    // The tool name is only half the request: dispatch is by the `verb` in the
    // body, so until this check the split above was decorative — a page could
    // POST Memory_query with `verb: "delete"` and delete. And the verb is
    // concatenated into a URL, so an unknown one must not be forwarded at all.
    let verb = args
        .get("verb")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let Some(mutates) = crate::server::api::memory::verb_mutates(verb)
        .filter(|_| crate::server::api::memory::is_safe_verb(verb))
    else {
        return (
            StatusCode::BAD_REQUEST,
            format!("Unknown memory verb '{verb}'"),
        )
            .into_response();
    };
    if mutates && !writes_allowed {
        return (
            StatusCode::FORBIDDEN,
            format!("'{verb}' changes the store — use Memory_write"),
        )
            .into_response();
    }

    if state.skills.get_skill(&skill_name).await.is_none() {
        return (
            StatusCode::NOT_FOUND,
            format!("Skill '{}' not found", skill_name),
        )
            .into_response();
    }

    // Attribute the row to this host. Used to be a default inside the ling-mem
    // client, which meant every caller of that function was silently named
    // `linggen` — including the `/mcp` front door, which is not. Each caller
    // says who it is now, and this one is a webpage running inside Linggen.
    let mut args = args;
    if let Some(obj) = args.as_object_mut() {
        if tool_name == "Memory_write" && !obj.contains_key("host") {
            obj.insert("host".to_string(), serde_json::json!("linggen"));
        }
    }

    let ling_mem_url = state.manager.get_config_snapshot().await.agent.ling_mem_url;
    match crate::engine::tools::memory_http::call_memory_http(&ling_mem_url, &tool_name, args).await
    {
        Ok(data) => axum::Json(data).into_response(),
        Err(e) => {
            let msg = format!("{:#}", e);
            (StatusCode::BAD_GATEWAY, msg).into_response()
        }
    }
}

/// Serve static files from an app-enabled skill's directory.
/// Route: GET /apps/{skill_name}/{*file_path}
pub(super) async fn serve_app_file(
    State(state): State<Arc<ServerState>>,
    axum::extract::Path((skill_name, file_path)): axum::extract::Path<(String, String)>,
    req: axum::extract::Request,
) -> Response {
    let build_err = |status: u16, msg: &str| -> Response {
        Response::builder()
            .status(status)
            .header("Content-Type", "text/plain")
            .body(axum::body::Body::from(msg.to_string()))
            .unwrap_or_else(|_| Response::new(axum::body::Body::from("internal server error")))
    };

    // Look up the skill.
    let skill = state.skills.get_skill(&skill_name).await;
    let Some(skill) = skill else {
        return build_err(404, &format!("Skill '{}' not found", skill_name));
    };

    // Verify it has app config with web launcher.
    let Some(ref app) = skill.app else {
        return build_err(403, &format!("Skill '{}' is not an app", skill_name));
    };
    if app.launcher != "web" {
        return build_err(
            403,
            &format!(
                "Skill '{}' app launcher is '{}', not 'web'",
                skill_name, app.launcher
            ),
        );
    }

    // Resolve the file within the skill directory.
    let Some(ref skill_dir) = skill.skill_dir else {
        return build_err(500, "Skill directory not available");
    };

    // Sanitize: reject path traversal.
    let file_path_clean = file_path.trim_start_matches('/');
    if file_path_clean.contains("..") {
        return build_err(403, "Path traversal not allowed");
    }

    let full_path = skill_dir.join(file_path_clean);

    // Canonicalize both paths to resolve symlinks and prevent escape.
    let canonical_dir = match tokio::fs::canonicalize(skill_dir).await {
        Ok(p) => p,
        Err(_) => return build_err(500, "Skill directory not accessible"),
    };
    let canonical_full = match tokio::fs::canonicalize(&full_path).await {
        Ok(p) => p,
        Err(_) => return build_err(404, &format!("File not found: {}", file_path_clean)),
    };
    if !canonical_full.starts_with(&canonical_dir) {
        return build_err(403, "Path traversal not allowed");
    }

    // Delegate to tower-http's ServeFile: it speaks Range/206 (video seeking
    // in skill pages needs partial content), conditional requests, and HEAD.
    use tower::ServiceExt as _;
    let range_hdr = req
        .headers()
        .get("range")
        .and_then(|v| v.to_str().ok())
        .map(String::from);
    match tower_http::services::ServeFile::new(&canonical_full)
        .oneshot(req)
        .await
    {
        Ok(res) => {
            tracing::debug!(
                "serve_app_file {} range={:?} -> {}",
                file_path_clean,
                range_hdr,
                res.status()
            );
            let mut res = res.map(axum::body::Body::new);
            let headers = res.headers_mut();
            headers.insert(
                "Content-Security-Policy",
                FRAME_ANCESTORS.parse().expect("static header"),
            );
            // No-store on skill assets: skills are user-iterated, often
            // edited mid-session, and ES-module URL caching makes a stale
            // scan.js indistinguishable from a missing feature. Forcing
            // revalidation costs nothing on localhost and removes a sharp
            // edge from the development loop.
            headers.insert("Cache-Control", "no-store".parse().expect("static header"));
            res
        }
        Err(_) => build_err(500, &format!("File not readable: {}", file_path_clean)),
    }
}

/// Who may frame the engine's pages: its own origin (the launcher, a skill
/// page framing its chat) and a VS Code webview. Through the linggen.dev
/// tunnel, pages run as blobs of the site's own origin and never meet this
/// header. Anything else — any site in the browser — may not.
const FRAME_ANCESTORS: &str = "frame-ancestors 'self' vscode-webview:";

pub(super) async fn static_handler(State(state): State<Arc<ServerState>>, uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');

    let build_response =
        |builder: axum::http::response::Builder, body: axum::body::Body| -> Response {
            builder
                .body(body)
                .unwrap_or_else(|_| Response::new(axum::body::Body::from("internal server error")))
        };

    if state.dev_mode {
        // In dev mode, static assets are served by the Vite dev server.
        // Return 404 so the user knows to use the Vite proxy.
        return build_response(
            Response::builder()
                .status(404)
                .header("Content-Type", "text/plain"),
            axum::body::Body::from(
                "Dev mode: static assets are served by Vite. Use the Vite dev server URL instead.",
            ),
        );
    }

    // All surface routes (/, /embed, /consumer) share a single index.html.
    // The JS entry inspects window.location to pick MainApp/EmbedApp/ConsumerApp.
    // This keeps the bundle as a single chunk — required for blob-URL loading
    // through the linggen.dev tunnel (shared chunks with relative imports fail).
    let path = if path.is_empty() { "index.html" } else { path };

    // Framed only by its own pages (skill iframes, the launcher) and a VS Code
    // webview — never by another site, which could drive the chat.
    let xfo = "Content-Security-Policy";
    let xfo_val = FRAME_ANCESTORS;

    match Assets::get(path) {
        Some(content) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            build_response(
                Response::builder()
                    .header("Content-Type", mime.as_ref())
                    .header(xfo, xfo_val),
                axum::body::Body::from(content.data),
            )
        }
        None => {
            // Fallback to index.html for SPA routing
            match Assets::get("index.html") {
                Some(index) => build_response(
                    Response::builder()
                        .header("Content-Type", "text/html")
                        .header(xfo, xfo_val),
                    axum::body::Body::from(index.data),
                ),
                None => build_response(
                    Response::builder().status(404),
                    axum::body::Body::from("Not found"),
                ),
            }
        }
    }
}
