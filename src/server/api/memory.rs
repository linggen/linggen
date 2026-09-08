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
//!
//! **Whose memory.** Two phones with two accounts can pair to one Mac, and the
//! store must never hand one person the other's life. So every verb that
//! arrives from a paired device is stamped here with the person behind it —
//! read from the pairing record the Mac keeps, never from anything the phone
//! put in the body — and the daemon scopes the verb to that person. The Mac's
//! owner (a phone signed in to this Mac's own account, or no device at all)
//! gets no stamp: their rows are the store's unmarked rows, as they always
//! were. A signed-out phone is stamped `device:<id>` and re-stamped to the
//! account the day it signs in (`restamp_device`). See `phone-memory-spec.md`.

use axum::{
    extract::{Json, Path, State},
    http::{HeaderMap, StatusCode},
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
    headers: HeaderMap,
    body: Option<Json<Value>>,
) -> Response {
    if !is_safe_verb(&verb) {
        return unreachable_envelope(
            StatusCode::BAD_REQUEST,
            format!("invalid memory verb: {verb}"),
            "BAD_REQUEST",
        );
    }

    let mut args = body.map(|Json(v)| v).unwrap_or_else(|| json!({}));
    if let Some(device) = crate::server::api::pair::device_for_headers(&headers) {
        if DEVICE_FORBIDDEN_VERBS.contains(&verb.as_str()) {
            return unreachable_envelope(
                StatusCode::FORBIDDEN,
                format!("{verb} is the Mac's to run, not a phone's"),
                "FORBIDDEN",
            );
        }
        stamp_person(&verb, &mut args, person_of(&device).as_ref());
    }
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

/// Verbs a device may not run: they cross account lines by design, and only
/// the Mac's own logic ever has cause to.
const DEVICE_FORBIDDEN_VERBS: &[&str] = &["restamp", "accounts"];

/// The person a phone's memory rows belong to, as the store names them.
/// `None` = the Mac's owner: rows stay unstamped.
#[derive(Debug, Clone, PartialEq)]
pub struct Person {
    pub id: String,
    pub name: Option<String>,
}

/// Who is behind this device, for the store. The Mac's own account is the
/// owner and gets no stamp; any other account is stamped as itself; a
/// signed-out phone is stamped as its device until it signs in.
fn person_of(device: &crate::server::api::pair::PairedDevice) -> Option<Person> {
    let owner = crate::account::load_account().and_then(|a| a.user_id);
    person_for(device, owner.as_deref())
}

fn person_for(
    device: &crate::server::api::pair::PairedDevice,
    owner_id: Option<&str>,
) -> Option<Person> {
    match &device.account {
        Some(a) if Some(a.id.as_str()) == owner_id => None,
        Some(a) => Some(Person {
            id: a.id.clone(),
            name: a
                .name
                .clone()
                .or_else(|| a.email.clone())
                .filter(|s| !s.trim().is_empty()),
        }),
        None => Some(Person {
            id: device_stamp(&device.id),
            name: Some(device.name.clone()),
        }),
    }
}

/// The stamp a signed-out phone writes under.
fn device_stamp(device_id: &str) -> String {
    format!("device:{device_id}")
}

/// Put the person on the wire, and take off anything the phone said about
/// accounts itself — the pairing record is the only authority here. `add`
/// and `add_batch` mark the rows they write; every other verb is scoped to
/// the person's rows. The owner's request ends up carrying nothing, which
/// the daemon reads as the owner.
fn stamp_person(verb: &str, args: &mut Value, person: Option<&Person>) {
    const CLAIMS: &[&str] = &["account", "account_id", "account_name", "all_accounts"];
    let strip = |o: &mut serde_json::Map<String, Value>| {
        for k in CLAIMS {
            o.remove(*k);
        }
    };
    let Some(obj) = args.as_object_mut() else {
        return;
    };
    strip(obj);
    match verb {
        "add" => {
            if let Some(p) = person {
                obj.insert("account_id".into(), json!(p.id));
                obj.insert("account_name".into(), json!(p.name));
            }
        }
        "add_batch" => {
            if let Some(facts) = obj.get_mut("facts").and_then(|f| f.as_array_mut()) {
                for f in facts.iter_mut() {
                    let Some(fo) = f.as_object_mut() else {
                        continue;
                    };
                    strip(fo);
                    if let Some(p) = person {
                        fo.insert("account_id".into(), json!(p.id));
                        fo.insert("account_name".into(), json!(p.name));
                    }
                }
            }
        }
        _ => {
            if let Some(p) = person {
                obj.insert("account".into(), json!(p.id));
            }
        }
    }
}

/// A phone that signed in just now: hand the rows it wrote as `device:<id>`
/// to the person it turned out to be — or back to the owner, if it was the
/// owner's phone all along. Failure is logged, not surfaced: the rows are
/// still there under the device stamp, and the next sign-in tries again.
pub async fn restamp_device(
    state: &Arc<ServerState>,
    device_id: &str,
    gained: &crate::server::api::pair::AccountRef,
) {
    let owner = crate::account::load_account().and_then(|a| a.user_id);
    let to_owner = owner.as_deref() == Some(gained.id.as_str());
    let body = json!({
        "from": device_stamp(device_id),
        "account_id": if to_owner { Value::Null } else { json!(gained.id) },
        "account_name": if to_owner {
            Value::Null
        } else {
            json!(gained.name.clone().or_else(|| gained.email.clone()))
        },
    });
    let ling_mem_url = state.manager.get_config_snapshot().await.agent.ling_mem_url;
    match crate::engine::tools::memory_http::post_memory_verb(&ling_mem_url, "restamp", &body).await
    {
        Ok((_, env)) => tracing::info!(
            "memory restamp for device {device_id}: {}",
            env.get("data").map(|d| d.to_string()).unwrap_or_default()
        ),
        Err(e) => tracing::warn!("memory restamp for device {device_id} failed: {e:#}"),
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
        | "harvest_day" | "issue_add" | "issue_resolve" | "restamp" => Some(true),
        "accounts" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::api::pair::{AccountRef, PairedDevice};

    fn device(account: Option<(&str, Option<&str>)>) -> PairedDevice {
        PairedDevice {
            id: "dev-1".into(),
            name: "Liang's iPhone".into(),
            secret: "s".into(),
            created_at: 0,
            account: account.map(|(id, name)| AccountRef {
                id: id.into(),
                email: Some("x@y.z".into()),
                name: name.map(str::to_string),
                at: None,
            }),
            device_id: None,
            settings: serde_json::Map::new(),
            relay_grant: None,
            superseded_grant: None,
        }
    }

    #[test]
    fn owner_phone_is_nobody_special() {
        // Signed in to the Mac's own account: unstamped, like the Mac itself.
        assert_eq!(
            person_for(&device(Some(("u-owner", Some("Liang")))), Some("u-owner")),
            None
        );
    }

    #[test]
    fn another_account_is_stamped_as_itself() {
        let p = person_for(&device(Some(("u-2", Some("Mei")))), Some("u-owner")).unwrap();
        assert_eq!(p.id, "u-2");
        assert_eq!(p.name.as_deref(), Some("Mei"));
        // No display name → the email labels them; never blank.
        let p = person_for(&device(Some(("u-2", None))), Some("u-owner")).unwrap();
        assert_eq!(p.name.as_deref(), Some("x@y.z"));
    }

    #[test]
    fn signed_out_phone_is_its_device() {
        let p = person_for(&device(None), Some("u-owner")).unwrap();
        assert_eq!(p.id, "device:dev-1");
        assert_eq!(p.name.as_deref(), Some("Liang's iPhone"));
        // A Mac with no account of its own still tells a guest from itself.
        assert_eq!(person_for(&device(None), None).unwrap().id, "device:dev-1");
    }

    #[test]
    fn stamp_marks_writes_and_scopes_reads() {
        let mei = Person {
            id: "u-2".into(),
            name: Some("Mei".into()),
        };
        // The phone's own claims come off first, whoever it is.
        let mut add =
            json!({"content": "likes 90s HK songs", "account_id": "u-owner", "all_accounts": true});
        stamp_person("add", &mut add, Some(&mei));
        assert_eq!(add["account_id"], "u-2");
        assert_eq!(add["account_name"], "Mei");
        assert!(add.get("all_accounts").is_none());

        let mut list = json!({"tier": "core", "account": "u-owner"});
        stamp_person("list", &mut list, Some(&mei));
        assert_eq!(list["account"], "u-2");

        let mut batch =
            json!({"facts": [{"content": "a", "account": "u-owner"}, {"content": "b"}]});
        stamp_person("add_batch", &mut batch, Some(&mei));
        for f in batch["facts"].as_array().unwrap() {
            assert_eq!(f["account_id"], "u-2");
            assert!(f.get("account").is_none());
        }

        // The owner's phone: claims stripped, nothing added.
        let mut owner = json!({"query": "x", "account": "u-2", "account_id": "u-2"});
        stamp_person("search", &mut owner, None);
        assert_eq!(owner, json!({"query": "x"}));
    }

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
