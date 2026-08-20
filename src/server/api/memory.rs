//! Memory for callers that reach this Mac from outside it — today, the phone.
//!
//! Yinyue is **resident on the phone**: her tool loop runs there, against the
//! phone's own model, so she is the one thing that needs the Mac's memory from
//! off-machine. (The phone's Ling tab needs nothing here — it drives a session
//! on this Mac, and that agent reaches memory on its own loopback like every
//! local caller.)
//!
//! She cannot dial `ling-mem` directly. Off the LAN the phone has no IP route
//! to this machine at all; every call arrives as an `http_request` frame on the
//! WebRTC data channel and terminates in *this* router. So memory has to be
//! reachable at an engine path, the same way `/api/sessions` and the DJ library
//! already are.
//!
//! What this is not: the `memory_*` tool proxy cut from `/mcp` on 2026-07-30.
//! That put a second copy of ling-mem's tools — same names, same schemas — in
//! front of a model, and the model had to guess between them. Nothing here is a
//! tool. There is no `TOOLS` entry, no schema, no dispatch translation: one
//! verb segment forwarded to the daemon that owns it, and its answer returned
//! whole. It is transport, exactly like the tunnel it rides on.
//!
//! The phone is a *program*, not an agent — three fixed verbs with fixed
//! argument shapes, choosing nothing — which is precisely the case
//! `network-spec.md` reserves for REST rather than MCP. It gets no JSON-RPC
//! envelope and no `content[0].text` to dig through, just the daemon's own
//! `{ok, data}`.
//!
//! Auth is the router's, not ours: the device gate in `server::mod` refuses any
//! non-loopback request without a paired `x-linggen-device` token before it
//! reaches this handler, and loopback callers are already inside the fence.

use axum::{
    extract::{Json, Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::{json, Value};
use std::sync::Arc;

use crate::server::state::ServerState;

/// `POST /api/memory/<verb>` — forward to the daemon that owns memory.
///
/// The verb is passed through rather than checked against a list. ling-mem
/// serves eighteen of them and is the authority on which exist; a copy of that
/// list here would be a second thing to keep in step, and it would answer 404
/// for a verb the daemon had just gained. Unknown verbs get the daemon's own
/// 404 instead.
///
/// The address is resolved per request from `[agent].ling_mem_url`, so moving
/// ling-mem to another port is a Mac-side config change and nothing else — the
/// phone never learns a port, stores one, or needs a rebuild when it moves.
pub async fn passthrough(
    State(state): State<Arc<ServerState>>,
    Path(verb): Path<String>,
    body: Option<Json<Value>>,
) -> Response {
    if !is_safe_verb(&verb) {
        return unreachable_envelope(
            StatusCode::BAD_REQUEST,
            format!("invalid memory verb: {verb}"),
            "BAD_REQUEST",
        );
    }

    let args = body.map(|Json(v)| v).unwrap_or_else(|| json!({}));
    let ling_mem_url = state.manager.get_config_snapshot().await.agent.ling_mem_url;

    // The engine logs its own ling-mem calls (`call_memory_http`); this path
    // is the one that arrives from another machine, so it is the one worth
    // seeing in the log at all. Keys only — a memory row's content is the
    // user's, and this line is not where it should turn up.
    tracing::info!(
        "memory passthrough → {verb} {:?}",
        args.as_object()
            .map(|o| o.keys().collect::<Vec<_>>())
            .unwrap_or_default()
    );

    match crate::engine::tools::memory_http::post_memory_verb(&ling_mem_url, &verb, &args).await {
        // The daemon answered. Its envelope is the answer — `{ok, data}` or
        // `{ok:false, error, code}` — and it goes back untouched, code and all,
        // under ling-mem's own status. Rewriting a failure into a bare status
        // would throw away the one field that says what went wrong; flattening
        // a 404 into 200 would claim a verb exists that doesn't.
        Ok((status, envelope)) => (
            StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY),
            Json(envelope),
        )
            .into_response(),
        // The daemon could not be reached, even after the autostart retry. That
        // is a fact about this hop, not about the operation, so it gets a status
        // of its own — in the same envelope shape, so a caller parses one thing.
        Err(e) => {
            tracing::warn!("memory passthrough: {verb} failed: {e:#}");
            unreachable_envelope(StatusCode::BAD_GATEWAY, format!("{e:#}"), "UNREACHABLE")
        }
    }
}

/// ling-mem's own error shape, for the failures that never reach it. One shape
/// on this route in every case, so the phone has a single thing to read.
fn unreachable_envelope(status: StatusCode, message: String, code: &str) -> Response {
    (
        status,
        Json(json!({ "ok": false, "error": message, "code": code })),
    )
        .into_response()
}

/// The verb becomes a path segment in a URL we build, so it must not be able to
/// climb out of `/api/memory/`. Axum hands us a single decoded segment — decoded
/// being the point: `%2F` arrives as a real slash, and a check on the raw path
/// would never see it. Every verb ling-mem serves is `[a-z_]`, so anything else
/// is refused here rather than concatenated into a URL and sent.
pub(crate) fn is_safe_verb(verb: &str) -> bool {
    !verb.is_empty() && verb.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Does this verb change the store?
///
/// `None` for a verb we do not know, which callers must refuse rather than
/// guess about — a verb that is neither provably a read nor provably a write
/// is exactly the one not to forward on a promise.
///
/// This exists because the capability door names its tools `Memory_query` and
/// `Memory_write`, which reads as a read/write split — and nothing checked it.
/// Dispatch is by the `verb` in the body, so a page could POST `Memory_query`
/// with `verb: "delete"` and delete. A promise nothing enforces is worse than
/// no promise: it is a knob that reads as a fence.
pub(crate) fn verb_mutates(verb: &str) -> Option<bool> {
    match verb {
        "search" | "list" | "get" | "count" | "days" | "chains" | "issues" | "stats" => Some(false),
        "add" | "add_batch" | "update" | "delete" | "forget" | "sweep" | "remember_day"
        | "harvest_day" | "issue_add" | "issue_resolve" => Some(true),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_verb_ling_mem_serves_is_accepted() {
        for verb in [
            "search",
            "list",
            "get",
            "add",
            "add_batch",
            "count",
            "update",
            "delete",
            "forget",
            "days",
            "remember_day",
            "harvest_day",
            "sweep",
            "chains",
            "issues",
            "issue_add",
            "issue_resolve",
            "stats",
        ] {
            assert!(is_safe_verb(verb), "{verb} should be forwardable");
        }
    }

    /// Every verb the daemon serves must be classified, or the capability door
    /// refuses it: `verb_mutates` returning `None` is a closed door, not an
    /// open one. This is the pairing that keeps a new ling-mem verb from
    /// silently becoming unreachable from a skill page.
    #[test]
    fn every_forwardable_verb_is_classified_as_read_or_write() {
        for verb in [
            "search",
            "list",
            "get",
            "count",
            "days",
            "chains",
            "issues",
            "stats",
            "add",
            "add_batch",
            "update",
            "delete",
            "forget",
            "sweep",
            "remember_day",
            "harvest_day",
            "issue_add",
            "issue_resolve",
        ] {
            assert!(is_safe_verb(verb), "{verb} should be forwardable");
            assert!(verb_mutates(verb).is_some(), "{verb} should be classified");
        }
    }

    /// The split the capability door's two tool names promise. `Memory_query`
    /// dispatching a delete is the bug this encodes.
    #[test]
    fn reads_and_writes_are_told_apart() {
        for verb in [
            "search", "list", "get", "count", "days", "chains", "issues", "stats",
        ] {
            assert_eq!(verb_mutates(verb), Some(false), "{verb} does not mutate");
        }
        for verb in [
            "add",
            "update",
            "delete",
            "forget",
            "sweep",
            "issue_resolve",
        ] {
            assert_eq!(verb_mutates(verb), Some(true), "{verb} mutates");
        }
        assert_eq!(verb_mutates("wat"), None);
        assert_eq!(verb_mutates(""), None);
    }

    /// A percent-encoded traversal is decoded before it reaches us, so these
    /// are what the guard actually has to stop.
    #[test]
    fn a_verb_that_could_leave_the_namespace_is_refused() {
        for verb in [
            "",
            "../health",
            "..",
            "a/b",
            "search?x=1",
            "search#f",
            "sea rch",
            "s\\b",
        ] {
            assert!(!is_safe_verb(verb), "{verb:?} should be refused");
        }
    }
}
