//! The door `POST /api/memory/<verb>` against a real hosted store (scratch
//! dir, memory's deterministic test embedder, served on a loopback port),
//! as three callers: the owner (this machine), the owner's own phone, and
//! the phone of another person. What each may run is pinned verb by verb
//! as literals, so a later table-driven door must reproduce it exactly.
//!
//! How the callers are made, without touching this machine's pairing
//! records or account file:
//! - the owner: the real `passthrough` handler from loopback;
//! - another person's phone: `scope_to_caller` with a phone row signed in
//!   to another account, then `passthrough` with the body it produced (the
//!   owner's scope leaves a body as it is, so the wire is the phone's);
//! - the owner's own phone: `scope_phone` with no person — which phone is
//!   the owner's is decided from `account.toml` (`person_of`, unit-tested
//!   above), and a test must not write that file.
//!
//! `memory_host::url` is pointed at the scratch store for each test's
//! future alone (`memory_host::TEST_URL`).

use super::*;
use crate::server::api::pair::{AccountRef, PairedDevice};
use axum::body::to_bytes;
use std::net::SocketAddr;

struct Store {
    base: String,
    _dir: tempfile::TempDir,
}

async fn store() -> Store {
    let dir = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let state = ling_mem::hosted::open_quiet_fake(dir.path(), base.clone())
        .await
        .unwrap();
    let app = ling_mem::hosted::router(state);
    tokio::spawn(async move {
        let _ = axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await;
    });
    Store { base, _dir: dir }
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
    let mut args = body;
    let refused = match who {
        Who::Owner => None,
        Who::OwnersPhone => scope_phone(verb, &mut args, None)
            .err()
            .map(|(status, message, code)| unreachable_envelope(status, message, code)),
        Who::GuestsPhone => {
            let guest = phone("dev-guest", Some(("u-guest", "Guest")));
            scope_to_caller(&Caller::Phone(guest), verb, &mut args).err()
        }
        Who::SignedOutPhone => {
            scope_to_caller(&Caller::Phone(phone("dev-2", None)), verb, &mut args).err()
        }
    };
    if let Some(refusal) = refused {
        return read(refusal).await;
    }
    let call = passthrough(
        ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 50000))),
        Path(verb.to_string()),
        HeaderMap::new(),
        Some(Json(args)),
    );
    let res = crate::memory_host::TEST_URL
        .scope(url.to_string(), call)
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
        env["error"], "POST /api/memory/list failed: builder error: relative URL without a base",
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
