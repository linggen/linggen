//! The door `POST /api/memory/<verb>` against a real hosted store (scratch
//! dir, memory's deterministic test embedder, called in-process),
//! as three callers: the owner (this machine), the owner's own phone, and
//! the phone of another person. What each may run is pinned verb by verb
//! as literals, so a later table-driven door must reproduce it exactly.
//!
//! How the callers are made, without touching this machine's pairing
//! records or account file:
//! - the owner: the real `passthrough` handler from loopback;
//! - another person's phone: the door's `answer` for the asker a phone row
//!   signed in to another account resolves to (`asker_of`);
//! - the owner's own phone: `answer` for a phone with no person — which
//!   phone is the owner's is decided from `account.toml` (`person_of`,
//!   unit-tested above), and a test must not write that file.
//!
//! `memory_host` reaches the scratch store in-process, for each test's
//! future alone (`memory_host::TEST_HOST`).

use super::*;
use crate::server::api::pair::{AccountRef, PairedDevice};
use axum::body::to_bytes;
use std::net::SocketAddr;

struct Store {
    base: String,
    _dir: tempfile::TempDir,
}

/// The scratch stores by name, for `door` to hand to `memory_host`.
static STORES: std::sync::Mutex<Vec<(String, ling_mem::http::state::SharedState)>> =
    std::sync::Mutex::new(Vec::new());

async fn store() -> Store {
    let dir = tempfile::tempdir().unwrap();
    let base = format!("scratch://{}", dir.path().display());
    let state = ling_mem::hosted::open_quiet_fake(dir.path(), base.clone())
        .await
        .unwrap();
    STORES.lock().unwrap().push((base.clone(), state));
    Store { base, _dir: dir }
}

/// What `url` names for `memory_host`: a scratch store; `""` — no memory
/// in this process; anything else — a store that never answers.
fn host_at(url: &str) -> crate::memory_host::TestHost {
    use crate::memory_host::TestHost;
    if url.is_empty() {
        return TestHost::Absent;
    }
    let stores = STORES.lock().unwrap();
    match stores.iter().find(|(base, _)| base == url) {
        Some((_, state)) => TestHost::Store(state.clone()),
        None => TestHost::Silent,
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Who {
    Owner,
    OwnersPhone,
    GuestsPhone,
    SignedOutPhone,
}

fn phone(id: &str, account: Option<(&str, &str)>) -> PairedDevice {
    PairedDevice {
        id: id.into(),
        name: format!("{id} phone"),
        secret: "s".into(),
        created_at: 0,
        kind: None,
        account: account.map(|(id, name)| AccountRef {
            id: id.into(),
            email: None,
            name: Some(name.into()),
            at: None,
        }),
        device_id: None,
        settings: serde_json::Map::new(),
        relay_grant: None,
        superseded_grant: None,
    }
}

async fn read(res: Response) -> (u16, Value) {
    let status = res.status().as_u16();
    let bytes = to_bytes(res.into_body(), 16 << 20).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

/// The door's answer for `who` sending `verb` with `body`, at `url`.
async fn door(url: &str, who: Who, verb: &str, body: Value) -> (u16, Value) {
    let guest = phone("dev-guest", Some(("u-guest", "Guest")));
    let asker = match who {
        Who::Owner => {
            let call = passthrough(
                ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 50000))),
                Path(verb.to_string()),
                HeaderMap::new(),
                Some(Json(body)),
            );
            let res = crate::memory_host::TEST_HOST.scope(host_at(url), call).await;
            return read(res).await;
        }
        Who::OwnersPhone => Asker::Phone(None),
        Who::GuestsPhone => asker_of(&Caller::Phone(guest)),
        Who::SignedOutPhone => asker_of(&Caller::Phone(phone("dev-2", None))),
    };
    let res = crate::memory_host::TEST_HOST
        .scope(host_at(url), answer(&asker, verb, body))
        .await;
    read(res).await
}

fn yesterday() -> String {
    (chrono::Local::now() - chrono::Duration::days(1))
        .format("%Y-%m-%d")
        .to_string()
}

/// Every verb memory serves at `/api/memory/<verb>` (and one it does not),
/// a body for it, and what the owner gets back.
fn verbs() -> Vec<(&'static str, Value, u16)> {
    let day = yesterday();
    vec![
        ("search", json!({"query": "tea"}), 200),
        ("list", json!({}), 200),
        ("count", json!({}), 200),
        ("get", json!({"id": "nope"}), 404),
        ("add", json!({"content": "likes tea"}), 200),
        (
            "add_batch",
            json!({"facts": [{"content": "likes jazz"}]}),
            200,
        ),
        ("update", json!({"id": "nope", "summary": "s"}), 404),
        ("delete", json!({"id": "nope"}), 200),
        ("forget", json!({"type": "tried"}), 200),
        ("session_start", json!({}), 200),
        ("restamp", json!({"from": "device:zz"}), 200),
        ("accounts", json!({}), 200),
        ("days", json!({}), 200),
        ("remember_day", json!({"date": day}), 200),
        ("harvest_day", json!({"date": day}), 200),
        ("sweep", json!({"dry_run": true}), 200),
        ("chains", json!({}), 200),
        ("issues", json!({}), 200),
        ("issue_add", json!({"kind": "chain", "note": "n"}), 200),
        (
            "issue_resolve",
            json!({"id": "nope", "outcome": "resolved"}),
            400,
        ),
        ("stats", json!({}), 200),
        ("dream_lease", json!({"day": day, "holder": "h"}), 200),
        ("role", json!({}), 200),
        ("wat", json!({}), 404),
    ]
}

/// The verbs a phone may never run, and the only verbs another person's
/// phone may run — written out, not read from the door's tables.
const MAC_ONLY: [&str; 2] = ["restamp", "accounts"];
const ONE_PERSON: [&str; 10] = [
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

fn expected(who: Who, verb: &str, owner_status: u16) -> Result<u16, Value> {
    let mac_only = json!({
        "ok": false, "code": "FORBIDDEN",
        "error": format!("{verb} is the Mac's to run, not a phone's"),
    });
    let not_per_person = json!({
        "ok": false, "code": "FORBIDDEN",
        "error": format!("{verb} is not kept per person, so only the Mac's owner may run it"),
    });
    match who {
        Who::Owner => Ok(owner_status),
        _ if MAC_ONLY.contains(&verb) => Err(mac_only),
        Who::OwnersPhone => Ok(owner_status),
        _ if ONE_PERSON.contains(&verb) => Ok(owner_status),
        _ => Err(not_per_person),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn each_caller_runs_exactly_its_verbs() {
    for who in [
        Who::Owner,
        Who::OwnersPhone,
        Who::GuestsPhone,
        Who::SignedOutPhone,
    ] {
        let s = store().await;
        for (verb, body, owner_status) in verbs() {
            let (status, env) = door(&s.base, who, verb, body).await;
            match expected(who, verb, owner_status) {
                Ok(want) => {
                    assert_eq!(status, want, "{who:?} {verb}: {env}");
                    assert_ne!(env["code"], "FORBIDDEN", "{who:?} {verb}: {env}");
                }
                Err(refusal) => {
                    assert_eq!((status, env), (403, refusal), "{who:?} {verb}");
                }
            }
        }
    }
}

/// Another person's phone writes rows marked as that person, whatever
/// the body claims, and sees only that person's rows.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_phone_is_stamped_with_its_person_and_cannot_claim_another() {
    let s = store().await;
    let owners = door(
        &s.base,
        Who::Owner,
        "add",
        json!({"content": "the owner likes folk", "tier": "semantic"}),
    )
    .await
    .1["data"]["fact"]
        .clone();
    let claim = json!({"content": "likes tea", "tier": "semantic", "account_id": "u-other", "account_name": "Other", "account": "u-other", "all_accounts": true});
    let (status, env) = door(&s.base, Who::GuestsPhone, "add", claim).await;
    assert_eq!(status, 200, "{env}");
    assert_eq!(env["data"]["fact"]["account_id"], "u-guest");
    assert_eq!(env["data"]["fact"]["account_name"], "Guest");
    let batch = json!({"facts": [{"content": "likes jazz", "account_id": "u-other"}, {"content": "went to a gig", "all_accounts": true}]});
    let (_, env) = door(&s.base, Who::GuestsPhone, "add_batch", batch).await;
    for f in env["data"]["facts"].as_array().unwrap() {
        assert_eq!(
            (f["account_id"].clone(), f["account_name"].clone()),
            (json!("u-guest"), json!("Guest")),
            "{f}"
        );
    }
    let (_, env) = door(
        &s.base,
        Who::SignedOutPhone,
        "add",
        json!({"content": "likes cocoa"}),
    )
    .await;
    assert_eq!(env["data"]["fact"]["account_id"], "device:dev-2");
    assert_eq!(env["data"]["fact"]["account_name"], "dev-2 phone");
    let (_, env) = door(
        &s.base,
        Who::OwnersPhone,
        "add",
        json!({"content": "the owner likes rye", "account_id": "u-guest"}),
    )
    .await;
    assert!(env["data"]["fact"].get("account_id").is_none(), "{env}");

    // Reads: the guest's own rows only, even when the body asks for everyone's.
    let (_, env) = door(
        &s.base,
        Who::GuestsPhone,
        "count",
        json!({"all_accounts": true, "account": "u-other"}),
    )
    .await;
    assert_eq!(env["data"]["count"], 3);
    let (_, env) = door(&s.base, Who::Owner, "count", json!({})).await;
    assert_eq!(env["data"]["count"], 2, "the owner's rows: folk and rye");
    let (_, env) = door(&s.base, Who::Owner, "accounts", json!({})).await;
    assert_eq!(
        env["data"],
        json!([
            {"account_id": "device:dev-2", "account_name": "dev-2 phone", "rows": 1},
            {"account_id": "u-guest", "account_name": "Guest", "rows": 3},
        ])
    );
    // The owner's row by id, from the guest's phone: exactly a missing row.
    let id = owners["id"].as_str().unwrap();
    let theirs = door(&s.base, Who::GuestsPhone, "get", json!({"id": id})).await;
    assert_eq!(
        theirs,
        (
            404,
            json!({"ok": false, "error": format!("no fact with id {id}"), "code": "NOT_FOUND"})
        )
    );
    let gone = door(&s.base, Who::GuestsPhone, "delete", json!({"id": id})).await;
    assert_eq!(
        gone,
        (
            200,
            json!({"ok": true, "data": {"id": id, "removed": false}})
        )
    );

    // A body the stamp cannot mark is refused for another person.
    let (status, env) = door(
        &s.base,
        Who::GuestsPhone,
        "add_batch",
        json!({"facts": ["likes tea"]}),
    )
    .await;
    assert_eq!(
        (status, env),
        (
            400,
            json!({"ok": false, "code": "BAD_REQUEST", "error": "add_batch takes a JSON object (and add_batch a list of them in facts)"})
        )
    );
}

/// Memory's answer goes back as it came — status, code and message — and
/// a non-JSON answer is given the envelope shape with its status as code.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn memorys_statuses_pass_through_unchanged() {
    let s = store().await;
    let owner = |verb: &'static str, body: Value| door(&s.base, Who::Owner, verb, body);
    let (status, env) = owner("add", json!({"content": "likes tea", "tier": "semantic"})).await;
    assert_eq!(status, 200);
    assert_eq!(env["ok"], true);
    assert_eq!(env["data"]["action"], "added");
    assert_eq!(
        owner("get", json!({"id": "nope"})).await,
        (
            404,
            json!({"ok": false, "error": "no fact with id nope", "code": "NOT_FOUND"})
        )
    );
    assert_eq!(
        owner("add", json!({"content": " "})).await,
        (
            400,
            json!({"ok": false, "error": "content must not be empty", "code": "BAD_REQUEST"})
        )
    );
    let day = yesterday();
    owner("dream_lease", json!({"day": day, "holder": "a"})).await;
    let (status, env) = owner("dream_lease", json!({"day": day, "holder": "b"})).await;
    assert_eq!((status, env["code"].clone()), (409, json!("CONFLICT")));
    assert!(env["error"]
        .as_str()
        .unwrap()
        .starts_with(&format!("day {day} is being dreamed by a (until ")));
    assert_eq!(
        owner("wat", json!({})).await,
        (
            404,
            json!({"ok": false, "error": "ling-mem error [404]: <empty body>", "code": "HTTP_404"})
        )
    );
    assert_eq!(
        owner("add", json!({"contents": "x"})).await,
        (
            422,
            json!({"ok": false, "error": "ling-mem error [422]: Failed to deserialize the JSON body into the target type: missing field `content` at line 1 column 16", "code": "HTTP_422"})
        )
    );
    // A verb that could leave the namespace never reaches memory.
    assert_eq!(
        owner("a.b", json!({})).await,
        (
            400,
            json!({"ok": false, "error": "invalid memory verb: a.b", "code": "BAD_REQUEST"})
        )
    );
}

/// Memory unreachable: 502 `UNREACHABLE`, in the same envelope shape.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unreachable_memory_is_a_502() {
    assert_eq!(
        door("http://127.0.0.1:1", Who::Owner, "list", json!({})).await,
        (
            502,
            json!({"ok": false, "error": "memory is not reachable — check `ling status`", "code": "UNREACHABLE"})
        )
    );
    let (status, env) = door(
        "http://127.0.0.1:1",
        Who::GuestsPhone,
        "search",
        json!({"query": "x"}),
    )
    .await;
    assert_eq!((status, env["code"].clone()), (502, json!("UNREACHABLE")));
    // No memory hosted in this process (a unit test, a CLI): the empty URL.
    let (status, env) = door("", Who::Owner, "list", json!({})).await;
    assert_eq!(
        (status, env["code"].clone()),
        (502, json!("UNREACHABLE")),
        "{env}"
    );
    assert_eq!(
        env["error"], "memory is not hosted by this process",
        "{env}"
    );
}

/// The stamp scopes every verb another person's phone may run, not only
/// the writes: `forget`, `update`, `search`, `list` and `session_start`
/// never reach the owner's rows.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_person_verb_stays_inside_the_persons_rows() {
    let s = store().await;
    let owner = |verb: &'static str, body: Value| door(&s.base, Who::Owner, verb, body);
    let guest = |verb: &'static str, body: Value| door(&s.base, Who::GuestsPhone, verb, body);
    let oat = owner(
        "add",
        json!({"content": "tried oat milk", "type": "tried", "tier": "semantic"}),
    )
    .await
    .1["data"]["fact"]
        .clone();
    owner(
        "add",
        json!({"content": "Alex lives in Lisbon", "tier": "core"}),
    )
    .await;
    guest(
        "add",
        json!({"content": "tried soy milk", "type": "tried", "tier": "semantic"}),
    )
    .await;
    guest(
        "add",
        json!({"content": "the guest lives in Porto", "tier": "core"}),
    )
    .await;

    let id = oat["id"].as_str().unwrap();
    assert_eq!(
        guest("update", json!({"id": id, "content": "tried rice milk"})).await,
        (
            404,
            json!({"ok": false, "error": format!("no fact with id {id}"), "code": "NOT_FOUND"})
        )
    );
    let (_, env) = guest("search", json!({"query": "tried oat milk"})).await;
    let hits: Vec<Value> = env["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["content"].clone())
        .collect();
    assert_eq!(
        hits,
        vec![json!("tried soy milk")],
        "the owner's exact row is not found"
    );
    let (_, env) = guest("list", json!({"sort": "oldest"})).await;
    let seen: Vec<Value> = env["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["content"].clone())
        .collect();
    assert_eq!(
        seen,
        vec![json!("tried soy milk"), json!("the guest lives in Porto")]
    );
    let (_, env) = guest("session_start", json!({})).await;
    let core: Vec<Value> = env["data"]["core"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["content"].clone())
        .collect();
    assert_eq!(core, vec![json!("the guest lives in Porto")]);

    assert_eq!(
        guest("forget", json!({"type": "tried"})).await,
        (200, json!({"ok": true, "data": {"removed": 1}}))
    );
    assert_eq!(
        owner("get", json!({"id": id})).await.1["data"]["content"],
        "tried oat milk"
    );
    let (_, env) = owner("count", json!({"type": "tried"})).await;
    assert_eq!(env["data"]["count"], 1);
}

/// The door's answer for `asker` with memory as `host` says.
async fn door_at(
    host: crate::memory_host::TestHost,
    asker: Asker,
    verb: &str,
    body: Value,
) -> (u16, Value) {
    let res = crate::memory_host::TEST_HOST
        .scope(host, answer(&asker, verb, body))
        .await;
    read(res).await
}

/// Memory that failed to open answers as its routes did: 503
/// `MEMORY_UNAVAILABLE` with the reason, for anyone the door lets through;
/// one that never answers is given up on after the door's own wait.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn memory_that_is_not_open_or_silent_answers_as_before() {
    use crate::memory_host::TestHost;
    let why = "the memory store under /x is already open (pid 7); one process at a time";
    let guest = asker_of(&Caller::Phone(phone("dev-guest", Some(("u-guest", "Guest")))));
    for asker in [Asker::Owner, Asker::Phone(None), guest] {
        assert_eq!(
            door_at(TestHost::NotOpen(why.into()), asker, "search", json!({"query": "x"})).await,
            (
                503,
                json!({"ok": false, "error": why, "code": "MEMORY_UNAVAILABLE"})
            )
        );
    }
    let started = std::time::Instant::now();
    let silent = door_at(TestHost::Silent, Asker::Owner, "count", json!({})).await;
    let waited = started.elapsed();
    assert_eq!(
        silent,
        (
            502,
            json!({"ok": false, "error": "memory is not reachable — check `ling status`", "code": "UNREACHABLE"})
        )
    );
    assert!(
        waited >= VERB_TIMEOUT && waited < VERB_TIMEOUT + std::time::Duration::from_secs(2),
        "{waited:?}"
    );
}

/// The four askers through the in-process call: a caller memory does not
/// know is refused before memory; `apply_schema` is not a route at
/// `/api/memory/` (memory's 404 for the owner, the person refusal for a
/// guest); the owner's own phone loses its account claims.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_asker_is_answered_through_the_call() {
    let s = store().await;
    let host = || host_at(&s.base);
    assert_eq!(
        door_at(host(), Asker::Unknown, "search", json!({"query": "x"})).await,
        (
            403,
            json!({"ok": false, "error": "memory does not know this caller", "code": "FORBIDDEN"})
        )
    );
    assert_eq!(
        door_at(host(), Asker::Owner, "apply_schema", json!({})).await,
        (
            404,
            json!({"ok": false, "error": "ling-mem error [404]: <empty body>", "code": "HTTP_404"})
        )
    );
    let guest = asker_of(&Caller::Phone(phone("dev-guest", Some(("u-guest", "Guest")))));
    assert_eq!(
        door_at(host(), guest.clone(), "apply_schema", json!({})).await,
        (
            403,
            json!({"ok": false, "code": "FORBIDDEN", "error": "apply_schema is not kept per person, so only the Mac's owner may run it"})
        )
    );
    let (_, env) = door_at(
        host(),
        Asker::Phone(None),
        "add",
        json!({"content": "the owner likes rye", "tier": "semantic", "account_id": "u-guest", "account_name": "Guest"}),
    )
    .await;
    assert!(env["data"]["fact"].get("account_id").is_none(), "{env}");
    let (_, env) = door_at(host(), guest, "list", json!({"all_accounts": true})).await;
    assert_eq!(env["data"], json!([]), "the owner's row is not the guest's");
}
