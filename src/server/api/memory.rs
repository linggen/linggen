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
//! verb segment handed to the memory this engine hosts (in-process,
//! `memory_host::post_verb`), and its answer returned whole. It is
//! transport, exactly like the tunnel it rides on.
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
//! **Whose memory** is decided by `memory_host::caller`, the one rule every
//! memory route uses: this machine and an approved machine (a `machine` row)
//! are the owner; a phone is its own person. Two phones with two accounts can
//! pair to one Mac, and the store must never hand one person the other's
//! life. So every verb a phone sends is stamped here with the person behind
//! it — read from the pairing record the Mac keeps, never from anything the
//! phone put in the body — and memory scopes the verb to that person. The
//! owner's rows are the store's unmarked rows; a phone signed in to this
//! Mac's own account is the owner too. A signed-out phone is stamped
//! `device:<id>` and re-stamped to the account the day it signs in
//! (`restamp_device`). See `phone-memory-spec.md`.

use axum::{
    extract::{ConnectInfo, Json, Path},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde_json::{json, Value};
use std::net::SocketAddr;

use crate::memory_host::caller::{self, Caller};
use ling_mem::{Person, Who};

/// `POST /api/memory/<verb>` — the verb, answered by the store this engine
/// hosts (`memory_host::post_verb`, in-process); the phone never learns a
/// port, stores one, or needs a rebuild.
///
/// For the owner the verb is passed through rather than checked against a
/// list: memory is the authority on which verbs exist, and an unknown one
/// gets memory's own 404. A phone that is another person may run only the
/// verbs memory keeps per person (`ling_mem::Who`).
pub async fn passthrough(
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path(verb): Path<String>,
    headers: HeaderMap,
    body: Option<Json<Value>>,
) -> Response {
    let args = body.map(|Json(v)| v).unwrap_or_else(|| json!({}));
    let asker = asker_of(&caller::resolve(peer.ip(), &headers));
    answer(&asker, &verb, args).await
}

/// How long the door, and the engine's own memory helpers, wait for a verb.
pub(crate) const VERB_TIMEOUT: std::time::Duration =
    crate::engine::tools::memory_http::DISPATCH_TIMEOUT;

/// The door's answer to `asker` for `verb` with `args`.
async fn answer(asker: &Asker, verb: &str, mut args: Value) -> Response {
    if !is_safe_verb(verb) {
        return unreachable_envelope(
            StatusCode::BAD_REQUEST,
            format!("invalid memory verb: {verb}"),
            "BAD_REQUEST",
        );
    }
    let who = match fit(asker, verb, &mut args) {
        Ok(who) => who,
        Err((status, message, code)) => return unreachable_envelope(status, message, code),
    };

    // The engine logs its own memory calls (`call_memory_http`); this path
    // is the one that arrives from another machine, so it is the one worth
    // seeing in the log at all. Keys only — a memory row's content is the
    // user's, and this line is not where it should turn up.
    tracing::info!(
        "memory passthrough → {verb} {:?}",
        args.as_object()
            .map(|o| o.keys().collect::<Vec<_>>())
            .unwrap_or_default()
    );

    match crate::memory_host::post_verb(who, verb, args, VERB_TIMEOUT).await {
        // Memory answered. Its envelope is the answer — `{ok, data}` or
        // `{ok:false, error, code}` — and it goes back untouched, code and all,
        // under memory's own status. Rewriting a failure into a bare status
        // would throw away the one field that says what went wrong; flattening
        // a 404 into 200 would claim a verb exists that doesn't.
        Ok((status, envelope)) => (
            StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY),
            Json(envelope),
        )
            .into_response(),
        // No answer: no memory in this process, or none in time. That is a
        // fact about this hop, not about the operation, so it gets a status of
        // its own — in the same envelope shape, so a caller parses one thing.
        Err(e) => {
            tracing::warn!("memory passthrough: {verb} failed: {e:#}");
            unreachable_envelope(StatusCode::BAD_GATEWAY, format!("{e:#}"), "UNREACHABLE")
        }
    }
}

/// Who is asking memory, as the door tells them apart.
#[derive(Debug, Clone)]
pub(crate) enum Asker {
    /// This machine, or a machine it approved.
    Owner,
    /// A phone: `None` = the owner's own, else the person behind it.
    Phone(Option<Person>),
    /// No one memory knows.
    Unknown,
}

pub(crate) fn asker_of(caller: &Caller) -> Asker {
    match caller {
        Caller::Owner | Caller::Machine(_) => Asker::Owner,
        Caller::Phone(device) => Asker::Phone(person_of(device)),
        Caller::Unknown => Asker::Unknown,
    }
}

/// Fit a memory verb's args to whoever is asking (`memory_host::caller`),
/// for a caller that then runs them as given (the owner's call): the door's
/// own checks, then memory's for the person behind a phone (`Who::scope` —
/// refused, or stamped). A skill page's memory calls come through here, so
/// the engine's clean-ups after it see the stamped body.
pub(crate) fn stamp_for_caller(
    caller: &Caller,
    verb: &str,
    args: &mut Value,
) -> Result<(), Response> {
    let who = fit(&asker_of(caller), verb, args)
        .map_err(|(status, message, code)| unreachable_envelope(status, message, code))?;
    who.scope(verb, args).map_err(refused)
}

fn fit(asker: &Asker, verb: &str, args: &mut Value) -> Result<Who, Refusal> {
    match asker {
        Asker::Owner => Ok(Who::Owner),
        Asker::Phone(person) => scope_phone(verb, args, person.as_ref()),
        Asker::Unknown => Err((
            StatusCode::FORBIDDEN,
            "memory does not know this caller".to_string(),
            "FORBIDDEN",
        )),
    }
}

/// A phone's request: `person` is who is behind it (`None` = the owner's
/// own phone). No phone runs a verb only this machine's owner may
/// (`ling_mem::call::machine_only`). The owner's own phone runs any other as
/// the owner, its account claims taken off; a phone that is another person
/// is that person, and memory fits the call to them (`Who::scope`: only the
/// verbs kept per person, with a body the stamp can mark, then the stamp).
fn scope_phone(verb: &str, args: &mut Value, person: Option<&Person>) -> Result<Who, Refusal> {
    if ling_mem::call::machine_only(verb) {
        return Err((
            StatusCode::FORBIDDEN,
            format!("{verb} is the Mac's to run, not a phone's"),
            "FORBIDDEN",
        ));
    }
    let Some(person) = person else {
        ling_mem::call::stamp(verb, args, None);
        return Ok(Who::Owner);
    };
    Ok(Who::Person(person.clone()))
}

/// Why the door refused: status, message, envelope code.
type Refusal = (StatusCode, String, &'static str);

/// Where a device's rows go when it gains `gained`: `Some(true)` to the
/// owner (the device is the owner's, verified), `Some(false)` to that
/// account, `None` nowhere (this Mac's account, claimed but not verified,
/// or an id that is not an account's own: `person::names_an_account`).
fn restamp_to_owner(device_id: &str, gained: &str, owner: Option<&str>) -> Option<bool> {
    let devices = crate::state_fs::devices::load_devices();
    let device = devices.iter().find(|d| d.id == device_id);
    owners_rows(device, gained, owner)
}

fn owners_rows(
    device: Option<&crate::server::api::pair::PairedDevice>,
    gained: &str,
    owner: Option<&str>,
) -> Option<bool> {
    if owner != Some(gained) {
        return crate::server::person::names_an_account(gained, owner).then_some(false);
    }
    device.is_some_and(|d| d.is_owners(owner)).then_some(true)
}

/// Memory's refusal, in the shape the door answers its own refusals with
/// (memory's own envelope).
fn refused(e: ling_mem::MemError) -> Response {
    let status = StatusCode::from_u16(e.code.http_status()).unwrap_or(StatusCode::FORBIDDEN);
    unreachable_envelope(status, e.message, e.code.code())
}

/// Whose name labels a caller's own messages.
#[derive(Debug, PartialEq)]
pub(crate) enum OwnName {
    /// The owner (this machine, an approved machine, the owner's own phone):
    /// core memory states it.
    Core,
    /// A phone that is another person: the name its pairing record holds. A
    /// signed-out phone is nobody yet and has none.
    Held(Option<String>),
    /// A caller memory does not know.
    NoOne,
}

pub(crate) fn own_name_of(caller: &Caller) -> OwnName {
    match caller {
        Caller::Owner | Caller::Machine(_) => OwnName::Core,
        Caller::Unknown => OwnName::NoOne,
        Caller::Phone(device) => match person_of(device) {
            None => OwnName::Core,
            Some(person) => OwnName::Held(device.account.as_ref().and(person.name)),
        },
    }
}

/// Who is behind this device, for the store, as `server::person::of_device`
/// resolves it — the one answer a device's standing and its day take too.
/// The owner's own device gets no stamp; anyone else is stamped as the
/// person they are, named as their account or their device is.
fn person_of(device: &crate::server::api::pair::PairedDevice) -> Option<Person> {
    let owner = crate::server::person::mac_account();
    person_for(device, owner.as_deref())
}

fn person_for(
    device: &crate::server::api::pair::PairedDevice,
    owner_id: Option<&str>,
) -> Option<Person> {
    let person = crate::server::person::of_device(device, owner_id);
    if person.is_owner() {
        return None;
    }
    let name = match (&device.account, person.is_device()) {
        (Some(a), false) => a
            .name
            .clone()
            .or_else(|| a.email.clone())
            .filter(|s| !s.trim().is_empty()),
        _ => Some(device.name.clone()),
    };
    Some(Person {
        id: person.wire(),
        name,
    })
}

/// A phone that signed in, or was verified as the owner's, just now: hand the
/// rows it wrote as `device:<id>` to the person it turned out to be — or to
/// the owner, when its claim of this Mac's account is verified. A claim of
/// this Mac's account not verified hands them to no one: they stay the
/// device's. Failure is logged, not surfaced: the rows are still there under
/// the device stamp, and the next sign-in tries again.
pub async fn restamp_device(device_id: &str, gained: &crate::server::api::pair::AccountRef) {
    let owner = crate::server::person::mac_account();
    let Some(to_owner) = restamp_to_owner(device_id, &gained.id, owner.as_deref()) else {
        return;
    };
    let body = json!({
        "from": crate::server::person::stamp_of(device_id),
        "account_id": if to_owner { Value::Null } else { json!(gained.id) },
        "account_name": if to_owner {
            Value::Null
        } else {
            json!(gained.name.clone().or_else(|| gained.email.clone()))
        },
    });
    match crate::memory_host::post_verb(Who::Owner, "restamp", body, VERB_TIMEOUT).await {
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
mod door_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::api::pair::{AccountRef, PairedDevice};

    fn device(account: Option<(&str, Option<&str>)>) -> PairedDevice {
        PairedDevice {
            id: "dev-1".into(),
            name: "Alex's iPhone".into(),
            secret: "s".into(),
            created_at: 0,
            kind: None,
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
            ..Default::default()
        }
    }

    /// The day's lookup takes its person from the same answer: the owner's
    /// other computer is the owner, a phone signed in to another account is
    /// that account, a signed-out phone its device.
    #[test]
    fn the_person_behind_a_device_is_the_doors() {
        let machine = PairedDevice {
            kind: Some(crate::state_fs::devices::DeviceKind::Machine),
            ..device(None)
        };
        assert_eq!(person_for(&machine, None), None);
        let phone = PairedDevice {
            kind: Some(crate::state_fs::devices::DeviceKind::Phone),
            ..device(Some(("u-2", None)))
        };
        assert_eq!(person_for(&phone, None).map(|p| p.id).as_deref(), Some("u-2"));
        let signed_out = PairedDevice {
            kind: Some(crate::state_fs::devices::DeviceKind::Phone),
            ..device(None)
        };
        assert_eq!(person_for(&signed_out, None).map(|p| p.id).as_deref(), Some("device:dev-1"));
    }

    #[test]
    fn owner_phone_is_nobody_special() {
        // Signed in to the Mac's own account, verified: unstamped, like the
        // Mac itself.
        let verified = PairedDevice {
            verified_account: Some("u-owner".into()),
            ..device(Some(("u-owner", Some("Alex"))))
        };
        assert_eq!(person_for(&verified, Some("u-owner")), None);
    }

    /// A claim of the Mac's own account that the relay never vouched for is
    /// not the owner: the phone is stamped as its device, and the rows it
    /// wrote signed out go to no one when it says it signed in.
    #[test]
    fn an_unverified_claim_of_the_owners_account_is_its_device() {
        let claimed = device(Some(("u-owner", Some("Alex"))));
        let p = person_for(&claimed, Some("u-owner")).unwrap();
        assert_eq!(p.id, "device:dev-1");
        assert_eq!(owners_rows(Some(&claimed), "u-owner", Some("u-owner")), None);
        let verified = PairedDevice {
            verified_account: Some("u-owner".into()),
            ..claimed
        };
        assert_eq!(owners_rows(Some(&verified), "u-owner", Some("u-owner")), Some(true));
        assert_eq!(owners_rows(None, "u-2", Some("u-owner")), Some(false));
    }

    /// A claimed account id that reads as another kind of person is no
    /// account: the phone is stamped as its device, and the rows it wrote
    /// signed out are handed to no one.
    #[test]
    fn a_claim_that_reads_as_another_person_is_stamped_as_its_device() {
        let local = ["__", "local__"].concat();
        for claim in ["", local.as_str(), "device:dev-2", " u-2"] {
            for mac in [Some("u-owner"), None] {
                let d = device(Some((claim, None)));
                assert_eq!(person_for(&d, mac).unwrap().id, "device:dev-1", "{claim:?} {mac:?}");
                assert_eq!(owners_rows(Some(&d), claim, mac), None, "{claim:?} {mac:?}");
            }
        }
        assert_eq!(owners_rows(None, "u-2", None), Some(false));
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
        assert_eq!(p.name.as_deref(), Some("Alex's iPhone"));
        // A Mac with no account of its own still tells a guest from itself.
        assert_eq!(person_for(&device(None), None).unwrap().id, "device:dev-1");
    }

    /// A phone that is another person is shown its own name, never the
    /// owner's; a signed-out phone has none; the owner and an approved
    /// machine get core memory's; a caller nobody knows gets no answer.
    #[test]
    fn a_callers_own_name_is_never_the_owners_for_another_person() {
        let alex = device(Some(("u-3", Some("Alex"))));
        assert_eq!(
            own_name_of(&Caller::Phone(alex)),
            OwnName::Held(Some("Alex".to_string()))
        );
        let signed_out = device(None);
        assert_eq!(own_name_of(&Caller::Phone(signed_out)), OwnName::Held(None));
        assert_eq!(own_name_of(&Caller::Unknown), OwnName::NoOne);
        assert_eq!(own_name_of(&Caller::Owner), OwnName::Core);
        let mac = PairedDevice {
            kind: Some(crate::state_fs::devices::DeviceKind::Machine),
            ..device(None)
        };
        assert_eq!(own_name_of(&Caller::Machine(mac)), OwnName::Core);
    }

    /// A body that names another person: the owner and an approved machine
    /// may say whose rows they mean; a phone's claim is replaced by its own
    /// person; a caller memory does not know is refused.
    #[test]
    fn whose_rows_comes_from_the_caller_not_the_body() {
        let body = || json!({"query": "x", "account": "u-2", "all_accounts": true});

        let mut owner = body();
        assert!(stamp_for_caller(&Caller::Owner, "search", &mut owner).is_ok());
        assert_eq!(owner, body());

        let mut machine = body();
        let mac = PairedDevice {
            kind: Some(crate::state_fs::devices::DeviceKind::Machine),
            ..device(None)
        };
        assert!(stamp_for_caller(&Caller::Machine(mac), "search", &mut machine).is_ok());
        assert_eq!(machine, body());

        let mut phone = body();
        let alex = device(Some(("u-3", Some("Alex"))));
        assert!(stamp_for_caller(&Caller::Phone(alex.clone()), "search", &mut phone).is_ok());
        assert_eq!(phone["account"], "u-3");
        assert!(phone.get("all_accounts").is_none());
        assert!(stamp_for_caller(&Caller::Phone(alex), "accounts", &mut body()).is_err());

        let mut unknown = body();
        assert!(stamp_for_caller(&Caller::Unknown, "search", &mut unknown).is_err());
    }

    /// A skill page open on another person's phone: its body is stamped
    /// before the engine's clean-ups, and memory's refusals come back whole.
    #[tokio::test]
    async fn a_skill_pages_call_from_another_persons_phone_is_stamped_or_refused() {
        let guest = Caller::Phone(device(Some(("u-3", Some("Alex")))));
        let mut add = json!({"verb": "add", "content": "x", "account_id": "u-1", "all_accounts": true});
        assert!(stamp_for_caller(&guest, "add", &mut add).is_ok());
        assert_eq!(
            add,
            json!({"verb": "add", "content": "x", "account_id": "u-3", "account_name": "Alex"})
        );
        async fn refusal(res: Response) -> (u16, Value) {
            let status = res.status().as_u16();
            let bytes = axum::body::to_bytes(res.into_body(), 1 << 20).await.unwrap();
            (status, serde_json::from_slice(&bytes).unwrap())
        }
        let accounts = stamp_for_caller(&guest, "accounts", &mut json!({})).unwrap_err();
        assert_eq!(
            refusal(accounts).await,
            (
                403,
                json!({"ok": false, "code": "FORBIDDEN", "error": "accounts is the Mac's to run, not a phone's"})
            )
        );
        let mut batch = json!({"verb": "add_batch", "facts": ["x"]});
        let batch = stamp_for_caller(&guest, "add_batch", &mut batch).unwrap_err();
        assert_eq!(
            refusal(batch).await,
            (
                400,
                json!({"ok": false, "code": "BAD_REQUEST", "error": "add_batch takes a JSON object (and add_batch a list of them in facts)"})
            )
        );
        let stats = stamp_for_caller(&guest, "stats", &mut json!({})).unwrap_err();
        assert_eq!(
            refusal(stats).await,
            (
                403,
                json!({"ok": false, "code": "FORBIDDEN", "error": "stats is not kept per person, so only the Mac's owner may run it"})
            )
        );
    }

    /// Every `/api/memory/<verb>` memory routes, read from its router's
    /// source (axum cannot list a router's routes), so a verb memory gains
    /// later is in this table without anyone adding it.
    fn memory_router_verbs() -> Vec<String> {
        let dir =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("crates/linggen-memory/src/http");
        let mut verbs = Vec::new();
        let mut pending = vec![dir];
        while let Some(d) = pending.pop() {
            for entry in std::fs::read_dir(&d).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    pending.push(path);
                    continue;
                }
                let text = std::fs::read_to_string(&path).unwrap_or_default();
                verbs.extend(text.lines().filter_map(routed_verb));
            }
        }
        verbs.sort();
        verbs.dedup();
        verbs
    }

    fn routed_verb(line: &str) -> Option<String> {
        let (_, rest) = line.split_once(".route(\"/api/memory/")?;
        let (verb, _) = rest.split_once('"')?;
        Some(verb.to_string())
    }

    /// For a phone that is another person, every verb memory serves is
    /// either on the allow-list or refused; for the owner, an approved
    /// machine and the owner's own phone nothing changed.
    #[test]
    fn a_phone_that_is_another_person_runs_only_its_persons_verbs() {
        const PERSON_VERBS: [&str; 10] = [
            "search",
            "list",
            "count",
            "get",
            "add",
            "add_batch",
            "update",
            "delete",
            "forget",
            "session_start",
        ];
        const DEVICE_FORBIDDEN_VERBS: [&str; 2] = ["restamp", "accounts"];
        let verbs = memory_router_verbs();
        assert!(verbs.len() >= 20, "router scan found {verbs:?}");
        for v in PERSON_VERBS {
            assert!(verbs.iter().any(|r| r == v), "{v} is not a memory verb");
        }
        let alex = Person {
            id: "u-3".into(),
            name: Some("Alex".into()),
        };
        let mac = PairedDevice {
            kind: Some(crate::state_fs::devices::DeviceKind::Machine),
            ..device(None)
        };
        let body = || json!({"query": "x", "facts": [{"content": "a"}], "account": "u-9"});
        for verb in verbs.iter().map(String::as_str).chain(["wat"]) {
            let mut args = body();
            let allowed = fitted(verb, &mut args, &alex);
            assert_eq!(
                allowed,
                PERSON_VERBS.contains(&verb),
                "{verb} for another person"
            );

            let mut args = body();
            let mut owner_phone = body();
            ling_mem::call::stamp(verb, &mut owner_phone, None);
            let mac_forbidden = DEVICE_FORBIDDEN_VERBS.contains(&verb);
            assert_eq!(
                scope_phone(verb, &mut args, None).is_ok(),
                !mac_forbidden,
                "{verb}"
            );
            if !mac_forbidden {
                assert_eq!(args, owner_phone, "{verb} for the owner's phone");
            }

            for caller in [Caller::Owner, Caller::Machine(mac.clone())] {
                let mut args = body();
                assert!(stamp_for_caller(&caller, verb, &mut args).is_ok(), "{verb}");
                assert_eq!(args, body(), "{verb} for the owner");
            }
        }
    }

    /// The door's checks, then memory's for the person: may it run?
    fn fitted(verb: &str, args: &mut Value, person: &Person) -> bool {
        scope_phone(verb, args, Some(person)).is_ok_and(|who| who.scope(verb, args).is_ok())
    }

    /// A body the stamp cannot mark would reach memory as the owner's; for
    /// another person it is refused instead.
    #[test]
    fn an_unstampable_body_is_refused_for_another_person() {
        let alex = Person {
            id: "u-3".into(),
            name: None,
        };
        for (verb, mut args) in [
            ("session_start", json!([])),
            ("add", json!(["likes tea"])),
            ("add_batch", json!({"facts": [{"content": "a"}, "b"]})),
            ("add_batch", json!({"facts": "a"})),
        ] {
            assert!(!fitted(verb, &mut args, &alex), "{verb} {args}");
        }
        let mut ok = json!({"facts": [{"content": "a"}]});
        assert!(fitted("add_batch", &mut ok, &alex));
        assert_eq!(ok["facts"][0]["account_id"], "u-3");
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

    /// Every `.rs` file under `dir`, with its text up to its test module and
    /// without comment lines.
    fn code_under(dir: &std::path::Path) -> Vec<(std::path::PathBuf, String)> {
        let mut out = Vec::new();
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                out.extend(code_under(&path));
                continue;
            }
            if path.extension().is_none_or(|e| e != "rs") || path.ends_with("door_tests.rs") {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            let code = text.split("#[cfg(test)]").next().unwrap_or("");
            let lines: Vec<&str> = code
                .lines()
                .filter(|l| !l.trim_start().starts_with("//"))
                .collect();
            out.push((path, lines.join("\n")));
        }
        out
    }

    /// The engine never POSTs a memory verb to itself: every verb call is
    /// in-process (`memory_host::post_verb`). What the scan allows: the
    /// door's own route (`/api/memory/{verb}`, the phone's way in), the
    /// words of `memory_host::routes`' refusal of a phone, the address a
    /// memory call's log line names (`memory_host::route_of`), and the
    /// address memory's MCP server is reached at by `mcp_client`
    /// (`memory_host::url` handed to `with_builtin`), which stays HTTP on
    /// purpose. Its limits:
    /// it reads each file up to its first `#[cfg(test)]`, skips comment
    /// lines, and knows a memory verb by `api/memory` in the line — an
    /// address built from pieces elsewhere would get past it.
    #[test]
    fn no_memory_verb_is_posted_to_this_engine() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut files = code_under(&root.join("src"));
        files.extend(code_under(&root.join("crates/linggen-engine/src")));
        assert!(files.len() > 100, "scanned {} files", files.len());
        const ALLOWED: [&str; 4] = [
            "format!(\"{}/api/memory/{verb}\", url())",
            ".route(\"/api/memory/{verb}\", post(api::memory::passthrough))",
            "(\"POST\", \"/api/memory/{verb}\", Own),",
            "\"a phone reaches memory at /api/memory/<verb>, and sees its own person's rows\",",
        ];
        for (path, code) in &files {
            for line in code.lines().map(str::trim) {
                if line.contains("api/memory") {
                    assert!(ALLOWED.contains(&line), "{}: {line}", path.display());
                }
                for gone in ["post_memory_verb", "call_memory_http", "TEST_URL"] {
                    assert!(!line.contains(gone), "{}: {line}", path.display());
                }
                if line.contains("memory_host::url()") {
                    assert!(
                        line.contains("mcp_client::with_builtin("),
                        "{}: {line}",
                        path.display()
                    );
                }
            }
        }
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
